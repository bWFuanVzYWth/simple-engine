// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/opaque/lobes.slang";

import openpbr.math.fresnel;
import openpbr.math.microfacet_distribution;
import openpbr.math.microfacet_energy;
import openpbr.math.microfacet_measure;
import openpbr.opaque.discrete;
import openpbr.opaque.state;
import openpbr.contract.flags;
import openpbr.contract.sample;
import openpbr.math.spectrum;

import math.constants;
namespace openpbr {
namespace kernel {
public func make_layer_state(base_energy: float3, transmittance_in: float3, specular_energy: float3)
    ->LayerState {
    var result : LayerState;
    result.transmittance_in = transmittance_in;
    result.base_sample_weight = spectrum_to_weight(transmittance_in * base_energy);
    result.specular_sample_weight = spectrum_to_weight(specular_energy);
    let sum : float = result.base_sample_weight + result.specular_sample_weight;
    if (sum > 0.0) {
        result.base_sample_weight /= sum;
        result.specular_sample_weight /= sum;
    }
    return result;
}

public func make_mix_state(weight: float, first_energy: float3, second_energy: float3)->MixState {
    var result : MixState;
    if (weight == 0.0) {
        result.first_sample_weight = 1.0;
        result.second_sample_weight = 0.0;
        return result;
    }
    if (weight == 1.0) {
        result.first_sample_weight = 0.0;
        result.second_sample_weight = 1.0;
        return result;
    }
    result.first_sample_weight = (1.0 - weight) * spectrum_to_weight(first_energy);
    result.second_sample_weight = weight * spectrum_to_weight(second_energy);
    let sum : float = result.first_sample_weight + result.second_sample_weight;
    if (sum > 0.0) {
        result.first_sample_weight /= sum;
        result.second_sample_weight /= sum;
    }
    return result;
}

public func mix_eval(first: Response, second: Response, weight: float)->Response {
    var second = second;
    var result : Response = empty_response();
    if (weight < 1.0) {
        result = first;
    }
    if (weight > 0.0) {
        result.value *= 1.0 - weight;
        second.value *= weight;
        result = add_response(result, second);
    }
    return result;
}

public func layer_eval(base: Response, specular: Response, layer: LayerState)->Response {
    let result : Response = scale_response(base, layer.transmittance_in);
    return add_response(result, specular);
}

public func eon_basis(wi: float3)->float3x3 {
    let length_squared : float = dot(wi.xy, wi.xy);
    let x : float3 = length_squared > 0.0 ? float3(wi.x, wi.y, 0.0) * rsqrt(length_squared)
                                          : float3(1.0, 0.0, 0.0);
    let y = float3(-x.y, x.x, 0.0);
    return float3x3(x, y, float3(0.0, 0.0, 1.0));
}

public func lambert_eval(wi: float3, wo: float3, state: OpaqueState, sampling_flags: uint)
    ->Response {
    if (wi.z < 0.0 || wo.z < 0.0 || (sampling_flags & LOBE_DIFFUSE_REFLECTION) == 0u) {
        return empty_response();
    }
    var result : Response;
    result.value = state.base_color * (math::frac_1_pi * wo.z);
    result.flags = LOBE_DIFFUSE_REFLECTION;
    return result;
}

public func lambert_pdf(wi: float3, wo: float3)->float {
    return wi.z >= 0.0 && wo.z > 0.0 ? math::frac_1_pi * wo.z : 0.0;
}

public func lambert_energy(wi: float3, state: OpaqueState, sampling_flags: uint)->float3 {
    return wi.z >= 0.0 && (sampling_flags & LOBE_DIFFUSE_REFLECTION) != 0u ? state.base_color
                                                                           : float3(0.0);
}

public func lambert_sample(wi: float3, random_value: float3, state: OpaqueState,
                           sampling_flags: uint)
    ->Proposal {
    var result : Proposal = empty_proposal();
    if (wi.z < 0.0 || (sampling_flags & LOBE_DIFFUSE_REFLECTION) == 0u) {
        return result;
    }
    result.wo = cosine_sample_hemisphere(random_value.xy);
    result.pdf = math::frac_1_pi * result.wo.z;
    result.response.value = state.base_color * result.pdf;
    result.response.flags = LOBE_DIFFUSE_REFLECTION;
    return result;
}

// The generic OpenPBR diffuse closure uses EON even at diffuse roughness zero.
// Its value, PDF and energy reduce to Lambert, but its cLTC sampler rotates the
// cosine sample into the incident-direction basis. Preserve that exact mapping
// for fractional diffuse/subsurface mixtures without retaining general EON state.
public func zero_roughness_eon_sample(wi: float3, random_value: float3, state: OpaqueState,
                                      sampling_flags: uint)
    ->Proposal {
    var result : Proposal = empty_proposal();
    if (wi.z < 0.0 || (sampling_flags & LOBE_DIFFUSE_REFLECTION) == 0u) {
        return result;
    }
    if (random_value.z <= 0.0) {
        // EON uses <= for its zero-probability uniform branch. Although this has
        // zero measure, preserving it keeps boundary random inputs deterministic.
        result.wo = uniform_sample_hemisphere(random_value.xy);
        result.pdf = result.wo.z * math::frac_1_pi;
        result.response = lambert_eval(wi, result.wo, state, sampling_flags);
        return result;
    }
    let radius : float = sqrt(random_value.x);
    let phi : float = math::tau * random_value.y;
    let x : float = -radius * cos(phi);
    let y : float = radius * sin(phi);
    var wo = float3(x, y, sqrt(max(1.0 - (x * x + y * y), 0.0)));
    let wo_length : float = length(wo);
    result.pdf = wo.z * math::frac_1_pi * wo_length * wo_length * wo_length;
    result.wo = normalize(mul(wo, eon_basis(wi)));
    result.response = lambert_eval(wi, result.wo, state, sampling_flags);
    return result;
}

public func subsurface_eval<let fixed_subsurface : bool = false>(wi: float3, wo: float3,
                                                                 state: OpaqueState,
                                                                 sampling_flags: uint)
    ->Response {
    let reflected : bool = wo.z * wi.z > 0.0;
    if ((reflected && (sampling_flags & LOBE_DIFFUSE_REFLECTION) == 0u) ||
        (!reflected && (sampling_flags & LOBE_DIFFUSE_TRANSMISSION) == 0u)) {
        return empty_response();
    }
    var result : Response;
    result.value = float3(math::frac_1_pi * abs(wo.z)) * state.subsurface_color * 0.5 *
                   (fixed_subsurface ? (reflected ? 1.3333333730697632 : 0.6666666269302368)
                                     : (reflected ? 1.0 - state.subsurface_anisotropy
                                                  : 1.0 + state.subsurface_anisotropy));
    result.flags = reflected ? LOBE_DIFFUSE_REFLECTION : LOBE_DIFFUSE_TRANSMISSION;
    return result;
}

public func subsurface_reflection_probability<let fixed_subsurface : bool = false>(
    state: OpaqueState, sampling_flags: uint)
    ->float {
    let probability : float = fixed_subsurface ? 0.6666666865348816
                                               : (1.0 - state.subsurface_anisotropy) * 0.5;
    return is_reflective(sampling_flags) ? probability : 0.0;
}

public func subsurface_transmission_probability<let fixed_subsurface : bool = false>(
    state: OpaqueState, sampling_flags: uint)
    ->float {
    let probability : float = fixed_subsurface ? 0.3333333134651184
                                               : (1.0 + state.subsurface_anisotropy) * 0.5;
    return is_transmissive(sampling_flags) ? probability : 0.0;
}

public func subsurface_pdf<let fixed_subsurface : bool = false>(wi: float3, wo: float3,
                                                                state: OpaqueState,
                                                                sampling_flags: uint)
    ->float {
    let reflection_probability
        : float = subsurface_reflection_probability<fixed_subsurface>(state, sampling_flags);
    let transmission_probability
        : float = subsurface_transmission_probability<fixed_subsurface>(state, sampling_flags);
    if (!fixed_subsurface && reflection_probability == 0.0 && transmission_probability == 0.0) {
        return 0.0;
    }
    return math::frac_1_pi * abs(wo.z) *
           (wi.z * wo.z > 0.0 ? reflection_probability : transmission_probability);
}

public func subsurface_energy(state: OpaqueState, sampling_flags: uint)->float3 {
    if ((sampling_flags & LOBE_DIFFUSE_REFLECTION) != 0u) {
        return state.subsurface_color;
    }
    return float3(0.0);
}

public func subsurface_sample<let fixed_subsurface : bool = false>(wi: float3, random_value: float3,
                                                                   state: OpaqueState,
                                                                   sampling_flags: uint)
    ->Proposal {
    var result : Proposal = empty_proposal();
    if ((sampling_flags & LOBE_DIFFUSE) == 0u) {
        return result;
    }
    let reflection_probability
        : float = subsurface_reflection_probability<fixed_subsurface>(state, sampling_flags);
    let transmission_probability
        : float = subsurface_transmission_probability<fixed_subsurface>(state, sampling_flags);
    if (!fixed_subsurface && reflection_probability == 0.0 && transmission_probability == 0.0) {
        return result;
    }
    result.wo = cosine_sample_hemisphere(random_value.xy);
    result.pdf = math::frac_1_pi * result.wo.z;
    result.response.value = float3(result.pdf);
    if (wi.z < 0.0) {
        result.wo.z = -result.wo.z;
    }
    if (random_value.z <= transmission_probability) {
        result.wo.z = -result.wo.z;
        result.pdf *= transmission_probability;
        result.response.value *= transmission_probability;
        result.response.flags = LOBE_DIFFUSE_TRANSMISSION;
        result.response.value *= state.subsurface_color;
    } else {
        result.pdf *= reflection_probability;
        result.response.value *= reflection_probability * state.subsurface_color;
        result.response.flags = LOBE_DIFFUSE_REFLECTION;
    }
    return result;
}

public func mixed_diffuse_eval<let fixed_subsurface : bool = false>(wi: float3, wo: float3,
                                                                    state: OpaqueState,
                                                                    sampling_flags: uint)
    ->Response {
    return mix_eval(state.subsurface_weight < 1.0 ? lambert_eval(wi, wo, state, sampling_flags)
                                                  : empty_response(),
                    state.subsurface_weight > 0.0
                        ? subsurface_eval<fixed_subsurface>(wi, wo, state, sampling_flags)
                        : empty_response(),
                    state.subsurface_weight);
}

public func mixed_diffuse_pdf<let fixed_subsurface : bool = false>(wi: float3, wo: float3,
                                                                   state: OpaqueState,
                                                                   sampling_flags: uint)
    ->float {
    if (state.diffuse_mix.first_sample_weight + state.diffuse_mix.second_sample_weight <= 0.0) {
        return 0.0;
    }
    let first
        : float = state.subsurface_weight < 1.0 && (sampling_flags & LOBE_DIFFUSE_REFLECTION) != 0u
                      ? lambert_pdf(wi, wo)
                      : 0.0;
    let second : float = state.subsurface_weight > 0.0
                             ? subsurface_pdf<fixed_subsurface>(wi, wo, state, sampling_flags)
                             : 0.0;
    return lerp(first, second, state.diffuse_mix.second_sample_weight);
}

public func mixed_diffuse_energy(state: OpaqueState, sampling_flags: uint)->float3 {
    let first
        : float3 = state.subsurface_weight < 1.0 && (sampling_flags & LOBE_DIFFUSE_REFLECTION) != 0u
                       ? state.base_color
                       : float3(0.0);
    let second : float3 = state.subsurface_weight > 0.0 ? subsurface_energy(state, sampling_flags)
                                                        : float3(0.0);
    var result = float3(0.0);
    if (state.subsurface_weight < 1.0) {
        result = first;
    }
    if (state.subsurface_weight > 0.0) {
        result = result * (1.0 - state.subsurface_weight) + second * state.subsurface_weight;
    }
    return result;
}

/// FixedSubsurface 固定当前子集的各向异性；InteriorWeight 要求 0 < subsurface_weight < 1。
public func mixed_diffuse_sample<let fixed_subsurface : bool = false,
                                 let interior_weight : bool = false>(wi: float3,
                                                                     random_value: float3,
                                                                     state: OpaqueState,
                                                                     sampling_flags: uint)
    ->Proposal {
    if (state.diffuse_mix.first_sample_weight + state.diffuse_mix.second_sample_weight <= 0.0) {
        return empty_proposal();
    }
    if (random_value.z <= state.diffuse_mix.second_sample_weight &&
        state.diffuse_mix.second_sample_weight > 0.0) {
        var result
            : Proposal = subsurface_sample<fixed_subsurface>(
                  wi,
                  float3(random_value.xy, random_value.z / state.diffuse_mix.second_sample_weight),
                  state, sampling_flags);
        if (interior_weight)
            result.response.value *= state.subsurface_weight;
        else
            result.response = scale_response(result.response, state.subsurface_weight);
        result.pdf *= state.diffuse_mix.second_sample_weight;
        return result;
    }
    var result
        : Proposal = zero_roughness_eon_sample(
              wi,
              float3(random_value.xy, (random_value.z - state.diffuse_mix.second_sample_weight) /
                                          state.diffuse_mix.first_sample_weight),
              state, sampling_flags);
    if (interior_weight)
        result.response.value *= 1.0 - state.subsurface_weight;
    else
        result.response = scale_response(result.response, 1.0 - state.subsurface_weight);
    result.pdf *= state.diffuse_mix.first_sample_weight;
    return result;
}

public func dielectric_eval(wi: float3, wo: float3, state: OpaqueState, sampling_flags: uint)
    ->Response {
    return dielectric_eval_impl<false>(wi, wo, state, sampling_flags);
}

public func dielectric_eval_impl<let continuous : bool>(wi: float3, wo: float3, state: OpaqueState,
                                                        sampling_flags: uint)
    ->Response {
    let base : ReflectionEvaluation = evaluate_reflection_impl<continuous>(
                   wi, wo, state.microfacet, sampling_flags, LOBE_REFLECTION);
    return dielectric_eval_from_base<continuous>(base, state);
}

/// Fresnel 与能量补偿；base 可供同方向的反射分量及 PDF 共用。
public func dielectric_eval_from_base<let continuous : bool>(base: ReflectionEvaluation,
                                                             state: OpaqueState)
    ->Response {
    if (base.flags == LOBE_NONE) {
        return empty_response();
    }
    var result : Response;
    result.value = specular_reflectance(state.dielectric_fresnel, base.micro_cosine) * base.factor;
    result.flags = base.flags;
    if (continuous || is_non_delta(result.flags)) {
        result.value *= state.dielectric_multiple_scattering.x;
    }
    return result;
}

public func dielectric_pdf(wi: float3, wo: float3, state: OpaqueState, sampling_flags: uint)
    ->float {
    return dielectric_pdf_impl<false>(wi, wo, state, sampling_flags);
}

public func dielectric_pdf_impl<let continuous : bool>(wi: float3, wo: float3, state: OpaqueState,
                                                       sampling_flags: uint)
    ->float {
    return reflection_pdf_impl<continuous>(wi, wo, state.microfacet, sampling_flags,
                                           LOBE_REFLECTION);
}

public func opaque_dielectric_energy(wi: float3, state: OpaqueState, sampling_flags: uint)->float3 {
    return dielectric_energy_impl<false>(wi, state, sampling_flags);
}

public func dielectric_energy_impl<let continuous : bool>(wi: float3, state: OpaqueState,
                                                          sampling_flags: uint)
    ->float3 {
    if (wi.z < 0.0) {
        return float3(0.0);
    }
    if ((continuous || !microfacet_effectively_smooth(state.microfacet)) &&
        state.dielectric_fresnel.ior != 1.0) {
        if (!is_specular(sampling_flags)) {
            return float3(0.0);
        }
    } else if (!is_delta(sampling_flags)) {
        return float3(0.0);
    }
    if (!is_reflective(sampling_flags)) {
        return float3(0.0);
    }
    return state.dielectric_multiple_scattering.z * state.dielectric_fresnel.color;
}

public func dielectric_sample(wi: float3, random_value: float3, state: OpaqueState,
                              sampling_flags: uint)
    ->Proposal {
    return dielectric_sample_impl<false>(wi, random_value, state, sampling_flags);
}

public func dielectric_sample_impl<let continuous : bool>(wi: float3, random_value: float3,
                                                          state: OpaqueState, sampling_flags: uint)
    ->Proposal {
    var result : Proposal = empty_proposal();
    let base : ReflectionProposal = sample_reflection_impl<continuous>(
                   wi, random_value, state.microfacet, sampling_flags, LOBE_REFLECTION);
    result = base.proposal;
    result.response.value *= specular_reflectance(state.dielectric_fresnel, base.micro_cosine);
    if (continuous || is_non_delta(result.response.flags)) {
        result.response.value *= state.dielectric_multiple_scattering.x;
    }
    return result;
}

public func conductor_eval(wi: float3, wo: float3, state: OpaqueState, sampling_flags: uint)
    ->Response {
    return conductor_eval_impl<false>(wi, wo, state, sampling_flags);
}

public func conductor_eval_impl<let continuous : bool>(wi: float3, wo: float3, state: OpaqueState,
                                                       sampling_flags: uint)
    ->Response {
    let base : ReflectionEvaluation = evaluate_reflection_impl<continuous>(
                   wi, wo, state.microfacet, sampling_flags, LOBE_REFLECTION);
    return conductor_eval_from_base<continuous>(base, state);
}

/// 金属端复用反射几何；不改变原 Fresnel 与多散射补偿算序。
public func conductor_eval_from_base<let continuous : bool>(base: ReflectionEvaluation,
                                                            state: OpaqueState)
    ->Response {
    if (base.flags == LOBE_NONE) {
        return empty_response();
    }
    var result : Response;
    result.value = conductor_reflectance(state.conductor_fresnel, base.micro_cosine) * base.factor;
    result.flags = base.flags;
    if (continuous || is_non_delta(result.flags)) {
        result.value *= float3(1.0) + state.conductor_fresnel.f0 * (1.0 - state.conductor_ess) /
                                          state.conductor_ess;
    }
    return result;
}

/// 同方向联合金属响应与 marginal PDF；不更改采样测度。
public func conductor_evaluate_impl<let continuous : bool>(wi: float3, wo: float3,
                                                           state: OpaqueState, sampling_flags: uint)
    ->LocalEvaluation {
    let base : ReflectionEvaluation = evaluate_reflection_impl<continuous>(
                   wi, wo, state.microfacet, sampling_flags, LOBE_REFLECTION);
    return { conductor_eval_from_base<continuous>(base, state), base.pdf };
}

public func conductor_pdf(wi: float3, wo: float3, state: OpaqueState, sampling_flags: uint)->float {
    return conductor_pdf_impl<false>(wi, wo, state, sampling_flags);
}

public func conductor_pdf_impl<let continuous : bool>(wi: float3, wo: float3, state: OpaqueState,
                                                      sampling_flags: uint)
    ->float {
    return reflection_pdf_impl<continuous>(wi, wo, state.microfacet, sampling_flags,
                                           LOBE_REFLECTION);
}

public func conductor_energy(wi: float3, state: OpaqueState, sampling_flags: uint)->float3 {
    if (wi.z < 0.0) {
        return float3(0.0);
    }
    if (microfacet_effectively_smooth(state.microfacet)) {
        if ((sampling_flags & LOBE_DELTA_REFLECTION) == 0u) {
            return float3(0.0);
        }
    } else if ((sampling_flags & LOBE_SPECULAR_REFLECTION) == 0u) {
        return float3(0.0);
    }
    let f0 : float3 = state.conductor_fresnel.energy_f0;
    return microfacet_directional_albedo_reflection(
               state.microfacet, wi.z, f0,
               clamp(float3(state.conductor_fresnel.weight), float3(0.0), float3(1.0))) *
           (float3(1.0) + f0 * (1.0 - state.conductor_ess) / state.conductor_ess);
}

public func conductor_sample(wi: float3, random_value: float3, state: OpaqueState,
                             sampling_flags: uint)
    ->Proposal {
    return conductor_sample_impl<false>(wi, random_value, state, sampling_flags);
}

public func conductor_sample_impl<let continuous : bool>(wi: float3, random_value: float3,
                                                         state: OpaqueState, sampling_flags: uint)
    ->Proposal {
    if (!continuous && microfacet_effectively_smooth(state.microfacet)) {
        return conductor_discrete_sample(wi, opaque_discrete_state_from_full(state),
                                         sampling_flags);
    }
    var result : Proposal = empty_proposal();
    let base : ReflectionProposal = sample_reflection_impl<continuous>(
                   wi, random_value, state.microfacet, sampling_flags, LOBE_REFLECTION);
    result = base.proposal;
    result.response.value *= conductor_reflectance(state.conductor_fresnel, base.micro_cosine);
    if (continuous || is_non_delta(result.response.flags)) {
        result.response.value *= float3(1.0) + state.conductor_fresnel.f0 *
                                                   (1.0 - state.conductor_ess) /
                                                   state.conductor_ess;
    }
    return result;
}

public func opaque_layer_eval(wi: float3, wo: float3, state: OpaqueState, sampling_flags: uint)
    ->Response {
    return opaque_layer_eval_impl<false>(wi, wo, state, sampling_flags);
}

public func opaque_layer_eval_impl<let continuous : bool>(wi: float3, wo: float3,
                                                          state: OpaqueState, sampling_flags: uint)
    ->Response {
    let base : Response = state.kind == FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                              ? mixed_diffuse_eval(wi, wo, state, sampling_flags)
                          : state.kind == FULL_COMPACT_OPAQUE_SUBSURFACE
                              ? subsurface_eval(wi, wo, state, sampling_flags)
                              : lambert_eval(wi, wo, state, sampling_flags);
    let specular : Response = dielectric_eval_impl<continuous>(wi, wo, state, sampling_flags);
    return layer_eval(base, specular, state.layer);
}

/// 联合层响应与完整混合 PDF，共享反射几何。
public func opaque_layer_evaluate_impl<let continuous : bool>(wi: float3, wo: float3,
                                                              state: OpaqueState)
    ->LocalEvaluation {
    let reflection : ReflectionEvaluation = evaluate_reflection_impl<continuous>(
                         wi, wo, state.microfacet, LOBE_ALL, LOBE_REFLECTION);
    return opaque_layer_evaluate_from_base<continuous>(wi, wo, state, reflection);
}

/// 已准备反射几何时，仅补充当前层的 base 与原混合权重。
public func opaque_layer_evaluate_from_base<let continuous : bool>(wi: float3, wo: float3,
                                                                   state: OpaqueState,
                                                                   reflection: ReflectionEvaluation)
    ->LocalEvaluation {
    let base : Response = state.kind == FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                              ? mixed_diffuse_eval(wi, wo, state, LOBE_ALL)
                          : state.kind == FULL_COMPACT_OPAQUE_SUBSURFACE
                              ? subsurface_eval(wi, wo, state, LOBE_ALL)
                              : lambert_eval(wi, wo, state, LOBE_ALL);
    let base_pdf : float = state.kind == FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                               ? mixed_diffuse_pdf(wi, wo, state, LOBE_ALL)
                           : state.kind == FULL_COMPACT_OPAQUE_SUBSURFACE
                               ? subsurface_pdf(wi, wo, state, LOBE_ALL)
                               : lambert_pdf(wi, wo);
    let specular : Response = dielectric_eval_from_base<continuous>(reflection, state);
    return { layer_eval(base, specular, state.layer),
             state.layer.base_sample_weight + state.layer.specular_sample_weight <= 0.0
                 ? 0.0
                 : lerp(base_pdf, reflection.pdf, state.layer.specular_sample_weight) };
}

public func opaque_layer_pdf(wi: float3, wo: float3, state: OpaqueState)->float {
    return opaque_layer_pdf_impl<false>(wi, wo, state);
}

public func opaque_layer_pdf_impl<let continuous : bool>(wi: float3, wo: float3, state: OpaqueState)
    ->float {
    let base_pdf : float = state.kind == FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                               ? mixed_diffuse_pdf(wi, wo, state, LOBE_ALL)
                           : state.kind == FULL_COMPACT_OPAQUE_SUBSURFACE
                               ? subsurface_pdf(wi, wo, state, LOBE_ALL)
                               : lambert_pdf(wi, wo);
    if (state.layer.base_sample_weight + state.layer.specular_sample_weight <= 0.0) {
        return 0.0;
    }
    return lerp(base_pdf, dielectric_pdf_impl<continuous>(wi, wo, state, LOBE_ALL),
                state.layer.specular_sample_weight);
}

public func opaque_layer_sample(wi: float3, random_value: float3, state: OpaqueState,
                                use_eon_diffuse_mapping: bool)
    ->Proposal {
    return opaque_layer_sample_impl<false>(wi, random_value, state, use_eon_diffuse_mapping);
}

public func opaque_layer_sample_impl<let continuous : bool>(wi: float3, random_value: float3,
                                                            state: OpaqueState,
                                                            use_eon_diffuse_mapping: bool)
    ->Proposal {
    if (state.layer.base_sample_weight + state.layer.specular_sample_weight <= 0.0) {
        return empty_proposal();
    }
    if (random_value.z <= state.layer.specular_sample_weight &&
        state.layer.specular_sample_weight > 0.0) {
        var result
            : Proposal = dielectric_sample_impl<continuous>(
                  wi, float3(random_value.xy, random_value.z / state.layer.specular_sample_weight),
                  state, LOBE_ALL);
        result.pdf *= state.layer.specular_sample_weight;
        return result;
    }
    var result
        : Proposal =
              state.kind == FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                  ? mixed_diffuse_sample(
                        wi,
                        float3(random_value.xy,
                               (random_value.z - state.layer.specular_sample_weight) /
                                   state.layer.base_sample_weight),
                        state, LOBE_ALL)
              : state.kind == FULL_COMPACT_OPAQUE_SUBSURFACE
                  ? subsurface_sample(wi,
                                      float3(random_value.xy,
                                             (random_value.z - state.layer.specular_sample_weight) /
                                                 state.layer.base_sample_weight),
                                      state, LOBE_ALL)
              : use_eon_diffuse_mapping
                  ? zero_roughness_eon_sample(
                        wi,
                        float3(random_value.xy,
                               (random_value.z - state.layer.specular_sample_weight) /
                                   state.layer.base_sample_weight),
                        state, LOBE_ALL)
                  : lambert_sample(wi,
                                   float3(random_value.xy,
                                          (random_value.z - state.layer.specular_sample_weight) /
                                              state.layer.base_sample_weight),
                                   state, LOBE_ALL);
    result.response = scale_response(result.response, state.layer.transmittance_in);
    result.pdf *= state.layer.base_sample_weight;
    return result;
}
}
}
