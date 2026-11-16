// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/opaque/discrete.slang";

import bsdf.openpbr.contract.flags;
import bsdf.openpbr.contract.material;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.math.fresnel;
import bsdf.openpbr.math.microfacet_energy;
import bsdf.openpbr.math.spectrum;
import bsdf.openpbr.opaque.state;

namespace openpbr
{
namespace kernel
{
public struct OpaqueDiscreteState
{
    public var frame : Frame;
    public var microfacet : Microfacet;
    public var conductor_fresnel : ConductorFresnel;
};

public func opaque_discrete_state_init(material: FullMaterial)->OpaqueDiscreteState
{
    var state : OpaqueDiscreteState;
    state.frame = material.geometry.frame;
    state.microfacet.alpha = specular_ndf_roughnesses(material.specular.roughness,
                                                      material.specular.roughness_anisotropy);
    state.conductor_fresnel.f0 = material.base.color * material.weight.base;
    state.conductor_fresnel.f82 = material.specular.color;
    state.conductor_fresnel.f90 = float3(1.0);
    state.conductor_fresnel.weight = material.weight.specular;
    state.conductor_fresnel.energy_f0 =
        clamp(material.base.color * material.weight.base * material.weight.specular, float3(0.0),
              float3(1.0));
    return state;
}

public func opaque_discrete_state_from_full(state: OpaqueState)->OpaqueDiscreteState
{
    var result : OpaqueDiscreteState;
    result.frame = state.frame;
    result.microfacet = state.microfacet;
    result.conductor_fresnel = state.conductor_fresnel;
    return result;
}

public func conductor_discrete_sample(wi: float3, state: OpaqueDiscreteState, sampling_flags: uint)
    ->Proposal
{
    var result : Proposal = empty_proposal();
    if (wi.z <= 0.0 || (sampling_flags & LOBE_DELTA_REFLECTION) == 0u)
    {
        return result;
    }
    result.wo = reflect(-wi, float3(0.0, 0.0, 1.0));
    result.response.value = conductor_reflectance(state.conductor_fresnel, wi.z);
    result.response.flags = LOBE_DELTA_REFLECTION;
    result.pdf = 1.0;
    return result;
}

public func conductor_discrete_energy(wi: float3, state: OpaqueDiscreteState)->float3
{
    if (wi.z < 0.0)
    {
        return float3(0.0);
    }
    return microfacet_directional_albedo_reflection(
        state.microfacet, wi.z, state.conductor_fresnel.energy_f0,
        clamp(float3(state.conductor_fresnel.weight), float3(0.0), float3(1.0)));
}
}
}
