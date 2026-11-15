// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/refract.slang";

import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/dielectric/discrete.slang";
import "bsdf/openpbr/dielectric/thin_wall.slang";
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/microfacet_distribution.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/microfacet_measure.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/math/spectrum.slang";

// 连续 core 仅供已证明 max(alpha)>=1e-4 且实际界面 ior!=1 的 solid 入口。
// 通用入口保留原光滑／匹配 IOR 分流；连续物理公式与后处理共用。
public func primeFullCompactRefractiveEval(wi: float3, wo: float3,
                                           state: PrimeFullCompactTransmissionState,
                                           samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    let result : PrimeFullOpenPbrThroughput = primeFullOpenPbrZeroThroughput();
    let microfacet : PrimeFullOpenPbrMicrofacet = state.specularMicrofacet;
    let fresnel : PrimeFullOpenPbrSpecularFresnel = state.specularFresnel;
    if (primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet) || fresnel.ior == 1.0)
    {
        return result;
    }
    return primeFullCompactRefractiveEvalContinuous(wi, wo, state, samplingFlags);
}

public func primeFullCompactRefractiveEvalContinuous(wi: float3, wo: float3,
                                                     state: PrimeFullCompactTransmissionState,
                                                     samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{

    var result : PrimeFullOpenPbrThroughput = primeFullOpenPbrZeroThroughput();
    let microfacet : PrimeFullOpenPbrMicrofacet = state.specularMicrofacet;
    let fresnel : PrimeFullOpenPbrSpecularFresnel = state.specularFresnel;
    let reflected : bool = wi.z * wo.z > 0.0;
    let etaPath : float = reflected ? 1.0 : (wi.z > 0.0 ? fresnel.ior : 1.0 / fresnel.ior);
    var halfVector : float3 = wi + etaPath * wo;
    let lengthSquared : float = dot(halfVector, halfVector);
    if (wo.z == 0.0 || wi.z == 0.0 || lengthSquared == 0.0)
    {
        return result;
    }
    halfVector *= rsqrt(lengthSquared);
    if (halfVector.z <= 0.0)
    {
        halfVector = -halfVector;
    }
    let microIncident : float = dot(wi, halfVector);
    let microOutgoing : float = dot(wo, halfVector);
    if (microIncident * wi.z < 0.0 || microOutgoing * wo.z < 0.0)
    {
        return result;
    }
    let rt : PrimeFullOpenPbrVecPair = primeFullOpenPbrSpecularFresnelRt(fresnel, microIncident);
    let distribution : float = primeFullOpenPbrMicrofacetD(microfacet, halfVector);
    if (reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION) != 0u)
    {
        result.value =
            rt.first * distribution * primeFullOpenPbrReflectionFactor(microfacet, wi, wo);
        result.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION;
    }
    else if (!reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION) != 0u)
    {
        let denominator : float = primeFullOpenPbrSquare(primeFullOpenPbrRefractionDenominator(
                                      etaPath, microIncident, microOutgoing)) *
                                  wi.z;
        result.value = rt.second * distribution *
                       primeFullOpenPbrMicrofacetG2t(microfacet, wi, wo) *
                       abs(microIncident * microOutgoing / denominator);
        result.value *= 1.0 / primeFullOpenPbrSquare(etaPath);
        result.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION;
    }
    return result;
}

/// 连续 solid 同方向共享半向量、Fresnel 和 D，保留响应与密度各自算序。
public func primeFullCompactRefractiveEvaluateContinuous(wi: float3, wo: float3,
                                                         state: PrimeFullCompactTransmissionState,
                                                         samplingFlags: uint)
    ->PrimeFullCompactTransmissionEvaluation
{

    var result : PrimeFullCompactTransmissionEvaluation = {};
    if (!primeFullOpenPbrIsSpecular(samplingFlags))
        return result;
    let microfacet : PrimeFullOpenPbrMicrofacet = state.specularMicrofacet;
    let fresnel : PrimeFullOpenPbrSpecularFresnel = state.specularFresnel;
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
    if (reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION) != 0u)
    {
        result.throughput.value =
            rt.first * distribution * primeFullOpenPbrReflectionFactor(microfacet, wi, wo);
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION;
    }
    else if (!reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION) != 0u)
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
    if (!primeFullOpenPbrIsReflective(samplingFlags))
        reflectionProbability = 0.0;
    if (!primeFullOpenPbrIsTransmissive(samplingFlags))
        transmissionProbability = 0.0;
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

