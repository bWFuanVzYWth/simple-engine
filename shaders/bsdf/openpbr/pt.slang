// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/pt.slang";

import bsdf.openpbr.contract.material;
import bsdf.openpbr.math.frame;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.contract.flags;
import bsdf.openpbr.service.support.conversion;
import bsdf.openpbr.math.spectrum;
import bsdf.openpbr.dielectric.contract;
import bsdf.openpbr.opaque.lobes;
import bsdf.openpbr.opaque.state;

import bsdf.openpbr.opaque.traits;
import bsdf.openpbr.opaque.evaluate;
import bsdf.openpbr.opaque.sample;
import bsdf.openpbr.dielectric.evaluate;
import bsdf.openpbr.dielectric.refract;
import bsdf.openpbr.dielectric.state;
import bsdf.openpbr.math.microfacet_energy;
import bsdf.openpbr.math.microfacet_measure;
import bsdf.openpbr.pt_subsurface;

// The legacy exact OpenPBR supported-domain kernels remain unchanged. PT supplies
// only their source-reachable parameters, after a visibility query; no generic
// material/default graph or closure survives a query.
namespace openpbr
{
namespace kernel
{
public func opaque_state<EnergyLut>(energy: EnergyLut, base_color: float3, roughness: float,
                                    interface_ior: float, subsurface_weight: float, conductor: bool,
                                    f0: float3, f82: float3, wi: float3)
    ->OpaqueState
    where EnergyLut : openpbr::IEnergyLut
{
    return opaque_state_impl<false>(energy, base_color, roughness, interface_ior, subsurface_weight,
                                    conductor, f0, f82, wi);
}

public func opaque_state_impl<let continuous : bool, let smooth : bool = false, EnergyLut>(
    energy: EnergyLut, base_color: float3, roughness: float, interface_ior: float,
    subsurface_weight: float, conductor: bool, f0: float3, f82: float3, wi: float3)
    ->OpaqueState
    where EnergyLut : openpbr::IEnergyLut
{
    var subsurface_weight = subsurface_weight;
    var state : OpaqueState = {};
    state.metalness = conductor ? 1.0 : 0.0;
    state.kind = conductor                  ? FULL_COMPACT_OPAQUE_CONDUCTOR
                 : subsurface_weight == 1.0 ? FULL_COMPACT_OPAQUE_SUBSURFACE
                 : subsurface_weight > 0.0  ? FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                                            : FULL_COMPACT_OPAQUE_DIELECTRIC;
    state.microfacet.alpha = smooth ? float2(0.0) : float2(roughness * roughness);
    if (conductor)
    {
        state.conductor_fresnel.f0 = f0;
        state.conductor_fresnel.f82 = f82;
        state.conductor_fresnel.f90 = float3(1.0);
        state.conductor_fresnel.weight = 1.0;
        state.conductor_fresnel.energy_f0 = clamp(f0, float3(0.0), float3(1.0));
        state.conductor_ess = (!continuous && microfacet_effectively_smooth(state.microfacet))
                                  ? 1.0
                                  : microfacet_directional_albedo_reflection(
                                        state.microfacet, wi.z, float3(1.0), float3(1.0))
                                        .x;
        return state;
    }
    state.base_color = base_color;
    state.subsurface_weight = subsurface_weight;
    state.subsurface_color = base_color;
    if (subsurface_weight > 0.0)
    {
        state.subsurface_color = subsurface_color(base_color);
    }
    state.subsurface_anisotropy = FULL_PT_SUBSURFACE_ANISOTROPY;
    state.dielectric_fresnel.ior = interface_ior;
    state.dielectric_fresnel.energy_ior = ior_adjustment(interface_ior, 1.0);
    state.dielectric_fresnel.color = float3(1.0);
    state.dielectric_multiple_scattering = dielectric_energy<smooth>(
        energy, state.microfacet, wi.z, state.dielectric_fresnel.energy_ior);
    let diffuse_energy
        : float3 = state.kind == FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE && subsurface_weight < 1.0
                       ? base_color
                       : float3(0.0);
    let subsurface_energy : float3 = subsurface_weight > 0.0 ? subsurface_energy(state, LOBE_ALL)
                                                             : float3(0.0);
    state.diffuse_mix = make_mix_state(subsurface_weight, diffuse_energy, subsurface_energy);
    let base_energy : float3 = state.kind == FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                                   ? mixed_diffuse_energy(state, LOBE_ALL)
                               : state.kind == FULL_COMPACT_OPAQUE_SUBSURFACE
                                   ? subsurface_energy
                                   : lambert_energy(wi, state, LOBE_ALL);
    state.layer =
        make_layer_state(base_energy, float3(1.0 - state.dielectric_multiple_scattering.z),
                         dielectric_energy_impl<continuous>(wi, state, LOBE_ALL));
    return state;
}

// Canonical metalness mixes the two existing physical endpoints. The component
// proposal uses the same m as its response weight, without another directional-energy
// lookup or a second state. State is prepared only at its final local consumer.
public func metallic_state<EnergyLut>(energy: EnergyLut, color: float3, roughness: float,
                                      interface_ior: float, subsurface: float, metalness: float,
                                      wi: float3)
    ->OpaqueState
    where EnergyLut : openpbr::IEnergyLut
{
    var metalness = metalness;
    var state : OpaqueState = opaque_state(energy, color, roughness, interface_ior, subsurface,
                                           metalness == 1.0, saturate(color), float3(1.0), wi);
    if (metalness == 0.0 || metalness == 1.0)
        return state;
    state.metalness = metalness;
    state.conductor_fresnel.f0 = saturate(color);
    state.conductor_fresnel.f82 = float3(1.0);
    state.conductor_fresnel.f90 = float3(1.0);
    state.conductor_fresnel.weight = 1.0;
    state.conductor_fresnel.energy_f0 = saturate(color);
    state.conductor_ess = microfacet_effectively_smooth(state.microfacet)
                              ? 1.0
                              : microfacet_directional_albedo_reflection(state.microfacet, wi.z,
                                                                         float3(1.0), float3(1.0))
                                    .x;
    return state;
}

public func transmission_state<EnergyLut>(energy: EnergyLut, roughness: float, interface_ior: float,
                                          thin_walled: bool, thin_extinction: float3, wi: float3)
    ->TransmissionState
    where EnergyLut : openpbr::IEnergyLut
{
    var state : TransmissionState = {};
    state.geometry_thin_walled = thin_walled ? 1u : 0u;
    state.entering = wi.z >= 0.0 ? 1u : 0u;
    state.specular_microfacet.alpha = float2(roughness * roughness);
    state.specular_fresnel.ior = interface_ior;
    state.specular_fresnel.energy_ior = ior_adjustment(interface_ior, 1.0);
    state.specular_fresnel.color = float3(1.0);
    state.transmission_tint = float3(1.0);
    state.transmission_multiple_scattering = float3(1.0, 1.0, 0.0);
    if (thin_walled)
    {
        state.transmission_volume.extinction = thin_extinction;
        state.transmission_microfacet.alpha = clamp(
            state.specular_microfacet.alpha * thin_dielectric_roughness_scaler2(interface_ior),
            float2(0.0), float2(1.0));
    }
    else
    {
        state.transmission_multiple_scattering =
            dielectric_energy(energy, state.specular_microfacet, wi.z, interface_ior);
    }
    return state;
}

// MIS consumes the full closure response and its marginal proposal density,
// retaining the legacy sampler's exact selected direction and random mapping.
public func sample_opaque_marginal(wi: float3, random_value: float3, state: OpaqueState)->Proposal
{
    var sampled : Proposal = opaque_sample(wi, random_value, state);
    if (state.metalness == 0.0 && has_sample(sampled) && !is_delta(sampled.response.flags))
    {
        let evaluated : LocalEvaluation = opaque_evaluate(wi, sampled.wo, state);
        sampled.response.value = evaluated.response.value;
        sampled.pdf = evaluated.pdf;
    }
    return sampled;
}
}
}
