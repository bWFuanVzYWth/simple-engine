// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/opaque/lobes.slang";

import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/microfacet_distribution.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/microfacet_measure.slang";
import "bsdf/openpbr/opaque/discrete.slang";
import "bsdf/openpbr/opaque/state.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/math/spectrum.slang";

public PrimeFullCompactLayerState primeFullCompactMakeLayerState(float3 baseEnergy,
                                                                 float3 transmittanceIn,
                                                                 float3 specularEnergy)
{
    PrimeFullCompactLayerState result;
    result.transmittanceIn = transmittanceIn;
    result.baseSampleWeight = primeFullOpenPbrSpectrumToWeight(transmittanceIn * baseEnergy);
    result.specularSampleWeight = primeFullOpenPbrSpectrumToWeight(specularEnergy);
    float sum = result.baseSampleWeight + result.specularSampleWeight;
    if (sum > 0.0)
    {
        result.baseSampleWeight /= sum;
        result.specularSampleWeight /= sum;
    }
    return result;
}

public PrimeFullCompactMixState primeFullCompactMakeMixState(float weight, float3 firstEnergy,
                                                             float3 secondEnergy)
{
    PrimeFullCompactMixState result;
    if (weight == 0.0)
    {
        result.firstSampleWeight = 1.0;
        result.secondSampleWeight = 0.0;
        return result;
    }
    if (weight == 1.0)
    {
        result.firstSampleWeight = 0.0;
        result.secondSampleWeight = 1.0;
        return result;
    }
    result.firstSampleWeight = (1.0 - weight) * primeFullOpenPbrSpectrumToWeight(firstEnergy);
    result.secondSampleWeight = weight * primeFullOpenPbrSpectrumToWeight(secondEnergy);
    float sum = result.firstSampleWeight + result.secondSampleWeight;
    if (sum > 0.0)
    {
        result.firstSampleWeight /= sum;
        result.secondSampleWeight /= sum;
    }
    return result;
}

public PrimeFullOpenPbrThroughput primeFullCompactMixEval(PrimeFullOpenPbrThroughput first,
                                                          PrimeFullOpenPbrThroughput second,
                                                          float weight)
{
    PrimeFullOpenPbrThroughput result = primeFullOpenPbrZeroThroughput();
    if (weight < 1.0)
    {
        result = first;
    }
    if (weight > 0.0)
    {
        result.value *= 1.0 - weight;
        second.value *= weight;
        result = primeFullOpenPbrThroughputAdd(result, second);
    }
    return result;
}

public PrimeFullOpenPbrThroughput primeFullCompactLayerEval(PrimeFullOpenPbrThroughput base,
                                                            PrimeFullOpenPbrThroughput specular,
                                                            PrimeFullCompactLayerState layer)
{
    PrimeFullOpenPbrThroughput result =
        primeFullOpenPbrThroughputScale(base, layer.transmittanceIn);
    return primeFullOpenPbrThroughputAdd(result, specular);
}

public float3x3 primeFullCompactEonBasis(float3 wi)
{
    float lengthSquared = dot(wi.xy, wi.xy);
    float3 x = lengthSquared > 0.0 ? float3(wi.x, wi.y, 0.0) * rsqrt(lengthSquared)
                                   : float3(1.0, 0.0, 0.0);
    float3 y = float3(-x.y, x.x, 0.0);
    return float3x3(x, y, float3(0.0, 0.0, 1.0));
}

public PrimeFullOpenPbrThroughput primeFullCompactLambertEval(float3 wi, float3 wo,
                                                              PrimeFullCompactOpaqueState state,
                                                              uint samplingFlags)
{
    if (wi.z < 0.0 || wo.z < 0.0 ||
        (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) == 0u)
    {
        return primeFullOpenPbrZeroThroughput();
    }
    PrimeFullOpenPbrThroughput result;
    result.value = state.baseColor * (PRIME_FULL_OPENPBR_INV_PI * wo.z);
    result.flags = PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION;
    return result;
}

public float primeFullCompactLambertPdf(float3 wi, float3 wo)
{
    return wi.z >= 0.0 && wo.z > 0.0 ? PRIME_FULL_OPENPBR_INV_PI * wo.z : 0.0;
}

public float3 primeFullCompactLambertEnergy(float3 wi, PrimeFullCompactOpaqueState state,
                                            uint samplingFlags)
{
    return wi.z >= 0.0 && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) != 0u
               ? state.baseColor
               : float3(0.0);
}

