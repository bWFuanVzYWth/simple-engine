/// 离散 alias 选择与任意 quad 的面积分布；CPU 协议由 light_sampling crate 建立。
/// 本模块不决定光源功率、纹理、材质、覆盖规则、空间位置或发光侧；输入由应用保活。
module light_area;

import geometry.ray;
import pt.transport;

namespace pt
{
public static const uint LIGHT_SELECTORS = 16777216u;
public static const uint LIGHT_FRONT = 1u;
public static const uint LIGHT_BACK = 2u;
public static const uint LIGHT_BOTH = 3u;

public struct LightAliasSample
{
    public uint index;
    public float pmf;
};

/// 无表活动registry选择；count<=2^24，selector为均匀24-bit格点。
/// floor/ceil的真实质量与反向查询一致，不构建alias，也不假称任意count精确1/count。
public LightAliasSample uniform_light_sample(uint count, float selector)
{
    LightAliasSample result = {};
    if (count == 0)
        return result;
    uint target = min(uint(selector * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
    uint base = LIGHT_SELECTORS / count;
    uint extra = LIGHT_SELECTORS % count;
    uint boundary = extra * (base + 1u);
    result.index = target < boundary ? target / (base + 1u) : extra + (target - boundary) / base;
    uint mass = base + uint(result.index < extra);
    result.pmf = float(mass) / float(LIGHT_SELECTORS);
    return result;
}
public float uniform_light_pmf(uint count, uint index)
{
    if (index >= count)
        return 0;
    return float(LIGHT_SELECTORS / count + uint(index < LIGHT_SELECTORS % count)) /
           float(LIGHT_SELECTORS);
}

/// entries 是 CPU AliasTable::entries，count/shift 来自同一个不可变表。
/// selector 须在 [0,1)，且量化后的 24-bit 整数均匀；空表返回 pmf=0。
/// 浮点权重只用于构建，采样和反向查询都读取编码的实际整数质量。
public LightAliasSample sample_light_alias(StructuredBuffer<uint4> entries, uint count, uint shift,
                                           float selector, uint base = 0u)
{
    LightAliasSample result = {};
    if (count == 0)
        return result;
    uint target = min(uint(selector * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
    uint column = target >> shift;
    uint local = target & ((1u << shift) - 1u);
    uint4 entry = entries[base + column];
    result.index = local < entry.x ? column : entry.y;
    result.pmf = float(entries[base + result.index].z) / float(LIGHT_SELECTORS);
    return result;
}

/// 越界（含空表）返回零；index 为原始权重索引，不是 alias 的填充列。
public float light_alias_pmf(StructuredBuffer<uint4> entries, uint count, uint index,
                             uint base = 0u)
{
    if (index >= count)
        return 0;
    return float(entries[base + index].z) / float(LIGHT_SELECTORS);
}

public struct LightQuadVertices
{
    public float3 a;
    public float3 b;
    public float3 c;
    public float3 d;
};

/// 解码 CPU pack_vertices 的三个 float4；保留实际第四角，不推导 d=a+c-b。
public LightQuadVertices light_quad_vertices(float4 p, float4 q, float4 r)
{
    return { p.xyz, float3(p.w, q.xy), float3(q.zw, r.x), r.yzw };
}

/// 过程几何的无表面积fallback；不建立alias。静态sourcequad应复用资源期QuadArea。
/// 无有效f32面积时返回零密度，调用方不得再重建退化片endpoint。
public uint4 dynamic_quad_area(LightQuadVertices vertices)
{
    float first = length(cross(vertices.b - vertices.a, vertices.c - vertices.a)) * 0.5;
    float second = length(cross(vertices.c - vertices.a, vertices.d - vertices.a)) * 0.5;
    float total = first + second;
    if (!(total > 0) || !isfinite(total))
        return 0;
    uint split = uint(round(first / total * float(LIGHT_SELECTORS)));
    if (first > 0 && second > 0)
        split = clamp(split, 1u, LIGHT_SELECTORS - 1u);
    float density0 = split == 0u ? 0.0 : (float(split) / float(LIGHT_SELECTORS)) / first;
    float density1 = split == LIGHT_SELECTORS
                         ? 0.0
                         : (float(LIGHT_SELECTORS - split) / float(LIGHT_SELECTORS)) / second;
    return uint4(split, asuint(density0), asuint(density1), 0);
}

public struct QuadAreaSample
{
    public uint half;
    public float2 bary;
    public float density;
};

/// area 是同一几何的 QuadArea::words，sample 在 [0,1)^2。
/// 片选择使用真实整数 split；退化半片质量为零，不会被选择或归一化其法线。
/// 条件区间取选择器格点的中点并保持开上端，返回 bary 对应该片的顶点 1/2。
public QuadAreaSample sample_quad_area(uint4 area, float2 sample)
{
    uint target = min(uint(sample.x * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
    bool second = target >= area.x;
    uint start = second ? area.x : 0u;
    uint mass = second ? LIGHT_SELECTORS - area.x : area.x;
    float u = min((float(target - start) + 0.5) / float(mass), 0.9999999403953552);
    float root = sqrt(u);
    QuadAreaSample result;
    result.half = uint(second);
    result.bary = float2(root * (1.0 - sample.y), root * sample.y);
    result.density = asfloat(second ? area.z : area.y);
    return result;
}

/// 片半编号来自实际硬件 primitive；包含片选择概率，不含离散灯选择概率。
public float quad_area_density(uint4 area, uint half)
{
    return half < 2u ? asfloat(half == 0u ? area.y : area.z) : 0.0;
}

/// 用实际两三角和匹配 AS 的变换重建目标端误差；数值域同 geometry::triangle_endpoint。
public geometry::TriangleEndpoint quad_endpoint(LightQuadVertices vertices, QuadAreaSample sample,
                                                float3x4 object_to_world, float3x4 world_to_object)
{
    return geometry::triangle_endpoint(vertices.a, sample.half == 0u ? vertices.b : vertices.c,
                                       sample.half == 0u ? vertices.c : vertices.d, sample.bary,
                                       object_to_world, world_to_object);
}

/// 输入已处于追踪坐标系时的重载；不代替有实例变换时的误差预算。
public geometry::TriangleEndpoint quad_endpoint(LightQuadVertices vertices, QuadAreaSample sample)
{
    return geometry::triangle_endpoint(vertices.a, sample.half == 0u ? vertices.b : vertices.c,
                                       sample.half == 0u ? vertices.c : vertices.d, sample.bary);
}

/// 将匹配的离散灯 PMF 与片面积密度换成实立体角 PDF。
/// normal 须是该片的单位绕序法线，sides 是应用声明的 LIGHT_FRONT/BACK/BOTH。
/// 双面只将允许侧扩为两侧，不再乘2或把灯复制为重合表面。
/// 有效坐标差平方、密度积和最终 PDF 须在有限 f32 域；无支持、背侧或掠射返回0。
public float quad_light_pdf(float3 receiver, float3 target, float3 normal, float area_density,
                            float selection_pmf, uint sides)
{
    float3 delta = receiver - target;
    float distance_squared = dot(delta, delta);
    if (area_density <= 0 || selection_pmf <= 0 || distance_squared <= 0 ||
        !isfinite(distance_squared))
        return 0;
    float cosine = dot(normal, delta * rsqrt(distance_squared));
    uint side = cosine >= 0 ? LIGHT_FRONT : LIGHT_BACK;
    if ((sides & side) == 0 || abs(cosine) <= 0)
        return 0;
    float pdf = area_solid_angle_pdf(selection_pmf * area_density, distance_squared, abs(cosine));
    return isfinite(pdf) ? pdf : 0;
}
}