public func primeFullCompactRefractiveSample(wi: float3, randomValue: float3,
                                             state: PrimeFullCompactTransmissionState,
                                             samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    let microfacet : PrimeFullOpenPbrMicrofacet = state.specularMicrofacet;
    let fresnel : PrimeFullOpenPbrSpecularFresnel = state.specularFresnel;
    if (primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet) || fresnel.ior == 1.0)
    {
        return primeFullCompactRefractiveDiscreteSample(wi, randomValue.z, state, samplingFlags);
    }
    return primeFullCompactRefractiveSampleContinuous(wi, randomValue, state, samplingFlags);
}

public func primeFullCompactRefractiveSampleContinuous(wi: float3, randomValue: float3,
                                                       state: PrimeFullCompactTransmissionState,
                                                       samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    let microfacet : PrimeFullOpenPbrMicrofacet = state.specularMicrofacet;
    let fresnel : PrimeFullOpenPbrSpecularFresnel = state.specularFresnel;
    var halfVector = float3(0.0, 0.0, 1.0);
    var microfacetPdf = 1.0;
    var distribution = 1.0;
    if (!primeFullOpenPbrIsSpecular(samplingFlags))
    {
        return primeFullOpenPbrZeroSample();
    }
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
    if (!primeFullOpenPbrIsReflective(samplingFlags))
    {
        reflectionProbability = 0.0;
    }
    if (!primeFullOpenPbrIsTransmissive(samplingFlags))
    {
        transmissionProbability = 0.0;
    }
    let refracted : PrimeFullOpenPbrRefractResult =
                        primeFullOpenPbrSpecularFresnelRefract(fresnel, -wi, halfVector);
    if (refracted.valid == 0u)
    {
        transmissionProbability = 0.0;
        reflection = float3(1.0);
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

public func primeFullCompactRefractivePdf(wi: float3, wo: float3,
                                          state: PrimeFullCompactTransmissionState,
                                          samplingFlags: uint)
    ->float
{
    if (primeFullOpenPbrMicrofacetEffectivelySmooth(state.specularMicrofacet) ||
        state.specularFresnel.ior == 1.0)
        return 0.0;
    return primeFullCompactRefractivePdfContinuous(wi, wo, state, samplingFlags);
}

public func primeFullCompactRefractivePdfContinuous(wi: float3, wo: float3,
                                                    state: PrimeFullCompactTransmissionState,
                                                    samplingFlags: uint)
    ->float
{

    let microfacet : PrimeFullOpenPbrMicrofacet = state.specularMicrofacet;
    let fresnel : PrimeFullOpenPbrSpecularFresnel = state.specularFresnel;
    if (!primeFullOpenPbrIsSpecular(samplingFlags))
    {
        return 0.0;
    }
    let reflected : bool = wi.z * wo.z > 0.0;
    let etaPath : float = reflected ? 1.0 : (wi.z > 0.0 ? fresnel.ior : 1.0 / fresnel.ior);
    var halfVector : float3 = wi + etaPath * wo;
    let lengthSquared : float = dot(halfVector, halfVector);
    if (wo.z == 0.0 || wi.z == 0.0 || lengthSquared == 0.0)
    {
        return 0.0;
    }
    halfVector *= rsqrt(lengthSquared);
    if (halfVector.z <= 0.0)
    {
        halfVector = -halfVector;
    }
    let microIncident : float = dot(wi, halfVector);
    let microOutgoing : float = dot(wo, halfVector);
    if (microIncident * wi.z < 0.0 || microOutgoing * wo.z < 0.0)
    {
        return 0.0;
    }
    let rt : PrimeFullOpenPbrVecPair = primeFullOpenPbrSpecularFresnelRt(fresnel, microIncident);
    var reflectionProbability : float = primeFullOpenPbrSpectrumToWeight(rt.first);
    var transmissionProbability : float = primeFullOpenPbrSpectrumToWeight(rt.second);
    if (!primeFullOpenPbrIsReflective(samplingFlags))
    {
        reflectionProbability = 0.0;
    }
    if (!primeFullOpenPbrIsTransmissive(samplingFlags))
    {
        transmissionProbability = 0.0;
    }
    let probabilitySum : float = reflectionProbability + transmissionProbability;
    if (probabilitySum == 0.0)
    {
        return 0.0;
    }
    let microfacetPdf : float = primeFullOpenPbrMicrofacetPdf(microfacet, wi.z > 0.0 ? wi : -wi,
                                                              halfVector, false);
    if (reflected)
    {
        return microfacetPdf / (4.0 * abs(microIncident)) * reflectionProbability / probabilitySum;
    }
    let denominator : float = primeFullOpenPbrSquare(primeFullOpenPbrRefractionDenominator(
                          etaPath, microIncident, microOutgoing));
    return microfacetPdf * abs(microOutgoing) / denominator * transmissionProbability /
           probabilitySum;
}

public func primeFullCompactFinishSolidEvaluation(result: PrimeFullOpenPbrThroughput,
                                                  state: PrimeFullCompactTransmissionState,
                                                  continuous: bool)
    ->PrimeFullOpenPbrThroughput
{
    var result = result;
    if (result.flags != PRIME_FULL_OPENPBR_FLAG_NONE)
    {
        if (primeFullOpenPbrIsTransmissive(result.flags))
        {
            result.value *= state.transmissionTint;
        }
        if (continuous || primeFullOpenPbrIsNonDelta(result.flags))
        {
            result.value *= primeFullOpenPbrIsTransmissive(result.flags)
                                ? state.transmissionMultipleScattering.y
                                : state.transmissionMultipleScattering.x;
        }
    }
    return result;
}

public func primeFullCompactTransmissionEvalRaw<let Continuous : bool = false>(
    wi: float3, inputWo: float3, state: PrimeFullCompactTransmissionState, samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    if (state.geometryThinWalled != 0u)
    {
        var wo : float3 = inputWo;
        var transmitted = false;
        if (wo.z < 0.0)
        {
            wo.z = -wo.z;
            transmitted = true;
            if (!primeFullOpenPbrIsTransmissive(samplingFlags))
            {
                return primeFullOpenPbrZeroThroughput();
            }
        }
        else if (!primeFullOpenPbrIsReflective(samplingFlags))
        {
            return primeFullOpenPbrZeroThroughput();
        }
        let base : PrimeFullOpenPbrReflectiveEvalBase =
                       primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
                           wi, wo, state.transmissionMicrofacet, samplingFlags,
                           PRIME_FULL_OPENPBR_FLAG_ALL);
        var result : PrimeFullOpenPbrThroughput;
        result.value = float3(base.factor);
        result.flags = base.flags;
        if (result.flags != PRIME_FULL_OPENPBR_FLAG_NONE)
        {
            let rt : PrimeFullOpenPbrVecPair =
                         primeFullCompactTransmissionThinWallRt(base.microCosine, state);
            if (transmitted)
            {
                result.value *= rt.second;
                result.flags = primeFullOpenPbrIsDelta(result.flags)
                                   ? PRIME_FULL_OPENPBR_FLAG_DELTA_TRANSMISSION
                                   : PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION;
            }
            else
            {
                result.value *= rt.first;
            }
            if (Continuous || primeFullOpenPbrIsNonDelta(result.flags))
            {
                result.value /= primeFullOpenPbrMicrofacetDirectionalAlbedoUnity(
                    state.transmissionMicrofacet, wi.z);
            }
        }
        return result;
    }
    let materialWi : float3 = primeFullCompactTransmissionMaterialFrame(wi, state);
    let materialWo : float3 = primeFullCompactTransmissionMaterialFrame(inputWo, state);
    let result : PrimeFullOpenPbrThroughput =
                     primeFullCompactRefractiveEval(materialWi, materialWo, state, samplingFlags);
    return primeFullCompactFinishSolidEvaluation(result, state, false);
}

public func primeFullCompactTransmissionEvalRawSolidContinuous(
    wi: float3, inputWo: float3, state: PrimeFullCompactTransmissionState, samplingFlags: uint)
    ->PrimeFullOpenPbrThroughput
{
    let materialWi : float3 = primeFullCompactTransmissionMaterialFrame(wi, state);
    let materialWo : float3 = primeFullCompactTransmissionMaterialFrame(inputWo, state);
    let result : PrimeFullOpenPbrThroughput = primeFullCompactRefractiveEvalContinuous(
                     materialWi, materialWo, state, samplingFlags);
    return primeFullCompactFinishSolidEvaluation(result, state, true);
}

/// 显式边界状态的联合评估；光滑／匹配 IOR、thin 的数值拒绝保持原入口约定。
public func primeFullCompactTransmissionEvaluateRaw<let Continuous : bool = false>(
    wi: float3, inputWo: float3, state: PrimeFullCompactTransmissionState, samplingFlags: uint)
    ->PrimeFullCompactTransmissionEvaluation
{
    if (state.geometryThinWalled == 0u)
    {
        var result : PrimeFullCompactTransmissionEvaluation = {};
        if (primeFullOpenPbrMicrofacetEffectivelySmooth(state.specularMicrofacet) ||
            state.specularFresnel.ior == 1.0)
            return result;
        result = primeFullCompactRefractiveEvaluateContinuous(
            primeFullCompactTransmissionMaterialFrame(wi, state),
            primeFullCompactTransmissionMaterialFrame(inputWo, state), state, samplingFlags);
        result.throughput = primeFullCompactFinishSolidEvaluation(result.throughput, state, false);
        return result;
    }
    var wo : float3 = inputWo;
    let transmitted : bool = wo.z < 0.0;
    if (transmitted)
        wo.z = -wo.z;
    let base
        : PrimeFullOpenPbrReflectiveEvalBase = primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
              wi, wo, state.transmissionMicrofacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_ALL);
    var result : PrimeFullCompactTransmissionEvaluation = {};
    if (transmitted ? primeFullOpenPbrIsTransmissive(samplingFlags)
                    : primeFullOpenPbrIsReflective(samplingFlags))
    {
        result.throughput.value = float3(base.factor);
        result.throughput.flags = base.flags;
        if (base.flags != PRIME_FULL_OPENPBR_FLAG_NONE)
        {
            let rt : PrimeFullOpenPbrVecPair =
                         primeFullCompactTransmissionThinWallRt(base.microCosine, state);
            if (transmitted)
            {
                result.throughput.value *= rt.second;
                result.throughput.flags = primeFullOpenPbrIsDelta(base.flags)
                                              ? PRIME_FULL_OPENPBR_FLAG_DELTA_TRANSMISSION
                                              : PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION;
            }
            else
                result.throughput.value *= rt.first;
            if (Continuous || primeFullOpenPbrIsNonDelta(result.throughput.flags))
                result.throughput.value /= primeFullOpenPbrMicrofacetDirectionalAlbedoUnity(
                    state.transmissionMicrofacet, wi.z);
        }
    }
    // PDF 的 Fresnel 原入口使用 normalize，保留该数值路径及无效输入边界。
    let halfVector : float3 = normalize(wi + wo);
    let rt : PrimeFullOpenPbrVecPair =
                 primeFullCompactTransmissionThinWallRt(dot(wi, halfVector), state);
    var reflectionProbability : float = primeFullOpenPbrSpectrumToWeight(rt.first);
    var transmissionProbability : float = primeFullOpenPbrSpectrumToWeight(rt.second);
    if (!primeFullOpenPbrIsReflective(samplingFlags))
        reflectionProbability = 0.0;
    if (!primeFullOpenPbrIsTransmissive(samplingFlags))
        transmissionProbability = 0.0;
    let probabilitySum : float = reflectionProbability + transmissionProbability;
    if (probabilitySum != 0.0)
        result.pdf = transmitted ? base.pdf * transmissionProbability / probabilitySum
                                 : base.pdf * reflectionProbability / probabilitySum;
    return result;
}