public PrimeFullOpenPbrSample primeFullCompactLambertSample(float3 wi, float3 randomValue,
                                                            PrimeFullCompactOpaqueState state,
                                                            uint samplingFlags)
{
    PrimeFullOpenPbrSample result = primeFullOpenPbrZeroSample();
    if (wi.z < 0.0 || (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) == 0u)
    {
        return result;
    }
    result.wo = primeFullOpenPbrCosineSampleHemisphere(randomValue.xy);
    result.pdf = PRIME_FULL_OPENPBR_INV_PI * result.wo.z;
    result.throughput.value = state.baseColor * result.pdf;
    result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION;
    return result;
}

// The generic OpenPBR diffuse closure uses EON even at diffuse roughness zero.
// Its value, PDF and energy reduce to Lambert, but its cLTC sampler rotates the
// cosine sample into the incident-direction basis. Preserve that exact mapping
// for fractional diffuse/subsurface mixtures without retaining general EON state.
public PrimeFullOpenPbrSample primeFullCompactZeroRoughnessEonSample(
    float3 wi, float3 randomValue, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    PrimeFullOpenPbrSample result = primeFullOpenPbrZeroSample();
    if (wi.z < 0.0 || (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) == 0u)
    {
        return result;
    }
    if (randomValue.z <= 0.0)
    {
        // EON uses <= for its zero-probability uniform branch. Although this has
        // zero measure, preserving it keeps boundary random inputs deterministic.
        result.wo = primeFullOpenPbrUniformSampleHemisphere(randomValue.xy);
        result.pdf = result.wo.z * PRIME_FULL_OPENPBR_INV_PI;
        result.throughput = primeFullCompactLambertEval(wi, result.wo, state, samplingFlags);
        return result;
    }
    float radius = sqrt(randomValue.x);
    float phi = 2.0 * PRIME_FULL_OPENPBR_PI * randomValue.y;
    float x = -radius * cos(phi);
    float y = radius * sin(phi);
    float3 wo = float3(x, y, sqrt(max(1.0 - (x * x + y * y), 0.0)));
    float woLength = length(wo);
    result.pdf = wo.z * PRIME_FULL_OPENPBR_INV_PI * woLength * woLength * woLength;
    result.wo = normalize(mul(wo, primeFullCompactEonBasis(wi)));
    result.throughput = primeFullCompactLambertEval(wi, result.wo, state, samplingFlags);
    return result;
}

public PrimeFullOpenPbrThroughput primeFullCompactSubsurfaceEval<
    let FixedSubsurface : bool = false>(float3 wi, float3 wo, PrimeFullCompactOpaqueState state,
                                        uint samplingFlags)
{
    bool reflected = wo.z * wi.z > 0.0;
    if ((reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) == 0u) ||
        (!reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION) == 0u))
    {
        return primeFullOpenPbrZeroThroughput();
    }
    PrimeFullOpenPbrThroughput result;
    result.value = float3(PRIME_FULL_OPENPBR_INV_PI * abs(wo.z)) * state.subsurfaceColor * 0.5 *
                   (FixedSubsurface ? (reflected ? 1.3333333730697632 : 0.6666666269302368)
                                    : (reflected ? 1.0 - state.subsurfaceAnisotropy
                                                 : 1.0 + state.subsurfaceAnisotropy));
    result.flags = reflected ? PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION
                             : PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION;
    return result;
}

public float primeFullCompactSubsurfaceReflectionProbability<let FixedSubsurface : bool = false>(
    PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    float probability =
        FixedSubsurface ? 0.6666666865348816 : (1.0 - state.subsurfaceAnisotropy) * 0.5;
    return primeFullOpenPbrIsReflective(samplingFlags) ? probability : 0.0;
}

public float primeFullCompactSubsurfaceTransmissionProbability<let FixedSubsurface : bool = false>(
    PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    float probability =
        FixedSubsurface ? 0.3333333134651184 : (1.0 + state.subsurfaceAnisotropy) * 0.5;
    return primeFullOpenPbrIsTransmissive(samplingFlags) ? probability : 0.0;
}

