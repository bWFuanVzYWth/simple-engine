// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/opaque/traits.slang";

import bsdf.openpbr.math.microfacet_measure;
import bsdf.openpbr.math.microfacet_energy;
import bsdf.openpbr.opaque.state;
import bsdf.openpbr.opaque.lobes;
import bsdf.openpbr.contract.flags;
import bsdf.openpbr.contract.material;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.math.spectrum;

namespace openpbr
{
namespace kernel
{
public func opaque_supports(material: FullMaterial)->bool
{
    return material.weight.base == 1.0 && material.weight.specular == 1.0 &&
           (material.weight.metalness == 0.0 || material.weight.metalness == 1.0) &&
           material.weight.transmission == 0.0 && material.weight.coat == 0.0 &&
           material.weight.fuzz == 0.0 && material.weight.thin_film == 0.0 &&
           material.weight.diffraction == 0.0 && material.weight.diffuse_roughness == 0.0 &&
           material.weight.subsurface >= 0.0 && material.weight.subsurface <= 1.0 &&
           (material.weight.subsurface == 0.0 ||
            (material.weight.metalness == 0.0 && material.geometry.thin_walled != 0u));
}

public func opaque_state_init<EnergyLut>(energy_lut: EnergyLut, material: FullMaterial, wi: float3,
                                         inverse_outside_ior: float)
    ->OpaqueState
    where EnergyLut : openpbr::IEnergyLut
{
    var state : OpaqueState;
    state.frame = material.geometry.frame;
    state.metalness = material.weight.metalness;
    state.kind = material.weight.metalness == 1.0    ? FULL_COMPACT_OPAQUE_CONDUCTOR
                 : material.weight.subsurface == 1.0 ? FULL_COMPACT_OPAQUE_SUBSURFACE
                 : material.weight.subsurface > 0.0  ? FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                                                     : FULL_COMPACT_OPAQUE_DIELECTRIC;
    state.base_color = material.base.color;
    state.subsurface_weight = material.weight.subsurface;
    state.subsurface_color = material.subsurface.color;
    state.subsurface_anisotropy = material.subsurface.scatter_anisotropy;
    state.microfacet.alpha = specular_ndf_roughnesses(material.specular.roughness,
                                                      material.specular.roughness_anisotropy);

    let interface_ior : float = material.specular.ior * inverse_outside_ior;
    state.dielectric_fresnel.ior = interface_ior;
    state.dielectric_fresnel.energy_ior = ior_adjustment(interface_ior, material.weight.specular);
    state.dielectric_fresnel.color = material.specular.color;
    state.dielectric_multiple_scattering =
        dielectric_energy(energy_lut, state.microfacet, wi.z, state.dielectric_fresnel.energy_ior);

    state.conductor_fresnel.f0 = material.base.color * material.weight.base;
    state.conductor_fresnel.f82 = material.specular.color;
    state.conductor_fresnel.f90 = float3(1.0);
    state.conductor_fresnel.weight = material.weight.specular;
    state.conductor_fresnel.energy_f0 =
        clamp(material.base.color * material.weight.base * material.weight.specular, float3(0.0),
              float3(1.0));
    state.conductor_ess = 1.0;
    if (state.kind == FULL_COMPACT_OPAQUE_CONDUCTOR &&
        !microfacet_effectively_smooth(state.microfacet))
    {
        state.conductor_ess = microfacet_directional_albedo_reflection(state.microfacet, wi.z,
                                                                       float3(1.0), float3(1.0))
                                  .x;
    }

    let diffuse_energy : float3 = state.kind == FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE &&
                                          state.subsurface_weight < 1.0
                                      ? state.base_color
                                      : float3(0.0);
    let subsurface_energy
        : float3 = state.kind != FULL_COMPACT_OPAQUE_CONDUCTOR && state.subsurface_weight > 0.0
                       ? subsurface_energy(state, LOBE_ALL)
                       : float3(0.0);
    state.diffuse_mix = make_mix_state(state.subsurface_weight, diffuse_energy, subsurface_energy);
    let base_energy : float3 = state.kind == FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                                   ? mixed_diffuse_energy(state, LOBE_ALL)
                               : state.kind == FULL_COMPACT_OPAQUE_SUBSURFACE
                                   ? subsurface_energy
                                   : lambert_energy(wi, state, LOBE_ALL);
    state.layer =
        make_layer_state(base_energy, float3(1.0 - state.dielectric_multiple_scattering.z),
                         opaque_dielectric_energy(wi, state, LOBE_ALL));
    return state;
}
}
}
