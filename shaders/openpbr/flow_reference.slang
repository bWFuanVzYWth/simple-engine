// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 当前 OpenPBR 子集的控制流能力；Flow 为资源世代发布时证明的编译期上界。
/// 位描述可达分量和微面测度，不描述材质名称；diffuse／SSS 自身始终连续。
module flow_reference;
__exported import openpbr.selection;

__exported import openpbr.api;
__exported import openpbr.specialized;
import openpbr.contract.material;
import openpbr.contract.sample;
import openpbr.contract.flags;
import openpbr.math.frame;
import openpbr.math.microfacet_energy;
import openpbr.math.microfacet_measure;
import openpbr.math.microfacet_distribution;
import openpbr.math.fresnel;
import openpbr.math.spectrum;
import openpbr.opaque.discrete;

import openpbr.compact_lobes;
import openpbr.subsurface;
import openpbr.opaque.lobes;
import openpbr.opaque.state;
import openpbr.dielectric.thin_wall;
import openpbr.dielectric.refract;
import openpbr.dielectric.contract;
import openpbr.service.sanitization;
import openpbr.service.conversion;

namespace openpbr {
public func flow_has<let flow : uint, let bits : uint>()->bool {
    return (flow & bits) != 0u;
}

/// 整个闭包包含连续测度；光滑塑料的 diffuse 与 delta specular 可以同时存在。
public func flow_continuous<let flow : uint>()->bool {
    return flow_has<flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION |
                              FLOW_CONTINUOUS_TRANSMISSION>();
}

/// 与is_discrete_only相同的全闭包分类；Flow由资源世代证明，纯测度域不保留数值分类分支。
/// 混合金属的非金属null proposal仍保留原metalness判定，不能把它当成纯金属。
public func is_discrete_only_flow<let flow : uint>(material: Material, incident: float4,
                                                   transmitted: float4)
    ->bool {
    static const let delta : bool = (flow & (FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION)) != 0;
    static const let Transmission
        : bool = (flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) != 0;
    static const let Conductor : bool = (flow & FLOW_CONDUCTOR_REFLECTION) != 0;
    if (!delta || !(Transmission || Conductor))
        return false;
    if (!flow_continuous<flow>() && (Transmission || (flow & FLOW_METAL_MIX) == 0))
        return true;
    return is_discrete_only(material, incident, transmitted);
}

internal func flow_transmission<let flow : uint>()->bool {
    return flow_has<flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>();
}

internal struct LegacyFlow<let Flow : uint> {
    static const let capability : uint =
        FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has
            ? (FlowProperties<Flow>.continuous ? CAP_SOLID_DIELECTRIC : CAP_DELTA_SOLID_DIELECTRIC)
        : FlowMask<Flow, FLOW_THIN_TRANSMISSION>.has
            ? (FlowProperties<Flow>.continuous ? CAP_THIN_DIELECTRIC : CAP_DELTA_THIN_DIELECTRIC)
        : !FlowProperties<Flow>.layer && !FlowMask<Flow, FLOW_METAL_MIX>.has
            ? (FlowProperties<Flow>.continuous ? CAP_CONDUCTOR : CAP_DELTA_CONDUCTOR)
        : FlowMask<Flow, FLOW_SUBSURFACE>.has ? CAP_OPAQUE_SUBSURFACE
                                              : CAP_OPAQUE_DIELECTRIC;
};

internal func require_flow<let flow : uint>()->void {
    static_assert((flow & ~4095u) == 0u, "unknown OpenPBR flow bits");
    static_assert(FlowMask<flow, FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION>.has ==
                      FlowMask<flow, FLOW_DELTA_REFLECTION | FLOW_CONTINUOUS_REFLECTION>.has,
                  "reflection components require their measurement domains");
    static_assert(FlowMask<flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ==
                      FlowMask<flow, FLOW_DELTA_TRANSMISSION | FLOW_CONTINUOUS_TRANSMISSION>.has,
                  "transmission topology requires its measurement domains");
    static_assert((flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) !=
                      (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION),
                  "solid and thin are distinct boundary contracts");
    static_assert(
        !FlowMask<flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ||
            !FlowMask<flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONDUCTOR_REFLECTION>.has,
        "opaque and transmission are disjoint in the supported subset");
    static_assert(!FlowMask<flow, FLOW_METAL_MIX>.has ||
                      (FlowMask<flow, FLOW_CONDUCTOR_REFLECTION>.has &&
                       !FlowMask<flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has),
                  "metal mixture requires an opaque conductor");
    static_assert(!FlowMask<flow, FLOW_CONDUCTOR_REFLECTION>.has || !FlowProperties<flow>.layer ||
                      FlowMask<flow, FLOW_METAL_MIX>.has,
                  "mixed metalness must retain its proposal control");
    static_assert(!FlowMask<flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ||
                      FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has ||
                      (FlowMask<flow, FLOW_DELTA_TRANSMISSION>.has &&
                       !FlowMask<flow, FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION |
                                           FLOW_DELTA_REFLECTION>.has),
                  "matched transmission is delta only");
    static_assert(!FlowMask<flow, FLOW_SOLID_TRANSMISSION>.has ||
                      ((!FlowMask<flow, FLOW_DELTA_REFLECTION>.has ||
                        FlowMask<flow, FLOW_DELTA_TRANSMISSION>.has) &&
                       (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has ==
                        FlowMask<flow, FLOW_CONTINUOUS_TRANSMISSION>.has)),
                  "solid reflection/transmission domains disagree");
    static_assert(!FlowMask<flow, FLOW_THIN_TRANSMISSION>.has ||
                      !FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has ||
                      ((FlowMask<flow, FLOW_DELTA_REFLECTION>.has ==
                        FlowMask<flow, FLOW_DELTA_TRANSMISSION>.has) &&
                       (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has ==
                        FlowMask<flow, FLOW_CONTINUOUS_TRANSMISSION>.has)),
                  "thin reflection/transmission domains disagree");
}

static const let binary_metal
    : bool = (closure::selected_variant & closure::variant_metal_switch) != 0;
static const let binary_sss
    : bool = (closure::selected_variant & closure::variant_subsurface_switch) != 0;

internal func flow_opaque<let flow : uint, EnergyLut>(energy: EnergyLut, material: Material,
                                                      wi: float3, incident: float4)
    ->kernel::OpaqueState
    where EnergyLut : IEnergyLut {
    var color : float3 = material.base_roughness.xyz;
    var state : kernel::OpaqueState = {};
    state.microfacet.alpha = FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has
                                 ? float2(material.base_roughness.w * material.base_roughness.w)
                                 : float2(0);
    if (FlowMask<flow, FLOW_CONDUCTOR_REFLECTION>.has) {
        state.metalness = FlowMask<flow, FLOW_METAL_MIX>.has ? material.surface.y : 1.0;
        if (!binary_metal || state.metalness > 0.0) {
            state.conductor_fresnel.f0 = saturate(color);
            state.conductor_fresnel.f82 = 1;
            state.conductor_fresnel.f90 = 1;
            state.conductor_fresnel.weight = 1;
            state.conductor_fresnel.energy_f0 = saturate(color);
            state.conductor_ess = !FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has ||
                                          (FlowMask<flow, FLOW_DELTA_REFLECTION>.has &&
                                           kernel::microfacet_effectively_smooth(state.microfacet))
                                      ? 1.0
                                      : kernel::microfacet_directional_albedo_reflection(
                                            state.microfacet, wi.z, float3(1), float3(1))
                                            .x;
        }
        if (!FlowProperties<flow>.layer || (binary_metal && state.metalness == 1.0)) {
            state.kind = kernel::FULL_COMPACT_OPAQUE_CONDUCTOR;
            return state;
        }
    }
    var base_energy : float3 = 0;
    if (FlowProperties<flow>.base) {
        state.base_color = color;
        base_energy = wi.z >= 0 ? color : float3(0);
        if (FlowMask<flow, FLOW_SUBSURFACE>.has) {
            let w : float = FlowMask<flow, FLOW_DIFFUSE>.has ? material.surface.z : 1.0;
            state.subsurface_weight = w;
            state.subsurface_anisotropy = kernel::FULL_PT_SUBSURFACE_ANISOTROPY;
            if (w > 0.0) {
                state.subsurface_color = kernel::subsurface_color(color);
                let sss : float3 = kernel::subsurface_energy(state, kernel::LOBE_ALL);
                if (binary_sss || !FlowMask<flow, FLOW_DIFFUSE>.has) {
                    state.kind = kernel::FULL_COMPACT_OPAQUE_SUBSURFACE;
                    base_energy = sss;
                } else {
                    state.kind = w == 1.0 ? kernel::FULL_COMPACT_OPAQUE_SUBSURFACE
                                          : kernel::FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE;
                    base_energy = color * (1.0 - w) + sss * w;
                    state.diffuse_mix = kernel::make_mix_state(w, color, sss);
                }
            } else
                state.diffuse_mix.first_sample_weight = 1;
        }
    }
    if (FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has) {
        state.dielectric_fresnel.ior = material.surface.x / incident.x;
        state.dielectric_fresnel.energy_ior =
            kernel::ior_adjustment(state.dielectric_fresnel.ior, 1.0);
        state.dielectric_fresnel.color = 1;
        state.dielectric_multiple_scattering =
            kernel::dielectric_energy<!FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has>(
                energy, state.microfacet, wi.z, state.dielectric_fresnel.energy_ior);
        if (FlowProperties<flow>.base)
            state.layer = kernel::make_layer_state(
                base_energy, float3(1.0 - state.dielectric_multiple_scattering.z),
                kernel::dielectric_energy_impl<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
                    wi, state, kernel::LOBE_ALL));
        else
            state.layer.specular_sample_weight =
                state.dielectric_multiple_scattering.z > 0.0 ? 1.0 : 0.0;
    } else {
        state.dielectric_fresnel.ior = 1;
        state.dielectric_fresnel.energy_ior = 1;
        state.dielectric_fresnel.color = 1;
        state.dielectric_multiple_scattering = float3(1, 1, 0);
        state.layer.transmittance_in = 1;
        state.layer.base_sample_weight = any(color > 0) ? 1 : 0;
    }
    return state;
}

