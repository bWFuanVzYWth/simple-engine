// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/math/delta.slang";

import openpbr.contract.sample;
import openpbr.contract.flags;
import openpbr.math.fresnel;
import openpbr.math.spectrum;
import openpbr.primary_policy;

namespace openpbr {
namespace kernel {
public func guarded_thin_wall_series_reciprocal(round_trip: float3)->float3 {
    return float3(1.0) / max(float3(denominator_tolerance), float3(1.0) - square(round_trip));
}

public func thin_wall_rt(cosine_theta: float, specular_fresnel: SpecularFresnel,
                         transmission_tint: float3, extinction: float3)
    ->InterfaceResponse {
    let fresnel : InterfaceResponse = specular_interface_response(specular_fresnel, cosine_theta);
    let t0 : float = specular_fresnel.ior == 1.0
                         ? cosine_theta * cosine_theta
                         : 1.0 - (1.0 - cosine_theta * cosine_theta) / square(specular_fresnel.ior);
    if (t0 <= 0.0) {
        return fresnel;
    }
    let cosine_theta_i : float = sqrt(t0);
    let absorption : float3 = transmission_tint * exp(-extinction / cosine_theta_i);
    let inverse_series : float3 =
                             guarded_thin_wall_series_reciprocal(fresnel.reflection * absorption);
    var result : InterfaceResponse;
    result.transmission = square(fresnel.transmission) * absorption * inverse_series;
    result.reflection = fresnel.reflection * (float3(1.0) + result.transmission * absorption);
    return result;
}

public func refractive_delta_sample<let balanced : bool = false>(wi: float3, random_event: float,
                                                                 specular_fresnel: SpecularFresnel,
                                                                 sampling_flags: uint)
    ->Proposal {
    let micro_incident : float = wi.z;
    let rt : InterfaceResponse = specular_interface_response(specular_fresnel, micro_incident);
    var reflection : float3 = rt.reflection;
    let transmission : float3 = rt.transmission;
    var reflection_probability : float = spectrum_to_weight(reflection);
    var transmission_probability : float = spectrum_to_weight(transmission);
    if (!is_reflective(sampling_flags)) {
        reflection_probability = 0.0;
    }
    if (!is_transmissive(sampling_flags)) {
        transmission_probability = 0.0;
    }
    let refracted : RefractResult =
                        specular_fresnel_refract(specular_fresnel, -wi, float3(0.0, 0.0, 1.0));
    if (refracted.valid == 0u) {
        transmission_probability = 0.0;
        reflection = float3(1.0);
    }
    if (balanced) {
        let reflection_weight : float = openpbr::primary::reflection_probability(
                                    reflection_probability, transmission_probability);
        let available : bool = reflection_probability + transmission_probability > 0;
        reflection_probability = available ? reflection_weight : 0;
        transmission_probability = available ? 1 - reflection_weight : 0;
    }
    let probability_sum : float = reflection_probability + transmission_probability;
    if (probability_sum == 0.0) {
        return empty_proposal();
    }
    var result : Proposal = empty_proposal();
    if (random_event < transmission_probability / probability_sum) {
        var wo : float3 = refracted.wo;
        if (wi.z * wo.z >= 0.0) {
            return empty_proposal();
        }
        result.wo = wo;
        result.eta = refracted.relative_ior;
        result.response.value = transmission / square(refracted.relative_ior);
        result.response.flags = LOBE_DELTA_TRANSMISSION;
        result.pdf = transmission_probability / probability_sum;
    } else {
        var wo : float3 = reflect(-wi, float3(0.0, 0.0, 1.0));
        if (wi.z * wo.z < 0.0) {
            return empty_proposal();
        }
        result.wo = wo;
        result.response.value = reflection;
        result.response.flags = LOBE_DELTA_REFLECTION;
        result.pdf = reflection_probability / probability_sum;
    }
    return result;
}

public func thin_wall_delta_sample<let balanced : bool = false>(wi: float3, random_event: float,
                                                                specular_fresnel: SpecularFresnel,
                                                                transmission_tint: float3,
                                                                extinction: float3,
                                                                sampling_flags: uint)
    ->Proposal {
    var result : Proposal = empty_proposal();
    if (wi.z <= 0.0 || !is_delta(sampling_flags)) {
        return result;
    }
    let rt : InterfaceResponse =
                 thin_wall_rt(wi.z, specular_fresnel, transmission_tint, extinction);
    var reflection_probability : float = spectrum_to_weight(rt.reflection);
    var transmission_probability : float = spectrum_to_weight(rt.transmission);
    if (!is_reflective(sampling_flags)) {
        reflection_probability = 0.0;
    }
    if (!is_transmissive(sampling_flags)) {
        transmission_probability = 0.0;
    }
    if (balanced) {
        let reflection_weight : float = openpbr::primary::reflection_probability(
                                    reflection_probability, transmission_probability);
        let available : bool = reflection_probability + transmission_probability > 0;
        reflection_probability = available ? reflection_weight : 0;
        transmission_probability = available ? 1 - reflection_weight : 0;
    }
    let probability_sum : float = reflection_probability + transmission_probability;
    if (probability_sum == 0.0) {
        return result;
    }
    result.wo = reflect(-wi, float3(0.0, 0.0, 1.0));
    result.pdf = 1.0;
    if (random_event < transmission_probability / probability_sum) {
        result.response.value = rt.transmission;
        result.pdf *= transmission_probability / probability_sum;
        result.response.flags = LOBE_DELTA_TRANSMISSION;
        result.wo.z = -result.wo.z;
    } else {
        result.response.value = rt.reflection;
        result.pdf *= reflection_probability / probability_sum;
        result.response.flags = LOBE_DELTA_REFLECTION;
    }
    return result;
}
}
}
