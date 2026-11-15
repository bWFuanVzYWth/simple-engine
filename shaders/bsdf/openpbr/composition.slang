// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 白名单使用的数值部件；状态由实际组合决定，不包含能力掩码或未使用的lobe。
module composition;
import bsdf.openpbr.selection;
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
    static func prepare(color: float3, weight: float)->State;
    static func energy(wi: float3, state: State)->float3;
    static func evaluate(wi: float3, wo: float3, state: State)->Value;
    static func sample(wi: float3, u: float3, state: State)->Proposal;
};
public struct Absorber : IBase
{
    public typedef Empty State;
    public static func prepare(color: float3, weight: float)->State
    {
        return {};
    }
    public static func energy(wi: float3, state: State)->float3
    {
        return 0;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->Value
    {
        return {};
    }
    public static func sample(wi: float3, u: float3, state: State)->Proposal
    {
        return primeFullOpenPbrZeroSample();
    }
};
public struct Lambert : IBase
{
    public typedef float3 State;
    public static func prepare(color: float3, weight: float)->State
    {
        return color;
    }
    public static func energy(wi: float3, state: State)->float3
    {
        return wi.z >= 0 ? state : float3(0);
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->Value
    {
        if (wi.z < 0 || wo.z < 0)
            return {};
        let pdf : float = PRIME_FULL_OPENPBR_INV_PI * wo.z;
        return { { state * pdf, PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION }, pdf };
    }
    public static func sample(wi: float3, u: float3, state: State)->Proposal
    {
        var p : Proposal = primeFullOpenPbrZeroSample();
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
    public static func prepare(color: float3, weight: float)->State
    {
        return primeFullPtSubsurfaceColor(color);
    }
    public static func energy(wi: float3, state: State)->float3
    {
        return state;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->Value
    {
        let reflected : bool = wi.z * wo.z > 0;
        let cosine : float = PRIME_FULL_OPENPBR_INV_PI * abs(wo.z);
        return { { float3(cosine) * state * 0.5 *
                       (reflected ? 1.3333333730697632 : 0.6666666269302368),
                   reflected ? PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION
                             : PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION },
                 cosine * (reflected ? 0.6666666865348816 : 0.3333333134651184) };
    }
    public static func sample(wi: float3, u: float3, state: State)->Proposal
    {
        var p : Proposal = primeFullOpenPbrZeroSample();
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
    public var color : float3;
    public var subsurface : bool;
};
public struct SubsurfaceSwitch : IBase
{
    public typedef BaseSwitchState State;
    public static func prepare(color: float3, weight: float)->State
    {
        return { weight == 1 ? Subsurface.prepare(color, 1) : color, weight == 1 };
    }
    public static func energy(wi: float3, s: State)->float3
    {
        return s.subsurface ? Subsurface.energy(wi, s.color) : Lambert.energy(wi, s.color);
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        return s.subsurface ? Subsurface.evaluate(wi, wo, s.color)
                            : Lambert.evaluate(wi, wo, s.color);
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        return s.subsurface ? Subsurface.sample(wi, u, s.color) : Lambert.sample(wi, u, s.color);
    }
};
public struct BaseBlendState
{
    public var diffuse : float3;
    public var subsurface : float3;
    public var weight : float;
    public var proposal : PrimeFullCompactMixState;
};
public struct SubsurfaceBlend : IBase
{
    public typedef BaseBlendState State;
    public static func prepare(color: float3, weight: float)->State
    {
        let sss : float3 = weight > 0 ? Subsurface.prepare(color, 1) : float3(0);
        return { color, sss, weight, primeFullCompactMakeMixState(weight, color, sss) };
    }
    public static func energy(wi: float3, s: State)->float3
    {
        return s.diffuse * (1 - s.weight) + s.subsurface * s.weight;
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        let a : Value = s.weight < 1 ? Lambert.evaluate(wi, wo, s.diffuse) : Value();
        let b : Value = s.weight > 0 ? Subsurface.evaluate(wi, wo, s.subsurface) : Value();
        return { primeFullCompactMixEval(a.throughput, b.throughput, s.weight),
                 s.proposal.firstSampleWeight + s.proposal.secondSampleWeight > 0
                     ? lerp(a.pdf, b.pdf, s.proposal.secondSampleWeight)
                     : 0 };
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        if (s.weight == 0)
            return Lambert.sample(wi, u, s.diffuse);
        if (s.weight == 1)
            return Subsurface.sample(wi, u, s.subsurface);
        if (s.proposal.firstSampleWeight + s.proposal.secondSampleWeight <= 0)
            return primeFullOpenPbrZeroSample();
        if (u.z <= s.proposal.secondSampleWeight && s.proposal.secondSampleWeight > 0)
        {
            var p : Proposal = Subsurface.sample(
                        wi, float3(u.xy, u.z / s.proposal.secondSampleWeight), s.subsurface);
            p.throughput.value *= s.weight;
            p.pdf *= s.proposal.secondSampleWeight;
            return p;
        }
        // Fractional SSS 保留参考 EON 的随机映射，包括零概率端点。
        let z : float = (u.z - s.proposal.secondSampleWeight) / s.proposal.firstSampleWeight;
        var p : Proposal = primeFullOpenPbrZeroSample();
        if (wi.z < 0)
            return p;
        if (z <= 0)
        {
            p.wo = primeFullOpenPbrUniformSampleHemisphere(u.xy);
            p.pdf = p.wo.z * PRIME_FULL_OPENPBR_INV_PI;
        }
        else
        {
            let radius : float = sqrt(u.x);
            let phi : float = 2 * PRIME_FULL_OPENPBR_PI * u.y;
            let x : float = -radius * cos(phi);
            let y : float = radius * sin(phi);
            var wo = float3(x, y, sqrt(max(1 - (x * x + y * y), 0)));
            let n : float = length(wo);
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
    static func has_delta()->bool;
    static func prepare(roughness: float)->State;
    static func prepare_alpha(alpha: float)->State;
    static func distribution(s: State)->PrimeFullOpenPbrMicrofacet;
    static func delta(s: State)->bool;
    static func evaluate(wi: float3, wo: float3, s: State)->PrimeFullOpenPbrReflectiveEvalBase;
    static func sample(wi: float3, u: float3, s: State)->PrimeFullOpenPbrReflectiveSampleBase;
    static func conductor_energy(wi: float3, s: State)->float;
    static func dielectric_energy<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P, wi: float3,
                                                                              ior: float, s: State)
        ->float3;
};
public struct Smooth : IMeasure
{
    public static func has_delta()->bool
    {
        return true;
    }
    public typedef Empty State;
    public static func prepare(roughness: float)->State
    {
        return {};
    }
    public static func prepare_alpha(alpha: float)->State
    {
        return {};
    }
    public static func distribution(s: State)->PrimeFullOpenPbrMicrofacet
    {
        return {};
    }
    public static func delta(s: State)->bool
    {
        return true;
    }
    public static func evaluate(wi: float3, wo: float3, s: State)
        ->PrimeFullOpenPbrReflectiveEvalBase
    {
        return {};
    }
    public static func sample(wi: float3, u: float3, s: State)->PrimeFullOpenPbrReflectiveSampleBase
    {
        return primeFullOpenPbrReflectiveSampleBase(wi, u, distribution(s),
                                                    PRIME_FULL_OPENPBR_FLAG_ALL,
                                                    PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    }
    public static func conductor_energy(wi: float3, s: State)->float
    {
        return 1;
    }
    public static func dielectric_energy<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                                     wi: float3,
                                                                                     ior: float,
                                                                                     s: State)
        ->float3
    {
        return primeFullOpenPbrMicrofacetDielectricMsCompensation<true>(energy, distribution(s),
                                                                        wi.z, ior);
    }
};
public struct Continuous : IMeasure
{
    public static func has_delta()->bool
    {
        return false;
    }
    public typedef float State;
    public static func prepare(roughness: float)->State
    {
        return roughness * roughness;
    }
    public static func prepare_alpha(alpha: float)->State
    {
        return alpha;
    }
    public static func distribution(s: State)->PrimeFullOpenPbrMicrofacet
    {
        return { float2(s) };
    }
    public static func delta(s: State)->bool
    {
        return false;
    }
    public static func evaluate(wi: float3, wo: float3, s: State)
        ->PrimeFullOpenPbrReflectiveEvalBase
    {
        return primeFullOpenPbrReflectiveEvalBaseImpl<true>(wi, wo, distribution(s),
                                                            PRIME_FULL_OPENPBR_FLAG_ALL,
                                                            PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    }
    public static func sample(wi: float3, u: float3, s: State)->PrimeFullOpenPbrReflectiveSampleBase
    {
        return primeFullOpenPbrReflectiveSampleBaseImpl<true>(wi, u, distribution(s),
                                                              PRIME_FULL_OPENPBR_FLAG_ALL,
                                                              PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    }
    public static func conductor_energy(wi: float3, s: State)->float
    {
        return primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(distribution(s), wi.z,
                                                                     float3(1), float3(1))
            .x;
    }
    public static func dielectric_energy<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                                     wi: float3,
                                                                                     ior: float,
                                                                                     s: State)
        ->float3
    {
        return primeFullOpenPbrMicrofacetDielectricMsCompensation<false>(energy, distribution(s),
                                                                         wi.z, ior);
    }
};
public struct MixedMeasure : IMeasure
{
    public static func has_delta()->bool
    {
        return true;
    }
    public typedef float State;
    public static func prepare(roughness: float)->State
    {
        return roughness * roughness;
    }
    public static func prepare_alpha(alpha: float)->State
    {
        return alpha;
    }
    public static func distribution(s: State)->PrimeFullOpenPbrMicrofacet
    {
        return { float2(s) };
    }
    public static func delta(s: State)->bool
    {
        return s < 1.0e-4;
    }
    public static func evaluate(wi: float3, wo: float3, s: State)
        ->PrimeFullOpenPbrReflectiveEvalBase
    {
        return primeFullOpenPbrReflectiveEvalBaseImpl<false>(wi, wo, distribution(s),
                                                             PRIME_FULL_OPENPBR_FLAG_ALL,
                                                             PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    }
    public static func sample(wi: float3, u: float3, s: State)->PrimeFullOpenPbrReflectiveSampleBase
    {
        return primeFullOpenPbrReflectiveSampleBaseImpl<false>(wi, u, distribution(s),
                                                               PRIME_FULL_OPENPBR_FLAG_ALL,
                                                               PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    }
    public static func conductor_energy(wi: float3, s: State)->float
    {
        return delta(s) ? 1 : Continuous.conductor_energy(wi, s);
    }
    public static func dielectric_energy<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                                     wi: float3,
                                                                                     ior: float,
                                                                                     s: State)
        ->float3
    {
        return Continuous.dielectric_energy(energy, wi, ior, s);
    }
};

public interface IClosure
{
    associatedtype State;
    static func has_delta()->bool;
    static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P, m: openpbr::Material,
                                                                    wi: float3, incident: float)
        ->State;
    static func evaluate(wi: float3, wo: float3, s: State)->Value;
    /// 单个proposal；外层组合在连续样本上统一恢复marginal响应和PDF。
    static func sample(wi: float3, u: float3, s: State)->Proposal;
    static func delta_response(wi: float3, s: State)->float3;
    static func delta_pdf(s: State)->float;
    static func marginal_flags(s: State)->bool;
    static func marginal_required(s: State)->bool;
};
/// 共享同一微表面分布的闭包只组合响应，不重复计算GGX几何项。
public interface IReflectionClosure : IClosure
{
    static func from_reflection(wi: float3, wo: float3, s: State,
                                reflection: PrimeFullOpenPbrReflectiveEvalBase)
        ->Value;
};
public struct ConductorState<G : IMeasure>
{
    public var geometry : G.State;
    public var color : float3;
    public var ess : float;
};
public func conductor_fresnel(color: float3)->PrimeFullOpenPbrConductorFresnel
{
    return { color, float3(1), float3(1), 1, color };
}
public struct Conductor<G : IMeasure> : IReflectionClosure
{
    public static func has_delta()->bool
    {
        return G.has_delta();
    }
    public typedef ConductorState<G> State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                           m: openpbr::Material,
                                                                           wi: float3,
                                                                           incident: float)
        ->State
    {
        let g : G.State = G.prepare(m.base_roughness.w);
        return { g, saturate(m.base_roughness.xyz), G.conductor_energy(wi, g) };
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        return from_reflection(wi, wo, s, G.evaluate(wi, wo, s.geometry));
    }
    public static func from_reflection(wi: float3, wo: float3, s: State,
                                       b: PrimeFullOpenPbrReflectiveEvalBase)
        ->Value
    {
        if (b.flags == 0)
            return {};
        var value : float3 = primeFullOpenPbrConductorFresnelUnpolarized(conductor_fresnel(s.color),
                                                                         b.microCosine) *
                             b.factor;
        value *= 1 + s.color * (1 - s.ess) / s.ess;
        return { { value, b.flags }, b.pdf };
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        let b : PrimeFullOpenPbrReflectiveSampleBase = G.sample(wi, u, s.geometry);
        var p : Proposal = b.bsdfSample;
        p.throughput.value *=
            primeFullOpenPbrConductorFresnelUnpolarized(conductor_fresnel(s.color), b.microCosine);
        if (!G.has_delta() || primeFullOpenPbrIsNonDelta(p.throughput.flags))
            p.throughput.value *= 1 + s.color * (1 - s.ess) / s.ess;
        return p;
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return primeFullOpenPbrConductorFresnelUnpolarized(conductor_fresnel(s.color), wi.z);
    }
    public static func delta_pdf(s: State)->float
    {
        return 1;
    }
    public static func marginal_flags(s: State)->bool
    {
        return false;
    }
    public static func marginal_required(s: State)->bool
    {
        return false;
    }
};
public struct LayerState<G : IMeasure, B : IBase>
{
    public var geometry : G.State;
    public var base : B.State;
    public var ior : float;
    public var compensation : float3;
    public var mixture : PrimeFullCompactLayerState;
};
public func dielectric_fresnel(ior: float)->PrimeFullOpenPbrSpecularFresnel
{
    return { ior, primeFullOpenPbrIorAdjustment(ior, 1), float3(1) };
}
public struct DeltaDielectric : IClosure
{
    public static func has_delta()->bool
    {
        return true;
    }
    public typedef float State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                           m: openpbr::Material,
                                                                           wi: float3,
                                                                           incident: float)
        ->State
    {
        return m.surface.x / incident;
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        return {};
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        var p : Proposal = primeFullOpenPbrZeroSample();
        if (wi.z > 0 && s != 1)
        {
            p.wo = reflect(-wi, float3(0, 0, 1));
            p.throughput = { delta_response(wi, s), PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION };
            p.pdf = 1;
        }
        return p;
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s), wi.z);
    }
    public static func delta_pdf(s: State)->float
    {
        return s != 1 ? 1 : 0;
    }
    public static func marginal_flags(s: State)->bool
    {
        return false;
    }
    public static func marginal_required(s: State)->bool
    {
        return false;
    }
};
public struct SpecularState<G : IMeasure>
{
    public var geometry : G.State;
    public var ior : float;
    public var compensation : float;
    public var available : bool;
};
public struct Specular<G : IMeasure> : IReflectionClosure
{
    public static func has_delta()->bool
    {
        return G.has_delta();
    }
    public typedef SpecularState<G> State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                           m: openpbr::Material,
                                                                           wi: float3,
                                                                           incident: float)
        ->State
    {
        let g : G.State = G.prepare(m.base_roughness.w);
        let ior : float = m.surface.x / incident;
        let ms : float3 = G.dielectric_energy(energy, wi, primeFullOpenPbrIorAdjustment(ior, 1), g);
        return { g, ior, ms.x, ms.z > 0 };
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        return from_reflection(wi, wo, s, G.evaluate(wi, wo, s.geometry));
    }
    public static func from_reflection(wi: float3, wo: float3, s: State,
                                       b: PrimeFullOpenPbrReflectiveEvalBase)
        ->Value
    {
        if (b.flags == 0)
            return {};
        return { { primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s.ior),
                                                              b.microCosine) *
                       b.factor * s.compensation,
                   b.flags },
                 s.available ? b.pdf : 0 };
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        if (!s.available)
            return primeFullOpenPbrZeroSample();
        let b : PrimeFullOpenPbrReflectiveSampleBase = G.sample(wi, u, s.geometry);
        var p : Proposal = b.bsdfSample;
        p.throughput.value *=
            primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s.ior), b.microCosine);
        if (!G.has_delta() || primeFullOpenPbrIsNonDelta(p.throughput.flags))
            p.throughput.value *= s.compensation;
        return p;
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s.ior), wi.z);
    }
    public static func delta_pdf(s: State)->float
    {
        return s.available ? 1 : 0;
    }
    public static func marginal_flags(s: State)->bool
    {
        return false;
    }
    public static func marginal_required(s: State)->bool
    {
        return true;
    }
};
public struct DielectricLayer<G : IMeasure, B : IBase> : IReflectionClosure
{
    public static func has_delta()->bool
    {
        return G.has_delta();
    }
    public typedef LayerState<G, B> State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                           m: openpbr::Material,
                                                                           wi: float3,
                                                                           incident: float)
        ->State
    {
        var s : State;
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
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        return from_reflection(wi, wo, s, G.evaluate(wi, wo, s.geometry));
    }
    public static func from_reflection(wi: float3, wo: float3, s: State,
                                       reflected: PrimeFullOpenPbrReflectiveEvalBase)
        ->Value
    {
        let base : Value = B.evaluate(wi, wo, s.base);
        var specular : PrimeFullOpenPbrThroughput = primeFullOpenPbrZeroThroughput();
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
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        if (s.mixture.baseSampleWeight + s.mixture.specularSampleWeight <= 0)
            return primeFullOpenPbrZeroSample();
        if (u.z <= s.mixture.specularSampleWeight && s.mixture.specularSampleWeight > 0)
        {
            let b
                : PrimeFullOpenPbrReflectiveSampleBase =
                      G.sample(wi, float3(u.xy, u.z / s.mixture.specularSampleWeight), s.geometry);
            var p : Proposal = b.bsdfSample;
            p.throughput.value *= primeFullOpenPbrSpecularFresnelUnpolarized(
                dielectric_fresnel(s.ior), b.microCosine);
            if (!G.has_delta() || primeFullOpenPbrIsNonDelta(p.throughput.flags))
                p.throughput.value *= s.compensation.x;
            p.pdf *= s.mixture.specularSampleWeight;
            return p;
        }
        var p : Proposal = B.sample(wi,
                                    float3(u.xy, (u.z - s.mixture.specularSampleWeight) /
                                                     s.mixture.baseSampleWeight),
                                    s.base);
        p.throughput = primeFullOpenPbrThroughputScale(p.throughput, s.mixture.transmittanceIn);
        p.pdf *= s.mixture.baseSampleWeight;
        return p;
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return primeFullOpenPbrSpecularFresnelUnpolarized(dielectric_fresnel(s.ior), wi.z);
    }
    public static func delta_pdf(s: State)->float
    {
        return s.mixture.specularSampleWeight;
    }
    public static func marginal_flags(s: State)->bool
    {
        return false;
    }
    public static func marginal_required(s: State)->bool
    {
        return true;
    }
};
public struct BaseClosure<B : IBase> : IClosure
{
    public static func has_delta()->bool
    {
        return false;
    }
    public typedef B.State State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                           m: openpbr::Material,
                                                                           wi: float3,
                                                                           incident: float)
        ->State
    {
        return B.prepare(m.base_roughness.xyz, m.surface.z);
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        var v : Value = B.evaluate(wi, wo, s);
        if (!any(B.energy(wi, s) > 0))
            v.pdf = 0;
        return v;
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        return any(B.energy(wi, s) > 0) ? B.sample(wi, u, s) : primeFullOpenPbrZeroSample();
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return 0;
    }
    public static func delta_pdf(s: State)->float
    {
        return 0;
    }
    public static func marginal_flags(s: State)->bool
    {
        return false;
    }
    public static func marginal_required(s: State)->bool
    {
        return true;
    }
};
public struct MetalBlendState<L : IClosure, M : IClosure>
{
    public var dielectric : L.State;
    public var conductor : M.State;
    public var metalness : float;
};
public struct MetalBlend<L : IClosure, M : IClosure> : IClosure
{
    public static func has_delta()->bool
    {
        return L.has_delta() || M.has_delta();
    }
    public typedef MetalBlendState<L, M> State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                           m: openpbr::Material,
                                                                           wi: float3,
                                                                           incident: float)
        ->State
    {
        return { L.prepare(energy, m, wi, incident), M.prepare(energy, m, wi, incident),
                 m.surface.y };
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        let a : Value = L.evaluate(wi, wo, s.dielectric);
        let b : Value = M.evaluate(wi, wo, s.conductor);
        return { primeFullCompactMixEval(a.throughput, b.throughput, s.metalness),
                 lerp(a.pdf, b.pdf, s.metalness) };
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return lerp(L.delta_response(wi, s.dielectric), M.delta_response(wi, s.conductor),
                    s.metalness);
    }
    public static func delta_pdf(s: State)->float
    {
        return s.metalness + (1 - s.metalness) * L.delta_pdf(s.dielectric);
    }
    public static func marginal_flags(s: State)->bool
    {
        return s.metalness > 0 && s.metalness < 1;
    }
    public static func marginal_required(s: State)->bool
    {
        return s.metalness < 1;
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        var p : Proposal;
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
    public static func has_delta()->bool
    {
        return G.has_delta();
    }
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                           m: openpbr::Material,
                                                                           wi: float3,
                                                                           incident: float)
        ->State
    {
        return Mixture.prepare(energy, m, wi, incident);
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        let reflection : PrimeFullOpenPbrReflectiveEvalBase =
                             G.evaluate(wi, wo, s.conductor.geometry);
        let dielectric : Value = D.from_reflection(wi, wo, s.dielectric, reflection);
        let metal : Value = Conductor<G>.from_reflection(wi, wo, s.conductor, reflection);
        return { primeFullCompactMixEval(dielectric.throughput, metal.throughput, s.metalness),
                 lerp(dielectric.pdf, metal.pdf, s.metalness) };
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        return Mixture.sample(wi, u, s);
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return Mixture.delta_response(wi, s);
    }
    public static func delta_pdf(s: State)->float
    {
        return Mixture.delta_pdf(s);
    }
    public static func marginal_flags(s: State)->bool
    {
        return Mixture.marginal_flags(s);
    }
    public static func marginal_required(s: State)->bool
    {
        return Mixture.marginal_required(s);
    }
};
public struct NullMetalState<M : IClosure>
{
    public var metal : M.State;
    public var weight : float;
};
/// 黑色非金属端仍保留null proposal，不重新归一化金属提案。
public struct NullMetal<M : IClosure> : IClosure
{
    public static func has_delta()->bool
    {
        return M.has_delta();
    }
    public typedef NullMetalState<M> State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P,
                                                                           m: openpbr::Material,
                                                                           wi: float3,
                                                                           incident: float)
        ->State
    {
        return { M.prepare(energy, m, wi, incident), m.surface.y };
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        var v : Value = M.evaluate(wi, wo, s.metal);
        v.throughput.value *= s.weight;
        v.pdf *= s.weight;
        return v;
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        if (u.z >= s.weight)
            return primeFullOpenPbrZeroSample();
        var p : Proposal = M.sample(wi, float3(u.xy, u.z / s.weight), s.metal);
        p.throughput.value *= s.weight;
        p.pdf *= s.weight;
        if (s.weight < 1 && (!M.has_delta() || primeFullOpenPbrIsNonDelta(p.throughput.flags)))
            p.throughput.flags |= PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION;
        return p;
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return M.delta_response(wi, s.metal) * s.weight;
    }
    public static func delta_pdf(s: State)->float
    {
        return s.weight;
    }
    public static func marginal_flags(s: State)->bool
    {
        return false;
    }
    public static func marginal_required(s: State)->bool
    {
        return s.weight < 1;
    }
};
public func sample_closure<C : IClosure>(wi: float3, u: float3, s: C.State)->Proposal
{
    var p : Proposal = C.sample(wi, u, s);
    if (C.marginal_required(s) && primeFullOpenPbrHasSample(p) &&
        (!C.has_delta() || primeFullOpenPbrIsNonDelta(p.throughput.flags)))
    {
        let v : Value = C.evaluate(wi, p.wo, s);
        if (C.marginal_flags(s))
            p.throughput = v.throughput;
        else
            p.throughput.value = v.throughput.value;
        p.pdf = v.pdf;
    }
    return p;
};