internal func flow_layer_evaluate<let flow : uint>(wi: float3, wo: float3,
                                                   state: kernel::OpaqueState,
                                                   reflection: kernel::ReflectionEvaluation)
    ->kernel::LocalEvaluation {
    if (!FlowProperties<flow>.base) {
        var result : kernel::LocalEvaluation = {};
        if (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has) {
            result.response =
                kernel::dielectric_eval_from_base<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
                    reflection, state);
            if (state.layer.specular_sample_weight > 0.0)
                result.pdf = reflection.pdf;
        }
        return result;
    }
    if (FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has &&
        flow_has<flow, FLOW_CONTINUOUS_REFLECTION>() && !flow_has<flow, FLOW_SUBSURFACE>())
        return kernel::opaque_layer_evaluate_from_base<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
            wi, wo, state, reflection);
    var base : kernel::Response;
    var base_pdf : float;
    if (flow_has<flow, FLOW_SUBSURFACE>() && flow_has<flow, FLOW_DIFFUSE>()) {
        if (binary_sss) {
            if (state.subsurface_weight == 1.0) {
                base = kernel::subsurface_eval<true>(wi, wo, state, kernel::LOBE_ALL);
                base_pdf = kernel::subsurface_pdf<true>(wi, wo, state, kernel::LOBE_ALL);
            } else {
                base = kernel::lambert_eval(wi, wo, state, kernel::LOBE_ALL);
                base_pdf = kernel::lambert_pdf(wi, wo);
            }
        } else {
            base = kernel::mixed_diffuse_eval<true>(wi, wo, state, kernel::LOBE_ALL);
            base_pdf = kernel::mixed_diffuse_pdf<true>(wi, wo, state, kernel::LOBE_ALL);
        }
    } else if (flow_has<flow, FLOW_SUBSURFACE>()) {
        base = kernel::subsurface_eval<true>(wi, wo, state, kernel::LOBE_ALL);
        base_pdf = kernel::subsurface_pdf<true>(wi, wo, state, kernel::LOBE_ALL);
    } else {
        base = kernel::lambert_eval(wi, wo, state, kernel::LOBE_ALL);
        base_pdf = kernel::lambert_pdf(wi, wo);
    }
    var result : kernel::LocalEvaluation;
    if (!FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has) {
        result.response = base;
        result.pdf = state.layer.base_sample_weight > 0.0 ? base_pdf : 0.0;
        return result;
    }
    var specular : kernel::Response = kernel::empty_response();
    var specular_pdf = 0.0;
    if (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has &&
        FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has) {
        specular = kernel::dielectric_eval_from_base<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
            reflection, state);
        specular_pdf = reflection.pdf;
    }
    result.response = kernel::layer_eval(base, specular, state.layer);
    result.pdf = state.layer.base_sample_weight + state.layer.specular_sample_weight > 0.0
                     ? lerp(base_pdf, specular_pdf, state.layer.specular_sample_weight)
                     : 0.0;
    return result;
}

