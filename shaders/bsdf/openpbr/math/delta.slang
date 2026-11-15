// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/delta.slang";

import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/spectrum.slang";
import bsdf.openpbr.primary_policy;

public func primeFullCompactGuardedThinWallSeriesReciprocal(roundTrip: float3)->float3
{
    return float3(1.0) / max(float3(PRIME_FULL_OPENPBR_DENOM_TOLERANCE),
                             float3(1.0) - primeFullOpenPbrSquare(roundTrip));
}

public func primeFullThinWallRt(cosineTheta: float,
                                specularFresnel: PrimeFullOpenPbrSpecularFresnel,
                                transmissionTint: float3, extinction: float3)
    ->PrimeFullOpenPbrVecPair
{
    let fresnel : PrimeFullOpenPbrVecPair =
                      primeFullOpenPbrSpecularFresnelRt(specularFresnel, cosineTheta);
    let t0 : float = specularFresnel.ior == 1.0
                         ? cosineTheta * cosineTheta
                         : 1.0 - (1.0 - cosineTheta * cosineTheta) /
                                     primeFullOpenPbrSquare(specularFresnel.ior);
    if (t0 <= 0.0)
    {
        return fresnel;
    }
    let cosineThetaI : float = sqrt(t0);
    let absorption : float3 = transmissionTint * exp(-extinction / cosineThetaI);
    let inverseSeries
        : float3 = primeFullCompactGuardedThinWallSeriesReciprocal(fresnel.first * absorption);
    var result : PrimeFullOpenPbrVecPair;
    result.second = primeFullOpenPbrSquare(fresnel.second) * absorption * inverseSeries;
    result.first = fresnel.first * (float3(1.0) + result.second * absorption);
    return result;
}

public func primeFullRefractiveDeltaSample<let Balanced : bool = false>(
    wi: float3, randomEvent: float, specularFresnel: PrimeFullOpenPbrSpecularFresnel,
    samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    let microIncident : float = wi.z;
    let rt : PrimeFullOpenPbrVecPair =
                 primeFullOpenPbrSpecularFresnelRt(specularFresnel, microIncident);
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
    let refracted
        : PrimeFullOpenPbrRefractResult =
              primeFullOpenPbrSpecularFresnelRefract(specularFresnel, -wi, float3(0.0, 0.0, 1.0));
    if (refracted.valid == 0u)
    {
        transmissionProbability = 0.0;
        reflection = float3(1.0);
    }
    if (Balanced)
    {
        let reflectionWeight : float = openpbr_primary::reflection_probability(
                                   reflectionProbability, transmissionProbability);
        let available : bool = reflectionProbability + transmissionProbability > 0;
        reflectionProbability = available ? reflectionWeight : 0;
        transmissionProbability = available ? 1 - reflectionWeight : 0;
    }
    let probabilitySum : float = reflectionProbability + transmissionProbability;
    if (probabilitySum == 0.0)
    {
        return primeFullOpenPbrZeroSample();
    }
    var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
    if (randomEvent < transmissionProbability / probabilitySum)
    {
        var wo : float3 = refracted.wo;
        if (wi.z * wo.z >= 0.0)
        {
            return primeFullOpenPbrZeroSample();
        }
        result.wo = wo;
        result.eta = refracted.relativeIor;
        result.throughput.value = transmission / primeFullOpenPbrSquare(refracted.relativeIor);
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DELTA_TRANSMISSION;
        result.pdf = transmissionProbability / probabilitySum;
    }
    else
    {
        var wo : float3 = reflect(-wi, float3(0.0, 0.0, 1.0));
        if (wi.z * wo.z < 0.0)
        {
            return primeFullOpenPbrZeroSample();
        }
        result.wo = wo;
        result.throughput.value = reflection;
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION;
        result.pdf = reflectionProbability / probabilitySum;
    }
    return result;
}

public func primeFullThinWallDeltaSample<let Balanced : bool = false>(
    wi: float3, randomEvent: float, specularFresnel: PrimeFullOpenPbrSpecularFresnel,
    transmissionTint: float3, extinction: float3, samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
    if (wi.z <= 0.0 || !primeFullOpenPbrIsDelta(samplingFlags))
    {
        return result;
    }
    let rt : PrimeFullOpenPbrVecPair =
                 primeFullThinWallRt(wi.z, specularFresnel, transmissionTint, extinction);
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
    if (Balanced)
    {
        let reflectionWeight : float = openpbr_primary::reflection_probability(
                                   reflectionProbability, transmissionProbability);
        let available : bool = reflectionProbability + transmissionProbability > 0;
        reflectionProbability = available ? reflectionWeight : 0;
        transmissionProbability = available ? 1 - reflectionWeight : 0;
    }
    let probabilitySum : float = reflectionProbability + transmissionProbability;
    if (probabilitySum == 0.0)
    {
        return result;
    }
    result.wo = reflect(-wi, float3(0.0, 0.0, 1.0));
    result.pdf = 1.0;
    if (randomEvent < transmissionProbability / probabilitySum)
    {
        result.throughput.value = rt.second;
        result.pdf *= transmissionProbability / probabilitySum;
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DELTA_TRANSMISSION;
        result.wo.z = -result.wo.z;
    }
    else
    {
        result.throughput.value = rt.first;
        result.pdf *= reflectionProbability / probabilitySum;
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION;
    }
    return result;
}
