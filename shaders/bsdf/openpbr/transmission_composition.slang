// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// Solid/Thin白名单数值部件。连续折射公式据RoboCute适配，来源见assets/NOTICE.md；第三方源保持不变。
module transmission_composition;
import bsdf.openpbr.composition;
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
    static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(P energy, float roughness,
                                                                     float ior, float3 extinction,
                                                                     float3 wi);
    static Value evaluate(float3 wi, float3 wo, State s);
    static Proposal sample(float3 wi, float3 u, State s);
};
public struct SolidDelta : ITransmission
{
    public typedef float State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, float roughness, float ior, float3 extinction, float3 wi)
    {
        return ior;
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return {};
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        return primeFullRefractiveDeltaSample(wi, u.z, dielectric_fresnel(s),
                                              PRIME_FULL_OPENPBR_FLAG_ALL);
    }
};
public struct SolidState
{
    public float alpha;
    public float ior;
    public float2 compensation;
};
public struct SolidContinuous : ITransmission
{
    public typedef SolidState State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, float roughness, float ior, float3 extinction, float3 wi)
    {
        float alpha = roughness * roughness;
        PrimeFullOpenPbrMicrofacet distribution = { float2(alpha) };
        return {
            alpha, ior,
            primeFullOpenPbrMicrofacetDielectricMsCompensation(energy, distribution, wi.z, ior).xy
        };
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        Value v = solid_evaluate(wi, wo, s.alpha, s.ior);
        if (v.throughput.flags != 0)
            v.throughput.value *= primeFullOpenPbrIsTransmissive(v.throughput.flags)
                                      ? s.compensation.y
                                      : s.compensation.x;
        return v;
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        Proposal p = solid_sample(wi, u, s.alpha, s.ior);
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
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, float roughness, float ior, float3 extinction, float3 wi)
    {
        if (roughness * roughness < 1.0e-4 || ior == 1)
            return { roughness * roughness, ior, float2(1) };
        return SolidContinuous.prepare(energy, roughness, ior, extinction, wi);
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return s.alpha < 1.0e-4 || s.ior == 1 ? Value() : SolidContinuous.evaluate(wi, wo, s);
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        return s.alpha < 1.0e-4 || s.ior == 1 ? SolidDelta.sample(wi, u, s.ior)
                                              : SolidContinuous.sample(wi, u, s);
    }
};
/// 粗糙度已排除delta，只允许IOR匹配产生直通。
public struct SolidRoughMatched : ITransmission
{
    public typedef SolidState State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, float roughness, float ior, float3 extinction, float3 wi)
    {
        return ior == 1 ? State(roughness * roughness, ior, float2(1))
                        : SolidContinuous.prepare(energy, roughness, ior, extinction, wi);
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return s.ior == 1 ? Value() : SolidContinuous.evaluate(wi, wo, s);
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        return s.ior == 1 ? SolidDelta.sample(wi, u, 1) : SolidContinuous.sample(wi, u, s);
    }
};
public struct MatchedSolid : ITransmission
{
    public typedef Empty State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, float roughness, float ior, float3 extinction, float3 wi)
    {
        return {};
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return {};
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        return SolidDelta.sample(wi, u, 1);
    }
};
public struct ThinDeltaState
{
    public float ior;
    public float3 extinction;
};
public struct ThinDelta : ITransmission
{
    public typedef ThinDeltaState State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, float roughness, float ior, float3 extinction, float3 wi)
    {
        return { ior, extinction };
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return {};
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        return primeFullThinWallDeltaSample(wi, u.z, dielectric_fresnel(s.ior), float3(1),
                                            s.extinction, PRIME_FULL_OPENPBR_FLAG_ALL);
    }
};
public struct ThinState
{
    public float alpha;
    public float ior;
    public float3 extinction;
};
public struct ThinRough<G : IMeasure> : ITransmission
{
    public typedef ThinState State;
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, float roughness, float ior, float3 extinction, float3 wi)
    {
        return { clamp(roughness * roughness * primeFullOpenPbrThinDielectricRoughnessScaler2(ior),
                       0.0, 1.0),
                 ior, extinction };
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        bool transmitted = wo.z < 0;
        if (transmitted)
            wo.z = -wo.z;
        PrimeFullOpenPbrReflectiveEvalBase b = G.evaluate(wi, wo, G.prepare_alpha(s.alpha));
        Value v = {};
        if (b.flags != 0)
        {
            PrimeFullOpenPbrVecPair rt = primeFullThinWallRt(
                b.microCosine, dielectric_fresnel(s.ior), float3(1), s.extinction);
            v.throughput = { float3(b.factor) * (transmitted ? rt.second : rt.first),
                             transmitted ? PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION
                                         : b.flags };
            v.throughput.value /= primeFullOpenPbrMicrofacetDirectionalAlbedoUnity(
                G.distribution(G.prepare_alpha(s.alpha)), wi.z);
        }
        PrimeFullOpenPbrVecPair rt = primeFullThinWallRt(
            dot(wi, normalize(wi + wo)), dielectric_fresnel(s.ior), float3(1), s.extinction);
        float reflection = primeFullOpenPbrSpectrumToWeight(rt.first),
              transmission = primeFullOpenPbrSpectrumToWeight(rt.second);
        float sum = reflection + transmission;
        if (sum != 0)
            v.pdf = b.pdf * (transmitted ? transmission : reflection) / sum;
        return v;
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        G.State geometry = G.prepare_alpha(s.alpha);
        if (G.delta(geometry))
            return ThinDelta.sample(wi, u, { s.ior, s.extinction });
        PrimeFullOpenPbrReflectiveSampleBase b = G.sample(wi, u, geometry);
        Proposal p = b.bsdfSample;
        if (p.throughput.flags == 0)
            return p;
        PrimeFullOpenPbrVecPair rt =
            primeFullThinWallRt(b.microCosine, dielectric_fresnel(s.ior), float3(1), s.extinction);
        float reflection = primeFullOpenPbrSpectrumToWeight(rt.first),
              transmission = primeFullOpenPbrSpectrumToWeight(rt.second);
        float sum = reflection + transmission;
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
    public static State prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, float roughness, float ior, float3 extinction, float3 wi)
    {
        return extinction;
    }
    public static Value evaluate(float3 wi, float3 wo, State s)
    {
        return {};
    }
    public static Proposal sample(float3 wi, float3 u, State s)
    {
        Proposal p = primeFullOpenPbrZeroSample();
        float3 response = wi.z > 0 ? exp(-s / wi.z) : float3(0);
        if (wi.z > 0 && primeFullOpenPbrSpectrumToWeight(response) > 0)
        {
            p.wo = -wi;
            p.throughput = { response, PRIME_FULL_OPENPBR_FLAG_DELTA_TRANSMISSION };
            p.pdf = 1;
        }
        return p;
    }
};
public Value solid_evaluate(float3 wi, float3 wo, float alpha, float ior)
{
    Value result = {};
    PrimeFullOpenPbrMicrofacet microfacet = PrimeFullOpenPbrMicrofacet(float2(alpha));
    PrimeFullOpenPbrSpecularFresnel fresnel = dielectric_fresnel(ior);
    bool reflected = wi.z * wo.z > 0.0;
    float etaPath = reflected ? 1.0 : (wi.z > 0.0 ? fresnel.ior : 1.0 / fresnel.ior);
    float3 halfVector = wi + etaPath * wo;
    float lengthSquared = dot(halfVector, halfVector);
    if (wo.z == 0.0 || wi.z == 0.0 || lengthSquared == 0.0)
        return result;
    halfVector *= rsqrt(lengthSquared);
    if (halfVector.z <= 0.0)
        halfVector = -halfVector;
    float microIncident = dot(wi, halfVector);
    float microOutgoing = dot(wo, halfVector);
    if (microIncident * wi.z < 0.0 || microOutgoing * wo.z < 0.0)
        return result;
    PrimeFullOpenPbrVecPair rt = primeFullOpenPbrSpecularFresnelRt(fresnel, microIncident);
    float distribution = primeFullOpenPbrMicrofacetD(microfacet, halfVector);
    if (reflected)
    {
        result.throughput.value =
            rt.first * distribution * primeFullOpenPbrReflectionFactor(microfacet, wi, wo);
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION;
    }
    else if (!reflected)
    {
        float denominator = primeFullOpenPbrSquare(primeFullOpenPbrRefractionDenominator(
                                etaPath, microIncident, microOutgoing)) *
                            wi.z;
        result.throughput.value = rt.second * distribution *
                                  primeFullOpenPbrMicrofacetG2t(microfacet, wi, wo) *
                                  abs(microIncident * microOutgoing / denominator);
        result.throughput.value *= 1.0 / primeFullOpenPbrSquare(etaPath);
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION;
    }
    float reflectionProbability = primeFullOpenPbrSpectrumToWeight(rt.first);
    float transmissionProbability = primeFullOpenPbrSpectrumToWeight(rt.second);
    float probabilitySum = reflectionProbability + transmissionProbability;
    if (probabilitySum == 0.0)
        return result;
    float microfacetPdf = primeFullOpenPbrMicrofacetPdfFromDistribution(
        microfacet, wi.z > 0.0 ? wi : -wi, halfVector, false, distribution);
    if (reflected)
        result.pdf =
            microfacetPdf / (4.0 * abs(microIncident)) * reflectionProbability / probabilitySum;
    else
    {
        float denominator = primeFullOpenPbrSquare(
            primeFullOpenPbrRefractionDenominator(etaPath, microIncident, microOutgoing));
        result.pdf = microfacetPdf * abs(microOutgoing) / denominator * transmissionProbability /
                     probabilitySum;
    }
    return result;
}