internal func flow_opaque_evaluate<let flow : uint>(wi: float3, wo: float3,
                                                    state: kernel::OpaqueState)
    ->kernel::LocalEvaluation {
    if (!FlowProperties<flow>.layer) {
        var result : kernel::LocalEvaluation = specialized_opaque_evaluate<
                         CAP_CONDUCTOR, !FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(wi, wo, state);
        if (FlowMask<flow, FLOW_METAL_MIX>.has) {
            result.response.value *= state.metalness;
            result.pdf *= state.metalness;
        }
        return result;
    }
    var reflection : kernel::ReflectionEvaluation = {};
    if (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has)
        reflection = kernel::evaluate_reflection_impl<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
            wi, wo, state.microfacet, kernel::LOBE_ALL, kernel::LOBE_REFLECTION);
    var second : kernel::LocalEvaluation = {};
    if (FlowMask<flow, FLOW_CONDUCTOR_REFLECTION>.has && (!binary_metal || state.metalness > 0)) {
        if (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has) {
            second.response =
                kernel::conductor_eval_from_base<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
                    reflection, state);
            second.pdf = reflection.pdf;
        }
        if (binary_metal && state.metalness == 1)
            return second;
    }
    var first : kernel::LocalEvaluation = flow_layer_evaluate<flow>(wi, wo, state, reflection);
    if (!FlowMask<flow, FLOW_CONDUCTOR_REFLECTION>.has || binary_metal)
        return first;

    first.response = kernel::mix_eval(first.response, second.response, state.metalness);
    first.pdf = lerp(first.pdf, second.pdf, state.metalness);
    return first;
}