public float primeFullCompactSubsurfacePdf<let FixedSubsurface : bool = false>(
    float3 wi, float3 wo, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    float reflectionProbability =
        primeFullCompactSubsurfaceReflectionProbability<FixedSubsurface>(state, samplingFlags);
    float transmissionProbability =
        primeFullCompactSubsurfaceTransmissionProbability<FixedSubsurface>(state, samplingFlags);
    if (!FixedSubsurface && reflectionProbability == 0.0 && transmissionProbability == 0.0)
    {
        return 0.0;
    }
    return PRIME_FULL_OPENPBR_INV_PI * abs(wo.z) *
           (wi.z * wo.z > 0.0 ? reflectionProbability : transmissionProbability);
}

public float3 primeFullCompactSubsurfaceEnergy(PrimeFullCompactOpaqueState state,
                                               uint samplingFlags)
{
    if ((samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) != 0u)
    {
        return state.subsurfaceColor;
    }
    return float3(0.0);
}

public PrimeFullOpenPbrSample primeFullCompactSubsurfaceSample<let FixedSubsurface : bool = false>(
    float3 wi, float3 randomValue, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    PrimeFullOpenPbrSample result = primeFullOpenPbrZeroSample();
    if ((samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE) == 0u)
    {
        return result;
    }
    float reflectionProbability =
        primeFullCompactSubsurfaceReflectionProbability<FixedSubsurface>(state, samplingFlags);
    float transmissionProbability =
        primeFullCompactSubsurfaceTransmissionProbability<FixedSubsurface>(state, samplingFlags);
    if (!FixedSubsurface && reflectionProbability == 0.0 && transmissionProbability == 0.0)
    {
        return result;
    }
    result.wo = primeFullOpenPbrCosineSampleHemisphere(randomValue.xy);
    result.pdf = PRIME_FULL_OPENPBR_INV_PI * result.wo.z;
    result.throughput.value = float3(result.pdf);
    if (wi.z < 0.0)
    {
        result.wo.z = -result.wo.z;
    }
    if (randomValue.z <= transmissionProbability)
    {
        result.wo.z = -result.wo.z;
        result.pdf *= transmissionProbability;
        result.throughput.value *= transmissionProbability;
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION;
        result.throughput.value *= state.subsurfaceColor;
    }
    else
    {
        result.pdf *= reflectionProbability;
        result.throughput.value *= reflectionProbability * state.subsurfaceColor;
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION;
    }
    return result;
}

public PrimeFullOpenPbrThroughput primeFullCompactMixedDiffuseEval<
    let FixedSubsurface : bool = false>(float3 wi, float3 wo, PrimeFullCompactOpaqueState state,
                                        uint samplingFlags)
{
    return primeFullCompactMixEval(
        state.subsurfaceWeight < 1.0 ? primeFullCompactLambertEval(wi, wo, state, samplingFlags)
                                     : primeFullOpenPbrZeroThroughput(),
        state.subsurfaceWeight > 0.0
            ? primeFullCompactSubsurfaceEval<FixedSubsurface>(wi, wo, state, samplingFlags)
            : primeFullOpenPbrZeroThroughput(),
        state.subsurfaceWeight);
}

public float primeFullCompactMixedDiffusePdf<let FixedSubsurface : bool = false>(
    float3 wi, float3 wo, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    if (state.diffuseMix.firstSampleWeight + state.diffuseMix.secondSampleWeight <= 0.0)
    {
        return 0.0;
    }
    float first = state.subsurfaceWeight < 1.0 &&
                          (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) != 0u
                      ? primeFullCompactLambertPdf(wi, wo)
                      : 0.0;
    float second =
        state.subsurfaceWeight > 0.0
            ? primeFullCompactSubsurfacePdf<FixedSubsurface>(wi, wo, state, samplingFlags)
            : 0.0;
    return lerp(first, second, state.diffuseMix.secondSampleWeight);
}

public float3 primeFullCompactMixedDiffuseEnergy(PrimeFullCompactOpaqueState state,
                                                 uint samplingFlags)
{
    float3 first = state.subsurfaceWeight < 1.0 &&
                           (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) != 0u
                       ? state.baseColor
                       : float3(0.0);
    float3 second = state.subsurfaceWeight > 0.0
                        ? primeFullCompactSubsurfaceEnergy(state, samplingFlags)
                        : float3(0.0);
    float3 result = float3(0.0);
    if (state.subsurfaceWeight < 1.0)
    {
        result = first;
    }
    if (state.subsurfaceWeight > 0.0)
    {
        result = result * (1.0 - state.subsurfaceWeight) + second * state.subsurfaceWeight;
    }
    return result;
}

