// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/sample.slang";

import "bsdf/openpbr/contract/volume_sample.slang";
import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/dielectric/refract.slang";
import "bsdf/openpbr/dielectric/state.slang";
import "bsdf/openpbr/dielectric/volume.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/contract/volume.slang";
import "bsdf/openpbr/contract/sample.slang";

public PrimeFullOpenPbrSampleResult primeFullCompactTransmissionSample(
    float3 wi, float3 randomValue, PrimeFullCompactTransmissionState state, uint samplingFlags,
    PrimeFullOpenPbrVolume medium)
{
    PrimeFullOpenPbrSampleResult result;
    result.medium = medium;
    result.rayT = state.rayT;
    result.bsdfSample = primeFullCompactTransmissionSampleRaw(
        wi, randomValue, primeFullCompactTransmissionClosureState(state), samplingFlags);
    primeFullCompactApplyTransmissionVolumeTransition(state, result);
    return result;
}