internal func flow_layer_sample<let flow : uint>(wi: float3, u: float3, state: kernel::OpaqueState)
    ->kernel::Proposal {
    if (!FlowProperties<flow>.base) {
        if (!(state.layer.specular_sample_weight > 0.0))
            return kernel::empty_proposal();
        if (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has)
            return kernel::dielectric_sample_impl<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
                wi, u, state, kernel::LOBE_ALL);
        let smooth : kernel::Microfacet = {};
        let base : kernel::ReflectionProposal = kernel::sample_reflection(
                       wi, u, smooth, kernel::LOBE_ALL, kernel::LOBE_REFLECTION);
        var result : kernel::Proposal = base.proposal;
        result.response.value *=
            kernel::specular_reflectance(state.dielectric_fresnel, base.micro_cosine);
        return result;
    }
    if (FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has &&
        flow_has<flow, FLOW_CONTINUOUS_REFLECTION>() && !flow_has<flow, FLOW_SUBSURFACE>())
        return kernel::opaque_layer_sample_impl<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
            wi, u, state, false);
    if (state.layer.base_sample_weight + state.layer.specular_sample_weight <= 0.0)
        return kernel::empty_proposal();
    if (FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has &&
        u.z <= state.layer.specular_sample_weight && state.layer.specular_sample_weight > 0.0) {
        if (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has) {
            var result
                : kernel::Proposal =
                      kernel::dielectric_sample_impl<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
                          wi, float3(u.xy, u.z / state.layer.specular_sample_weight), state,
                          kernel::LOBE_ALL);
            result.pdf *= state.layer.specular_sample_weight;
            return result;
        }
        let smooth : kernel::Microfacet = {};
        let base : kernel::ReflectionProposal = kernel::sample_reflection(
                       wi, u, smooth, kernel::LOBE_ALL, kernel::LOBE_REFLECTION);
        var result : kernel::Proposal = base.proposal;
        result.response.value *=
            kernel::specular_reflectance(state.dielectric_fresnel, base.micro_cosine);
        result.pdf *= state.layer.specular_sample_weight;
        return result;
    }
    let random : float3 = FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has
                              ? float3(u.xy, (u.z - state.layer.specular_sample_weight) /
                                                 state.layer.base_sample_weight)
                              : u;
    var result : kernel::Proposal;
    if (flow_has<flow, FLOW_SUBSURFACE>() && flow_has<flow, FLOW_DIFFUSE>()) {
        if (state.subsurface_weight == 0.0)
            result = kernel::lambert_sample(wi, random, state, kernel::LOBE_ALL);
        else if (binary_sss || state.subsurface_weight == 1.0)
            result = kernel::subsurface_sample<true>(wi, random, state, kernel::LOBE_ALL);
        else
            result = kernel::mixed_diffuse_sample<true, true>(wi, random, state, kernel::LOBE_ALL);
    } else if (flow_has<flow, FLOW_SUBSURFACE>())
        result = kernel::subsurface_sample<true>(wi, random, state, kernel::LOBE_ALL);
    else
        result = kernel::lambert_sample(wi, random, state, kernel::LOBE_ALL);
    if (FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has) {
        result.response = kernel::scale_response(result.response, state.layer.transmittance_in);
        result.pdf *= state.layer.base_sample_weight;
    }
    return result;
}

