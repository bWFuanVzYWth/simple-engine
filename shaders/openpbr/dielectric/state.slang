// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/dielectric/state.slang";

import openpbr.dielectric.contract;
import openpbr.math.microfacet_measure;
import openpbr.math.microfacet_energy;
import openpbr.contract.material;
import openpbr.contract.volume;
import openpbr.contract.sample;
import openpbr.math.spectrum;
import openpbr.model.volume;

namespace openpbr {
namespace kernel {
public func transmission_closure_state(state: TransmissionState)->TransmissionState {
    var state = state;
    if (state.geometry_thin_walled == 0u) {
        state.entering = 1u;
    }
    return state;
}

public func transmission_supports(material: FullMaterial)->bool {
    return material.weight.base == 1.0 && material.weight.diffuse_roughness == 0.0 &&
           material.weight.specular == 1.0 && material.weight.metalness == 0.0 &&
           material.weight.subsurface == 0.0 && material.weight.transmission == 1.0 &&
           material.weight.coat == 0.0 && material.weight.fuzz == 0.0 &&
           material.weight.thin_film == 0.0 && material.weight.diffraction == 0.0 &&
           material.transmission.dispersion_scale == 0.0;
}

// energy_cosine is abs(wi.z) for the material frame, or wi.z for the physical interface.
// Selecting it here avoids constructing and then replacing the exiting-interface LUT result.
public func transmission_state_init<EnergyLut>(energy_lut: EnergyLut, material: FullMaterial,
                                               wi: float3, outside_medium: Volume, ray_t: float,
                                               energy_cosine: float)
    ->TransmissionState
    where EnergyLut : openpbr::IEnergyLut {
    var state : TransmissionState;
    state.frame = material.geometry.frame;
    state.geometry_thin_walled = material.geometry.thin_walled;
    state.entering = wi.z >= 0.0 ? 1u : 0u;
    state.ray_t = ray_t;
    state.original_ior = material.specular.ior;
    state.transmission_color = material.transmission.color;
    state.specular_microfacet.alpha = specular_ndf_roughnesses(
        material.specular.roughness, material.specular.roughness_anisotropy);
    state.specular_fresnel.ior = material.specular.ior * (1.0 / outside_medium.ior);
    state.specular_fresnel.energy_ior =
        ior_adjustment(state.specular_fresnel.ior, material.weight.specular);
    state.specular_fresnel.color = material.specular.color;
    state.transmission_tint =
        material.transmission.depth == 0.0 ? material.transmission.color : float3(1.0);
    state.transmission_volume = volume_from_transmission(material.transmission);
    state.transmission_volume.ior = material.specular.ior;
    if (state.geometry_thin_walled == 0u && state.entering == 0u) {
        state.transmission_volume = outside_medium;
    }
    state.transmission_microfacet.alpha = float2(0.0);
    state.transmission_multiple_scattering = float3(1.0, 1.0, 0.0);
    if (state.geometry_thin_walled != 0u) {
        state.transmission_microfacet.alpha =
            clamp(state.specular_microfacet.alpha *
                      thin_dielectric_roughness_scaler2(state.specular_fresnel.ior),
                  float2(0.0), float2(1.0));
    } else {
        state.transmission_multiple_scattering = dielectric_energy(
            energy_lut, state.specular_microfacet, energy_cosine, state.specular_fresnel.ior);
    }
    return state;
}

public func transmission_state_init<EnergyLut>(energy_lut: EnergyLut, material: FullMaterial,
                                               wi: float3, outside_medium: Volume, ray_t: float)
    ->TransmissionState
    where EnergyLut : openpbr::IEnergyLut {
    return transmission_state_init(energy_lut, material, wi, outside_medium, ray_t, abs(wi.z));
}

public func transmission_interface_state<EnergyLut>(energy_lut: EnergyLut, wi: float3,
                                                    state: TransmissionState)
    ->TransmissionState
    where EnergyLut : openpbr::IEnergyLut {
    var state = state;
    if (state.geometry_thin_walled != 0u || state.entering != 0u) {
        return state;
    }
    state.transmission_multiple_scattering =
        dielectric_energy(energy_lut, state.specular_microfacet, wi.z, state.specular_fresnel.ior);
    return state;
}
}
}
