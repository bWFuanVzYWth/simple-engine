// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// Solid/Thin白名单数值部件。连续折射公式据RoboCute适配，来源见assets/NOTICE.md；第三方源保持不变。
module transmission_composition;
import bsdf.openpbr.selection;
import bsdf.openpbr.composition;
import bsdf.openpbr.primary_policy;
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/delta.slang";
import "bsdf/openpbr/math/microfacet_distribution.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/dielectric/refract.slang";
namespace openpbr_composition
{
public interface ITransmission
{
    associatedtype State;
    static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(energy: P, roughness: float,
                                                                    ior: float, extinction: float3,
                                                                    wi: float3)
        ->State;
    static func evaluate<let Balanced : bool = false>(wi: float3, wo: float3, s: State)->Value;
    static func sample<let Balanced : bool = false>(wi: float3, u: float3, s: State)->Proposal;
};
public struct SolidDelta : ITransmission
{
    public typedef float State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, roughness: float, ior: float, extinction: float3, wi: float3)
        ->State
    {
        return ior;
    }
    public static func evaluate<let Balanced : bool = false>(wi: float3, wo: float3, s: State)
        ->Value
    {
        return {};
    }
    public static func sample<let Balanced : bool = false>(wi: float3, u: float3, s: State)
        ->Proposal
    {
        return primeFullRefractiveDeltaSample<Balanced>(wi, u.z, dielectric_fresnel(s),
                                                        PRIME_FULL_OPENPBR_FLAG_ALL);
    }
};
public struct SolidState
{
    public var alpha : float;
    public var ior : float;
    public var compensation : float2;
};
public struct SolidContinuous : ITransmission
{
    public typedef SolidState State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, roughness: float, ior: float, extinction: float3, wi: float3)
        ->State
    {
        let alpha : float = roughness * roughness;
        let distribution : PrimeFullOpenPbrMicrofacet = { float2(alpha) };
        return {
            alpha, ior,
            primeFullOpenPbrMicrofacetDielectricMsCompensation(energy, distribution, wi.z, ior).xy
        };
    }
    public static func evaluate<let Balanced : bool = false>(wi: float3, wo: float3, s: State)
        ->Value
    {
        var v : Value = solid_evaluate<Balanced>(wi, wo, s.alpha, s.ior);
        if (v.throughput.flags != 0)
            v.throughput.value *= primeFullOpenPbrIsTransmissive(v.throughput.flags)
                                      ? s.compensation.y
                                      : s.compensation.x;
        return v;
    }
    public static func sample<let Balanced : bool = false>(wi: float3, u: float3, s: State)
        ->Proposal
    {
        var p : Proposal = solid_sample<Balanced>(wi, u, s.alpha, s.ior);
        if (p.throughput.flags != 0)
            p.throughput.value *= primeFullOpenPbrIsTransmissive(p.throughput.flags)
                                      ? s.compensation.y
                                      : s.compensation.x;
        return p;
    }
};
public struct SolidMixed : ITransmission
{
    public typedef SolidState State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, roughness: float, ior: float, extinction: float3, wi: float3)
        ->State
    {
        if (roughness * roughness < 1.0e-4 || ior == 1)
            return { roughness * roughness, ior, float2(1) };
        return SolidContinuous.prepare(energy, roughness, ior, extinction, wi);
    }
    public static func evaluate<let Balanced : bool = false>(wi: float3, wo: float3, s: State)
        ->Value
    {
        return s.alpha < 1.0e-4 || s.ior == 1 ? Value()
                                              : SolidContinuous.evaluate<Balanced>(wi, wo, s);
    }
    public static func sample<let Balanced : bool = false>(wi: float3, u: float3, s: State)
        ->Proposal
    {
        return s.alpha < 1.0e-4 || s.ior == 1 ? SolidDelta.sample<Balanced>(wi, u, s.ior)
                                              : SolidContinuous.sample<Balanced>(wi, u, s);
    }
};
/// 粗糙度已排除delta，只允许IOR匹配产生直通。
public struct SolidRoughMatched : ITransmission
{
    public typedef SolidState State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, roughness: float, ior: float, extinction: float3, wi: float3)
        ->State
    {
        return ior == 1 ? State(roughness * roughness, ior, float2(1))
                        : SolidContinuous.prepare(energy, roughness, ior, extinction, wi);
    }
    public static func evaluate<let Balanced : bool = false>(wi: float3, wo: float3, s: State)
        ->Value
    {
        return s.ior == 1 ? Value() : SolidContinuous.evaluate<Balanced>(wi, wo, s);
    }
    public static func sample<let Balanced : bool = false>(wi: float3, u: float3, s: State)
        ->Proposal
    {
        return s.ior == 1 ? SolidDelta.sample<Balanced>(wi, u, 1)
                          : SolidContinuous.sample<Balanced>(wi, u, s);
    }
};
public struct MatchedSolid : ITransmission
{
    public typedef Empty State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, roughness: float, ior: float, extinction: float3, wi: float3)
        ->State
    {
        return {};
    }
    public static func evaluate<let Balanced : bool = false>(wi: float3, wo: float3, s: State)
        ->Value
    {
        return {};
    }
    public static func sample<let Balanced : bool = false>(wi: float3, u: float3, s: State)
        ->Proposal
    {
        return SolidDelta.sample<Balanced>(wi, u, 1);
    }
};
public struct ThinDeltaState
{
    public var ior : float;
    public var extinction : float3;
};
public struct ThinDelta : ITransmission
{
    public typedef ThinDeltaState State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, roughness: float, ior: float, extinction: float3, wi: float3)
        ->State
    {
        return { ior, extinction };
    }
    public static func evaluate<let Balanced : bool = false>(wi: float3, wo: float3, s: State)
        ->Value
    {
        return {};
    }
    public static func sample<let Balanced : bool = false>(wi: float3, u: float3, s: State)
        ->Proposal
    {
        return primeFullThinWallDeltaSample<Balanced>(wi, u.z, dielectric_fresnel(s.ior), float3(1),
                                                      s.extinction, PRIME_FULL_OPENPBR_FLAG_ALL);
    }
};
public struct ThinState
{
    public var alpha : float;
    public var ior : float;
    public var extinction : float3;
};
public struct ThinRough<G : IMeasure> : ITransmission
{
    public typedef ThinState State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, roughness: float, ior: float, extinction: float3, wi: float3)
        ->State
    {
        return { clamp(roughness * roughness * primeFullOpenPbrThinDielectricRoughnessScaler2(ior),
                       0.0, 1.0),
                 ior, extinction };
    }
    public static func evaluate<let Balanced : bool = false>(wi: float3, wo: float3, s: State)
        ->Value
    {
        var wo = wo;
        let transmitted : bool = wo.z < 0;
        if (transmitted)
            wo.z = -wo.z;
        let b : PrimeFullOpenPbrReflectiveEvalBase = G.evaluate(wi, wo, G.prepare_alpha(s.alpha));
        var v : Value = {};
        if (b.flags != 0)
        {
            let rt : PrimeFullOpenPbrVecPair = primeFullThinWallRt(
                         b.microCosine, dielectric_fresnel(s.ior), float3(1), s.extinction);
            v.throughput = { float3(b.factor) * (transmitted ? rt.second : rt.first),
                             transmitted ? PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION
                                         : b.flags };
            v.throughput.value /= primeFullOpenPbrMicrofacetDirectionalAlbedoUnity(
                G.distribution(G.prepare_alpha(s.alpha)), wi.z);
        }
        let rt : PrimeFullOpenPbrVecPair =
                     primeFullThinWallRt(dot(wi, normalize(wi + wo)), dielectric_fresnel(s.ior),
                                         float3(1), s.extinction);
        var reflection : float = primeFullOpenPbrSpectrumToWeight(rt.first);
        var transmission : float = primeFullOpenPbrSpectrumToWeight(rt.second);
        if (Balanced)
        {
            let available : bool = reflection + transmission > 0;
            let probability : float =
                                  openpbr_primary::reflection_probability(reflection, transmission);
            reflection = available ? probability : 0;
            transmission = available ? 1 - probability : 0;
        }
        let sum : float = reflection + transmission;
        if (sum != 0)
            v.pdf = b.pdf * (transmitted ? transmission : reflection) / sum;
        return v;
    }
    public static func sample<let Balanced : bool = false>(wi: float3, u: float3, s: State)
        ->Proposal
    {
        let geometry : G.State = G.prepare_alpha(s.alpha);
        if (G.delta(geometry))
            return ThinDelta.sample<Balanced>(wi, u, { s.ior, s.extinction });
        let b : PrimeFullOpenPbrReflectiveSampleBase = G.sample(wi, u, geometry);
        var p : Proposal = b.bsdfSample;
        if (p.throughput.flags == 0)
            return p;
        let rt : PrimeFullOpenPbrVecPair = primeFullThinWallRt(
                     b.microCosine, dielectric_fresnel(s.ior), float3(1), s.extinction);
        var reflection : float = primeFullOpenPbrSpectrumToWeight(rt.first);
        var transmission : float = primeFullOpenPbrSpectrumToWeight(rt.second);
        if (Balanced)
        {
            let available : bool = reflection + transmission > 0;
            let probability : float =
                                  openpbr_primary::reflection_probability(reflection, transmission);
            reflection = available ? probability : 0;
            transmission = available ? 1 - probability : 0;
        }
        let sum : float = reflection + transmission;
        if (sum == 0)
            return primeFullOpenPbrZeroSample();
        if (u.z < transmission / sum)
        {
            p.throughput.value *= rt.second;
            p.pdf *= transmission / sum;
            p.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION;
            p.wo.z = -p.wo.z;
        }
        else
        {
            p.throughput.value *= rt.first;
            p.pdf *= reflection / sum;
        }
        p.throughput.value /=
            primeFullOpenPbrMicrofacetDirectionalAlbedoUnity(G.distribution(geometry), wi.z);
        return p;
    }
};
public struct MatchedThin : ITransmission
{
    public typedef float3 State;
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, roughness: float, ior: float, extinction: float3, wi: float3)
        ->State
    {
        return extinction;
    }
    public static func evaluate<let Balanced : bool = false>(wi: float3, wo: float3, s: State)
        ->Value
    {
        return {};
    }
    public static func sample<let Balanced : bool = false>(wi: float3, u: float3, s: State)
        ->Proposal
    {
        var p : Proposal = primeFullOpenPbrZeroSample();
        let response : float3 = wi.z > 0 ? exp(-s / wi.z) : float3(0);
        if (wi.z > 0 && primeFullOpenPbrSpectrumToWeight(response) > 0)
        {
            p.wo = -wi;
            p.throughput = { response, PRIME_FULL_OPENPBR_FLAG_DELTA_TRANSMISSION };
            p.pdf = 1;
        }
        return p;
    }
};
public func solid_evaluate<let Balanced : bool = false>(wi: float3, wo: float3, alpha: float,
                                                        ior: float)
    ->Value
{

    var result : Value = {};
    let microfacet = PrimeFullOpenPbrMicrofacet(float2(alpha));
    let fresnel : PrimeFullOpenPbrSpecularFresnel = dielectric_fresnel(ior);
    let reflected : bool = wi.z * wo.z > 0.0;
    let etaPath : float = reflected ? 1.0 : (wi.z > 0.0 ? fresnel.ior : 1.0 / fresnel.ior);
    var halfVector : float3 = wi + etaPath * wo;
    let lengthSquared : float = dot(halfVector, halfVector);
    if (wo.z == 0.0 || wi.z == 0.0 || lengthSquared == 0.0)
        return result;
    halfVector *= rsqrt(lengthSquared);
    if (halfVector.z <= 0.0)
        halfVector = -halfVector;
    let microIncident : float = dot(wi, halfVector);
    let microOutgoing : float = dot(wo, halfVector);
    if (microIncident * wi.z < 0.0 || microOutgoing * wo.z < 0.0)
        return result;
    let rt : PrimeFullOpenPbrVecPair = primeFullOpenPbrSpecularFresnelRt(fresnel, microIncident);
    let distribution : float = primeFullOpenPbrMicrofacetD(microfacet, halfVector);
    if (reflected)
    {
        result.throughput.value =
            rt.first * distribution * primeFullOpenPbrReflectionFactor(microfacet, wi, wo);
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION;
    }
    else if (!reflected)
    {
        let denominator : float = primeFullOpenPbrSquare(primeFullOpenPbrRefractionDenominator(
                                      etaPath, microIncident, microOutgoing)) *
                                  wi.z;
        result.throughput.value = rt.second * distribution *
                                  primeFullOpenPbrMicrofacetG2t(microfacet, wi, wo) *
                                  abs(microIncident * microOutgoing / denominator);
        result.throughput.value *= 1.0 / primeFullOpenPbrSquare(etaPath);
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION;
    }
    var reflectionProbability : float = primeFullOpenPbrSpectrumToWeight(rt.first);
    var transmissionProbability : float = primeFullOpenPbrSpectrumToWeight(rt.second);
    if (Balanced)
    {
        let available : bool = reflectionProbability + transmissionProbability > 0;
        let probability : float = openpbr_primary::reflection_probability(reflectionProbability,
                                                                          transmissionProbability);
        reflectionProbability = available ? probability : 0;
        transmissionProbability = available ? 1 - probability : 0;
    }
    let probabilitySum : float = reflectionProbability + transmissionProbability;
    if (probabilitySum == 0.0)
        return result;
    let microfacetPdf : float = primeFullOpenPbrMicrofacetPdfFromDistribution(
                            microfacet, wi.z > 0.0 ? wi : -wi, halfVector, false, distribution);
    if (reflected)
        result.pdf =
            microfacetPdf / (4.0 * abs(microIncident)) * reflectionProbability / probabilitySum;
    else
    {
        let denominator : float = primeFullOpenPbrSquare(primeFullOpenPbrRefractionDenominator(
                              etaPath, microIncident, microOutgoing));
        result.pdf = microfacetPdf * abs(microOutgoing) / denominator * transmissionProbability /
                     probabilitySum;
    }
    return result;
}