/// 已证明连续且非匹配 IOR 的 solid 联合评估，补偿仍在几何响应之后应用。
public func primeFullCompactTransmissionEvaluateRawSolidContinuous(
    wi: float3, inputWo: float3, state: PrimeFullCompactTransmissionState, samplingFlags: uint)
    ->PrimeFullCompactTransmissionEvaluation
{
    var result
        : PrimeFullCompactTransmissionEvaluation = primeFullCompactRefractiveEvaluateContinuous(
              primeFullCompactTransmissionMaterialFrame(wi, state),
              primeFullCompactTransmissionMaterialFrame(inputWo, state), state, samplingFlags);
    result.throughput = primeFullCompactFinishSolidEvaluation(result.throughput, state, true);
    return result;
}

public func primeFullCompactFinishSolidSample(result: PrimeFullOpenPbrSample,
                                              state: PrimeFullCompactTransmissionState,
                                              continuous: bool)
    ->PrimeFullOpenPbrSample
{
    var result = result;
    if (result.throughput.flags == PRIME_FULL_OPENPBR_FLAG_NONE)
    {
        return result;
    }
    result.wo = primeFullCompactTransmissionMaterialFrame(result.wo, state);
    if (primeFullOpenPbrIsTransmissive(result.throughput.flags))
    {
        result.throughput.value *= state.transmissionTint;
    }
    if (continuous || primeFullOpenPbrIsNonDelta(result.throughput.flags))
    {
        result.throughput.value *= primeFullOpenPbrIsTransmissive(result.throughput.flags)
                                       ? state.transmissionMultipleScattering.y
                                       : state.transmissionMultipleScattering.x;
    }
    return result;
}