/// 先选择组合，再实例化其闭包约束；未使用的共享反射组合无需满足额外约束。
public struct MetalMixture<let Shared : bool, G : IMeasure, B : IClosure>
{
};
public extension<G : IMeasure, B : IReflectionClosure> MetalMixture<true, G, B>
{
    public typedef SharedMetalBlend<G, B> Type;
};
public extension<G : IMeasure, B : IClosure> MetalMixture<false, G, B>
{
    public typedef MetalBlend<B, Conductor<G>> Type;
};

static const let BaseIsMixed : bool = (SelectedFlow & 9) == 9;
static const let BaseIsBinary : bool = (SelectedVariant & 4096) != 0;
static const let BaseHasDiffuse : bool = (SelectedFlow & 1) != 0;
static const let BaseHasSubsurface : bool = (SelectedFlow & 8) != 0;
typealias WeightedBase = TypeSelect<BaseIsBinary, SubsurfaceSwitch, SubsurfaceBlend>.Type;
typealias DiffuseBase = TypeSelect<BaseHasDiffuse, Lambert, Absorber>.Type;
typealias UnmixedBase = TypeSelect<BaseHasSubsurface, Subsurface, DiffuseBase>.Type;
public typealias SelectedBase = TypeSelect<BaseIsMixed, WeightedBase, UnmixedBase>.Type;