public func solid_sample<let Balanced : bool = false>(wi: float3, randomValue: float3, alpha: float,
                                                      ior: float)
    ->PrimeFullOpenPbrSample
{
    if (wi.z == 0)
        return primeFullOpenPbrZeroSample();
    let microfacet = PrimeFullOpenPbrMicrofacet(float2(alpha));
    let fresnel : PrimeFullOpenPbrSpecularFresnel = dielectric_fresnel(ior);
    var halfVector = float3(0.0, 0.0, 1.0);
    var microfacetPdf = 1.0;
    var distribution = 1.0;
    let sampleDirection : float3 = wi.z > 0.0 ? wi : -wi;
    halfVector =
        primeFullOpenPbrMicrofacetSample(microfacet, sampleDirection, randomValue.xy, false);
    microfacetPdf = primeFullOpenPbrMicrofacetPdf(microfacet, sampleDirection, halfVector, false);
    distribution = primeFullOpenPbrMicrofacetD(microfacet, halfVector);
    let microIncident : float = dot(halfVector, wi);
    let rt : PrimeFullOpenPbrVecPair = primeFullOpenPbrSpecularFresnelRt(fresnel, microIncident);
    var reflection : float3 = rt.first;
    let transmission : float3 = rt.second;
    var reflectionProbability : float = primeFullOpenPbrSpectrumToWeight(reflection);
    var transmissionProbability : float = primeFullOpenPbrSpectrumToWeight(transmission);
    let refracted : PrimeFullOpenPbrRefractResult =
                        primeFullOpenPbrSpecularFresnelRefract(fresnel, -wi, halfVector);
    if (refracted.valid == 0u)
    {
        transmissionProbability = 0.0;
        reflection = float3(1.0);
    }
    if (Balanced)
    {
        let available : bool = reflectionProbability + transmissionProbability > 0;
        let probability : float = openpbr_primary::reflection_probability(reflectionProbability,
                                                                          transmissionProbability);
        reflectionProbability = available ? probability : 0;
        transmissionProbability = available ? 1 - probability : 0;
    }
    let probabilitySum : float = reflectionProbability + transmissionProbability;
    if (probabilitySum == 0.0)
    {
        return primeFullOpenPbrZeroSample();
    }
    var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
    if (randomValue.z < transmissionProbability / probabilitySum)
    {
        var wo : float3 = refracted.wo;
        if (wi.z * wo.z >= 0.0)
        {
            return primeFullOpenPbrZeroSample();
        }
        let etaPath : float = refracted.relativeIor;
        result.wo = wo;
        result.eta = etaPath;
        result.throughput.value = transmission;
        result.pdf = transmissionProbability / probabilitySum;
        let microOutgoing : float = dot(wo, halfVector);
        let denominator : float = primeFullOpenPbrSquare(primeFullOpenPbrRefractionDenominator(
                              etaPath, microIncident, microOutgoing));
        result.pdf *= microfacetPdf * abs(microOutgoing) / denominator;
        result.throughput.value = transmission * distribution *
                                  primeFullOpenPbrMicrofacetG2t(microfacet, wi, wo) *
                                  abs(microIncident * microOutgoing / (wi.z * denominator));
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION;
        result.throughput.value *= 1.0 / primeFullOpenPbrSquare(etaPath);
    }
    else
    {
        var wo : float3 = reflect(-wi, halfVector);
        if (wi.z * wo.z < 0.0)
        {
            return primeFullOpenPbrZeroSample();
        }
        result.wo = wo;
        result.throughput.value = reflection;
        result.pdf = reflectionProbability / probabilitySum;
        result.pdf *= microfacetPdf / (4.0 * abs(microIncident));
        result.throughput.value =
            reflection * distribution * primeFullOpenPbrReflectionFactor(microfacet, wi, wo);
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION;
    }
    return result;
}