internal func flow_opaque_sample<let flow : uint>(wi: float3, u: float3, state: kernel::OpaqueState)
    ->kernel::Proposal {
    if (!FlowProperties<flow>.layer) {
        if (FlowMask<flow, FLOW_METAL_MIX>.has && u.z >= state.metalness)
            return kernel::empty_proposal();
        var result : kernel::Proposal;
        if (!FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has)
            result = kernel::conductor_discrete_sample(
                wi, kernel::opaque_discrete_state_from_full(state), kernel::LOBE_ALL);
        else
            result = specialized_opaque_sample<CAP_CONDUCTOR,
                                               !FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
                wi, FlowMask<flow, FLOW_METAL_MIX>.has ? float3(u.xy, u.z / state.metalness) : u,
                state);
        if (FlowMask<flow, FLOW_METAL_MIX>.has) {
            if (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has && state.metalness < 1.0 &&
                kernel::has_sample(result) &&
                (!FlowMask<flow, FLOW_DELTA_REFLECTION>.has ||
                 kernel::is_non_delta(result.response.flags))) {
                let evaluated
                    : kernel::LocalEvaluation =
                          specialized_opaque_evaluate<CAP_CONDUCTOR,
                                                      !FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
                              wi, result.wo, state);
                result.response = evaluated.response;
                result.pdf = evaluated.pdf;
                // 原 marginal 保留零非金属层的 diffuse 标签，不需要恢复其求值。
                result.response.flags |= kernel::LOBE_DIFFUSE_REFLECTION;
            }
            result.response.value *= state.metalness;
            result.pdf *= state.metalness;
        }
        return result;
    }
    var result : kernel::Proposal;
    if (binary_metal && FlowMask<flow, FLOW_CONDUCTOR_REFLECTION>.has && state.metalness == 1.0) {
        if (!FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has)
            return kernel::conductor_discrete_sample(
                wi, kernel::opaque_discrete_state_from_full(state), kernel::LOBE_ALL);
        return kernel::conductor_sample_impl<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
            wi, u, state, kernel::LOBE_ALL);
    }
    if (!binary_metal && flow_has<flow, FLOW_CONDUCTOR_REFLECTION>()) {
        let m : float = state.metalness;
        if (u.z < m) {
            if (!flow_has<flow, FLOW_CONTINUOUS_REFLECTION>())
                result = kernel::conductor_discrete_sample(
                    wi, kernel::opaque_discrete_state_from_full(state), kernel::LOBE_ALL);
            else
                result = kernel::conductor_sample_impl<!FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
                    wi, float3(u.xy, u.z / m), state, kernel::LOBE_ALL);
        } else
            result = flow_layer_sample<flow>(wi, float3(u.xy, (u.z - m) / (1.0 - m)), state);
        if (!kernel::has_sample(result))
            return result;
        if (m == 1.0)
            return result;
        if (flow_has<flow, FLOW_DELTA_REFLECTION>() && kernel::is_delta(result.response.flags)) {
            if (m == 0.0)
                return result;
            result.response.value =
                lerp(kernel::specular_reflectance(state.dielectric_fresnel, wi.z),
                     kernel::conductor_reflectance(state.conductor_fresnel, wi.z), m);
            result.response.flags = kernel::LOBE_DELTA_REFLECTION;
            result.pdf = m + (1.0 - m) * state.layer.specular_sample_weight;
            return result;
        }
    } else {
        result = flow_layer_sample<flow>(wi, u, state);
        if (!kernel::has_sample(result) ||
            (flow_has<flow, FLOW_DELTA_REFLECTION>() && kernel::is_delta(result.response.flags)))
            return result;
    }
    let evaluated : kernel::LocalEvaluation = flow_opaque_evaluate<flow>(wi, result.wo, state);
    if (!binary_metal && flow_has<flow, FLOW_CONDUCTOR_REFLECTION>() && state.metalness > 0.0 &&
        state.metalness < 1.0)
        result.response = evaluated.response;
    else
        result.response.value = evaluated.response.value;
    result.pdf = evaluated.pdf;
    return result;
}

/// 编译期能力的连续支持域；不读取纹理。动态 TIR、有效方向等仍由实际闭包处理。
public func may_evaluate_flow<let flow : uint>(material: Material, normal: float3, view: float3,
                                               direction: float3, incident: float4,
                                               transmitted: float4)
    ->bool {
    require_flow<flow>();
    if (!flow_continuous<flow>())
        return false;
    if (flow_transmission<flow>())
        return true;
    if (FlowMask<flow, FLOW_SUBSURFACE>.has) {
        let dielectric_base : bool =
                                  !FlowMask<flow, FLOW_METAL_MIX>.has || material.surface.y < 1.0;
        let subsurface : bool = !FlowMask<flow, FLOW_DIFFUSE>.has || material.surface.z > 0.0;
        if (dielectric_base && subsurface)
            return true;
    }
    if (!FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has && FlowMask<flow, FLOW_METAL_MIX>.has &&
        material.surface.y == 1.0)
        return false;
    return may_evaluate_specialized<CAP_OPAQUE_DIELECTRIC>(material, normal, view, direction,
                                                           incident, transmitted);
}