public func primeFullCompactTransmissionSampleRaw<let Continuous : bool = false>(
    wi: float3, randomValue: float3, state: PrimeFullCompactTransmissionState, samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
    if (state.geometryThinWalled != 0u)
    {
        if (!Continuous &&
            primeFullOpenPbrMicrofacetEffectivelySmooth(state.transmissionMicrofacet))
        {
            return primeFullCompactThinWallDiscreteSampleRaw(wi, randomValue.z, state,
                                                             samplingFlags);
        }
        let base : PrimeFullOpenPbrReflectiveSampleBase =
                       primeFullOpenPbrReflectiveSampleBaseImpl<Continuous>(
                           wi, randomValue, state.transmissionMicrofacet, samplingFlags,
                           PRIME_FULL_OPENPBR_FLAG_ALL);
        result = base.bsdfSample;
        if (result.throughput.flags != PRIME_FULL_OPENPBR_FLAG_NONE)
        {
            let rt : PrimeFullOpenPbrVecPair =
                         primeFullCompactTransmissionThinWallRt(base.microCosine, state);
            var reflectionProbability : float = primeFullOpenPbrSpectrumToWeight(rt.first);
            var transmissionProbability : float = primeFullOpenPbrSpectrumToWeight(rt.second);
            if (!primeFullOpenPbrIsReflective(samplingFlags))
            {
                reflectionProbability = 0.0;
            }
            if (!primeFullOpenPbrIsTransmissive(samplingFlags))
            {
                transmissionProbability = 0.0;
            }
            let probabilitySum : float = reflectionProbability + transmissionProbability;
            if (probabilitySum == 0.0)
            {
                result = primeFullOpenPbrZeroSample();
                return result;
            }
            if (randomValue.z < transmissionProbability / probabilitySum)
            {
                result.throughput.value *= rt.second;
                result.pdf *= transmissionProbability / probabilitySum;
                result.throughput.flags =
                    (!Continuous && primeFullOpenPbrIsDelta(result.throughput.flags))
                        ? PRIME_FULL_OPENPBR_FLAG_DELTA_TRANSMISSION
                        : PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION;
                result.wo.z = -result.wo.z;
            }
            else
            {
                result.throughput.value *= rt.first;
                result.pdf *= reflectionProbability / probabilitySum;
            }
            if (Continuous || primeFullOpenPbrIsNonDelta(result.throughput.flags))
            {
                result.throughput.value /= primeFullOpenPbrMicrofacetDirectionalAlbedoUnity(
                    state.transmissionMicrofacet, wi.z);
            }
        }
        return result;
    }

    let materialWi : float3 = primeFullCompactTransmissionMaterialFrame(wi, state);
    result = primeFullCompactRefractiveSample(materialWi, randomValue, state, samplingFlags);
    return primeFullCompactFinishSolidSample(result, state, false);
}

