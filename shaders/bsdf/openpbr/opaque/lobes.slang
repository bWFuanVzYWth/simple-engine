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

public func primeFullCompactMakeLayerState(baseEnergy: float3, transmittanceIn: float3,
                                           specularEnergy: float3)
    ->PrimeFullCompactLayerState
{
    var result : PrimeFullCompactLayerState;
    result.transmittanceIn = transmittanceIn;
    result.baseSampleWeight = primeFullOpenPbrSpectrumToWeight(transmittanceIn * baseEnergy);
    result.specularSampleWeight = primeFullOpenPbrSpectrumToWeight(specularEnergy);
    let sum : float = result.baseSampleWeight + result.specularSampleWeight;
    if (sum > 0.0)
    {
        result.baseSampleWeight /= sum;
        result.specularSampleWeight /= sum;
    }
    return result;
}

public func primeFullCompactMakeMixState(weight: float, firstEnergy: float3, secondEnergy: float3)
    ->PrimeFullCompactMixState
{
    var result : PrimeFullCompactMixState;
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
    let sum : float = result.firstSampleWeight + result.secondSampleWeight;
    if (sum > 0.0)
    {
        result.firstSampleWeight /= sum;
        result.secondSampleWeight /= sum;
    }
    return result;
}

public func primeFullCompactMixEval(first: PrimeFullOpenPbrThroughput,
                                    second: PrimeFullOpenPbrThroughput, weight: float)
    ->PrimeFullOpenPbrThroughput
{
    var second = second;
    var result : PrimeFullOpenPbrThroughput = primeFullOpenPbrZeroThroughput();
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

public func primeFullCompactLayerEval(base: PrimeFullOpenPbrThroughput,
                                      specular: PrimeFullOpenPbrThroughput,
                                      layer: PrimeFullCompactLayerState)
    ->PrimeFullOpenPbrThroughput
{
    let result : PrimeFullOpenPbrThroughput =
                     primeFullOpenPbrThroughputScale(base, layer.transmittanceIn);
    return primeFullOpenPbrThroughputAdd(result, specular);
}

public func primeFullCompactEonBasis(wi: float3)->float3x3
{
    let lengthSquared : float = dot(wi.xy, wi.xy);
    let x : float3 = lengthSquared > 0.0 ? float3(wi.x, wi.y, 0.0) * rsqrt(lengthSquared)
                                         : float3(1.0, 0.0, 0.0);
    let y = float3(-x.y, x.x, 0.0);
    return float3x3(x, y, float3(0.0, 0.0, 1.0));
}

public func primeFullCompactLambertEval(wi: float3, wo: float3, state: PrimeFullCompactOpaqueState,
                                        samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    if (wi.z < 0.0 || wo.z < 0.0 ||
        (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) == 0u)
    {
        return primeFullOpenPbrZeroThroughput();
    }
    var result : PrimeFullOpenPbrThroughput;
    result.value = state.baseColor * (PRIME_FULL_OPENPBR_INV_PI * wo.z);
    result.flags = PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION;
    return result;
}

public func primeFullCompactLambertPdf(wi: float3, wo: float3)->float
{
    return wi.z >= 0.0 && wo.z > 0.0 ? PRIME_FULL_OPENPBR_INV_PI * wo.z : 0.0;
}

public func primeFullCompactLambertEnergy(wi: float3, state: PrimeFullCompactOpaqueState,
                                          samplingFlags: uint)
    ->float3
{
    return wi.z >= 0.0 && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) != 0u
               ? state.baseColor
               : float3(0.0);
}

