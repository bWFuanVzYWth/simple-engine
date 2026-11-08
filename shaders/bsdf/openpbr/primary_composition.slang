/// 首表面提案组合。物理响应复用原闭包；固定概率只作用于界面反射／进入内部的选择。
module primary_composition;
import bsdf.openpbr.composition;
import bsdf.openpbr.primary_policy;
import bsdf.openpbr.api;
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/opaque/state.slang";
import "bsdf/openpbr/opaque/lobes.slang";
import "bsdf/openpbr/math/microfacet_distribution.slang";

namespace openpbr_composition
{
public struct PrimaryLayer<G : IMeasure, B : IBase> : IClosure
{
    public typedef DielectricLayer<G, B> Closure;
    public typedef Closure.State State;
    public static bool has_delta()
    {
        return G.has_delta();
    }
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material material, float3 wi, float incident)
    {
        State s = Closure.prepare(energy, material, wi, incident);
        bool available = s.mixture.specularSampleWeight + s.mixture.baseSampleWeight > 0;
        float q = openpbr_primary::reflection_probability(s.mixture.specularSampleWeight,
                                                          s.mixture.baseSampleWeight);
        s.mixture.specularSampleWeight = available ? q : 0;
        s.mixture.baseSampleWeight = available ? 1 - q : 0;
        return s;
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return Closure.evaluate(wi, wo, s);
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        return Closure.sample(wi, u, s);
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return Closure.delta_response(wi, s);
    }
    public static float delta_pdf(State s)
    {
        return Closure.delta_pdf(s);
    }
    public static bool marginal_flags(State s)
    {
        return false;
    }
    public static bool marginal_required(State s)
    {
        return true;
    }
};

/// 同一微表面分布上的金属／介电反射合并为一个提案。metalness只参与响应，不冒充提案概率。
public struct PrimaryMetalLayer<G : IMeasure, B : IBase> : IClosure
{
    public typedef DielectricLayer<G, B> Dielectric;
    public typedef SharedMetalBlend<G, Dielectric> Closure;
    public typedef Closure.State State;
    public static bool has_delta()
    {
        return G.has_delta();
    }
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material material, float3 wi, float incident)
    {
        return Closure.prepare(energy, material, wi, incident);
    }
    public static float probability(State s)
    {
        return openpbr_primary::reflection_probability(
            s.metalness + (1 - s.metalness) * s.dielectric.mixture.specularSampleWeight,
            (1 - s.metalness) * s.dielectric.mixture.baseSampleWeight);
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        PrimeFullOpenPbrReflectiveEvalBase reflected = G.evaluate(wi, wo, s.conductor.geometry);
        Value a = Dielectric.from_reflection(wi, wo, s.dielectric, reflected);
        Value b = Conductor<G>.from_reflection(wi, wo, s.conductor, reflected);
        float q = probability(s);
        float pdf = q * reflected.pdf + (1 - q) * B.evaluate(wi, wo, s.dielectric.base).pdf;
        return { primeFullCompactMixEval(a.throughput, b.throughput, s.metalness), pdf };
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        float q = probability(s);
        Proposal p;
        if (q > 0 && u.z < q)
        {
            p = G.sample(wi, float3(u.xy, u.z / q), s.conductor.geometry).bsdfSample;
            if (G.has_delta() && primeFullOpenPbrIsDelta(p.throughput.flags))
            {
                p.throughput.value = Closure.delta_response(wi, s);
                p.pdf *= q;
            }
        }
        else
            p = B.sample(wi, float3(u.xy, (u.z - q) / (1 - q)), s.dielectric.base);
        return p;
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return Closure.delta_response(wi, s);
    }
    public static float delta_pdf(State s)
    {
        return probability(s);
    }
    public static bool marginal_flags(State s)
    {
        return true;
    }
    public static bool marginal_required(State s)
    {
        return true;
    }
};

public struct PrimaryMetalBase<G : IMeasure, B : IBase> : IClosure
{
    public typedef MetalBlend<BaseClosure<B>, Conductor<G>> Closure;
    public typedef Closure.State State;
    public static bool has_delta()
    {
        return G.has_delta();
    }
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material material, float3 wi, float incident)
    {
        return Closure.prepare(energy, material, wi, incident);
    }
    public static float probability(float3 wi, State s)
    {
        return openpbr_primary::reflection_probability(
            s.metalness, (1 - s.metalness) *
                             max(B.energy(wi, s.dielectric).x,
                                 max(B.energy(wi, s.dielectric).y, B.energy(wi, s.dielectric).z)));
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        Value a = B.evaluate(wi, wo, s.dielectric);
        Value b = Conductor<G>.evaluate(wi, wo, s.conductor);
        return { primeFullCompactMixEval(a.throughput, b.throughput, s.metalness),
                 lerp(a.pdf, b.pdf, probability(wi, s)) };
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        float q = probability(wi, s);
        Proposal p;
        if (q > 0 && u.z < q)
        {
            p = Conductor<G>.sample(wi, float3(u.xy, u.z / q), s.conductor);
            if (G.has_delta() && primeFullOpenPbrIsDelta(p.throughput.flags))
            {
                p.throughput.value *= s.metalness;
                p.pdf *= q;
            }
        }
        else
            p = B.sample(wi, float3(u.xy, (u.z - q) / (1 - q)), s.dielectric);
        return p;
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return Closure.delta_response(wi, s);
    }
    // 不由采样器消费：此组合直接在离散样本处写入依赖wi的概率。
    public static float delta_pdf(State s)
    {
        return 0;
    }
    public static bool marginal_flags(State s)
    {
        return true;
    }
    public static bool marginal_required(State s)
    {
        return true;
    }
};

#if defined(OPENPBR_FLOW)
#define PRIMARY_FLOW OPENPBR_FLOW
#elif defined(GAME_FLOW)
#define PRIMARY_FLOW GAME_FLOW
#endif
#if defined(PRIMARY_FLOW) && !(PRIMARY_FLOW & 48)
#if (PRIMARY_FLOW & 2) && (PRIMARY_FLOW & 9)
public typedef PrimaryLayer<SelectedMeasure, SelectedBase> PrimaryDielectric;
#else
public typedef Dielectric PrimaryDielectric;
#endif
#if (PRIMARY_FLOW & 2048) && (PRIMARY_FLOW & 9)
#if PRIMARY_FLOW & 2
public typedef PrimaryMetalLayer<SelectedMeasure, SelectedBase> PrimaryOpaque;
#else
public typedef PrimaryMetalBase<SelectedMeasure, SelectedBase> PrimaryOpaque;
#endif
#elif PRIMARY_FLOW & 2048
public typedef Opaque PrimaryOpaque;
#elif PRIMARY_FLOW & 4
public typedef Metal PrimaryOpaque;
#else
public typedef PrimaryDielectric PrimaryOpaque;
#endif
#endif
}
