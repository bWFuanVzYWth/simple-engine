// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/volume.slang";

import "bsdf/openpbr/contract/volume_sample.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/contract/volume.slang";
import "bsdf/openpbr/dielectric/contract.slang";

public void primeFullCompactApplyTransmissionVolumeTransition(
    PrimeFullCompactTransmissionState state, inout PrimeFullOpenPbrSampleResult result)
{
    if (state.geometryThinWalled != 0u ||
        !primeFullOpenPbrIsTransmissive(result.bsdfSample.throughput.flags))
    {
        return;
    }
    result.medium = state.transmissionVolume;
    if (state.entering == 0u)
        result.rayT = 0.0;
}
