// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/opaque/sample.slang";

import bsdf.openpbr.opaque.state;
import bsdf.openpbr.opaque.lobes;
import bsdf.openpbr.contract.flags;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.opaque.evaluate;
import bsdf.openpbr.math.spectrum;
import bsdf.openpbr.math.fresnel;
import bsdf.openpbr.service.support.conversion;

namespace openpbr
{
namespace kernel
{
func metallic_sample(wi: float3, random_value: float3, state: OpaqueState,
                     use_eon_diffuse_mapping: bool)
    ->Proposal
{
    let m : float = state.metalness;
    var result : Proposal = random_value.z < m
                                ? conductor_sample(wi, float3(random_value.xy, random_value.z / m),
                                                   state, LOBE_ALL)
                                : opaque_layer_sample(
                                      wi, float3(random_value.xy, (random_value.z - m) / (1.0 - m)),
                                      state, use_eon_diffuse_mapping);
    if (!has_sample(result))
        return result;
    if (is_delta(result.response.flags))
    {
        // Both smooth specular lobes put mass on the same reflection direction.
        // Diffuse response/density must never enter this discrete measure.
        result.response.value = lerp(specular_reflectance(state.dielectric_fresnel, wi.z),
                                     conductor_reflectance(state.conductor_fresnel, wi.z), m);
        result.response.flags = LOBE_DELTA_REFLECTION;
        result.pdf = m + (1.0 - m) * state.layer.specular_sample_weight;
    }
    else
    {
        let evaluated : LocalEvaluation = opaque_evaluate(wi, result.wo, state);
        result.response = evaluated.response;
        result.pdf = evaluated.pdf;
    }
    return result;
}

public func opaque_sample(wi: float3, random_value: float3, state: OpaqueState)->Proposal
{
    if (state.metalness > 0.0 && state.metalness < 1.0)
        return metallic_sample(wi, random_value, state, false);
    return state.kind == FULL_COMPACT_OPAQUE_CONDUCTOR
               ? conductor_sample(wi, random_value, state, LOBE_ALL)
               : opaque_layer_sample(wi, random_value, state, false);
}

public func opaque_sample_with_eon_diffuse(wi: float3, random_value: float3, state: OpaqueState)
    ->Proposal
{
    if (state.metalness > 0.0 && state.metalness < 1.0)
        return metallic_sample(wi, random_value, state, true);
    return state.kind == FULL_COMPACT_OPAQUE_CONDUCTOR
               ? conductor_sample(wi, random_value, state, LOBE_ALL)
               : opaque_layer_sample(wi, random_value, state, true);
}
}
}
