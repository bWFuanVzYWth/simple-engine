// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/dielectric/volume.slang";

import openpbr.contract.volume_sample;
import openpbr.contract.flags;
import openpbr.contract.sample;
import openpbr.contract.volume;
import openpbr.dielectric.contract;

namespace openpbr {
namespace kernel {
public func apply_transmission_volume_transition(state: TransmissionState,
                                                 inout result: SampleResult)
    ->void {
    if (state.geometry_thin_walled != 0u || !is_transmissive(result.proposal.response.flags)) {
        return;
    }
    result.medium = state.transmission_volume;
    if (state.entering == 0u)
        result.ray_t = 0.0;
}
}
}