public func primeFullCompactTransmissionSampleRawSolidContinuous(
    wi: float3, randomValue: float3, state: PrimeFullCompactTransmissionState, samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    let materialWi : float3 = primeFullCompactTransmissionMaterialFrame(wi, state);
    let result : PrimeFullOpenPbrSample = primeFullCompactRefractiveSampleContinuous(
                     materialWi, randomValue, state, samplingFlags);
    return primeFullCompactFinishSolidSample(result, state, true);
}

public func primeFullCompactTransmissionPdfRaw<let Continuous : bool = false>(
    wi: float3, inputWo: float3, state: PrimeFullCompactTransmissionState, samplingFlags: uint)
    ->float
{
    if (state.geometryThinWalled != 0u)
    {
        var wo : float3 = inputWo;
        var transmitted = false;
        if (wo.z < 0.0)
        {
            wo.z = -wo.z;
            transmitted = true;
        }
        let halfVector : float3 = normalize(wi + wo);
        let rt : PrimeFullOpenPbrVecPair =
                     primeFullCompactTransmissionThinWallRt(dot(wi, halfVector), state);
        var reflectionProbability : float = primeFullOpenPbrSpectrumToWeight(rt.first);
        var transmissionProbability : float = primeFullOpenPbrSpectrumToWeight(rt.second);
        if (!primeFullOpenPbrIsReflective(samplingFlags))
        {
            reflectionProbability = 0.0;
        }
        if (!primeFullOpenPbrIsTransmissive(samplingFlags))
        {
            transmissionProbability = 0.0;
        }
        let probabilitySum : float = reflectionProbability + transmissionProbability;
        if (probabilitySum == 0.0)
        {
            return 0.0;
        }
        let microfacetPdf
            : float = primeFullOpenPbrReflectivePdfImpl<Continuous>(
                  wi, wo, state.transmissionMicrofacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_ALL);
        return transmitted ? microfacetPdf * transmissionProbability / probabilitySum
                           : microfacetPdf * reflectionProbability / probabilitySum;
    }
    return primeFullCompactRefractivePdf(primeFullCompactTransmissionMaterialFrame(wi, state),
                                         primeFullCompactTransmissionMaterialFrame(inputWo, state),
                                         state, samplingFlags);
}

public func primeFullCompactTransmissionPdfRawSolidContinuous(
    wi: float3, inputWo: float3, state: PrimeFullCompactTransmissionState, samplingFlags: uint)
    ->float
{
    return primeFullCompactRefractivePdfContinuous(
        primeFullCompactTransmissionMaterialFrame(wi, state),
        primeFullCompactTransmissionMaterialFrame(inputWo, state), state, samplingFlags);
}