public func primeFullCompactLambertSample(wi: float3, randomValue: float3,
                                          state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
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
public func primeFullCompactZeroRoughnessEonSample(wi: float3, randomValue: float3,
                                                   state: PrimeFullCompactOpaqueState,
                                                   samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
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
    let radius : float = sqrt(randomValue.x);
    let phi : float = 2.0 * PRIME_FULL_OPENPBR_PI * randomValue.y;
    let x : float = -radius * cos(phi);
    let y : float = radius * sin(phi);
    var wo = float3(x, y, sqrt(max(1.0 - (x * x + y * y), 0.0)));
    let woLength : float = length(wo);
    result.pdf = wo.z * PRIME_FULL_OPENPBR_INV_PI * woLength * woLength * woLength;
    result.wo = normalize(mul(wo, primeFullCompactEonBasis(wi)));
    result.throughput = primeFullCompactLambertEval(wi, result.wo, state, samplingFlags);
    return result;
}

public func primeFullCompactSubsurfaceEval<let FixedSubsurface : bool = false>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    let reflected : bool = wo.z * wi.z > 0.0;
    if ((reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) == 0u) ||
        (!reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION) == 0u))
    {
        return primeFullOpenPbrZeroThroughput();
    }
    var result : PrimeFullOpenPbrThroughput;
    result.value = float3(PRIME_FULL_OPENPBR_INV_PI * abs(wo.z)) * state.subsurfaceColor * 0.5 *
                   (FixedSubsurface ? (reflected ? 1.3333333730697632 : 0.6666666269302368)
                                    : (reflected ? 1.0 - state.subsurfaceAnisotropy
                                                 : 1.0 + state.subsurfaceAnisotropy));
    result.flags = reflected ? PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION
                             : PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION;
    return result;
}

