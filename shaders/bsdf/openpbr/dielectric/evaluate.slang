// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/evaluate.slang";

import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/dielectric/refract.slang";
import "bsdf/openpbr/dielectric/state.slang";

public PrimeFullCompactTransmissionEvaluation primeFullCompactTransmissionEvaluate(
    float3 wi, float3 wo, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    PrimeFullCompactTransmissionState closure = primeFullCompactTransmissionClosureState(state);
    return primeFullCompactTransmissionEvaluateRaw(wi, wo, closure, samplingFlags);
}

public PrimeFullCompactTransmissionEvaluation primeFullCompactTransmissionEvaluateSolidContinuous(
    float3 wi, float3 wo, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    PrimeFullCompactTransmissionState closure = primeFullCompactTransmissionClosureState(state);
    return primeFullCompactTransmissionEvaluateRawSolidContinuous(wi, wo, closure, samplingFlags);
}
