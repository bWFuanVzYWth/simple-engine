/// 与接收点相关的 quad 光源离散采样；不决定辐射功率、法线、遮挡、面积采样或积分策略。
/// 输入由 light_bvh CPU 构建器校验并拥有；所有位置须在同一坐标系，运算处于 float32 范围。
/// uint4 buffer 按 sibling pair 保存树；base 以 uint4 为单位。L 个灯有 L-1 对，根为 pair 0。
/// 每至多四对一块：先存每对左右 center.xyz/power（32B），再存每对 child0/child1（8B）。
/// 完整块160B；末块按实际对数紧存。叶目录起点 base+roundup(10*(L-1),8)/4，
/// 每灯两项：lower.xyz/edge²、upper.xyz/path。编码见 light_bvh::Tree::words。
/// child 高位标记叶且低位为原始灯索引，否则为 pair 索引；L<=1 时不读取树数据。
/// 查询携带子树叶数 n，由固定 median 二分推导 n/2 与 n-n/2，不新增目录或身份映射。
/// 任意朝向、非平面 quad 均合法；不能将该数量推导用于其他树拓扑。
/// path 低 27 位按根到叶保存左右选择，高 5 位为深度；树最多 2^24 叶，深度最多 24。
/// 以功率/距离平方选择分支，在 24-bit 随机格点上量化，且为每个后代保留至少一个格点。
/// 叶子分母为质心距离²+(|v1-v0|²+|v3-v0|²)/4，平滑近灯与盒内查询；内部节点不平滑。
module light_bvh;

