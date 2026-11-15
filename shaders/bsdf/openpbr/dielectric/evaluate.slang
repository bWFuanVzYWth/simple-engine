// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/evaluate.slang";

import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/dielectric/refract.slang";
import "bsdf/openpbr/dielectric/state.slang";

public func primeFullCompactTransmissionEvaluate(wi: float3, wo: float3,
                                                 state: PrimeFullCompactTransmissionState,
                                                 samplingFlags: uint)
    ->PrimeFullCompactTransmissionEvaluation
{
    let closure : PrimeFullCompactTransmissionState =
                      primeFullCompactTransmissionClosureState(state);
    return primeFullCompactTransmissionEvaluateRaw(wi, wo, closure, samplingFlags);
}

public func primeFullCompactTransmissionEvaluateSolidContinuous(
    wi: float3, wo: float3, state: PrimeFullCompactTransmissionState, samplingFlags: uint)
    ->PrimeFullCompactTransmissionEvaluation
{
    let closure : PrimeFullCompactTransmissionState =
                      primeFullCompactTransmissionClosureState(state);
    return primeFullCompactTransmissionEvaluateRawSolidContinuous(wi, wo, closure, samplingFlags);
}