public func primeFullCompactSubsurfaceReflectionProbability<let FixedSubsurface : bool = false>(
    state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->float
{
    let probability : float = FixedSubsurface ? 0.6666666865348816
                                              : (1.0 - state.subsurfaceAnisotropy) * 0.5;
    return primeFullOpenPbrIsReflective(samplingFlags) ? probability : 0.0;
}

public func primeFullCompactSubsurfaceTransmissionProbability<let FixedSubsurface : bool = false>(
    state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->float
{
    let probability : float = FixedSubsurface ? 0.3333333134651184
                                              : (1.0 + state.subsurfaceAnisotropy) * 0.5;
    return primeFullOpenPbrIsTransmissive(samplingFlags) ? probability : 0.0;
}

public func primeFullCompactSubsurfacePdf<let FixedSubsurface : bool = false>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->float
{
    let reflectionProbability
        : float = primeFullCompactSubsurfaceReflectionProbability<FixedSubsurface>(state,
                                                                                   samplingFlags);
    let transmissionProbability
        : float = primeFullCompactSubsurfaceTransmissionProbability<FixedSubsurface>(state,
                                                                                     samplingFlags);
    if (!FixedSubsurface && reflectionProbability == 0.0 && transmissionProbability == 0.0)
    {
        return 0.0;
    }
    return PRIME_FULL_OPENPBR_INV_PI * abs(wo.z) *
           (wi.z * wo.z > 0.0 ? reflectionProbability : transmissionProbability);
}

public func primeFullCompactSubsurfaceEnergy(state: PrimeFullCompactOpaqueState,
                                             samplingFlags: uint)
    ->float3
{
    if ((samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) != 0u)
    {
        return state.subsurfaceColor;
    }
    return float3(0.0);
}

public func primeFullCompactSubsurfaceSample<let FixedSubsurface : bool = false>(
    wi: float3, randomValue: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
    if ((samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE) == 0u)
    {
        return result;
    }
    let reflectionProbability
        : float = primeFullCompactSubsurfaceReflectionProbability<FixedSubsurface>(state,
                                                                                   samplingFlags);
    let transmissionProbability
        : float = primeFullCompactSubsurfaceTransmissionProbability<FixedSubsurface>(state,
                                                                                     samplingFlags);
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

public func primeFullCompactMixedDiffuseEval<let FixedSubsurface : bool = false>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    return primeFullCompactMixEval(
        state.subsurfaceWeight < 1.0 ? primeFullCompactLambertEval(wi, wo, state, samplingFlags)
                                     : primeFullOpenPbrZeroThroughput(),
        state.subsurfaceWeight > 0.0
            ? primeFullCompactSubsurfaceEval<FixedSubsurface>(wi, wo, state, samplingFlags)
            : primeFullOpenPbrZeroThroughput(),
        state.subsurfaceWeight);
}

public func primeFullCompactMixedDiffusePdf<let FixedSubsurface : bool = false>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->float
{
    if (state.diffuseMix.firstSampleWeight + state.diffuseMix.secondSampleWeight <= 0.0)
    {
        return 0.0;
    }
    let first : float = state.subsurfaceWeight < 1.0 &&
                                (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) != 0u
                            ? primeFullCompactLambertPdf(wi, wo)
                            : 0.0;
    let second
        : float = state.subsurfaceWeight > 0.0
                      ? primeFullCompactSubsurfacePdf<FixedSubsurface>(wi, wo, state, samplingFlags)
                      : 0.0;
    return lerp(first, second, state.diffuseMix.secondSampleWeight);
}

public func primeFullCompactMixedDiffuseEnergy(state: PrimeFullCompactOpaqueState,
                                               samplingFlags: uint)
    ->float3
{
    let first : float3 = state.subsurfaceWeight < 1.0 &&
                                 (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION) != 0u
                             ? state.baseColor
                             : float3(0.0);
    let second : float3 = state.subsurfaceWeight > 0.0
                              ? primeFullCompactSubsurfaceEnergy(state, samplingFlags)
                              : float3(0.0);
    var result = float3(0.0);
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
public func primeFullCompactMixedDiffuseSample<let FixedSubsurface : bool = false,
                                               let InteriorWeight : bool = false>(
    wi: float3, randomValue: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    if (state.diffuseMix.firstSampleWeight + state.diffuseMix.secondSampleWeight <= 0.0)
    {
        return primeFullOpenPbrZeroSample();
    }
    if (randomValue.z <= state.diffuseMix.secondSampleWeight &&
        state.diffuseMix.secondSampleWeight > 0.0)
    {
        var result
            : PrimeFullOpenPbrSample = primeFullCompactSubsurfaceSample<FixedSubsurface>(
                  wi, float3(randomValue.xy, randomValue.z / state.diffuseMix.secondSampleWeight),
                  state, samplingFlags);
        if (InteriorWeight)
            result.throughput.value *= state.subsurfaceWeight;
        else
            result.throughput =
                primeFullOpenPbrThroughputScale(result.throughput, state.subsurfaceWeight);
        result.pdf *= state.diffuseMix.secondSampleWeight;
        return result;
    }
    var result : PrimeFullOpenPbrSample = primeFullCompactZeroRoughnessEonSample(
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

public func primeFullCompactDielectricEval(wi: float3, wo: float3,
                                           state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    return primeFullCompactDielectricEvalImpl<false>(wi, wo, state, samplingFlags);
}

public func primeFullCompactDielectricEvalImpl<let Continuous : bool>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    let base : PrimeFullOpenPbrReflectiveEvalBase =
                   primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
                       wi, wo, state.microfacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    return primeFullCompactDielectricEvalFromBase<Continuous>(base, state);
}

/// Fresnel 与能量补偿；base 可供同方向的反射分量及 PDF 共用。
public func primeFullCompactDielectricEvalFromBase<let Continuous : bool>(
    base: PrimeFullOpenPbrReflectiveEvalBase, state: PrimeFullCompactOpaqueState)
    ->PrimeFullOpenPbrThroughput
{
    if (base.flags == PRIME_FULL_OPENPBR_FLAG_NONE)
    {
        return primeFullOpenPbrZeroThroughput();
    }
    var result : PrimeFullOpenPbrThroughput;
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

public func primeFullCompactDielectricPdf(wi: float3, wo: float3,
                                          state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->float
{
    return primeFullCompactDielectricPdfImpl<false>(wi, wo, state, samplingFlags);
}

public func primeFullCompactDielectricPdfImpl<let Continuous : bool>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->float
{
    return primeFullOpenPbrReflectivePdfImpl<Continuous>(wi, wo, state.microfacet, samplingFlags,
                                                         PRIME_FULL_OPENPBR_FLAG_REFLECTION);
}

public func primeFullCompactDielectricEnergy(wi: float3, state: PrimeFullCompactOpaqueState,
                                             samplingFlags: uint)
    ->float3
{
    return primeFullCompactDielectricEnergyImpl<false>(wi, state, samplingFlags);
}

public func primeFullCompactDielectricEnergyImpl<let Continuous : bool>(
    wi: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->float3
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

public func primeFullCompactDielectricSample(wi: float3, randomValue: float3,
                                             state: PrimeFullCompactOpaqueState,
                                             samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    return primeFullCompactDielectricSampleImpl<false>(wi, randomValue, state, samplingFlags);
}

public func primeFullCompactDielectricSampleImpl<let Continuous : bool>(
    wi: float3, randomValue: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
    let base : PrimeFullOpenPbrReflectiveSampleBase =
                   primeFullOpenPbrReflectiveSampleBaseImpl<Continuous>(
                       wi, randomValue, state.microfacet, samplingFlags,
                       PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    result = base.bsdfSample;
    result.throughput.value *=
        primeFullOpenPbrSpecularFresnelUnpolarized(state.dielectricFresnel, base.microCosine);
    if (Continuous || primeFullOpenPbrIsNonDelta(result.throughput.flags))
    {
        result.throughput.value *= state.dielectricMultipleScattering.x;
    }
    return result;
}

public func primeFullCompactConductorEval(wi: float3, wo: float3,
                                          state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    return primeFullCompactConductorEvalImpl<false>(wi, wo, state, samplingFlags);
}

public func primeFullCompactConductorEvalImpl<let Continuous : bool>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    let base : PrimeFullOpenPbrReflectiveEvalBase =
                   primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
                       wi, wo, state.microfacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    return primeFullCompactConductorEvalFromBase<Continuous>(base, state);
}

/// 金属端复用反射几何；不改变原 Fresnel 与多散射补偿算序。
public func primeFullCompactConductorEvalFromBase<let Continuous : bool>(
    base: PrimeFullOpenPbrReflectiveEvalBase, state: PrimeFullCompactOpaqueState)
    ->PrimeFullOpenPbrThroughput
{
    if (base.flags == PRIME_FULL_OPENPBR_FLAG_NONE)
    {
        return primeFullOpenPbrZeroThroughput();
    }
    var result : PrimeFullOpenPbrThroughput;
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
public func primeFullCompactConductorEvaluateImpl<let Continuous : bool>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullCompactOpaqueEvaluation
{
    let base : PrimeFullOpenPbrReflectiveEvalBase =
                   primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
                       wi, wo, state.microfacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    return { primeFullCompactConductorEvalFromBase<Continuous>(base, state), base.pdf };
}

public func primeFullCompactConductorPdf(wi: float3, wo: float3, state: PrimeFullCompactOpaqueState,
                                         samplingFlags: uint)
    ->float
{
    return primeFullCompactConductorPdfImpl<false>(wi, wo, state, samplingFlags);
}

public func primeFullCompactConductorPdfImpl<let Continuous : bool>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->float
{
    return primeFullOpenPbrReflectivePdfImpl<Continuous>(wi, wo, state.microfacet, samplingFlags,
                                                         PRIME_FULL_OPENPBR_FLAG_REFLECTION);
}

public func primeFullCompactConductorEnergy(wi: float3, state: PrimeFullCompactOpaqueState,
                                            samplingFlags: uint)
    ->float3
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
    let f0 : float3 = state.conductorFresnel.energyF0;
    return primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(
               state.microfacet, wi.z, f0,
               clamp(float3(state.conductorFresnel.weight), float3(0.0), float3(1.0))) *
           (float3(1.0) + f0 * (1.0 - state.conductorEss) / state.conductorEss);
}

public func primeFullCompactConductorSample(wi: float3, randomValue: float3,
                                            state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    return primeFullCompactConductorSampleImpl<false>(wi, randomValue, state, samplingFlags);
}

public func primeFullCompactConductorSampleImpl<let Continuous : bool>(
    wi: float3, randomValue: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    if (!Continuous && primeFullOpenPbrMicrofacetEffectivelySmooth(state.microfacet))
    {
        return primeFullCompactConductorDiscreteSample(
            wi, primeFullCompactOpaqueDiscreteStateFromFull(state), samplingFlags);
    }
    var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
    let base : PrimeFullOpenPbrReflectiveSampleBase =
                   primeFullOpenPbrReflectiveSampleBaseImpl<Continuous>(
                       wi, randomValue, state.microfacet, samplingFlags,
                       PRIME_FULL_OPENPBR_FLAG_REFLECTION);
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

public func primeFullCompactOpaqueLayerEval(wi: float3, wo: float3,
                                            state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    return primeFullCompactOpaqueLayerEvalImpl<false>(wi, wo, state, samplingFlags);
}

public func primeFullCompactOpaqueLayerEvalImpl<let Continuous : bool>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState, samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    let base : PrimeFullOpenPbrThroughput =
                   state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                       ? primeFullCompactMixedDiffuseEval(wi, wo, state, samplingFlags)
                   : state.kind == PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
                       ? primeFullCompactSubsurfaceEval(wi, wo, state, samplingFlags)
                       : primeFullCompactLambertEval(wi, wo, state, samplingFlags);
    let specular : PrimeFullOpenPbrThroughput =
                       primeFullCompactDielectricEvalImpl<Continuous>(wi, wo, state, samplingFlags);
    return primeFullCompactLayerEval(base, specular, state.layer);
}

/// 联合层响应与完整混合 PDF，共享反射几何。
public func primeFullCompactOpaqueLayerEvaluateImpl<let Continuous : bool>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState)
    ->PrimeFullCompactOpaqueEvaluation
{
    let reflection : PrimeFullOpenPbrReflectiveEvalBase =
                         primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
                             wi, wo, state.microfacet, PRIME_FULL_OPENPBR_FLAG_ALL,
                             PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    return primeFullCompactOpaqueLayerEvaluateFromBase<Continuous>(wi, wo, state, reflection);
}

/// 已准备反射几何时，仅补充当前层的 base 与原混合权重。
public func primeFullCompactOpaqueLayerEvaluateFromBase<let Continuous : bool>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState,
    reflection: PrimeFullOpenPbrReflectiveEvalBase)
    ->PrimeFullCompactOpaqueEvaluation
{
    let base
        : PrimeFullOpenPbrThroughput =
              state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                  ? primeFullCompactMixedDiffuseEval(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL)
              : state.kind == PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
                  ? primeFullCompactSubsurfaceEval(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL)
                  : primeFullCompactLambertEval(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    let basePdf
        : float = state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                      ? primeFullCompactMixedDiffusePdf(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL)
                  : state.kind == PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
                      ? primeFullCompactSubsurfacePdf(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL)
                      : primeFullCompactLambertPdf(wi, wo);
    let specular : PrimeFullOpenPbrThroughput =
                       primeFullCompactDielectricEvalFromBase<Continuous>(reflection, state);
    return { primeFullCompactLayerEval(base, specular, state.layer),
             state.layer.baseSampleWeight + state.layer.specularSampleWeight <= 0.0
                 ? 0.0
                 : lerp(basePdf, reflection.pdf, state.layer.specularSampleWeight) };
}

public func primeFullCompactOpaqueLayerPdf(wi: float3, wo: float3,
                                           state: PrimeFullCompactOpaqueState)
    ->float
{
    return primeFullCompactOpaqueLayerPdfImpl<false>(wi, wo, state);
}

public func primeFullCompactOpaqueLayerPdfImpl<let Continuous : bool>(
    wi: float3, wo: float3, state: PrimeFullCompactOpaqueState)
    ->float
{
    let basePdf
        : float = state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
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

public func primeFullCompactOpaqueLayerSample(wi: float3, randomValue: float3,
                                              state: PrimeFullCompactOpaqueState,
                                              useEonDiffuseMapping: bool)
    ->PrimeFullOpenPbrSample
{
    return primeFullCompactOpaqueLayerSampleImpl<false>(wi, randomValue, state,
                                                        useEonDiffuseMapping);
}

public func primeFullCompactOpaqueLayerSampleImpl<let Continuous : bool>(
    wi: float3, randomValue: float3, state: PrimeFullCompactOpaqueState, useEonDiffuseMapping: bool)
    ->PrimeFullOpenPbrSample
{
    if (state.layer.baseSampleWeight + state.layer.specularSampleWeight <= 0.0)
    {
        return primeFullOpenPbrZeroSample();
    }
    if (randomValue.z <= state.layer.specularSampleWeight && state.layer.specularSampleWeight > 0.0)
    {
        var result
            : PrimeFullOpenPbrSample = primeFullCompactDielectricSampleImpl<Continuous>(
                  wi, float3(randomValue.xy, randomValue.z / state.layer.specularSampleWeight),
                  state, PRIME_FULL_OPENPBR_FLAG_ALL);
        result.pdf *= state.layer.specularSampleWeight;
        return result;
    }
    var result
        : PrimeFullOpenPbrSample =
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
