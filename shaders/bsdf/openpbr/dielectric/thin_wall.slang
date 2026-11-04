// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/thin_wall.slang";

import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/spectrum.slang";

__exported import "bsdf/openpbr/math/delta.slang";

public PrimeFullOpenPbrVecPair primeFullCompactTransmissionThinWallRt(
    float cosineTheta, PrimeFullCompactTransmissionState state)
{
    return primeFullThinWallRt(cosineTheta, state.specularFresnel, state.transmissionTint,
                               state.transmissionVolume.extinction);
}
