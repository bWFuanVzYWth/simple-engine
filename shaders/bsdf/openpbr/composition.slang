// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 白名单使用的数值部件；状态由实际组合决定，不包含能力掩码或未使用的lobe。
module composition;
import bsdf.openpbr.api;
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/math/frame.slang";
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/microfacet_distribution.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/microfacet_measure.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/opaque/state.slang";
import "bsdf/openpbr/opaque/lobes.slang";
import "bsdf/openpbr/pt_subsurface.slang";
import "bsdf/openpbr/service/support/conversion.slang";

#ifdef OPENPBR_RECIPE_VARIANT
#define COMPOSITION_VARIANT OPENPBR_VARIANT
#else
#define COMPOSITION_VARIANT 0
#endif

namespace openpbr_composition
{
public typedef PrimeFullCompactOpaqueEvaluation Value;
public typedef PrimeFullOpenPbrSample Proposal;
public struct Empty
{
};

public interface IBase
{
    associatedtype State;
    static State prepare(float3 color, float weight);
    static float3 energy(float3 wi, State state);
    static Value evaluate(float3 wi, float3 wo, State state);
    static Proposal sample(float3 wi, float3 u, State state);
};
public struct Absorber : IBase
{
    public typedef Empty State;
    public static State prepare(float3 color, float weight)
    {
        return {};
    }
    public static float3 energy(float3 wi, State state)
    {
        return 0;
    }
    public static Value evaluate(float3 wi, float3 wo, State state)
    {
        return {};
    }
    public static Proposal sample(float3 wi, float3 u, State state)
    {
        return primeFullOpenPbrZeroSample();
    }
};
public struct Lambert : IBase
{
    public typedef float3 State;
    public static State prepare(float3 color, float weight)
    {
        return color;
    }
    public static float3 energy(float3 wi, State state)
    {
        return wi.z >= 0 ? state : float3(0);
    }
    public static Value evaluate(float3 wi, float3 wo, State state)
    {
        if (wi.z < 0 || wo.z < 0)
            return {};
        float pdf = PRIME_FULL_OPENPBR_INV_PI * wo.z;
        return { { state * pdf, PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION }, pdf };
    }
    public static Proposal sample(float3 wi, float3 u, State state)
    {
        Proposal p = primeFullOpenPbrZeroSample();
        if (wi.z < 0)
            return p;
        p.wo = primeFullOpenPbrCosineSampleHemisphere(u.xy);
        p.pdf = PRIME_FULL_OPENPBR_INV_PI * p.wo.z;
        p.throughput = { state * p.pdf, PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION };
        return p;
    }
};
public struct Subsurface : IBase
{
    public typedef float3 State;
    public static State prepare(float3 color, float weight)
    {
        return primeFullPtSubsurfaceColor(color);
    }
    public static float3 energy(float3 wi, State state)
    {
        return state;
    }
    public static Value evaluate(float3 wi, float3 wo, State state)
    {
        bool reflected = wi.z * wo.z > 0;
        float cosine = PRIME_FULL_OPENPBR_INV_PI * abs(wo.z);
        return { { float3(cosine) * state * 0.5 *
                       (reflected ? 1.3333333730697632 : 0.6666666269302368),
                   reflected ? PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION
                             : PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION },
                 cosine * (reflected ? 0.6666666865348816 : 0.3333333134651184) };
    }
    public static Proposal sample(float3 wi, float3 u, State state)
    {
        Proposal p = primeFullOpenPbrZeroSample();
        p.wo = primeFullOpenPbrCosineSampleHemisphere(u.xy);
        p.pdf = PRIME_FULL_OPENPBR_INV_PI * p.wo.z;
        p.throughput.value = float3(p.pdf);
        if (wi.z < 0)
            p.wo.z = -p.wo.z;
        if (u.z <= 0.3333333134651184)
        {
            p.wo.z = -p.wo.z;
            p.pdf *= 0.3333333134651184;
            p.throughput.value *= 0.3333333134651184;
            p.throughput.value *= state;
            p.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION;
        }
        else
        {
            p.pdf *= 0.6666666865348816;
            p.throughput.value *= 0.6666666865348816 * state;
            p.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION;
        }
        return p;
    }
};
public struct BaseSwitchState
{
    public float3 color;
    public bool subsurface;
};
public struct SubsurfaceSwitch : IBase
{
    public typedef BaseSwitchState State;
    public static State prepare(float3 color, float weight)
    {
        return { weight == 1 ? Subsurface.prepare(color, 1) : color, weight == 1 };
    }
    public static float3 energy(float3 wi, State s)
    {
        return s.subsurface ? Subsurface.energy(wi, s.color) : Lambert.energy(wi, s.color);
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return s.subsurface ? Subsurface.evaluate(wi, wo, s.color)
                            : Lambert.evaluate(wi, wo, s.color);
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        return s.subsurface ? Subsurface.sample(wi, u, s.color) : Lambert.sample(wi, u, s.color);
    }
};
public struct BaseBlendState
{
    public float3 diffuse;
    public float3 subsurface;
    public float weight;
    public PrimeFullCompactMixState proposal;
};
public struct SubsurfaceBlend : IBase
{
    public typedef BaseBlendState State;
    public static State prepare(float3 color, float weight)
    {
        float3 sss = weight > 0 ? Subsurface.prepare(color, 1) : float3(0);
        return { color, sss, weight, primeFullCompactMakeMixState(weight, color, sss) };
    }
    public static float3 energy(float3 wi, State s)
    {
        return s.diffuse * (1 - s.weight) + s.subsurface * s.weight;
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        Value a = s.weight < 1 ? Lambert.evaluate(wi, wo, s.diffuse) : Value();
        Value b = s.weight > 0 ? Subsurface.evaluate(wi, wo, s.subsurface) : Value();
        return { primeFullCompactMixEval(a.throughput, b.throughput, s.weight),
                 s.proposal.firstSampleWeight + s.proposal.secondSampleWeight > 0
                     ? lerp(a.pdf, b.pdf, s.proposal.secondSampleWeight)
                     : 0 };
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        if (s.weight == 0)
            return Lambert.sample(wi, u, s.diffuse);
        if (s.weight == 1)
            return Subsurface.sample(wi, u, s.subsurface);
        if (s.proposal.firstSampleWeight + s.proposal.secondSampleWeight <= 0)
            return primeFullOpenPbrZeroSample();
        if (u.z <= s.proposal.secondSampleWeight && s.proposal.secondSampleWeight > 0)
        {
            Proposal p = Subsurface.sample(wi, float3(u.xy, u.z / s.proposal.secondSampleWeight),
                                           s.subsurface);
            p.throughput.value *= s.weight;
            p.pdf *= s.proposal.secondSampleWeight;
            return p;
        }
        // Fractional SSS 保留参考 EON 的随机映射，包括零概率端点。
        float z = (u.z - s.proposal.secondSampleWeight) / s.proposal.firstSampleWeight;
        Proposal p = primeFullOpenPbrZeroSample();
        if (wi.z < 0)
            return p;
        if (z <= 0)
        {
            p.wo = primeFullOpenPbrUniformSampleHemisphere(u.xy);
            p.pdf = p.wo.z * PRIME_FULL_OPENPBR_INV_PI;
        }
        else
        {
            float radius = sqrt(u.x), phi = 2 * PRIME_FULL_OPENPBR_PI * u.y;
            float x = -radius * cos(phi), y = radius * sin(phi);
            float3 wo = float3(x, y, sqrt(max(1 - (x * x + y * y), 0)));
            float n = length(wo);
            p.pdf = wo.z * PRIME_FULL_OPENPBR_INV_PI * n * n * n;
            p.wo = normalize(mul(wo, primeFullCompactEonBasis(wi)));
        }
        p.throughput = Lambert.evaluate(wi, p.wo, s.diffuse).throughput;
        p.throughput.value *= 1 - s.weight;
        p.pdf *= s.proposal.firstSampleWeight;
        return p;
    }
};

/// 测度部件隔离纯delta、纯连续和跨阈值输入；仅跨阈值部件执行数值分类。
public interface IMeasure
{
    associatedtype State;
    static bool has_delta();
    static State prepare(float roughness);
    static State prepare_alpha(float alpha);
    static PrimeFullOpenPbrMicrofacet distribution(State s);
    static bool delta(State s);
    static PrimeFullOpenPbrReflectiveEvalBase evaluate(float3 wi, float3 wo, State s);
    static PrimeFullOpenPbrReflectiveSampleBase sample(float3 wi, float3 u, State s);
    static float conductor_energy(float3 wi, State s);
    static float3 dielectric_energy<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy, float3 wi,
                                                                                float ior, State s);
};
public struct Smooth : IMeasure
{
    public static bool has_delta()
    {
        return true;
    }
    public typedef Empty State;
    public static State prepare(float roughness)
    {
        return {};
    }
    public static State prepare_alpha(float alpha)
    {
        return {};
    }
    public static PrimeFullOpenPbrMicrofacet distribution(State s)
    {
        return {};
    }
    public static bool delta(State s)
    {
        return true;
    }
    public static PrimeFullOpenPbrReflectiveEvalBase evaluate(float3 wi, float3 wo, State s)
    {
        return {};
    }
    public static PrimeFullOpenPbrReflectiveSampleBase sample(float3 wi, float3 u, State s)
    {
        return primeFullOpenPbrReflectiveSampleBase(wi, u, distribution(s),
                                                    PRIME_FULL_OPENPBR_FLAG_ALL,
                                                    PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    }
    public static float conductor_energy(float3 wi, State s)
    {
        return 1;
    }
    public static float3 dielectric_energy<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                                       float3 wi,
                                                                                       float ior,
                                                                                       State s)
    {
        return primeFullOpenPbrMicrofacetDielectricMsCompensation<true>(energy, distribution(s),
                                                                        wi.z, ior);
    }
};
public struct Continuous : IMeasure
{
    public static bool has_delta()
    {
        return false;
    }
    public typedef float State;
    public static State prepare(float roughness)
    {
        return roughness * roughness;
    }
    public static State prepare_alpha(float alpha)
    {
        return alpha;
    }
    public static PrimeFullOpenPbrMicrofacet distribution(State s)
    {
        return { float2(s) };
    }
    public static bool delta(State s)
    {
        return false;
    }
    public static PrimeFullOpenPbrReflectiveEvalBase evaluate(float3 wi, float3 wo, State s)
    {
        return primeFullOpenPbrReflectiveEvalBaseImpl<true>(wi, wo, distribution(s),
                                                            PRIME_FULL_OPENPBR_FLAG_ALL,
                                                            PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    }
    public static PrimeFullOpenPbrReflectiveSampleBase sample(float3 wi, float3 u, State s)
    {
        return primeFullOpenPbrReflectiveSampleBaseImpl<true>(wi, u, distribution(s),
                                                              PRIME_FULL_OPENPBR_FLAG_ALL,
                                                              PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    }
    public static float conductor_energy(float3 wi, State s)
    {
        return primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(distribution(s), wi.z,
                                                                     float3(1), float3(1))
            .x;
    }
    public static float3 dielectric_energy<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                                       float3 wi,
                                                                                       float ior,
                                                                                       State s)
    {
        return primeFullOpenPbrMicrofacetDielectricMsCompensation<false>(energy, distribution(s),
                                                                         wi.z, ior);
    }
};
public struct MixedMeasure : IMeasure
{
    public static bool has_delta()
    {
        return true;
    }
    public typedef float State;
    public static State prepare(float roughness)
    {
        return roughness * roughness;
    }
    public static State prepare_alpha(float alpha)
    {
        return alpha;
    }
    public static PrimeFullOpenPbrMicrofacet distribution(State s)
    {
        return { float2(s) };
    }
    public static bool delta(State s)
    {
        return s < 1.0e-4;
    }
    public static PrimeFullOpenPbrReflectiveEvalBase evaluate(float3 wi, float3 wo, State s)
    {
        return primeFullOpenPbrReflectiveEvalBaseImpl<false>(wi, wo, distribution(s),
                                                             PRIME_FULL_OPENPBR_FLAG_ALL,
                                                             PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    }
    public static PrimeFullOpenPbrReflectiveSampleBase sample(float3 wi, float3 u, State s)
    {
        return primeFullOpenPbrReflectiveSampleBaseImpl<false>(wi, u, distribution(s),
                                                               PRIME_FULL_OPENPBR_FLAG_ALL,
                                                               PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    }
    public static float conductor_energy(float3 wi, State s)
    {
        return delta(s) ? 1 : Continuous.conductor_energy(wi, s);
    }
    public static float3 dielectric_energy<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                                       float3 wi,
                                                                                       float ior,
                                                                                       State s)
    {
        return Continuous.dielectric_energy(energy, wi, ior, s);
    }
};

public interface IClosure
{
    associatedtype State;
    static bool has_delta();
    static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy, openpbr::Material m,
                                                                     float3 wi, float incident);
    static Value evaluate(float3 wi, float3 wo, State s);
    /// 单个proposal；外层组合在连续样本上统一恢复marginal响应和PDF。
    static Proposal sample(float3 wi, float3 u, State s);
    static float3 delta_response(float3 wi, State s);
    static float delta_pdf(State s);
    static bool marginal_flags(State s);
    static bool marginal_required(State s);
};
/// 共享同一微表面分布的闭包只组合响应，不重复计算GGX几何项。
public interface IReflectionClosure : IClosure
{
    static Value from_reflection(float3 wi, float3 wo, State s,
                                 PrimeFullOpenPbrReflectiveEvalBase reflection);
};
public struct ConductorState<G : IMeasure>
{
    public G.State geometry;
    public float3 color;
    public float ess;
};
public PrimeFullOpenPbrConductorFresnel conductor_fresnel(float3 color)
{
    return { color, float3(1), float3(1), 1, color };
}
public struct Conductor<G : IMeasure> : IReflectionClosure
{
    public static bool has_delta()
    {
        return G.has_delta();
    }
    public typedef ConductorState<G> State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                            openpbr::Material m,
                                                                            float3 wi,
                                                                            float incident)
    {
        G.State g = G.prepare(m.base_roughness.w);
        return { g, saturate(m.base_roughness.xyz), G.conductor_energy(wi, g) };
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return from_reflection(wi, wo, s, G.evaluate(wi, wo, s.geometry));
    }
    public static Value from_reflection(float3 wi, float3 wo, State s,
                                        PrimeFullOpenPbrReflectiveEvalBase b)
    {
        if (b.flags == 0)
            return {};
        float3 value =
            primeFullOpenPbrConductorFresnelUnpolarized(conductor_fresnel(s.color), b.microCosine) *
            b.factor;
        value *= 1 + s.color * (1 - s.ess) / s.ess;
        return { { value, b.flags }, b.pdf };
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        PrimeFullOpenPbrReflectiveSampleBase b = G.sample(wi, u, s.geometry);
        Proposal p = b.bsdfSample;
        p.throughput.value *=
            primeFullOpenPbrConductorFresnelUnpolarized(conductor_fresnel(s.color), b.microCosine);
        if (!G.has_delta() || primeFullOpenPbrIsNonDelta(p.throughput.flags))
            p.throughput.value *= 1 + s.color * (1 - s.ess) / s.ess;
        return p;
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return primeFullOpenPbrConductorFresnelUnpolarized(conductor_fresnel(s.color), wi.z);
    }
    public static float delta_pdf(State s)
    {
        return 1;
    }
    public static bool marginal_flags(State s)
    {
        return false;
    }
    public static bool marginal_required(State s)
    {
        return false;
    }
};
public struct LayerState<G : IMeasure, B : IBase>
{
    public G.State geometry;
    public B.State base;
    public float ior;
    public float3 compensation;
    public PrimeFullCompactLayerState mixture;
};
public PrimeFullOpenPbrSpecularFresnel dielectric_fresnel(float ior)
{
    return { ior, primeFullOpenPbrIorAdjustment(ior, 1), float3(1) };
}
public struct DeltaDielectric : IClosure
{
    public static bool has_delta()
    {
        return true;
    }
    public typedef float State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                            openpbr::Material m,
                                                                            float3 wi,
                                                                            float incident)
    {
        return m.surface.x / incident;
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return {};
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        Proposal p = primeFullOpenPbrZeroSample();
        if (wi.z > 0 && s != 1)
        {
            p.wo = reflect(-wi, float3(0, 0, 1));
            p.throughput = { delta_response(wi, s), PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION };
            p.pdf = 1;
        }
        return p;
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s), wi.z);
    }
    public static float delta_pdf(State s)
    {
        return s != 1 ? 1 : 0;
    }
    public static bool marginal_flags(State s)
    {
        return false;
    }
    public static bool marginal_required(State s)
    {
        return false;
    }
};
public struct SpecularState<G : IMeasure>
{
    public G.State geometry;
    public float ior;
    public float compensation;
    public bool available;
};
public struct Specular<G : IMeasure> : IReflectionClosure
{
    public static bool has_delta()
    {
        return G.has_delta();
    }
    public typedef SpecularState<G> State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                            openpbr::Material m,
                                                                            float3 wi,
                                                                            float incident)
    {
        G.State g = G.prepare(m.base_roughness.w);
        float ior = m.surface.x / incident;
        float3 ms = G.dielectric_energy(energy, wi, primeFullOpenPbrIorAdjustment(ior, 1), g);
        return { g, ior, ms.x, ms.z > 0 };
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return from_reflection(wi, wo, s, G.evaluate(wi, wo, s.geometry));
    }
    public static Value from_reflection(float3 wi, float3 wo, State s,
                                        PrimeFullOpenPbrReflectiveEvalBase b)
    {
        if (b.flags == 0)
            return {};
        return { { primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s.ior),
                                                              b.microCosine) *
                       b.factor * s.compensation,
                   b.flags },
                 s.available ? b.pdf : 0 };
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        if (!s.available)
            return primeFullOpenPbrZeroSample();
        PrimeFullOpenPbrReflectiveSampleBase b = G.sample(wi, u, s.geometry);
        Proposal p = b.bsdfSample;
        p.throughput.value *=
            primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s.ior), b.microCosine);
        if (!G.has_delta() || primeFullOpenPbrIsNonDelta(p.throughput.flags))
            p.throughput.value *= s.compensation;
        return p;
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s.ior), wi.z);
    }
    public static float delta_pdf(State s)
    {
        return s.available ? 1 : 0;
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
public struct DielectricLayer<G : IMeasure, B : IBase> : IReflectionClosure
{
    public static bool has_delta()
    {
        return G.has_delta();
    }
    public typedef LayerState<G, B> State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                            openpbr::Material m,
                                                                            float3 wi,
                                                                            float incident)
    {
        State s;
        s.geometry = G.prepare(m.base_roughness.w);
        s.base = B.prepare(m.base_roughness.xyz, m.surface.z);
        s.ior = m.surface.x / incident;
        s.compensation =
            G.dielectric_energy(energy, wi, primeFullOpenPbrIorAdjustment(s.ior, 1), s.geometry);
        s.mixture =
            primeFullCompactMakeLayerState(B.energy(wi, s.base), float3(1 - s.compensation.z),
                                           wi.z >= 0 ? float3(s.compensation.z) : float3(0));
        return s;
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return from_reflection(wi, wo, s, G.evaluate(wi, wo, s.geometry));
    }
    public static Value from_reflection(float3 wi, float3 wo, State s,
                                        PrimeFullOpenPbrReflectiveEvalBase reflected)
    {
        Value base = B.evaluate(wi, wo, s.base);
        PrimeFullOpenPbrThroughput specular = primeFullOpenPbrZeroThroughput();
        if (reflected.flags != 0)
            specular = { primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s.ior),
                                                                    reflected.microCosine) *
                             reflected.factor * s.compensation.x,
                         reflected.flags };
        return { primeFullCompactLayerEval(base.throughput, specular, s.mixture),
                 s.mixture.baseSampleWeight + s.mixture.specularSampleWeight > 0
                     ? lerp(base.pdf, reflected.pdf, s.mixture.specularSampleWeight)
                     : 0 };
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        if (s.mixture.baseSampleWeight + s.mixture.specularSampleWeight <= 0)
            return primeFullOpenPbrZeroSample();
        if (u.z <= s.mixture.specularSampleWeight && s.mixture.specularSampleWeight > 0)
        {
            PrimeFullOpenPbrReflectiveSampleBase b =
                G.sample(wi, float3(u.xy, u.z / s.mixture.specularSampleWeight), s.geometry);
            Proposal p = b.bsdfSample;
            p.throughput.value *= primeFullOpenPbrSpecularFresnelUnpolarized(
                dielectric_fresnel(s.ior), b.microCosine);
            if (!G.has_delta() || primeFullOpenPbrIsNonDelta(p.throughput.flags))
                p.throughput.value *= s.compensation.x;
            p.pdf *= s.mixture.specularSampleWeight;
            return p;
        }
        Proposal p = B.sample(
            wi, float3(u.xy, (u.z - s.mixture.specularSampleWeight) / s.mixture.baseSampleWeight),
            s.base);
        p.throughput = primeFullOpenPbrThroughputScale(p.throughput, s.mixture.transmittanceIn);
        p.pdf *= s.mixture.baseSampleWeight;
        return p;
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s.ior), wi.z);
    }
    public static float delta_pdf(State s)
    {
        return s.mixture.specularSampleWeight;
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
public struct BaseClosure<B : IBase> : IClosure
{
    public static bool has_delta()
    {
        return false;
    }
    public typedef B.State State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                            openpbr::Material m,
                                                                            float3 wi,
                                                                            float incident)
    {
        return B.prepare(m.base_roughness.xyz, m.surface.z);
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        Value v = B.evaluate(wi, wo, s);
        if (!any(B.energy(wi, s) > 0))
            v.pdf = 0;
        return v;
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        return any(B.energy(wi, s) > 0) ? B.sample(wi, u, s) : primeFullOpenPbrZeroSample();
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return 0;
    }
    public static float delta_pdf(State s)
    {
        return 0;
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
public struct MetalBlendState<L : IClosure, M : IClosure>
{
    public L.State dielectric;
    public M.State conductor;
    public float metalness;
};
public struct MetalBlend<L : IClosure, M : IClosure> : IClosure
{
    public static bool has_delta()
    {
        return L.has_delta() || M.has_delta();
    }
    public typedef MetalBlendState<L, M> State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                            openpbr::Material m,
                                                                            float3 wi,
                                                                            float incident)
    {
        return { L.prepare(energy, m, wi, incident), M.prepare(energy, m, wi, incident),
                 m.surface.y };
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        Value a = L.evaluate(wi, wo, s.dielectric);
        Value b = M.evaluate(wi, wo, s.conductor);
        return { primeFullCompactMixEval(a.throughput, b.throughput, s.metalness),
                 lerp(a.pdf, b.pdf, s.metalness) };
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return lerp(L.delta_response(wi, s.dielectric), M.delta_response(wi, s.conductor),
                    s.metalness);
    }
    public static float delta_pdf(State s)
    {
        return s.metalness + (1 - s.metalness) * L.delta_pdf(s.dielectric);
    }
    public static bool marginal_flags(State s)
    {
        return s.metalness > 0 && s.metalness < 1;
    }
    public static bool marginal_required(State s)
    {
        return s.metalness < 1;
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        Proposal p;
        if (u.z < s.metalness)
            p = M.sample(wi, float3(u.xy, u.z / s.metalness), s.conductor);
        else
            p = L.sample(wi, float3(u.xy, (u.z - s.metalness) / (1 - s.metalness)), s.dielectric);
        if (primeFullOpenPbrHasSample(p) && marginal_flags(s) && M.has_delta() &&
            primeFullOpenPbrIsDelta(p.throughput.flags))
        {
            p.throughput = { delta_response(wi, s), PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION };
            p.pdf = delta_pdf(s);
        }
        return p;
    }
};
/// 分数金属与介电反射共用GGX；保留一般混合的采样与delta规则。
public struct SharedMetalBlend<G : IMeasure, D : IReflectionClosure> : IClosure
{
    public typedef MetalBlend<D, Conductor<G>> Mixture;
    public typedef MetalBlendState<D, Conductor<G>> State;
    public static bool has_delta()
    {
        return G.has_delta();
    }
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                            openpbr::Material m,
                                                                            float3 wi,
                                                                            float incident)
    {
        return Mixture.prepare(energy, m, wi, incident);
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        PrimeFullOpenPbrReflectiveEvalBase reflection = G.evaluate(wi, wo, s.conductor.geometry);
        Value dielectric = D.from_reflection(wi, wo, s.dielectric, reflection);
        Value metal = Conductor<G>.from_reflection(wi, wo, s.conductor, reflection);
        return { primeFullCompactMixEval(dielectric.throughput, metal.throughput, s.metalness),
                 lerp(dielectric.pdf, metal.pdf, s.metalness) };
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        return Mixture.sample(wi, u, s);
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return Mixture.delta_response(wi, s);
    }
    public static float delta_pdf(State s)
    {
        return Mixture.delta_pdf(s);
    }
    public static bool marginal_flags(State s)
    {
        return Mixture.marginal_flags(s);
    }
    public static bool marginal_required(State s)
    {
        return Mixture.marginal_required(s);
    }
};
public struct NullMetalState<M : IClosure>
{
    public M.State metal;
    public float weight;
};
/// 黑色非金属端仍保留null proposal，不重新归一化金属提案。
public struct NullMetal<M : IClosure> : IClosure
{
    public static bool has_delta()
    {
        return M.has_delta();
    }
    public typedef NullMetalState<M> State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy,
                                                                            openpbr::Material m,
                                                                            float3 wi,
                                                                            float incident)
    {
        return { M.prepare(energy, m, wi, incident), m.surface.y };
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        Value v = M.evaluate(wi, wo, s.metal);
        v.throughput.value *= s.weight;
        v.pdf *= s.weight;
        return v;
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        if (u.z >= s.weight)
            return primeFullOpenPbrZeroSample();
        Proposal p = M.sample(wi, float3(u.xy, u.z / s.weight), s.metal);
        p.throughput.value *= s.weight;
        p.pdf *= s.weight;
        if (s.weight < 1 && (!M.has_delta() || primeFullOpenPbrIsNonDelta(p.throughput.flags)))
            p.throughput.flags |= PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION;
        return p;
    }
    public static float3 delta_response(float3 wi, State s)
    {
        return M.delta_response(wi, s.metal) * s.weight;
    }
    public static float delta_pdf(State s)
    {
        return s.weight;
    }
    public static bool marginal_flags(State s)
    {
        return false;
    }
    public static bool marginal_required(State s)
    {
        return s.weight < 1;
    }
};
public Proposal sample_closure<C : IClosure>(float3 wi, float3 u, C.State s)
{
    Proposal p = C.sample(wi, u, s);
    if (C.marginal_required(s) && primeFullOpenPbrHasSample(p) &&
        (!C.has_delta() || primeFullOpenPbrIsNonDelta(p.throughput.flags)))
    {
        Value v = C.evaluate(wi, p.wo, s);
        if (C.marginal_flags(s))
            p.throughput = v.throughput;
        else
            p.throughput.value = v.throughput.value;
        p.pdf = v.pdf;
    }
    return p;
};

