/// 离散 alias 选择与任意 quad 的面积分布；CPU 协议由 light_sampling crate 建立。
/// 本模块不决定光源功率、纹理、材质、覆盖规则、空间位置或发光侧；输入由应用保活。
module area;

import geometry.ray;
import transport.weights;

namespace light_sampling {
public static const let LIGHT_SELECTORS = 16777216u;
public static const let LIGHT_INVERSE_SELECTORS = 1.0 / 16777216.0;
public static const let LIGHT_FRONT = 1u;
public static const let LIGHT_BACK = 2u;
public static const let LIGHT_BOTH = 3u;

public struct LightAliasSample {
    public var index : uint;
    public var pmf : float;
};

/// 无表活动registry选择；count<=2^24，selector为均匀24-bit格点。
/// floor/ceil的真实质量与反向查询一致，不构建alias，也不假称任意count精确1/count。
public func uniform_light_sample(count: uint, selector: float)->LightAliasSample {
    var result : LightAliasSample = {};
    if (count == 0)
        return result;
    let target = min(uint(selector * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
    let base = LIGHT_SELECTORS / count;
    let extra = LIGHT_SELECTORS % count;
    let boundary = extra * (base + 1u);
    result.index = target < boundary ? target / (base + 1u) : extra + (target - boundary) / base;
    let mass = base + uint(result.index < extra);
    result.pmf = float(mass) * LIGHT_INVERSE_SELECTORS;
    return result;
}
public func uniform_light_pmf(count: uint, index: uint)->float {
    if (index >= count)
        return 0;
    return float(LIGHT_SELECTORS / count + uint(index < LIGHT_SELECTORS % count)) *
           LIGHT_INVERSE_SELECTORS;
}

/// entries 是 CPU AliasTable::entries，count/shift 来自同一个不可变表。
/// selector 须在 [0,1)，且量化后的 24-bit 整数均匀；空表返回 pmf=0。
/// 浮点权重只用于构建，采样和反向查询都读取编码的实际整数质量。
public func sample_light_alias(entries: StructuredBuffer<uint4>, count: uint, shift: uint,
                               selector: float, base: uint = 0u)
    ->LightAliasSample {
    var result : LightAliasSample = {};
    if (count == 0)
        return result;
    let target = min(uint(selector * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
    let column : uint = target >> shift;
    let local : uint = target & ((1u << shift) - 1u);
    let entry = entries[base + column];
    result.index = local < entry.x ? column : entry.y;
    result.pmf = float(entries[base + result.index].z) * LIGHT_INVERSE_SELECTORS;
    return result;
}

/// 越界（含空表）返回零；index 为原始权重索引，不是 alias 的填充列。
public func light_alias_pmf(entries: StructuredBuffer<uint4>, count: uint, index: uint,
                            base: uint = 0u)
    ->float {
    if (index >= count)
        return 0;
    return float(entries[base + index].z) * LIGHT_INVERSE_SELECTORS;
}

public struct LightQuadVertices {
    public var a : float3;
    public var b : float3;
    public var c : float3;
    public var d : float3;
};

/// 解码 CPU pack_vertices 的三个 float4；保留实际第四角，不推导 d=a+c-b。
public func light_quad_vertices(p: float4, q: float4, r: float4)->LightQuadVertices {
    return { p.xyz, float3(p.w, q.xy), float3(q.zw, r.x), r.yzw };
}

/// 过程几何的无表面积fallback；不建立alias。静态sourcequad应复用资源期QuadArea。
/// 无有效f32面积时返回零密度，调用方不得再重建退化片endpoint。
public func dynamic_quad_area(vertices_value: LightQuadVertices)->uint4 {
    let first =
        length(cross(vertices_value.b - vertices_value.a, vertices_value.c - vertices_value.a)) *
        0.5;
    let second =
        length(cross(vertices_value.c - vertices_value.a, vertices_value.d - vertices_value.a)) *
        0.5;
    let total = first + second;
    if (!(total > 0) || !isfinite(total))
        return 0;
    var split = uint(round(first / total * float(LIGHT_SELECTORS)));
    if (first > 0 && second > 0)
        split = clamp(split, 1u, LIGHT_SELECTORS - 1u);
    let density0 : float = split == 0u ? 0.0 : (float(split) * LIGHT_INVERSE_SELECTORS) / first;
    let density1 : float =
                       split == LIGHT_SELECTORS
                           ? 0.0
                           : (float(LIGHT_SELECTORS - split) * LIGHT_INVERSE_SELECTORS) / second;
    return uint4(split, asuint(density0), asuint(density1), 0);
}

public struct QuadAreaSample {
    public var half : uint;
    public var bary : float2;
    public var density : float;
};

/// area 是同一几何的 QuadArea::words，sample 在 [0,1)^2。
/// 片选择使用真实整数 split；退化半片质量为零，不会被选择或归一化其法线。
/// 条件区间取选择器格点的中点并保持开上端，返回 bary 对应该片的顶点 1/2。
public func sample_quad_area(area: uint4, sample_value: float2)->QuadAreaSample {
    let target = min(uint(sample_value.x * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
    let second = target >= area.x;
    let start = second ? area.x : 0u;
    let mass = second ? LIGHT_SELECTORS - area.x : area.x;
    let u : float = min((float(target - start) + 0.5) / float(mass), 0.9999999403953552);
    let root = sqrt(u);
    var result : QuadAreaSample;
    result.half = uint(second);
    result.bary = float2(root * (1.0 - sample_value.y), root * sample_value.y);
    result.density = asfloat(second ? area.z : area.y);
    return result;
}

/// 片半编号来自实际硬件 primitive；包含片选择概率，不含离散灯选择概率。
public func quad_area_density(area: uint4, half: uint)->float {
    return half < 2u ? asfloat(half == 0u ? area.y : area.z) : 0.0;
}

/// 用实际两三角和匹配 AS 的变换重建目标端误差；数值域同 geometry::triangle_endpoint。
public func quad_endpoint(vertices_value: LightQuadVertices, sample_value: QuadAreaSample,
                          object_to_world: float3x4, world_to_object: float3x4)
    ->geometry::TriangleEndpoint {
    return geometry::triangle_endpoint(
        vertices_value.a, sample_value.half == 0u ? vertices_value.b : vertices_value.c,
        sample_value.half == 0u ? vertices_value.c : vertices_value.d, sample_value.bary,
        object_to_world, world_to_object);
}

/// 输入已处于追踪坐标系时的重载；不代替有实例变换时的误差预算。
public func quad_endpoint(vertices_value: LightQuadVertices, sample_value: QuadAreaSample)
    ->geometry::TriangleEndpoint {
    return geometry::triangle_endpoint(
        vertices_value.a, sample_value.half == 0u ? vertices_value.b : vertices_value.c,
        sample_value.half == 0u ? vertices_value.c : vertices_value.d, sample_value.bary);
}

/// 将匹配的离散灯 PMF 与片面积密度换成实立体角 PDF。
/// normal 须是该片的单位绕序法线，sides 是应用声明的 LIGHT_FRONT/BACK/BOTH。
/// 双面只将允许侧扩为两侧，不再乘2或把灯复制为重合表面。
/// 有效坐标差平方、密度积和最终 PDF 须在有限 f32 域；无支持、背侧或掠射返回0。
public func quad_light_pdf(receiver: float3, target: float3, normal: float3, area_density: float,
                           selection_pmf: float, sides: uint)
    ->float {
    let delta = receiver - target;
    let distance_squared = dot(delta, delta);
    if (area_density <= 0 || selection_pmf <= 0 || distance_squared <= 0 ||
        !isfinite(distance_squared))
        return 0;
    let cosine = dot(normal, delta * rsqrt(distance_squared));
    return quad_light_pdf_supported(distance_squared, cosine, area_density * selection_pmf, sides);
}

/// 复用连接采样已计算的距离平方和带符号发光侧余弦；其输入域和失败语义同quad_light_pdf。
/// cosine是单位normal与从灯到接收点单位方向的点积，不重复归一化或重建坐标差。
public func quad_light_pdf_geometry(distance_squared: float, cosine: float, area_density: float,
                                    selection_pmf: float, sides: uint)
    ->float {
    if (area_density <= 0 || selection_pmf <= 0 || distance_squared <= 0 ||
        !isfinite(distance_squared))
        return 0;
    return quad_light_pdf_supported(distance_squared, cosine, area_density * selection_pmf, sides);
}

func quad_light_pdf_supported(distance_squared: float, cosine: float, area_pdf: float, sides: uint)
    ->float {
    let side : uint = cosine >= 0 ? LIGHT_FRONT : LIGHT_BACK;
    let absolute_cosine = abs(cosine);
    if ((sides & side) == 0 || absolute_cosine <= 0)
        return 0;
    let pdf = transport::product_over(area_pdf, distance_squared, absolute_cosine);
    return isfinite(pdf) ? pdf : 0;
}

/// 面积密度乘 distance²/lightCosine 转成立体角密度；无支持域时返回0。
/// 输入使用同一长度单位，light_cosine 是发光侧单位法线与连接方向的正余弦。
public func area_solid_angle_pdf(area_pdf: float, distance_squared: float, light_cosine: float)
    ->float {
    if (distance_squared <= 0.0 || light_cosine <= 0.0 || area_pdf <= 0.0)
        return 0.0;
    return transport::product_over(area_pdf, distance_squared, light_cosine);
}
}