/// 与未裁剪 evaluate 相同的完整连续响应与 marginal PDF；Flow 必须覆盖全部可达分量。
public func evaluate_flow<let flow : uint, EnergyLut>(energy: EnergyLut, material: Material,
                                                      normal: float3, view: float3,
                                                      direction: float3, incident: float4,
                                                      transmitted: float4)
    ->Evaluation
    where EnergyLut : IEnergyLut {
    require_flow<flow>();
    if (!flow_continuous<flow>())
        return { float3(0.0), 0.0 };
    if (flow_transmission<flow>()) {
        if (FlowMask<flow, FLOW_SOLID_TRANSMISSION>.has &&
            !FlowMask<flow, FLOW_DELTA_REFLECTION>.has &&
            FlowMask<flow, FLOW_DELTA_TRANSMISSION>.has) {
            let ior : float = incident.x > transmitted.x ? incident.x / transmitted.x
                                                         : transmitted.x / incident.x;
            if (ior == 1.0)
                return { float3(0.0), 0.0 };
            return evaluate_solid_continuous(energy, material, normal, view, direction, incident,
                                             transmitted);
        }
        if (FlowMask<flow, FLOW_THIN_TRANSMISSION>.has &&
            !FlowMask<flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>.has) {
            let frame
                : kernel::Frame =
                      specialized_frame<CAP_THIN_DIELECTRIC>(normal, view, incident, transmitted);
            let wi : float3 = kernel::to_local(frame, view);
            let wo : float3 = kernel::to_local(frame, direction);
            let state : kernel::TransmissionState = kernel::transmission_state(
                            energy, material.base_roughness.w, transmitted.x / incident.x, true,
                            transmitted.yzw * material.surface.w, wi);
            let evaluated
                : kernel::TransmissionEvaluation =
                      kernel::transmission_evaluate_raw<true>(wi, wo, state, kernel::LOBE_ALL);
            return specialized_evaluation(evaluated.response, evaluated.pdf);
        }
        if (flow_has<flow, FLOW_SOLID_TRANSMISSION>() &&
            !flow_has<flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>())
            return evaluate_solid_continuous(energy, material, normal, view, direction, incident,
                                             transmitted);
        return evaluate_specialized<LegacyFlow<flow>.capability>(energy, material, normal, view,
                                                                 direction, incident, transmitted);
    }
    let frame : kernel::Frame =
                    specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let wo : float3 = kernel::to_local(frame, direction);
    if (!flow_has<flow, FLOW_SUBSURFACE>() && wo.z < 0.0)
        return { float3(0.0), 0.0 };
    let evaluated
        : kernel::LocalEvaluation =
              flow_opaque_evaluate<flow>(wi, wo, flow_opaque<flow>(energy, material, wi, incident));
    return specialized_evaluation < FlowMask<flow, FLOW_SUBSURFACE>.has &&
           !FlowMask<flow, FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION>.has >
               (evaluated.response, evaluated.pdf);
}

internal func flow_matched_solid(normal: float3, view: float3, incident: float4,
                                 transmitted: float4)
    ->Sample {
    let frame
        : kernel::Frame =
              specialized_frame<CAP_DELTA_SOLID_DIELECTRIC>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let refracted : kernel::RefractResult =
                        kernel::dielectric_refract(1.0, -wi, float3(0.0, 0.0, 1.0));
    var result : Sample = {};
    result.relative_eta = 1.0;
    result.medium = incident;
    if (refracted.valid != 0u && wi.z * refracted.wo.z < 0.0) {
        var direction : float3 = kernel::to_world(frame, refracted.wo);
        if (kernel::is_direction(direction)) {
            result.direction = direction;
            result.response = float3(1.0);
            result.events = TRANSMISSION | DISCRETE;
            result.pdf = 1.0;
            result.medium = transmitted;
        }
    }
    return result;
}

