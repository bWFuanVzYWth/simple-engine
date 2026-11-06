/// 光源提案的公共边界：位置树选灯、灯面alias，以及两个独立的加性灯族。
/// 材质、纹理读解码、实例身份和坐标变换由应用提供，不进入提案所有者。
module light_proposal;
__exported import pt.light_bvh;
__exported import pt.emission_alias;

namespace pt
{
/// 与同一发布事务的树和原始灯顺序绑定；不拥有GPU缓冲。
public struct LightBvhProposal
{
    public StructuredBuffer<uint4> words;
    public uint base;
    public uint count;
    public LightBvhSample sample(float3 receiver, float selector)
    {
        return sample_light_bvh(words, base, count, receiver, selector);
    }
    public float pmf(float3 receiver, uint light)
    {
        return light_bvh_pmf(words, base, count, receiver, light);
    }
};

/// 均匀面积和纹理alias共用的无损描述；offset按uint word计，0为均匀alias占位。
public struct LightSurfaceProposal
{
    public uint4 area;
    public uint alias_offset;
};

/// 纯色灯预留的单列alias；与真实alias相同地建立选择概率和条件残量。
/// 目前唯一列质量为1，编译器可消除选择；未来纹理发光由非零offset承接。
public struct UniformEmitterAlias
{
    public QuadAreaSample sample(LightSurfaceProposal proposal, float2 u)
    {
        static const uint4 RESERVED_UNIFORM_ALIAS = uint4(LIGHT_SELECTORS, 0, LIGHT_SELECTORS, 0);
        uint selector = min(uint(u.x * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
        // 保留24-bit格点；面积采样器只在选定半片内取中点，避免两次取中点舍入。
        float residual = float(selector) / float(RESERVED_UNIFORM_ALIAS.x);
        return sample_quad_area(proposal.area, float2(residual, u.y));
    }
    public float density(LightSurfaceProposal proposal, uint half, float2 uv)
    {
        return quad_area_density(proposal.area, half);
    }
};

/// 静态表在资源准备阶段建立；uniform占位和纹理分布使用同一个提案描述。
public struct TexturedEmitterAlias
{
    public ByteAddressBuffer words;
    public QuadAreaSample sample(LightSurfaceProposal proposal, float2 u)
    {
        return proposal.alias_offset == 0u ? UniformEmitterAlias().sample(proposal, u)
                                           : sample_emitter_alias(words, proposal.alias_offset, u);
    }
    public float density(LightSurfaceProposal proposal, uint half, float2 uv)
    {
        return proposal.alias_offset == 0u
                   ? UniformEmitterAlias().density(proposal, half, uv)
                   : emitter_alias_density(words, proposal.alias_offset, uv);
    }
};

/// 两个族分别独立采一次，返回条件随机数；PDF不乘二选一概率，缺失族贡献为0。
/// 输入z在[0,1)，下半区为局部灯，上半区为环境灯；调用方不能按活动族重新分配。
public bool local_light_family(inout float3 u)
{
    bool local = u.z < 0.5;
    u.z = local ? 2.0 * u.z : 2.0 * u.z - 1.0;
    return local;
}
}
