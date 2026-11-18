/// 四边形光源的提案：灯面分布、几何连接与正反向 PDF。
/// 发光材质、纹理和实例身份由调用方提供。
module quad;
import geometry.ray;
import light_sampling.area;
import light_sampling.emission_alias;
import "light_sampling/sample.slang";

namespace light_sampling {
/// 均匀面积和纹理alias共用的无损描述；offset按uint word计，0为均匀alias占位。
public struct LightSurfaceProposal {
    public var area : uint4;
    public var alias_offset : uint;
};

/// 编译期灯面分布；均匀和纹理 alias 使用完全相同的正反向契约。
public interface IEmitterAlias {
    func sample(proposal: LightSurfaceProposal, u: float2)->QuadAreaSample;
    func density(proposal: LightSurfaceProposal, half: uint, uv: float2)->float;
};

/// 均匀灯面没有alias离散选择；仅由面积采样器量化一次24-bit选择器。
public struct UniformEmitterAlias : IEmitterAlias {
    public func sample(proposal: LightSurfaceProposal, u: float2)->QuadAreaSample {
        return sample_quad_area(proposal.area, u);
    }
    public func density(proposal: LightSurfaceProposal, half: uint, uv: float2)->float {
        return quad_area_density(proposal.area, half);
    }
};

/// 静态表在资源准备阶段建立；uniform占位和纹理分布使用同一个提案描述。
public struct TexturedEmitterAlias : IEmitterAlias {
    public var words : ByteAddressBuffer;
    public func sample(proposal: LightSurfaceProposal, u: float2)->QuadAreaSample {
        return proposal.alias_offset == 0u ? UniformEmitterAlias().sample(proposal, u)
                                           : sample_emitter_alias(words, proposal.alias_offset, u);
    }
    public func density(proposal: LightSurfaceProposal, half: uint, uv: float2)->float {
        return proposal.alias_offset == 0u
                   ? UniformEmitterAlias().density(proposal, half, uv)
                   : emitter_alias_density(words, proposal.alias_offset, uv);
    }
};

func connect_quad_emitter(point_value: geometry::SurfacePoint, target: geometry::TriangleEndpoint,
                          density: float, selection_pmf: float, sides: uint)
    ->LightSample {
    var result : LightSample = {};
    let delta = target.position - point_value.position;
    let squared = dot(delta, delta);
    if (!(squared > 0) || !isfinite(squared))
        return result;
    let inverse_distance = rsqrt(squared);
    result.direction = delta * inverse_distance;
    result.pdf = quad_light_pdf_geometry(squared, dot(target.normal, -result.direction), density,
                                         selection_pmf, sides);
    if (result.pdf <= 0)
        return result;
    let distance = squared * inverse_distance;
    if (!geometry::connection_ray(point_value, target, result.visibility)) {
        result.pdf = 0;
        return result;
    }
    result.distance = distance;
    return result;
}

/// 在选中灯内采样并构造连接；选择 PMF 由调用方按同一接收点查询。
/// 变换须与 BLAS/TLAS 一致，发射侧由资源所有者声明。无支持时 light.pdf=0。
public func sample_quad_emitter<A : IEmitterAlias>(distribution: A, proposal: LightSurfaceProposal,
                                                   vertices_value: LightQuadVertices,
                                                   point_value: geometry::SurfacePoint, u: float2,
                                                   selection_pmf: float, sides: uint,
                                                   transform: float3x4, inverse: float3x4,
                                                   out area: QuadAreaSample)
    ->LightSample {
    area = {};
    if (selection_pmf <= 0 || (asfloat(proposal.area.y) <= 0 && asfloat(proposal.area.z) <= 0))
        return {};
    area = distribution.sample(proposal, u);
    return connect_quad_emitter(point_value,
                                quad_endpoint(vertices_value, area, transform, inverse),
                                area.density, selection_pmf, sides);
}

/// 世界坐标、无实例变换的重载；保留 geometry 的恒等变换误差预算和运算顺序。
public func sample_quad_emitter<A : IEmitterAlias>(distribution: A, proposal: LightSurfaceProposal,
                                                   vertices_value: LightQuadVertices,
                                                   point_value: geometry::SurfacePoint, u: float2,
                                                   selection_pmf: float, sides: uint,
                                                   out area: QuadAreaSample)
    ->LightSample {
    area = {};
    if (selection_pmf <= 0 || (asfloat(proposal.area.y) <= 0 && asfloat(proposal.area.z) <= 0))
        return {};
    area = distribution.sample(proposal, u);
    return connect_quad_emitter(point_value, quad_endpoint(vertices_value, area), area.density,
                                selection_pmf, sides);
}

/// 反查同一灯面分布和发射侧；selection_pmf 为同一接收点的选灯概率。
public func quad_emitter_pdf<A : IEmitterAlias>(distribution: A, surface: LightSurfaceProposal,
                                                selection_pmf: float, receiver: float3,
                                                target: float3, normal: float3, half: uint,
                                                uv: float2, sides: uint)
    ->float {
    return quad_light_pdf(receiver, target, normal, distribution.density(surface, half, uv),
                          selection_pmf, sides);
}

}