public PrimeFullOpenPbrSample solid_sample(float3 wi, float3 randomValue, float alpha, float ior)
{
    if (wi.z == 0)
        return primeFullOpenPbrZeroSample();
    PrimeFullOpenPbrMicrofacet microfacet = PrimeFullOpenPbrMicrofacet(float2(alpha));
    PrimeFullOpenPbrSpecularFresnel fresnel = dielectric_fresnel(ior);
    float3 halfVector = float3(0.0, 0.0, 1.0);
    float microfacetPdf = 1.0;
    float distribution = 1.0;
    float3 sampleDirection = wi.z > 0.0 ? wi : -wi;
    halfVector =
        primeFullOpenPbrMicrofacetSample(microfacet, sampleDirection, randomValue.xy, false);
    microfacetPdf = primeFullOpenPbrMicrofacetPdf(microfacet, sampleDirection, halfVector, false);
    distribution = primeFullOpenPbrMicrofacetD(microfacet, halfVector);
    float microIncident = dot(halfVector, wi);
    PrimeFullOpenPbrVecPair rt = primeFullOpenPbrSpecularFresnelRt(fresnel, microIncident);
    float3 reflection = rt.first;
    float3 transmission = rt.second;
    float reflectionProbability = primeFullOpenPbrSpectrumToWeight(reflection);
    float transmissionProbability = primeFullOpenPbrSpectrumToWeight(transmission);
    PrimeFullOpenPbrRefractResult refracted =
        primeFullOpenPbrSpecularFresnelRefract(fresnel, -wi, halfVector);
    if (refracted.valid == 0u)
    {
        transmissionProbability = 0.0;
        reflection = float3(1.0);
    }
    float probabilitySum = reflectionProbability + transmissionProbability;
    if (probabilitySum == 0.0)
    {
        return primeFullOpenPbrZeroSample();
    }
    PrimeFullOpenPbrSample result = primeFullOpenPbrZeroSample();
    if (randomValue.z < transmissionProbability / probabilitySum)
    {
        float3 wo = refracted.wo;
        if (wi.z * wo.z >= 0.0)
        {
            return primeFullOpenPbrZeroSample();
        }
        float etaPath = refracted.relativeIor;
        result.wo = wo;
        result.eta = etaPath;
        result.throughput.value = transmission;
        result.pdf = transmissionProbability / probabilitySum;
        float microOutgoing = dot(wo, halfVector);
        float denominator = primeFullOpenPbrSquare(
            primeFullOpenPbrRefractionDenominator(etaPath, microIncident, microOutgoing));
        result.pdf *= microfacetPdf * abs(microOutgoing) / denominator;
        result.throughput.value = transmission * distribution *
                                  primeFullOpenPbrMicrofacetG2t(microfacet, wi, wo) *
                                  abs(microIncident * microOutgoing / (wi.z * denominator));
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION;
        result.throughput.value *= 1.0 / primeFullOpenPbrSquare(etaPath);
    }
    else
    {
        float3 wo = reflect(-wi, halfVector);
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

#if defined(OPENPBR_FLOW)
#define TRANSMISSION_FLOW OPENPBR_FLOW
#elif defined(GAME_FLOW)
#define TRANSMISSION_FLOW GAME_FLOW
#endif
#ifdef TRANSMISSION_FLOW
#if TRANSMISSION_FLOW & 16
#if !(TRANSMISSION_FLOW & 2)
public typedef MatchedSolid Transmission;
#elif !(TRANSMISSION_FLOW & 640)
public typedef SolidDelta Transmission;
#elif TRANSMISSION_FLOW & 64
public typedef SolidMixed Transmission;
#elif TRANSMISSION_FLOW & 256
public typedef SolidRoughMatched Transmission;
#else
public typedef SolidContinuous Transmission;
#endif
#elif TRANSMISSION_FLOW & 32
#if !(TRANSMISSION_FLOW & 2)
public typedef MatchedThin Transmission;
#elif !(TRANSMISSION_FLOW & 640)
public typedef ThinDelta Transmission;
#elif TRANSMISSION_FLOW & 320
public typedef ThinRough<MixedMeasure> Transmission;
#else
public typedef ThinRough<Continuous> Transmission;
#endif
#endif
#endif
}