/// 与未裁剪 sample 相同的随机映射、完整 response/PDF、事件、eta 与介质转移。
/// 只消除能力排除的计算；delta 与连续分量共存时维持两种不同测度。
public func sample_flow<let flow : uint, EnergyLut>(energy: EnergyLut, material: Material,
                                                    normal: float3, view: float3, u: float3,
                                                    incident: float4, transmitted: float4)
    ->Sample
    where EnergyLut : IEnergyLut {
    require_flow<flow>();
    if (!FlowMask<flow, FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION | FLOW_DIFFUSE |
                            FLOW_SUBSURFACE | FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has)
    {
        var result : Sample = {};
        result.relative_eta = 1.0;
        result.medium = incident;
        return result;
    }
    if (FlowMask<flow, FLOW_SOLID_TRANSMISSION>.has &&
        !FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has)
        return flow_matched_solid(normal, view, incident, transmitted);
    if (FlowMask<flow, FLOW_THIN_TRANSMISSION>.has &&
        !FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has) {
        var result : Sample = {};
        result.relative_eta = 1.0;
        result.medium = incident;
        let cosine : float = abs(dot(normal, view));
        var response : float3 = cosine > 0.0 ? exp(-transmitted.yzw * material.surface.w / cosine)
                                             : float3(0.0);
        if (cosine > 0.0 && kernel::spectrum_to_weight(response) > 0.0) {
            result.direction = -view;
            result.response = response;
            result.pdf = 1.0;
            result.events = TRANSMISSION | DISCRETE;
        }
        return result;
    }
    if (!flow_continuous<flow>() &&
        ((!FlowProperties<flow>.layer && !FlowMask<flow, FLOW_METAL_MIX>.has) ||
         flow_transmission<flow>()))
        return sample_specialized<LegacyFlow<flow>.capability>(energy, material, normal, view, u,
                                                               incident, transmitted);
    if (!FlowProperties<flow>.base &&
        !FlowMask<flow, FLOW_CONDUCTOR_REFLECTION | FLOW_SOLID_TRANSMISSION |
                            FLOW_THIN_TRANSMISSION | FLOW_CONTINUOUS_REFLECTION>.has)
    {
        let frame : kernel::Frame = specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident,
                                                                             transmitted);
        let wi : float3 = kernel::to_local(frame, view);
        var result : kernel::Proposal = kernel::empty_proposal();
        var fresnel : kernel::SpecularFresnel;
        fresnel.ior = material.surface.x / incident.x;
        fresnel.energy_ior = kernel::ior_adjustment(fresnel.ior, 1.0);
        fresnel.color = float3(1.0);
        if (wi.z > 0.0 && fresnel.ior != 1.0) {
            result.wo = reflect(-wi, float3(0.0, 0.0, 1.0));
            result.response.value = kernel::specular_reflectance(fresnel, wi.z);
            result.response.flags = kernel::LOBE_DELTA_REFLECTION;
            result.pdf = 1.0;
        }
        return specialized_result<CAP_OPAQUE_DIELECTRIC, false, true, REFLECTION | DISCRETE>(
            result, frame, view, incident, transmitted);
    }
    if (flow_transmission<flow>()) {
        if (FlowMask<flow, FLOW_SOLID_TRANSMISSION>.has &&
            !FlowMask<flow, FLOW_DELTA_REFLECTION>.has &&
            FlowMask<flow, FLOW_DELTA_TRANSMISSION>.has) {
            let ior : float = incident.x > transmitted.x ? incident.x / transmitted.x
                                                         : transmitted.x / incident.x;
            if (ior != 1.0)
                return sample_solid_continuous(energy, material, normal, view, u, incident,
                                               transmitted);
            return flow_matched_solid(normal, view, incident, transmitted);
        }
        if (FlowMask<flow, FLOW_THIN_TRANSMISSION>.has &&
            !FlowMask<flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>.has) {
            let frame
                : kernel::Frame =
                      specialized_frame<CAP_THIN_DIELECTRIC>(normal, view, incident, transmitted);
            let wi : float3 = kernel::to_local(frame, view);
            let state : kernel::TransmissionState = kernel::transmission_state(
                            energy, material.base_roughness.w, transmitted.x / incident.x, true,
                            transmitted.yzw * material.surface.w, wi);
            return specialized_result<CAP_THIN_DIELECTRIC, true>(
                kernel::transmission_sample_raw<true>(wi, u, state, kernel::LOBE_ALL), frame, view,
                incident, transmitted);
        }
        if (flow_has<flow, FLOW_SOLID_TRANSMISSION>() &&
            !flow_has<flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>())
            return sample_solid_continuous(energy, material, normal, view, u, incident,
                                           transmitted);
        return sample_specialized<LegacyFlow<flow>.capability>(energy, material, normal, view, u,
                                                               incident, transmitted);
    }
    let frame : kernel::Frame =
                    specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let sampled
        : kernel::Proposal =
              flow_opaque_sample<flow>(wi, u, flow_opaque<flow>(energy, material, wi, incident));
    return specialized_result<LegacyFlow<flow>.capability,
                              !FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(sampled, frame, view,
                                                                          incident, transmitted);
}

internal func flow_transmission_evaluate_prepared<let flow : uint>(wi: float3, wo: float3,
                                                                   state: kernel::TransmissionState)
    ->Evaluation {
    var result : kernel::TransmissionEvaluation;
    if (FlowMask<flow, FLOW_SOLID_TRANSMISSION>.has && !FlowMask<flow, FLOW_DELTA_REFLECTION>.has)
        result =
            kernel::transmission_evaluate_raw_solid_continuous(wi, wo, state, kernel::LOBE_ALL);
    else
        result = kernel::transmission_evaluate_raw<
            !FlowMask<flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>.has>(wi, wo, state,
                                                                                  kernel::LOBE_ALL);
    return specialized_evaluation(result.response, result.pdf);
}

internal func flow_transmission_sample_prepared<let flow : uint>(wi: float3, u: float3,
                                                                 state: kernel::TransmissionState)
    ->kernel::Proposal {
    if (FlowMask<flow, FLOW_SOLID_TRANSMISSION>.has && !FlowMask<flow, FLOW_DELTA_REFLECTION>.has)
        return kernel::transmission_sample_raw_solid_continuous(wi, u, state, kernel::LOBE_ALL);
    return kernel::transmission_sample_raw<
        !FlowMask<flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>.has>(wi, u, state,
                                                                              kernel::LOBE_ALL);
}

/// 按方向即时消费连续评估；direction=0/1 对应 first/second，只回调掩码请求的方向。
/// 回调在共享闭包状态有效期内执行，不得发起 callable 或主查询。
public interface IFlowEvaluationConsumer {
    [mutating]
    func consume(direction: uint, evaluated: Evaluation)->void;
};

