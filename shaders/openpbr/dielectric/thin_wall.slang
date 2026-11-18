// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/dielectric/thin_wall.slang";

import openpbr.dielectric.contract;
import openpbr.contract.sample;
import openpbr.math.fresnel;
import openpbr.math.spectrum;

__exported import openpbr.math.delta;

namespace openpbr {
namespace kernel {
public func transmission_thin_wall_rt(cosine_theta: float, state: TransmissionState)
    ->InterfaceResponse {
    return thin_wall_rt(cosine_theta, state.specular_fresnel, state.transmission_tint,
                        state.transmission_volume.extinction);
}
}
}
