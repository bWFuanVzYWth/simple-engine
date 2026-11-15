/// 光源提案的公共边界：位置树选灯、灯面alias，以及两个独立的加性灯族。
/// 材质、纹理读解码、实例身份和坐标变换由应用提供，不进入提案所有者。
module light_sampling;
__exported import pt.light_bvh;
__exported import pt.emission_alias;
import pt.scene;
import geometry.ray;

namespace pt
{
/// 与同一发布事务的树和原始灯顺序绑定；不拥有GPU缓冲。
public struct LightBvhProposal
{
    public var words : StructuredBuffer<uint4>;
    public var base : uint;
    public var count : uint;
    public func sample(receiver: float3, selector: float)->LightBvhSample
    {
        return sample_light_bvh(words, base, count, receiver, selector);
    }
    public func pmf(receiver: float3, light: uint)->float
    {
        return light_bvh_pmf(words, base, count, receiver, light);
    }
};

/// 均匀面积和纹理alias共用的无损描述；offset按uint word计，0为均匀alias占位。
public struct LightSurfaceProposal
{
    public var area : uint4;
    public var alias_offset : uint;
};

/// 编译期灯面分布；均匀和纹理 alias 使用完全相同的正反向契约。
public interface IEmitterAlias
{
    func sample(proposal: LightSurfaceProposal, u: float2)->QuadAreaSample;
    func density(proposal: LightSurfaceProposal, half: uint, uv: float2)->float;
};

/// 纯色灯预留的单列alias；与真实alias相同地建立选择概率和条件残量。
/// 目前唯一列质量为1，编译器可消除选择；未来纹理发光由非零offset承接。
public struct UniformEmitterAlias : IEmitterAlias
{
    public func sample(proposal: LightSurfaceProposal, u: float2)->QuadAreaSample
    {
        static const let RESERVED_UNIFORM_ALIAS = uint4(LIGHT_SELECTORS, 0, LIGHT_SELECTORS, 0);
        let selector : uint = min(uint(u.x * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
        // 保留24-bit格点；面积采样器只在选定半片内取中点，避免两次取中点舍入。
        let residual = float(selector) / float(RESERVED_UNIFORM_ALIAS.x);
        return sample_quad_area(proposal.area, float2(residual, u.y));
    }
    public func density(proposal: LightSurfaceProposal, half: uint, uv: float2)->float
    {
        return quad_area_density(proposal.area, half);
    }
};

/// 静态表在资源准备阶段建立；uniform占位和纹理分布使用同一个提案描述。
public struct TexturedEmitterAlias : IEmitterAlias
{
    public var words : ByteAddressBuffer;
    public func sample(proposal: LightSurfaceProposal, u: float2)->QuadAreaSample
    {
        return proposal.alias_offset == 0u ? UniformEmitterAlias().sample(proposal, u)
                                           : sample_emitter_alias(words, proposal.alias_offset, u);
    }
    public func density(proposal: LightSurfaceProposal, half: uint, uv: float2)->float
    {
        return proposal.alias_offset == 0u
                   ? UniformEmitterAlias().density(proposal, half, uv)
                   : emitter_alias_density(words, proposal.alias_offset, uv);
    }
};

func connect_quad_emitter(point_value: geometry::SurfacePoint, target: geometry::TriangleEndpoint,
                          density: float, selectionPmf: float, sides: uint)
    ->LightSample
{
    var result : LightSample = {};
    let delta : float3 = target.position - point_value.position;
    let squared : float = dot(delta, delta);
    if (!(squared > 0) || !isfinite(squared))
        return result;
    result.direction = delta * rsqrt(squared);
    result.pdf = quad_light_pdf(point_value.position, target.position, target.normal, density,
                                selectionPmf, sides);
    if (result.pdf <= 0 || !geometry::connection_ray(point_value, target, result.visibility))
    {
        result.pdf = 0;
        return result;
    }
    result.distance = sqrt(squared);
    return result;
}

/// 在选中灯内采样并构造连接；选择 PMF 来自同接收点的 LightBvhProposal。
/// 变换须与 BLAS/TLAS 一致，发射侧由资源所有者声明。无支持时 light.pdf=0。
public func sample_quad_emitter<A : IEmitterAlias>(distribution: A, proposal: LightSurfaceProposal,
                                                   vertices_value: LightQuadVertices,
                                                   point_value: geometry::SurfacePoint, u: float2,
                                                   selectionPmf: float, sides: uint,
                                                   transform: float3x4, inverse: float3x4,
                                                   out area: QuadAreaSample)
    ->LightSample
{
    area = {};
    if (selectionPmf <= 0 || (asfloat(proposal.area.y) <= 0 && asfloat(proposal.area.z) <= 0))
        return {};
    area = distribution.sample(proposal, u);
    return connect_quad_emitter(point_value,
                                quad_endpoint(vertices_value, area, transform, inverse),
                                area.density, selectionPmf, sides);
}

/// 世界坐标、无实例变换的重载；保留 geometry 的恒等变换误差预算和运算顺序。
public func sample_quad_emitter<A : IEmitterAlias>(distribution: A, proposal: LightSurfaceProposal,
                                                   vertices_value: LightQuadVertices,
                                                   point_value: geometry::SurfacePoint, u: float2,
                                                   selectionPmf: float, sides: uint,
                                                   out area: QuadAreaSample)
    ->LightSample
{
    area = {};
    if (selectionPmf <= 0 || (asfloat(proposal.area.y) <= 0 && asfloat(proposal.area.z) <= 0))
        return {};
    area = distribution.sample(proposal, u);
    return connect_quad_emitter(point_value, quad_endpoint(vertices_value, area), area.density,
                                selectionPmf, sides);
}

/// 反查同一树、灯面分布和发射侧；treeReceiver 允许树在独立的近原点坐标系。
public func quad_emitter_pdf<A : IEmitterAlias>(distribution: A, surface: LightSurfaceProposal,
                                                tree: LightBvhProposal, treeReceiver: float3,
                                                rank: uint, receiver: float3, target: float3,
                                                normal: float3, half: uint, uv: float2, sides: uint)
    ->float
{
    return quad_light_pdf(receiver, target, normal, distribution.density(surface, half, uv),
                          tree.pmf(treeReceiver, rank), sides);
}

/// 两个族分别独立采一次，返回条件随机数；PDF不乘二选一概率，缺失族贡献为0。
/// 输入z在[0,1)，下半区为局部灯，上半区为环境灯；调用方不能按活动族重新分配。
public func local_light_family(inout u: float3)->bool
{
    let local : bool = u.z < 0.5;
    u.z = local ? 2.0 * u.z : 2.0 * u.z - 1.0;
    return local;
}
}