/// 正向组合：共享一次局部坐标和能量准备，仅求所请求的方向和散射。
public func shade_flow<let flow : uint, EnergyLut, let directions : uint = 3>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    transmitted: float4)
    ->Batch
    where EnergyLut : IEnergyLut {
    require_flow<flow>();
    static_assert((directions & ~3u) == 0u, "unknown evaluation direction mask");
    var batch : Batch = {};
    var result : Sample = {};
    result.relative_eta = 1.0;
    result.medium = incident;
    if (!flow_continuous<flow>()) {
        if (continuation)
            result = sample_flow<flow>(energy, material, normal, view, u, incident, transmitted);
        return { batch.first, batch.second, result };
    }
    if ((evaluation_mask & directions) == 0u && !continuation)
        return { batch.first, batch.second, result };
    if (flow_transmission<flow>()) {
        let ior : float = FlowMask<flow, FLOW_THIN_TRANSMISSION>.has ? transmitted.x / incident.x
                          : incident.x > transmitted.x               ? incident.x / transmitted.x
                                                                     : transmitted.x / incident.x;
        if (FlowMask<flow, FLOW_SOLID_TRANSMISSION>.has &&
            !FlowMask<flow, FLOW_DELTA_REFLECTION>.has &&
            FlowMask<flow, FLOW_DELTA_TRANSMISSION>.has && ior == 1.0) {
            if (continuation)
                result = flow_matched_solid(normal, view, incident, transmitted);
            return { batch.first, batch.second, result };
        }
        let frame : kernel::Frame = specialized_frame<LegacyFlow<flow>.capability>(
                        normal, view, incident, transmitted);
        let wi : float3 = kernel::to_local(frame, view);
        var state : kernel::TransmissionState =
                        kernel::transmission_state(energy, material.base_roughness.w, ior,
                                                   FlowMask<flow, FLOW_THIN_TRANSMISSION>.has,
                                                   transmitted.yzw * material.surface.w, wi);
        if (FlowMask<flow, FLOW_SOLID_TRANSMISSION>.has)
            state.entering = 1u;
        if ((directions & 1u) != 0u && (evaluation_mask & 1u) != 0u)
            batch.first = flow_transmission_evaluate_prepared<flow>(
                wi, kernel::to_local(frame, first), state);
        if ((directions & 2u) != 0u && (evaluation_mask & 2u) != 0u)
            batch.second = flow_transmission_evaluate_prepared<flow>(
                wi, kernel::to_local(frame, second), state);
        if (continuation)
            result = specialized_result<
                LegacyFlow<flow>.capability,
                !FlowMask<flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>.has>(
                flow_transmission_sample_prepared<flow>(wi, u, state), frame, view, incident,
                transmitted);
        return { batch.first, batch.second, result };
    }
    let frame : kernel::Frame =
                    specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let state : kernel::OpaqueState = flow_opaque<flow>(energy, material, wi, incident);
    if ((directions & 1u) != 0u && (evaluation_mask & 1u) != 0u) {
        let evaluated : kernel::LocalEvaluation =
                            flow_opaque_evaluate<flow>(wi, kernel::to_local(frame, first), state);
        batch.first =
            specialized_evaluation < FlowMask<flow, FLOW_SUBSURFACE>.has &&
            !FlowMask<flow, FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION>.has >
                (evaluated.response, evaluated.pdf);
    }
    if ((directions & 2u) != 0u && (evaluation_mask & 2u) != 0u) {
        let evaluated : kernel::LocalEvaluation =
                            flow_opaque_evaluate<flow>(wi, kernel::to_local(frame, second), state);
        batch.second =
            specialized_evaluation < FlowMask<flow, FLOW_SUBSURFACE>.has &&
            !FlowMask<flow, FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION>.has >
                (evaluated.response, evaluated.pdf);
    }
    if (continuation)
        result = specialized_result<LegacyFlow<flow>.capability,
                                    !FlowMask<flow, FLOW_DELTA_REFLECTION>.has>(
            flow_opaque_sample<flow>(wi, u, state), frame, view, incident, transmitted);
    return { batch.first, batch.second, result };
}

/// 延迟着色适配器按方向接收上述数值结果。
public func shade_flow_consume<let flow : uint, EnergyLut, Consumer, let directions : uint = 3>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    transmitted: float4, inout consumer: Consumer)
    ->Sample
    where EnergyLut : IEnergyLut
    where Consumer : IFlowEvaluationConsumer {
    let b : Batch = shade_flow<flow, EnergyLut, directions>(energy, material, normal, view, first,
                                                            second, evaluation_mask, u,
                                                            continuation, incident, transmitted);
    if ((directions & evaluation_mask & 1u) != 0)
        consumer.consume(0u, b.first);
    if ((directions & evaluation_mask & 2u) != 0)
        consumer.consume(1u, b.second);
    return b.scattered;
}

}