/// FixedSubsurface 固定当前子集的各向异性；InteriorWeight 要求 0 < subsurfaceWeight < 1。
public PrimeFullOpenPbrSample primeFullCompactMixedDiffuseSample<let FixedSubsurface : bool = false,
                                                                 let InteriorWeight : bool = false>(
    float3 wi, float3 randomValue, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    if (state.diffuseMix.firstSampleWeight + state.diffuseMix.secondSampleWeight <= 0.0)
    {
        return primeFullOpenPbrZeroSample();
    }
    if (randomValue.z <= state.diffuseMix.secondSampleWeight &&
        state.diffuseMix.secondSampleWeight > 0.0)
    {
        PrimeFullOpenPbrSample result = primeFullCompactSubsurfaceSample<FixedSubsurface>(
            wi, float3(randomValue.xy, randomValue.z / state.diffuseMix.secondSampleWeight), state,
            samplingFlags);
        if (InteriorWeight)
            result.throughput.value *= state.subsurfaceWeight;
        else
            result.throughput =
                primeFullOpenPbrThroughputScale(result.throughput, state.subsurfaceWeight);
        result.pdf *= state.diffuseMix.secondSampleWeight;
        return result;
    }
    PrimeFullOpenPbrSample result = primeFullCompactZeroRoughnessEonSample(
        wi,
        float3(randomValue.xy, (randomValue.z - state.diffuseMix.secondSampleWeight) /
                                   state.diffuseMix.firstSampleWeight),
        state, samplingFlags);
    if (InteriorWeight)
        result.throughput.value *= 1.0 - state.subsurfaceWeight;
    else
        result.throughput =
            primeFullOpenPbrThroughputScale(result.throughput, 1.0 - state.subsurfaceWeight);
    result.pdf *= state.diffuseMix.firstSampleWeight;
    return result;
}

public PrimeFullOpenPbrThroughput primeFullCompactDielectricEval(float3 wi, float3 wo,
                                                                 PrimeFullCompactOpaqueState state,
                                                                 uint samplingFlags)
{
    return primeFullCompactDielectricEvalImpl<false>(wi, wo, state, samplingFlags);
}

public PrimeFullOpenPbrThroughput primeFullCompactDielectricEvalImpl<let Continuous : bool>(
    float3 wi, float3 wo, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    PrimeFullOpenPbrReflectiveEvalBase base = primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
        wi, wo, state.microfacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    return primeFullCompactDielectricEvalFromBase<Continuous>(base, state);
}

/// Fresnel 与能量补偿；base 可供同方向的反射分量及 PDF 共用。
public PrimeFullOpenPbrThroughput primeFullCompactDielectricEvalFromBase<let Continuous : bool>(
    PrimeFullOpenPbrReflectiveEvalBase base, PrimeFullCompactOpaqueState state)
{
    if (base.flags == PRIME_FULL_OPENPBR_FLAG_NONE)
    {
        return primeFullOpenPbrZeroThroughput();
    }
    PrimeFullOpenPbrThroughput result;
    result.value =
        primeFullOpenPbrSpecularFresnelUnpolarized(state.dielectricFresnel, base.microCosine) *
        base.factor;
    result.flags = base.flags;
    if (Continuous || primeFullOpenPbrIsNonDelta(result.flags))
    {
        result.value *= state.dielectricMultipleScattering.x;
    }
    return result;
}

public float primeFullCompactDielectricPdf(float3 wi, float3 wo, PrimeFullCompactOpaqueState state,
                                           uint samplingFlags)
{
    return primeFullCompactDielectricPdfImpl<false>(wi, wo, state, samplingFlags);
}

public float primeFullCompactDielectricPdfImpl<let Continuous : bool>(
    float3 wi, float3 wo, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    return primeFullOpenPbrReflectivePdfImpl<Continuous>(wi, wo, state.microfacet, samplingFlags,
                                                         PRIME_FULL_OPENPBR_FLAG_REFLECTION);
}

public float3 primeFullCompactDielectricEnergy(float3 wi, PrimeFullCompactOpaqueState state,
                                               uint samplingFlags)
{
    return primeFullCompactDielectricEnergyImpl<false>(wi, state, samplingFlags);
}

