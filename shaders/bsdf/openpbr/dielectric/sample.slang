// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/sample.slang";

import bsdf.openpbr.contract.volume_sample;
import bsdf.openpbr.dielectric.contract;
import bsdf.openpbr.dielectric.refract;
import bsdf.openpbr.dielectric.state;
import bsdf.openpbr.dielectric.volume;
import bsdf.openpbr.contract.flags;
import bsdf.openpbr.contract.volume;
import bsdf.openpbr.contract.sample;

namespace openpbr
{
namespace kernel
{
public func transmission_sample(wi: float3, random_value: float3, state: TransmissionState,
                                sampling_flags: uint, medium: Volume)
    ->SampleResult
{
    var result : SampleResult;
    result.medium = medium;
    result.ray_t = state.ray_t;
    result.proposal = transmission_sample_raw(wi, random_value, transmission_closure_state(state),
                                              sampling_flags);
    apply_transmission_volume_transition(state, result);
    return result;
}
}
}