namespace pt
{
static const uint LIGHT_BVH_LEAF = 0x80000000u;
static const uint LIGHT_BVH_SELECTORS = 16777216u;
static const float LIGHT_BVH_INVERSE_SELECTORS = 5.9604644775390625e-8;

/// 一次离散选灯的结果；不包含灯内面积或立体角 PDF。
public struct LightBvhSample
{
    /// 原始输入灯索引；空树为 0xffffffff。
    public uint light;
    /// 实际量化分布中该灯的选择概率；空树为零，单叶为一。
    public float pmf;
};

struct LightBvhNode
{
    float3 center;
    float power;
    uint child;
    uint leaves;
};

void light_bvh_pair(StructuredBuffer<uint4> words, uint base, uint packets, uint at, uint leaves,
                    out LightBvhNode first, out LightBvhNode second)
{
    uint block = at >> 2u;
    uint within = at & 3u;
    uint block_base = base + 10u * block;
    uint block_count = min(4u, packets - 4u * block);
    float4 a = asfloat(words[block_base + 2u * within]);
    float4 b = asfloat(words[block_base + 2u * within + 1u]);
    uint4 topology = words[block_base + 2u * block_count + (within >> 1u)];
    uint2 children = (within & 1u) == 0 ? topology.xy : topology.zw;
    first.center = a.xyz;
    first.power = a.w;
    first.child = children.x;
    first.leaves = leaves >> 1u;
    second.center = b.xyz;
    second.power = b.w;
    second.child = children.y;
    second.leaves = leaves - first.leaves;
}

float2 light_bvh_metrics(StructuredBuffer<uint4> words, uint leaf_base, LightBvhNode node,
                         float3 point)
{
    float3 delta = node.center - point;
    float distance_squared = dot(delta, delta);
    if ((node.child & LIGHT_BVH_LEAF) != 0)
    {
        uint at = node.child & ~LIGHT_BVH_LEAF;
        distance_squared += 0.25 * asfloat(words[leaf_base + 2u * at].w);
    }
    return float2(distance_squared, node.power);
}

uint light_bvh_left_count(StructuredBuffer<uint4> words, uint leaf_base, LightBvhNode first,
                          LightBvhNode second, float3 point, uint count)
{
    float2 a = light_bvh_metrics(words, leaf_base, first, point);
    float2 b = light_bvh_metrics(words, leaf_base, second, point);
    float2 power = float2(a.y, b.y) / max(a.y, b.y);
    float scale = max(a.x, b.x);
    float probability;
    if (scale == 0)
        probability = power.x / (power.x + power.y);
    else
    {
        float2 score = power * (float2(b.x, a.x) / scale);
        float total = score.x + score.y;
        probability = total > 0 ? score.x / total : -1.0;
    }
    probability = clamp(probability, 1.0e-6, 1.0 - 1.0e-6);
    precise float scaled = float(count) * probability;
    uint integral = uint(scaled);
    float fraction = scaled - float(integral);
    uint left = integral + uint(fraction > 0.5 || (fraction == 0.5 && (integral & 1u) != 0));
    return clamp(left, first.leaves, count - second.leaves);
}

/// 用 [0,1) 中的一个均匀数选择灯，返回真实格点 PMF；不消费额外随机数。
/// words/base/light_count 须来自同一有效 CPU 树，point 有限；树与输入不在 shader 内重验。
/// 分支格点数为 round-to-nearest-even(count*p)，再按后代叶数夹紧；空树返回无灯。
/// 概率依赖 point；用于 MIS 时须以同一接收点调用 light_bvh_pmf，不能替换成 1/light_count。
public LightBvhSample sample_light_bvh(StructuredBuffer<uint4> words, uint base, uint light_count,
                                       float3 point, float sample)
{
    LightBvhSample result;
    result.light = 0xffffffffu;
    result.pmf = 0;
    if (light_count == 0)
        return result;
    if (light_count == 1)
    {
        result.light = 0;
        result.pmf = 1;
        return result;
    }
    uint packets = light_count - 1u;
    uint leaf_base = base + (((10u * packets + 7u) & ~7u) >> 2u);
    uint target = uint(sample * float(LIGHT_BVH_SELECTORS));
    uint first = 0, count = LIGHT_BVH_SELECTORS;
    uint at = 0, leaves = light_count;
    while (true)
    {
        LightBvhNode a, b;
        light_bvh_pair(words, base, packets, at, leaves, a, b);
        uint left = light_bvh_left_count(words, leaf_base, a, b, point, count);
        uint child;
        if (target < first + left)
        {
            count = left;
            child = a.child;
            leaves = a.leaves;
        }
        else
        {
            first += left;
            count -= left;
            child = b.child;
            leaves = b.leaves;
        }
        if ((child & LIGHT_BVH_LEAF) != 0)
        {
            result.light = child & ~LIGHT_BVH_LEAF;
            break;
        }
        at = child;
    }
    result.pmf = float(count) * LIGHT_BVH_INVERSE_SELECTORS;
    return result;
}

/// 在相同接收点重算指定灯的真实选择 PMF；空树及不在输入目录中的索引返回零。
/// 其余输入契约同 sample_light_bvh。只重放该灯的根到叶路径，不扫描其他叶或重用固定 PMF。
/// 返回离散概率；转换为面积或立体角 PDF 的几何项由调用方组合。
public float light_bvh_pmf(StructuredBuffer<uint4> words, uint base, uint light_count, float3 point,
                           uint light)
{
    if (light >= light_count)
        return 0;
    if (light_count == 1)
        return 1;
    uint packets = light_count - 1u;
    uint leaf_base = base + (((10u * packets + 7u) & ~7u) >> 2u);
    uint path = words[leaf_base + 2u * light + 1u].w;
    uint count = LIGHT_BVH_SELECTORS;
    uint at = 0, leaves = light_count;
    for (uint depth = 0; depth < (path >> 27u); ++depth)
    {
        LightBvhNode a, b;
        light_bvh_pair(words, base, packets, at, leaves, a, b);
        uint left = light_bvh_left_count(words, leaf_base, a, b, point, count);
        bool second = ((path >> depth) & 1u) != 0;
        count = second ? count - left : left;
        at = second ? b.child : a.child;
        leaves = second ? b.leaves : a.leaves;
    }
    return float(count) * LIGHT_BVH_INVERSE_SELECTORS;
}
}