#if defined(OPENPBR_FLOW)
#define COMPOSITION_FLOW OPENPBR_FLOW
#elif defined(GAME_FLOW)
#define COMPOSITION_FLOW GAME_FLOW
#endif
#ifdef COMPOSITION_FLOW
#if !(COMPOSITION_FLOW & 48)
#if (COMPOSITION_FLOW & 9) == 9
#if (COMPOSITION_VARIANT & 4096)
public typedef SubsurfaceSwitch SelectedBase;
#else
public typedef SubsurfaceBlend SelectedBase;
#endif
#elif COMPOSITION_FLOW & 8
public typedef Subsurface SelectedBase;
#elif COMPOSITION_FLOW & 1
public typedef Lambert SelectedBase;
#else
public typedef Absorber SelectedBase;
#endif
#if (COMPOSITION_FLOW & 192) == 192
public typedef MixedMeasure SelectedMeasure;
#elif COMPOSITION_FLOW & 128
public typedef Continuous SelectedMeasure;
#else
public typedef Smooth SelectedMeasure;
#endif
#if COMPOSITION_FLOW & 2
#if COMPOSITION_FLOW & 9
public typedef DielectricLayer<SelectedMeasure, SelectedBase> Dielectric;
#elif COMPOSITION_FLOW & 128
public typedef Specular<SelectedMeasure> Dielectric;
#else
public typedef DeltaDielectric Dielectric;
#endif
#else
public typedef BaseClosure<SelectedBase> Dielectric;
#endif
public typedef Conductor<SelectedMeasure> Metal;
#if COMPOSITION_FLOW & 2048
#if COMPOSITION_FLOW & 11
#if (COMPOSITION_FLOW & 130) == 130
public typedef SharedMetalBlend<SelectedMeasure, Dielectric> Opaque;
#else
public typedef MetalBlend<Dielectric, Metal> Opaque;
#endif
#else
public typedef NullMetal<Metal> Opaque;
#endif
#elif COMPOSITION_FLOW & 4
public typedef Metal Opaque;
#else
public typedef Dielectric Opaque;
#endif
#endif
#endif
}