static const let ReflectionIsMixed : bool = (SelectedFlow & 192) == 192;
static const let ReflectionIsContinuous : bool = (SelectedFlow & 128) != 0;
typealias SingleMeasure = TypeSelect<ReflectionIsContinuous, Continuous, Smooth>.Type;
public typealias SelectedMeasure = TypeSelect<ReflectionIsMixed, MixedMeasure, SingleMeasure>.Type;

static const let OpaqueHasReflection : bool = (SelectedFlow & 2) != 0;
static const let OpaqueHasBase : bool = (SelectedFlow & 9) != 0;
static const let OpaqueHasDielectric : bool = (SelectedFlow & 11) != 0;
static const let OpaqueHasMetal : bool = (SelectedFlow & 4) != 0;
static const let OpaqueIsMixture : bool = (SelectedFlow & 2048) != 0;
static const let OpaqueSharesReflection : bool = (SelectedFlow & 130) == 130;
typealias SpecularOnly =
    TypeSelect<ReflectionIsContinuous, Specular<SelectedMeasure>, DeltaDielectric>.Type;
typealias ReflectingDielectric =
    TypeSelect<OpaqueHasBase, DielectricLayer<SelectedMeasure, SelectedBase>, SpecularOnly>.Type;
public typealias Dielectric =
    TypeSelect<OpaqueHasReflection, ReflectingDielectric, BaseClosure<SelectedBase>>.Type;
public typealias Metal = Conductor<SelectedMeasure>;
typealias MixedOpaque = MetalMixture<OpaqueSharesReflection, SelectedMeasure, Dielectric>.Type;
typealias WeightedOpaque = TypeSelect<OpaqueHasDielectric, MixedOpaque, NullMetal<Metal>>.Type;
typealias UnmixedOpaque = TypeSelect<OpaqueHasMetal, Metal, Dielectric>.Type;
public typealias Opaque = TypeSelect<OpaqueIsMixture, WeightedOpaque, UnmixedOpaque>.Type;
}
