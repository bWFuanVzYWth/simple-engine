// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/thin_wall.slang";

import bsdf.openpbr.dielectric.contract;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.math.fresnel;
import bsdf.openpbr.math.spectrum;

__exported import bsdf.openpbr.math.delta;

namespace openpbr
{
namespace kernel
{
public func transmission_thin_wall_rt(cosine_theta: float, state: TransmissionState)
    ->InterfaceResponse
{
    return thin_wall_rt(cosine_theta, state.specular_fresnel, state.transmission_tint,
                        state.transmission_volume.extinction);
}
}
}