static const let TransmissionIsThin : bool = (SelectedFlow & 32) != 0;
static const let TransmissionIsMatched : bool = (SelectedFlow & 2) == 0;
static const let TransmissionIsDelta : bool = (SelectedFlow & 640) == 0;
static const let ThinHasDelta : bool = (SelectedFlow & 320) != 0;
static const let SolidHasDeltaReflection : bool = (SelectedFlow & 64) != 0;
static const let SolidHasMatchedDomain : bool = (SelectedFlow & 256) != 0;
typealias SelectedThinRough =
    TypeSelect<ThinHasDelta, ThinRough<MixedMeasure>, ThinRough<Continuous>>.Type;
typealias ReflectingThin = TypeSelect<TransmissionIsDelta, ThinDelta, SelectedThinRough>.Type;
typealias SelectedThin = TypeSelect<TransmissionIsMatched, MatchedThin, ReflectingThin>.Type;
typealias SelectedSolidRough =
    TypeSelect<SolidHasMatchedDomain, SolidRoughMatched, SolidContinuous>.Type;
typealias SelectedSolidMeasure =
    TypeSelect<SolidHasDeltaReflection, SolidMixed, SelectedSolidRough>.Type;
typealias ReflectingSolid = TypeSelect<TransmissionIsDelta, SolidDelta, SelectedSolidMeasure>.Type;
typealias SelectedSolid = TypeSelect<TransmissionIsMatched, MatchedSolid, ReflectingSolid>.Type;
public typealias Transmission = TypeSelect<TransmissionIsThin, SelectedThin, SelectedSolid>.Type;
}
