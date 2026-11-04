// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/opaque/sample.slang";

import "bsdf/openpbr/opaque/state.slang";
import "bsdf/openpbr/opaque/lobes.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/opaque/evaluate.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/service/support/conversion.slang";

PrimeFullOpenPbrSample primeFullCompactMetallicSample(float3 wi, float3 randomValue,
                                                      PrimeFullCompactOpaqueState state,
                                                      bool useEonDiffuseMapping)
{
    float m = state.metalness;
    PrimeFullOpenPbrSample result =
        randomValue.z < m
            ? primeFullCompactConductorSample(wi, float3(randomValue.xy, randomValue.z / m), state,
                                              PRIME_FULL_OPENPBR_FLAG_ALL)
            : primeFullCompactOpaqueLayerSample(
                  wi, float3(randomValue.xy, (randomValue.z - m) / (1.0 - m)), state,
                  useEonDiffuseMapping);
    if (!primeFullOpenPbrHasSample(result))
        return result;
    if (primeFullOpenPbrIsDelta(result.throughput.flags))
    {
        // Both smooth specular lobes put mass on the same reflection direction.
        // Diffuse response/density must never enter this discrete measure.
        result.throughput.value =
            lerp(primeFullOpenPbrSpecularFresnelUnpolarized(state.dielectricFresnel, wi.z),
                 primeFullOpenPbrConductorFresnelUnpolarized(state.conductorFresnel, wi.z), m);
        result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION;
        result.pdf = m + (1.0 - m) * state.layer.specularSampleWeight;
    }
    else
    {
        PrimeFullCompactOpaqueEvaluation evaluated =
            primeFullCompactOpaqueEvaluate(wi, result.wo, state);
        result.throughput = evaluated.throughput;
        result.pdf = evaluated.pdf;
    }
    return result;
}

public PrimeFullOpenPbrSample primeFullCompactOpaqueSample(float3 wi, float3 randomValue,
                                                           PrimeFullCompactOpaqueState state)
{
    if (state.metalness > 0.0 && state.metalness < 1.0)
        return primeFullCompactMetallicSample(wi, randomValue, state, false);
    return state.kind == PRIME_FULL_COMPACT_OPAQUE_CONDUCTOR
               ? primeFullCompactConductorSample(wi, randomValue, state,
                                                 PRIME_FULL_OPENPBR_FLAG_ALL)
               : primeFullCompactOpaqueLayerSample(wi, randomValue, state, false);
}

public PrimeFullOpenPbrSample primeFullCompactOpaqueSampleWithEonDiffuse(
    float3 wi, float3 randomValue, PrimeFullCompactOpaqueState state)
{
    if (state.metalness > 0.0 && state.metalness < 1.0)
        return primeFullCompactMetallicSample(wi, randomValue, state, true);
    return state.kind == PRIME_FULL_COMPACT_OPAQUE_CONDUCTOR
               ? primeFullCompactConductorSample(wi, randomValue, state,
                                                 PRIME_FULL_OPENPBR_FLAG_ALL)
               : primeFullCompactOpaqueLayerSample(wi, randomValue, state, true);
}
