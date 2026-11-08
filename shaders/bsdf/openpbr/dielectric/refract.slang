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
public PrimeFullOpenPbrThroughput primeFullCompactRefractiveEval(
    float3 wi, float3 wo, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    PrimeFullOpenPbrThroughput result = primeFullOpenPbrZeroThroughput();
    PrimeFullOpenPbrMicrofacet microfacet = state.specularMicrofacet;
    PrimeFullOpenPbrSpecularFresnel fresnel = state.specularFresnel;
    if (primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet) || fresnel.ior == 1.0)
    {
        return result;
    }
    return primeFullCompactRefractiveEvalContinuous(wi, wo, state, samplingFlags);
}

public PrimeFullOpenPbrThroughput primeFullCompactRefractiveEvalContinuous(
    float3 wi, float3 wo, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    PrimeFullOpenPbrThroughput result = primeFullOpenPbrZeroThroughput();
    PrimeFullOpenPbrMicrofacet microfacet = state.specularMicrofacet;
    PrimeFullOpenPbrSpecularFresnel fresnel = state.specularFresnel;
    bool reflected = wi.z * wo.z > 0.0;
    float etaPath = reflected ? 1.0 : (wi.z > 0.0 ? fresnel.ior : 1.0 / fresnel.ior);
    float3 halfVector = wi + etaPath * wo;
    float lengthSquared = dot(halfVector, halfVector);
    if (wo.z == 0.0 || wi.z == 0.0 || lengthSquared == 0.0)
    {
        return result;
    }
    halfVector *= rsqrt(lengthSquared);
    if (halfVector.z <= 0.0)
    {
        halfVector = -halfVector;
    }
    float microIncident = dot(wi, halfVector);
    float microOutgoing = dot(wo, halfVector);
    if (microIncident * wi.z < 0.0 || microOutgoing * wo.z < 0.0)
    {
        return result;
    }
    PrimeFullOpenPbrVecPair rt = primeFullOpenPbrSpecularFresnelRt(fresnel, microIncident);
    float distribution = primeFullOpenPbrMicrofacetD(microfacet, halfVector);
    if (reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION) != 0u)
    {
        result.value =
            rt.first * distribution * primeFullOpenPbrReflectionFactor(microfacet, wi, wo);
        result.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION;
    }
    else if (!reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION) != 0u)
    {
        float denominator = primeFullOpenPbrSquare(primeFullOpenPbrRefractionDenominator(
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
public PrimeFullCompactTransmissionEvaluation primeFullCompactRefractiveEvaluateContinuous(
    float3 wi, float3 wo, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    PrimeFullCompactTransmissionEvaluation result = {};
    if (!primeFullOpenPbrIsSpecular(samplingFlags))
        return result;
    PrimeFullOpenPbrMicrofacet microfacet = state.specularMicrofacet;
    PrimeFullOpenPbrSpecularFresnel fresnel = state.specularFresnel;
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
    if (reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION) != 0u)
    {
        result.throughput.value =
            rt.first * distribution * primeFullOpenPbrReflectionFactor(microfacet, wi, wo);
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION;
    }
    else if (!reflected && (samplingFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION) != 0u)
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
    if (!primeFullOpenPbrIsReflective(samplingFlags))
        reflectionProbability = 0.0;
    if (!primeFullOpenPbrIsTransmissive(samplingFlags))
        transmissionProbability = 0.0;
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

public PrimeFullOpenPbrSample primeFullCompactRefractiveSample(
    float3 wi, float3 randomValue, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    PrimeFullOpenPbrMicrofacet microfacet = state.specularMicrofacet;
    PrimeFullOpenPbrSpecularFresnel fresnel = state.specularFresnel;
    if (primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet) || fresnel.ior == 1.0)
    {
        return primeFullCompactRefractiveDiscreteSample(wi, randomValue.z, state, samplingFlags);
    }
    return primeFullCompactRefractiveSampleContinuous(wi, randomValue, state, samplingFlags);
}

public PrimeFullOpenPbrSample primeFullCompactRefractiveSampleContinuous(
    float3 wi, float3 randomValue, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    PrimeFullOpenPbrMicrofacet microfacet = state.specularMicrofacet;
    PrimeFullOpenPbrSpecularFresnel fresnel = state.specularFresnel;
    float3 halfVector = float3(0.0, 0.0, 1.0);
    float microfacetPdf = 1.0;
    float distribution = 1.0;
    if (!primeFullOpenPbrIsSpecular(samplingFlags))
    {
        return primeFullOpenPbrZeroSample();
    }
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
    if (!primeFullOpenPbrIsReflective(samplingFlags))
    {
        reflectionProbability = 0.0;
    }
    if (!primeFullOpenPbrIsTransmissive(samplingFlags))
    {
        transmissionProbability = 0.0;
    }
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

public float primeFullCompactRefractivePdf(float3 wi, float3 wo,
                                           PrimeFullCompactTransmissionState state,
                                           uint samplingFlags)
{
    if (primeFullOpenPbrMicrofacetEffectivelySmooth(state.specularMicrofacet) ||
        state.specularFresnel.ior == 1.0)
        return 0.0;
    return primeFullCompactRefractivePdfContinuous(wi, wo, state, samplingFlags);
}

public float primeFullCompactRefractivePdfContinuous(float3 wi, float3 wo,
                                                     PrimeFullCompactTransmissionState state,
                                                     uint samplingFlags)
{
    PrimeFullOpenPbrMicrofacet microfacet = state.specularMicrofacet;
    PrimeFullOpenPbrSpecularFresnel fresnel = state.specularFresnel;
    if (!primeFullOpenPbrIsSpecular(samplingFlags))
    {
        return 0.0;
    }
    bool reflected = wi.z * wo.z > 0.0;
    float etaPath = reflected ? 1.0 : (wi.z > 0.0 ? fresnel.ior : 1.0 / fresnel.ior);
    float3 halfVector = wi + etaPath * wo;
    float lengthSquared = dot(halfVector, halfVector);
    if (wo.z == 0.0 || wi.z == 0.0 || lengthSquared == 0.0)
    {
        return 0.0;
    }
    halfVector *= rsqrt(lengthSquared);
    if (halfVector.z <= 0.0)
    {
        halfVector = -halfVector;
    }
    float microIncident = dot(wi, halfVector);
    float microOutgoing = dot(wo, halfVector);
    if (microIncident * wi.z < 0.0 || microOutgoing * wo.z < 0.0)
    {
        return 0.0;
    }
    PrimeFullOpenPbrVecPair rt = primeFullOpenPbrSpecularFresnelRt(fresnel, microIncident);
    float reflectionProbability = primeFullOpenPbrSpectrumToWeight(rt.first);
    float transmissionProbability = primeFullOpenPbrSpectrumToWeight(rt.second);
    if (!primeFullOpenPbrIsReflective(samplingFlags))
    {
        reflectionProbability = 0.0;
    }
    if (!primeFullOpenPbrIsTransmissive(samplingFlags))
    {
        transmissionProbability = 0.0;
    }
    float probabilitySum = reflectionProbability + transmissionProbability;
    if (probabilitySum == 0.0)
    {
        return 0.0;
    }
    float microfacetPdf =
        primeFullOpenPbrMicrofacetPdf(microfacet, wi.z > 0.0 ? wi : -wi, halfVector, false);
    if (reflected)
    {
        return microfacetPdf / (4.0 * abs(microIncident)) * reflectionProbability / probabilitySum;
    }
    float denominator = primeFullOpenPbrSquare(
        primeFullOpenPbrRefractionDenominator(etaPath, microIncident, microOutgoing));
    return microfacetPdf * abs(microOutgoing) / denominator * transmissionProbability /
           probabilitySum;
}

public PrimeFullOpenPbrThroughput primeFullCompactFinishSolidEvaluation(
    PrimeFullOpenPbrThroughput result, PrimeFullCompactTransmissionState state, bool continuous)
{
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

public PrimeFullOpenPbrThroughput primeFullCompactTransmissionEvalRaw<
    let Continuous : bool = false>(float3 wi, float3 inputWo,
                                   PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    if (state.geometryThinWalled != 0u)
    {
        float3 wo = inputWo;
        bool transmitted = false;
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
        PrimeFullOpenPbrReflectiveEvalBase base =
            primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
                wi, wo, state.transmissionMicrofacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_ALL);
        PrimeFullOpenPbrThroughput result;
        result.value = float3(base.factor);
        result.flags = base.flags;
        if (result.flags != PRIME_FULL_OPENPBR_FLAG_NONE)
        {
            PrimeFullOpenPbrVecPair rt =
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
    float3 materialWi = primeFullCompactTransmissionMaterialFrame(wi, state);
    float3 materialWo = primeFullCompactTransmissionMaterialFrame(inputWo, state);
    PrimeFullOpenPbrThroughput result =
        primeFullCompactRefractiveEval(materialWi, materialWo, state, samplingFlags);
    return primeFullCompactFinishSolidEvaluation(result, state, false);
}

public PrimeFullOpenPbrThroughput primeFullCompactTransmissionEvalRawSolidContinuous(
    float3 wi, float3 inputWo, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    float3 materialWi = primeFullCompactTransmissionMaterialFrame(wi, state);
    float3 materialWo = primeFullCompactTransmissionMaterialFrame(inputWo, state);
    PrimeFullOpenPbrThroughput result =
        primeFullCompactRefractiveEvalContinuous(materialWi, materialWo, state, samplingFlags);
    return primeFullCompactFinishSolidEvaluation(result, state, true);
}

/// 显式边界状态的联合评估；光滑／匹配 IOR、thin 的数值拒绝保持原入口约定。
public PrimeFullCompactTransmissionEvaluation primeFullCompactTransmissionEvaluateRaw<
    let Continuous : bool = false>(float3 wi, float3 inputWo,
                                   PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    if (state.geometryThinWalled == 0u)
    {
        PrimeFullCompactTransmissionEvaluation result = {};
        if (primeFullOpenPbrMicrofacetEffectivelySmooth(state.specularMicrofacet) ||
            state.specularFresnel.ior == 1.0)
            return result;
        result = primeFullCompactRefractiveEvaluateContinuous(
            primeFullCompactTransmissionMaterialFrame(wi, state),
            primeFullCompactTransmissionMaterialFrame(inputWo, state), state, samplingFlags);
        result.throughput = primeFullCompactFinishSolidEvaluation(result.throughput, state, false);
        return result;
    }
    float3 wo = inputWo;
    bool transmitted = wo.z < 0.0;
    if (transmitted)
        wo.z = -wo.z;
    PrimeFullOpenPbrReflectiveEvalBase base = primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(
        wi, wo, state.transmissionMicrofacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_ALL);
    PrimeFullCompactTransmissionEvaluation result = {};
    if (transmitted ? primeFullOpenPbrIsTransmissive(samplingFlags)
                    : primeFullOpenPbrIsReflective(samplingFlags))
    {
        result.throughput.value = float3(base.factor);
        result.throughput.flags = base.flags;
        if (base.flags != PRIME_FULL_OPENPBR_FLAG_NONE)
        {
            PrimeFullOpenPbrVecPair rt =
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
    float3 halfVector = normalize(wi + wo);
    PrimeFullOpenPbrVecPair rt = primeFullCompactTransmissionThinWallRt(dot(wi, halfVector), state);
    float reflectionProbability = primeFullOpenPbrSpectrumToWeight(rt.first);
    float transmissionProbability = primeFullOpenPbrSpectrumToWeight(rt.second);
    if (!primeFullOpenPbrIsReflective(samplingFlags))
        reflectionProbability = 0.0;
    if (!primeFullOpenPbrIsTransmissive(samplingFlags))
        transmissionProbability = 0.0;
    float probabilitySum = reflectionProbability + transmissionProbability;
    if (probabilitySum != 0.0)
        result.pdf = transmitted ? base.pdf * transmissionProbability / probabilitySum
                                 : base.pdf * reflectionProbability / probabilitySum;
    return result;
}

/// 已证明连续且非匹配 IOR 的 solid 联合评估，补偿仍在几何响应之后应用。
public PrimeFullCompactTransmissionEvaluation
primeFullCompactTransmissionEvaluateRawSolidContinuous(float3 wi, float3 inputWo,
                                                       PrimeFullCompactTransmissionState state,
                                                       uint samplingFlags)
{
    PrimeFullCompactTransmissionEvaluation result = primeFullCompactRefractiveEvaluateContinuous(
        primeFullCompactTransmissionMaterialFrame(wi, state),
        primeFullCompactTransmissionMaterialFrame(inputWo, state), state, samplingFlags);
    result.throughput = primeFullCompactFinishSolidEvaluation(result.throughput, state, true);
    return result;
}

public PrimeFullOpenPbrSample primeFullCompactFinishSolidSample(
    PrimeFullOpenPbrSample result, PrimeFullCompactTransmissionState state, bool continuous)
{
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

public PrimeFullOpenPbrSample primeFullCompactTransmissionSampleRaw<let Continuous : bool = false>(
    float3 wi, float3 randomValue, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    PrimeFullOpenPbrSample result = primeFullOpenPbrZeroSample();
    if (state.geometryThinWalled != 0u)
    {
        if (!Continuous &&
            primeFullOpenPbrMicrofacetEffectivelySmooth(state.transmissionMicrofacet))
        {
            return primeFullCompactThinWallDiscreteSampleRaw(wi, randomValue.z, state,
                                                             samplingFlags);
        }
        PrimeFullOpenPbrReflectiveSampleBase base =
            primeFullOpenPbrReflectiveSampleBaseImpl<Continuous>(
                wi, randomValue, state.transmissionMicrofacet, samplingFlags,
                PRIME_FULL_OPENPBR_FLAG_ALL);
        result = base.bsdfSample;
        if (result.throughput.flags != PRIME_FULL_OPENPBR_FLAG_NONE)
        {
            PrimeFullOpenPbrVecPair rt =
                primeFullCompactTransmissionThinWallRt(base.microCosine, state);
            float reflectionProbability = primeFullOpenPbrSpectrumToWeight(rt.first);
            float transmissionProbability = primeFullOpenPbrSpectrumToWeight(rt.second);
            if (!primeFullOpenPbrIsReflective(samplingFlags))
            {
                reflectionProbability = 0.0;
            }
            if (!primeFullOpenPbrIsTransmissive(samplingFlags))
            {
                transmissionProbability = 0.0;
            }
            float probabilitySum = reflectionProbability + transmissionProbability;
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

    float3 materialWi = primeFullCompactTransmissionMaterialFrame(wi, state);
    result = primeFullCompactRefractiveSample(materialWi, randomValue, state, samplingFlags);
    return primeFullCompactFinishSolidSample(result, state, false);
}

public PrimeFullOpenPbrSample primeFullCompactTransmissionSampleRawSolidContinuous(
    float3 wi, float3 randomValue, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    float3 materialWi = primeFullCompactTransmissionMaterialFrame(wi, state);
    PrimeFullOpenPbrSample result =
        primeFullCompactRefractiveSampleContinuous(materialWi, randomValue, state, samplingFlags);
    return primeFullCompactFinishSolidSample(result, state, true);
}

public float primeFullCompactTransmissionPdfRaw<let Continuous : bool = false>(
    float3 wi, float3 inputWo, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    if (state.geometryThinWalled != 0u)
    {
        float3 wo = inputWo;
        bool transmitted = false;
        if (wo.z < 0.0)
        {
            wo.z = -wo.z;
            transmitted = true;
        }
        float3 halfVector = normalize(wi + wo);
        PrimeFullOpenPbrVecPair rt =
            primeFullCompactTransmissionThinWallRt(dot(wi, halfVector), state);
        float reflectionProbability = primeFullOpenPbrSpectrumToWeight(rt.first);
        float transmissionProbability = primeFullOpenPbrSpectrumToWeight(rt.second);
        if (!primeFullOpenPbrIsReflective(samplingFlags))
        {
            reflectionProbability = 0.0;
        }
        if (!primeFullOpenPbrIsTransmissive(samplingFlags))
        {
            transmissionProbability = 0.0;
        }
        float probabilitySum = reflectionProbability + transmissionProbability;
        if (probabilitySum == 0.0)
        {
            return 0.0;
        }
        float microfacetPdf = primeFullOpenPbrReflectivePdfImpl<Continuous>(
            wi, wo, state.transmissionMicrofacet, samplingFlags, PRIME_FULL_OPENPBR_FLAG_ALL);
        return transmitted ? microfacetPdf * transmissionProbability / probabilitySum
                           : microfacetPdf * reflectionProbability / probabilitySum;
    }
    return primeFullCompactRefractivePdf(primeFullCompactTransmissionMaterialFrame(wi, state),
                                         primeFullCompactTransmissionMaterialFrame(inputWo, state),
                                         state, samplingFlags);
}

public float primeFullCompactTransmissionPdfRawSolidContinuous(
    float3 wi, float3 inputWo, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    return primeFullCompactRefractivePdfContinuous(
        primeFullCompactTransmissionMaterialFrame(wi, state),
        primeFullCompactTransmissionMaterialFrame(inputWo, state), state, samplingFlags);
}