public float3 primeFullCompactDielectricEnergyImpl<let Continuous : bool>(
    float3 wi, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    if (wi.z < 0.0)
    {
        return float3(0.0);
    }
    if ((Continuous || !primeFullOpenPbrMicrofacetEffectivelySmooth(state.microfacet)) &&
        state.dielectricFresnel.ior != 1.0)
    {
        if (!primeFullOpenPbrIsSpecular(samplingFlags))
        {
            return float3(0.0);
        }
    }
    else if (!primeFullOpenPbrIsDelta(samplingFlags))
    {
        return float3(0.0);
    }
    if (!primeFullOpenPbrIsReflective(samplingFlags))
    {
        return float3(0.0);
    }
    return state.dielectricMultipleScattering.z * state.dielectricFresnel.color;
}

public PrimeFullOpenPbrSample primeFullCompactDielectricSample(float3 wi, float3 randomValue,
                                                               PrimeFullCompactOpaqueState state,
                                                               uint samplingFlags)
{
    return primeFullCompactDielectricSampleImpl<false>(wi, randomValue, state, samplingFlags);
}

public PrimeFullOpenPbrSample primeFullCompactDielectricSampleImpl<let Continuous : bool>(
    float3 wi, float3 randomValue, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    PrimeFullOpenPbrSample result = primeFullOpenPbrZeroSample();
    PrimeFullOpenPbrReflectiveSampleBase base =
        primeFullOpenPbrReflectiveSampleBaseImpl<Continuous>(
            wi, randomValue, state.microfacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    result = base.bsdfSample;
    result.throughput.value *=
        primeFullOpenPbrSpecularFresnelUnpolarized(state.dielectricFresnel, base.microCosine);
    if (Continuous || primeFullOpenPbrIsNonDelta(result.throughput.flags))
    {
        result.throughput.value *= state.dielectricMultipleScattering.x;
    }
    return result;
}

public PrimeFullOpenPbrThroughput primeFullCompactConductorEval(float3 wi, float3 wo,
                                                                PrimeFullCompactOpaqueState state,
                                                                uint samplingFlags)
{
    return primeFullCompactConductorEvalImpl<false>(wi, wo, state, samplingFlags);
}

public PrimeFullOpenPbrThroughput primeFullCompactConductorEvalImpl<let Continuous : bool>(
    float3 wi, float3 wo, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    PrimeFullOpenPbrReflectiveEvalBase base = primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
        wi, wo, state.microfacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    return primeFullCompactConductorEvalFromBase<Continuous>(base, state);
}

/// 金属端复用反射几何；不改变原 Fresnel 与多散射补偿算序。
public PrimeFullOpenPbrThroughput primeFullCompactConductorEvalFromBase<let Continuous : bool>(
    PrimeFullOpenPbrReflectiveEvalBase base, PrimeFullCompactOpaqueState state)
{
    if (base.flags == PRIME_FULL_OPENPBR_FLAG_NONE)
    {
        return primeFullOpenPbrZeroThroughput();
    }
    PrimeFullOpenPbrThroughput result;
    result.value =
        primeFullOpenPbrConductorFresnelUnpolarized(state.conductorFresnel, base.microCosine) *
        base.factor;
    result.flags = base.flags;
    if (Continuous || primeFullOpenPbrIsNonDelta(result.flags))
    {
        result.value *= float3(1.0) +
                        state.conductorFresnel.f0 * (1.0 - state.conductorEss) / state.conductorEss;
    }
    return result;
}

/// 同方向联合金属响应与 marginal PDF；不更改采样测度。
public PrimeFullCompactOpaqueEvaluation primeFullCompactConductorEvaluateImpl<
    let Continuous : bool>(float3 wi, float3 wo, PrimeFullCompactOpaqueState state,
                           uint samplingFlags)
{
    PrimeFullOpenPbrReflectiveEvalBase base = primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
        wi, wo, state.microfacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    return { primeFullCompactConductorEvalFromBase<Continuous>(base, state), base.pdf };
}

public float primeFullCompactConductorPdf(float3 wi, float3 wo, PrimeFullCompactOpaqueState state,
                                          uint samplingFlags)
{
    return primeFullCompactConductorPdfImpl<false>(wi, wo, state, samplingFlags);
}

public float primeFullCompactConductorPdfImpl<let Continuous : bool>(
    float3 wi, float3 wo, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    return primeFullOpenPbrReflectivePdfImpl<Continuous>(wi, wo, state.microfacet, samplingFlags,
                                                         PRIME_FULL_OPENPBR_FLAG_REFLECTION);
}

public float3 primeFullCompactConductorEnergy(float3 wi, PrimeFullCompactOpaqueState state,
                                              uint samplingFlags)
{
    if (wi.z < 0.0)
    {
        return float3(0.0);
    }
    if (primeFullOpenPbrMicrofacetEffectivelySmooth(state.microfacet))
    {
        if ((samplingFlags & PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION) == 0u)
        {
            return float3(0.0);
        }
    }
    else if ((samplingFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION) == 0u)
    {
        return float3(0.0);
    }
    float3 f0 = state.conductorFresnel.energyF0;
    return primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(
               state.microfacet, wi.z, f0,
               clamp(float3(state.conductorFresnel.weight), float3(0.0), float3(1.0))) *
           (float3(1.0) + f0 * (1.0 - state.conductorEss) / state.conductorEss);
}

public PrimeFullOpenPbrSample primeFullCompactConductorSample(float3 wi, float3 randomValue,
                                                              PrimeFullCompactOpaqueState state,
                                                              uint samplingFlags)
{
    return primeFullCompactConductorSampleImpl<false>(wi, randomValue, state, samplingFlags);
}

public PrimeFullOpenPbrSample primeFullCompactConductorSampleImpl<let Continuous : bool>(
    float3 wi, float3 randomValue, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    if (!Continuous && primeFullOpenPbrMicrofacetEffectivelySmooth(state.microfacet))
    {
        return primeFullCompactConductorDiscreteSample(
            wi, primeFullCompactOpaqueDiscreteStateFromFull(state), samplingFlags);
    }
    PrimeFullOpenPbrSample result = primeFullOpenPbrZeroSample();
    PrimeFullOpenPbrReflectiveSampleBase base =
        primeFullOpenPbrReflectiveSampleBaseImpl<Continuous>(
            wi, randomValue, state.microfacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    result = base.bsdfSample;
    result.throughput.value *=
        primeFullOpenPbrConductorFresnelUnpolarized(state.conductorFresnel, base.microCosine);
    if (Continuous || primeFullOpenPbrIsNonDelta(result.throughput.flags))
    {
        result.throughput.value *= float3(1.0) + state.conductorFresnel.f0 *
                                                     (1.0 - state.conductorEss) /
                                                     state.conductorEss;
    }
    return result;
}

public PrimeFullOpenPbrThroughput primeFullCompactOpaqueLayerEval(float3 wi, float3 wo,
                                                                  PrimeFullCompactOpaqueState state,
                                                                  uint samplingFlags)
{
    return primeFullCompactOpaqueLayerEvalImpl<false>(wi, wo, state, samplingFlags);
}

public PrimeFullOpenPbrThroughput primeFullCompactOpaqueLayerEvalImpl<let Continuous : bool>(
    float3 wi, float3 wo, PrimeFullCompactOpaqueState state, uint samplingFlags)
{
    PrimeFullOpenPbrThroughput base =
        state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
            ? primeFullCompactMixedDiffuseEval(wi, wo, state, samplingFlags)
        : state.kind == PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
            ? primeFullCompactSubsurfaceEval(wi, wo, state, samplingFlags)
            : primeFullCompactLambertEval(wi, wo, state, samplingFlags);
    PrimeFullOpenPbrThroughput specular =
        primeFullCompactDielectricEvalImpl<Continuous>(wi, wo, state, samplingFlags);
    return primeFullCompactLayerEval(base, specular, state.layer);
}

/// 联合层响应与完整混合 PDF，共享反射几何。
public PrimeFullCompactOpaqueEvaluation primeFullCompactOpaqueLayerEvaluateImpl<
    let Continuous : bool>(float3 wi, float3 wo, PrimeFullCompactOpaqueState state)
{
    PrimeFullOpenPbrReflectiveEvalBase reflection =
        primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(wi, wo, state.microfacet,
                                                           PRIME_FULL_OPENPBR_FLAG_ALL,
                                                           PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    return primeFullCompactOpaqueLayerEvaluateFromBase<Continuous>(wi, wo, state, reflection);
}

/// 已准备反射几何时，仅补充当前层的 base 与原混合权重。
public PrimeFullCompactOpaqueEvaluation primeFullCompactOpaqueLayerEvaluateFromBase<
    let Continuous : bool>(float3 wi, float3 wo, PrimeFullCompactOpaqueState state,
                           PrimeFullOpenPbrReflectiveEvalBase reflection)
{
    PrimeFullOpenPbrThroughput base =
        state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
            ? primeFullCompactMixedDiffuseEval(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL)
        : state.kind == PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
            ? primeFullCompactSubsurfaceEval(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL)
            : primeFullCompactLambertEval(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    float basePdf =
        state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
            ? primeFullCompactMixedDiffusePdf(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL)
        : state.kind == PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
            ? primeFullCompactSubsurfacePdf(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL)
            : primeFullCompactLambertPdf(wi, wo);
    PrimeFullOpenPbrThroughput specular =
        primeFullCompactDielectricEvalFromBase<Continuous>(reflection, state);
    return { primeFullCompactLayerEval(base, specular, state.layer),
             state.layer.baseSampleWeight + state.layer.specularSampleWeight <= 0.0
                 ? 0.0
                 : lerp(basePdf, reflection.pdf, state.layer.specularSampleWeight) };
}

public float primeFullCompactOpaqueLayerPdf(float3 wi, float3 wo, PrimeFullCompactOpaqueState state)
{
    return primeFullCompactOpaqueLayerPdfImpl<false>(wi, wo, state);
}

public float primeFullCompactOpaqueLayerPdfImpl<let Continuous : bool>(
    float3 wi, float3 wo, PrimeFullCompactOpaqueState state)
{
    float basePdf =
        state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
            ? primeFullCompactMixedDiffusePdf(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL)
        : state.kind == PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
            ? primeFullCompactSubsurfacePdf(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL)
            : primeFullCompactLambertPdf(wi, wo);
    if (state.layer.baseSampleWeight + state.layer.specularSampleWeight <= 0.0)
    {
        return 0.0;
    }
    return lerp(
        basePdf,
        primeFullCompactDielectricPdfImpl<Continuous>(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL),
        state.layer.specularSampleWeight);
}

public PrimeFullOpenPbrSample primeFullCompactOpaqueLayerSample(float3 wi, float3 randomValue,
                                                                PrimeFullCompactOpaqueState state,
                                                                bool useEonDiffuseMapping)
{
    return primeFullCompactOpaqueLayerSampleImpl<false>(wi, randomValue, state,
                                                        useEonDiffuseMapping);
}

public PrimeFullOpenPbrSample primeFullCompactOpaqueLayerSampleImpl<let Continuous : bool>(
    float3 wi, float3 randomValue, PrimeFullCompactOpaqueState state, bool useEonDiffuseMapping)
{
    if (state.layer.baseSampleWeight + state.layer.specularSampleWeight <= 0.0)
    {
        return primeFullOpenPbrZeroSample();
    }
    if (randomValue.z <= state.layer.specularSampleWeight && state.layer.specularSampleWeight > 0.0)
    {
        PrimeFullOpenPbrSample result = primeFullCompactDielectricSampleImpl<Continuous>(
            wi, float3(randomValue.xy, randomValue.z / state.layer.specularSampleWeight), state,
            PRIME_FULL_OPENPBR_FLAG_ALL);
        result.pdf *= state.layer.specularSampleWeight;
        return result;
    }
    PrimeFullOpenPbrSample result =
        state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
            ? primeFullCompactMixedDiffuseSample(
                  wi,
                  float3(randomValue.xy, (randomValue.z - state.layer.specularSampleWeight) /
                                             state.layer.baseSampleWeight),
                  state, PRIME_FULL_OPENPBR_FLAG_ALL)
        : state.kind == PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
            ? primeFullCompactSubsurfaceSample(
                  wi,
                  float3(randomValue.xy, (randomValue.z - state.layer.specularSampleWeight) /
                                             state.layer.baseSampleWeight),
                  state, PRIME_FULL_OPENPBR_FLAG_ALL)
        : useEonDiffuseMapping
            ? primeFullCompactZeroRoughnessEonSample(
                  wi,
                  float3(randomValue.xy, (randomValue.z - state.layer.specularSampleWeight) /
                                             state.layer.baseSampleWeight),
                  state, PRIME_FULL_OPENPBR_FLAG_ALL)
            : primeFullCompactLambertSample(
                  wi,
                  float3(randomValue.xy, (randomValue.z - state.layer.specularSampleWeight) /
                                             state.layer.baseSampleWeight),
                  state, PRIME_FULL_OPENPBR_FLAG_ALL);
    result.throughput =
        primeFullOpenPbrThroughputScale(result.throughput, state.layer.transmittanceIn);
    result.pdf *= state.layer.baseSampleWeight;
    return result;
}
