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
module "light_bvh/sample.slang";

namespace light_bvh {
static const let LIGHT_BVH_LEAF = 0x80000000u;
static const let LIGHT_BVH_SELECTORS = 16777216u;
static const let LIGHT_BVH_INVERSE_SELECTORS = 5.9604644775390625e-8;

/// 一次离散选灯的结果；不包含灯内面积或立体角 PDF。
public struct LightBvhSample {
    /// 原始输入灯索引；空树为 0xffffffff。
    public var light : uint;
    /// 实际量化分布中该灯的选择概率；空树为零，单叶为一。
    public var pmf : float;
};

struct LightBvhNode {
    var center : float3;
    var power : float;
    var child : uint;
    var leaves : uint;
};

func light_bvh_pair(words: StructuredBuffer<uint4>, base: uint, packets: uint, at: uint,
                    leaves: uint, out first: LightBvhNode, out second: LightBvhNode)
    ->void {
    let block : uint = at >> 2u;
    let within = at & 3u;
    let block_base = base + 10u * block;
    let block_count = min(4u, packets - 4u * block);
    let a : float4 = asfloat(words[block_base + 2u * within]);
    let b : float4 = asfloat(words[block_base + 2u * within + 1u]);
    let topology = words[block_base + 2u * block_count + (within >> 1u)];
    let children : uint2 = (within & 1u) == 0 ? topology.xy : topology.zw;
    first.center = a.xyz;
    first.power = a.w;
    first.child = children.x;
    first.leaves = leaves >> 1u;
    second.center = b.xyz;
    second.power = b.w;
    second.child = children.y;
    second.leaves = leaves - first.leaves;
}

func light_bvh_metrics(words: StructuredBuffer<uint4>, leaf_base: uint, node: LightBvhNode,
                       point_value: float3)
    ->float2 {
    let delta = node.center - point_value;
    var distance_squared : float = dot(delta, delta);
    if ((node.child & LIGHT_BVH_LEAF) != 0) {
        let at : uint = node.child & ~LIGHT_BVH_LEAF;
        distance_squared += 0.25 * asfloat(words[leaf_base + 2u * at].w);
    }
    return float2(distance_squared, node.power);
}

func light_bvh_left_count(words: StructuredBuffer<uint4>, leaf_base: uint, first: LightBvhNode,
                          second: LightBvhNode, point_value: float3, count: uint)
    ->uint {
    let a = light_bvh_metrics(words, leaf_base, first, point_value);
    let b = light_bvh_metrics(words, leaf_base, second, point_value);
    let power = float2(a.y, b.y) / max(a.y, b.y);
    let scale = max(a.x, b.x);
    var probability : float;
    if (scale == 0)
        probability = power.x / (power.x + power.y);
    else {
        let score = power * (float2(b.x, a.x) / scale);
        let total = score.x + score.y;
        probability = total > 0 ? score.x / total : -1.0;
    }
    probability = clamp(probability, 1.0e-6, 1.0 - 1.0e-6);
    precise let scaled = float(count) * probability;
    let integral = uint(scaled);
    let fraction = scaled - float(integral);
    let left : uint = integral + uint(fraction > 0.5 || (fraction == 0.5 && (integral & 1u) != 0));
    return clamp(left, first.leaves, count - second.leaves);
}

/// 用 [0,1) 中的一个均匀数选择灯，返回真实格点 PMF；不消费额外随机数。
/// words/base/light_count 须来自同一有效 CPU 树，point 有限；树与输入不在 shader 内重验。
/// 分支格点数为 round-to-nearest-even(count*p)，再按后代叶数夹紧；空树返回无灯。
/// 概率依赖 point；用于 MIS 时须以同一接收点调用 light_bvh_pmf，不能替换成 1/light_count。
public func sample_light_bvh(words: StructuredBuffer<uint4>, base: uint, light_count: uint,
                             point_value: float3, sample_value: float)
    ->LightBvhSample {
    var result : LightBvhSample;
    result.light = 0xffffffffu;
    result.pmf = 0;
    if (light_count == 0)
        return result;
    if (light_count == 1) {
        result.light = 0;
        result.pmf = 1;
        return result;
    }
    let packets = light_count - 1u;
    let leaf_base : uint = base + (((10u * packets + 7u) & ~7u) >> 2u);
    let target = uint(sample_value * float(LIGHT_BVH_SELECTORS));
    var first : uint = 0;
    var count : uint = LIGHT_BVH_SELECTORS;
    var at : uint = 0;
    var leaves : uint = light_count;
    while (true) {
        var a : LightBvhNode;
        var b : LightBvhNode;
        light_bvh_pair(words, base, packets, at, leaves, a, b);
        let left = light_bvh_left_count(words, leaf_base, a, b, point_value, count);
        var child : uint;
        if (target < first + left) {
            count = left;
            child = a.child;
            leaves = a.leaves;
        } else {
            first += left;
            count -= left;
            child = b.child;
            leaves = b.leaves;
        }
        if ((child & LIGHT_BVH_LEAF) != 0) {
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
public func light_bvh_pmf(words: StructuredBuffer<uint4>, base: uint, light_count: uint,
                          point_value: float3, light: uint)
    ->float {
    if (light >= light_count)
        return 0;
    if (light_count == 1)
        return 1;
    let packets = light_count - 1u;
    let leaf_base : uint = base + (((10u * packets + 7u) & ~7u) >> 2u);
    let path : uint = words[leaf_base + 2u * light + 1u].w;
    var count : uint = LIGHT_BVH_SELECTORS;
    var at : uint = 0;
    var leaves : uint = light_count;
    for (var depth: uint = 0; depth < (path >> 27u); ++depth) {
        var a : LightBvhNode;
        var b : LightBvhNode;
        light_bvh_pair(words, base, packets, at, leaves, a, b);
        let left = light_bvh_left_count(words, leaf_base, a, b, point_value, count);
        let second : bool = ((path >> depth) & 1u) != 0;
        count = second ? count - left : left;
        at = second ? b.child : a.child;
        leaves = second ? b.leaves : a.leaves;
    }
    return float(count) * LIGHT_BVH_INVERSE_SELECTORS;
}

/// 与同一发布事务的树和原始灯顺序绑定；不拥有GPU缓冲。
public struct LightBvhProposal {
    public var words : StructuredBuffer<uint4>;
    public var base : uint;
    public var count : uint;
    public func sample(receiver: float3, selector: float)->LightBvhSample {
        return sample_light_bvh(words, base, count, receiver, selector);
    }
    public func pmf(receiver: float3, light: uint)->float {
        return light_bvh_pmf(words, base, count, receiver, light);
    }
};
}
