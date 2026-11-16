// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/discrete.slang";

import bsdf.openpbr.contract.volume_sample;
import bsdf.openpbr.contract.flags;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.contract.volume;
import bsdf.openpbr.dielectric.contract;
import bsdf.openpbr.dielectric.thin_wall;
import bsdf.openpbr.dielectric.volume;
import bsdf.openpbr.math.fresnel;
import bsdf.openpbr.math.delta;
import bsdf.openpbr.math.spectrum;

namespace openpbr
{
namespace kernel
{
public func refractive_discrete_sample(wi: float3, random_event: float, state: TransmissionState,
                                       sampling_flags: uint)
    ->Proposal
{
    return refractive_delta_sample(wi, random_event, state.specular_fresnel, sampling_flags);
}

public func thin_wall_discrete_sample_raw(wi: float3, random_event: float, state: TransmissionState,
                                          sampling_flags: uint)
    ->Proposal
{
    return thin_wall_delta_sample(wi, random_event, state.specular_fresnel, state.transmission_tint,
                                  state.transmission_volume.extinction, sampling_flags);
}

public func transmission_discrete_sample(wi: float3, random_event: float, state: TransmissionState,
                                         sampling_flags: uint, medium: Volume)
    ->SampleResult
{
    var closure_state : TransmissionState = state;
    if (closure_state.geometry_thin_walled == 0u)
    {
        closure_state.entering = 1u;
    }
    var result : SampleResult;
    result.medium = medium;
    result.ray_t = state.ray_t;
    if (closure_state.geometry_thin_walled != 0u)
    {
        result.proposal =
            thin_wall_discrete_sample_raw(wi, random_event, closure_state, sampling_flags);
    }
    else
    {
        result.proposal = refractive_discrete_sample(transmission_material_frame(wi, closure_state),
                                                     random_event, closure_state, sampling_flags);
        if (result.proposal.response.flags != LOBE_NONE)
        {
            result.proposal.wo = transmission_material_frame(result.proposal.wo, closure_state);
            if (is_transmissive(result.proposal.response.flags))
            {
                result.proposal.response.value *= closure_state.transmission_tint;
            }
        }
    }
    apply_transmission_volume_transition(state, result);
    return result;
}
}
}
