// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// Solid/Thin白名单数值部件。连续折射公式据RoboCute适配，来源见assets/NOTICE.md；第三方源保持不变。
module transmission_composition;
import openpbr.selection;
import openpbr.composition;
import openpbr.primary_policy;
import openpbr.math.fresnel;
import openpbr.math.delta;
import openpbr.math.microfacet_distribution;
import openpbr.math.microfacet_energy;
import openpbr.math.spectrum;
import openpbr.contract.sample;
import openpbr.contract.flags;
import openpbr.dielectric.refract;
namespace openpbr {
namespace closure {
public interface ITransmission {
    associatedtype State;
    static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut, roughness: float, ior: float,
                                                extinction: float3, wi: float3)
        ->State;
    static func evaluate<let balanced : bool = false>(wi: float3, wo: float3, state: State)
        ->LocalEvaluation;
    static func sample<let balanced : bool = false>(wi: float3, random: float3, state: State)
        ->Proposal;
};
public struct SolidDelta : ITransmission {
    public typedef float State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut, roughness: float,
                                                       ior: float, extinction: float3, wi: float3)
        ->State {
        return ior;
    }
    public static func evaluate<let balanced : bool = false>(wi: float3, wo: float3, state: State)
        ->LocalEvaluation {
        return {};
    }
    public static func sample<let balanced : bool = false>(wi: float3, random: float3, state: State)
        ->Proposal {
        return kernel::refractive_delta_sample<balanced>(wi, random.z, dielectric_fresnel(state),
                                                         kernel::LOBE_ALL);
    }
};
public struct SolidState {
    public var alpha : float;
    public var ior : float;
    public var compensation : float2;
};
public struct SolidContinuous : ITransmission {
    public typedef SolidState State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut, roughness: float,
                                                       ior: float, extinction: float3, wi: float3)
        ->State {
        let alpha : float = roughness * roughness;
        let distribution : kernel::Microfacet = { float2(alpha) };
        return { alpha, ior,
                 kernel::dielectric_energy<false, true, true, true>(energy, distribution, wi.z, ior,
                                                                    roughness)
                     .xy };
    }
    public static func evaluate<let balanced : bool = false>(wi: float3, wo: float3, state: State)
        ->LocalEvaluation {
        var evaluated : LocalEvaluation = solid_evaluate<balanced>(wi, wo, state.alpha, state.ior);
        if (evaluated.response.flags != 0)
            evaluated.response.value *= kernel::is_transmissive(evaluated.response.flags)
                                            ? state.compensation.y
                                            : state.compensation.x;
        return evaluated;
    }
    public static func sample<let balanced : bool = false>(wi: float3, random: float3, state: State)
        ->Proposal {
        var proposal : Proposal = solid_sample<balanced>(wi, random, state.alpha, state.ior);
        if (proposal.response.flags != 0)
            proposal.response.value *= kernel::is_transmissive(proposal.response.flags)
                                           ? state.compensation.y
                                           : state.compensation.x;
        return proposal;
    }
};
public struct SolidMixed : ITransmission {
    public typedef SolidState State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut, roughness: float,
                                                       ior: float, extinction: float3, wi: float3)
        ->State {
        if (roughness * roughness < 1.0e-4 || ior == 1)
            return { roughness * roughness, ior, float2(1) };
        return SolidContinuous.prepare(energy, roughness, ior, extinction, wi);
    }
    public static func evaluate<let balanced : bool = false>(wi: float3, wo: float3, state: State)
        ->LocalEvaluation {
        return state.alpha < 1.0e-4 || state.ior == 1
                   ? LocalEvaluation()
                   : SolidContinuous.evaluate<balanced>(wi, wo, state);
    }
    public static func sample<let balanced : bool = false>(wi: float3, random: float3, state: State)
        ->Proposal {
        return state.alpha < 1.0e-4 || state.ior == 1
                   ? SolidDelta.sample<balanced>(wi, random, state.ior)
                   : SolidContinuous.sample<balanced>(wi, random, state);
    }
};
/// 粗糙度已排除delta，只允许IOR匹配产生直通。
public struct SolidRoughMatched : ITransmission {
    public typedef SolidState State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut, roughness: float,
                                                       ior: float, extinction: float3, wi: float3)
        ->State {
        return ior == 1 ? State(roughness * roughness, ior, float2(1))
                        : SolidContinuous.prepare(energy, roughness, ior, extinction, wi);
    }
    public static func evaluate<let balanced : bool = false>(wi: float3, wo: float3, state: State)
        ->LocalEvaluation {
        return state.ior == 1 ? LocalEvaluation()
                              : SolidContinuous.evaluate<balanced>(wi, wo, state);
    }
    public static func sample<let balanced : bool = false>(wi: float3, random: float3, state: State)
        ->Proposal {
        return state.ior == 1 ? SolidDelta.sample<balanced>(wi, random, 1)
                              : SolidContinuous.sample<balanced>(wi, random, state);
    }
};
public struct MatchedSolid : ITransmission {
    public typedef Empty State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut, roughness: float,
                                                       ior: float, extinction: float3, wi: float3)
        ->State {
        return {};
    }
    public static func evaluate<let balanced : bool = false>(wi: float3, wo: float3, state: State)
        ->LocalEvaluation {
        return {};
    }
    public static func sample<let balanced : bool = false>(wi: float3, random: float3, state: State)
        ->Proposal {
        return SolidDelta.sample<balanced>(wi, random, 1);
    }
};
public struct ThinDeltaState {
    public var ior : float;
    public var extinction : float3;
};
public struct ThinDelta : ITransmission {
    public typedef ThinDeltaState State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut, roughness: float,
                                                       ior: float, extinction: float3, wi: float3)
        ->State {
        return { ior, extinction };
    }
    public static func evaluate<let balanced : bool = false>(wi: float3, wo: float3, state: State)
        ->LocalEvaluation {
        return {};
    }
    public static func sample<let balanced : bool = false>(wi: float3, random: float3, state: State)
        ->Proposal {
        return kernel::thin_wall_delta_sample<balanced>(wi, random.z, dielectric_fresnel(state.ior),
                                                        float3(1), state.extinction,
                                                        kernel::LOBE_ALL);
    }
};
public struct ThinState {
    public var alpha : float;
    public var ior : float;
    public var extinction : float3;
};
public struct ThinRough<Measure : IMeasure> : ITransmission {
    public typedef ThinState State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut, roughness: float,
                                                       ior: float, extinction: float3, wi: float3)
        ->State {
        return { clamp(roughness * roughness * kernel::thin_dielectric_roughness_scaler2(ior), 0.0,
                       1.0),
                 ior, extinction };
    }
    public static func evaluate<let balanced : bool = false>(wi: float3, wo: float3, state: State)
        ->LocalEvaluation {
        var wo = wo;
        let transmitted : bool = wo.z < 0;
        if (transmitted)
            wo.z = -wo.z;
        let b : kernel::ReflectionEvaluation =
                    Measure.evaluate(wi, wo, Measure.prepare_alpha(state.alpha));
        if (b.flags == 0)
            return {};
        let rt : kernel::InterfaceResponse = kernel::thin_wall_rt(
                     b.micro_cosine, dielectric_fresnel(state.ior), float3(1), state.extinction);
        var evaluated : LocalEvaluation = {};
        evaluated.response = { float3(b.factor) * (transmitted ? rt.transmission : rt.reflection),
                               transmitted ? kernel::LOBE_SPECULAR_TRANSMISSION : b.flags };
        evaluated.response.value /= kernel::microfacet_directional_albedo_unity<true>(
            Measure.distribution(Measure.prepare_alpha(state.alpha)), wi.z);
        var reflection : float = kernel::spectrum_to_weight(rt.reflection);
        var transmission : float = kernel::spectrum_to_weight(rt.transmission);
        if (balanced) {
            let available : bool = reflection + transmission > 0;
            let probability
                : float = openpbr::primary::reflection_probability(reflection, transmission);
            reflection = available ? probability : 0;
            transmission = available ? 1 - probability : 0;
        }
        let sum : float = reflection + transmission;
        if (sum != 0)
            evaluated.pdf = b.pdf * (transmitted ? transmission : reflection) / sum;
        return evaluated;
    }
    public static func sample<let balanced : bool = false>(wi: float3, random: float3, state: State)
        ->Proposal {
        let geometry : Measure.State = Measure.prepare_alpha(state.alpha);
        if (Measure.delta(geometry))
            return ThinDelta.sample<balanced>(wi, random, { state.ior, state.extinction });
        let b : kernel::ReflectionProposal = Measure.sample(wi, random, geometry);
        var proposal : Proposal = b.proposal;
        if (proposal.response.flags == 0)
            return proposal;
        let rt : kernel::InterfaceResponse = kernel::thin_wall_rt(
                     b.micro_cosine, dielectric_fresnel(state.ior), float3(1), state.extinction);
        var reflection : float = kernel::spectrum_to_weight(rt.reflection);
        var transmission : float = kernel::spectrum_to_weight(rt.transmission);
        if (balanced) {
            let available : bool = reflection + transmission > 0;
            let probability
                : float = openpbr::primary::reflection_probability(reflection, transmission);
            reflection = available ? probability : 0;
            transmission = available ? 1 - probability : 0;
        }
        let sum : float = reflection + transmission;
        if (sum == 0)
            return kernel::empty_proposal();
        if (random.z < transmission / sum) {
            proposal.response.value *= rt.transmission;
            proposal.pdf *= transmission / sum;
            proposal.response.flags = kernel::LOBE_SPECULAR_TRANSMISSION;
            proposal.wo.z = -proposal.wo.z;
        } else {
            proposal.response.value *= rt.reflection;
            proposal.pdf *= reflection / sum;
        }
        proposal.response.value /=
            kernel::microfacet_directional_albedo_unity<true>(Measure.distribution(geometry), wi.z);
        return proposal;
    }
};
public struct MatchedThin : ITransmission {
    public typedef float3 State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut, roughness: float,
                                                       ior: float, extinction: float3, wi: float3)
        ->State {
        return extinction;
    }
    public static func evaluate<let balanced : bool = false>(wi: float3, wo: float3, state: State)
        ->LocalEvaluation {
        return {};
    }
    public static func sample<let balanced : bool = false>(wi: float3, random: float3, state: State)
        ->Proposal {
        var proposal : Proposal = kernel::empty_proposal();
        let response : float3 = wi.z > 0 ? exp(-state / wi.z) : float3(0);
        if (wi.z > 0 && kernel::spectrum_to_weight(response) > 0) {
            proposal.wo = -wi;
            proposal.response = { response, kernel::LOBE_DELTA_TRANSMISSION };
            proposal.pdf = 1;
        }
        return proposal;
    }
};
public func solid_evaluate<let balanced : bool = false>(wi: float3, wo: float3, alpha: float,
                                                        ior: float)
    ->LocalEvaluation {

    var result : LocalEvaluation = {};
    let microfacet = kernel::Microfacet(float2(alpha));
    let fresnel : kernel::SpecularFresnel = dielectric_fresnel(ior);
    let reflected : bool = wi.z * wo.z > 0.0;
    let eta_path : float = reflected ? 1.0 : (wi.z > 0.0 ? fresnel.ior : 1.0 / fresnel.ior);
    var half_vector : float3 = wi + eta_path * wo;
    let length_squared : float = dot(half_vector, half_vector);
    if (wo.z == 0.0 || wi.z == 0.0 || length_squared == 0.0)
        return result;
    half_vector *= rsqrt(length_squared);
    if (half_vector.z <= 0.0)
        half_vector = -half_vector;
    let micro_incident : float = dot(wi, half_vector);
    let micro_outgoing : float = dot(wo, half_vector);
    if (micro_incident * wi.z < 0.0 || micro_outgoing * wo.z < 0.0)
        return result;
    let rt : kernel::InterfaceResponse =
                 kernel::specular_interface_response(fresnel, micro_incident);
    let distribution : float = kernel::ggx_distribution<true>(microfacet, half_vector);
    if (reflected) {
        result.response.value =
            rt.reflection * distribution * kernel::reflection_factor(microfacet, wi, wo);
        result.response.flags = kernel::LOBE_SPECULAR_REFLECTION;
    } else if (!reflected) {
        let denominator : float = kernel::square(kernel::refraction_denominator(
                                      eta_path, micro_incident, micro_outgoing)) *
                                  wi.z;
        result.response.value = rt.transmission * distribution *
                                kernel::ggx_transmission_masking(microfacet, wi, wo) *
                                abs(micro_incident * micro_outgoing / denominator);
        result.response.value *= 1.0 / kernel::square(eta_path);
        result.response.flags = kernel::LOBE_SPECULAR_TRANSMISSION;
    }
    var reflection_probability : float = kernel::spectrum_to_weight(rt.reflection);
    var transmission_probability : float = kernel::spectrum_to_weight(rt.transmission);
    if (balanced) {
        let available : bool = reflection_probability + transmission_probability > 0;
        let probability : float = openpbr::primary::reflection_probability(
                              reflection_probability, transmission_probability);
        reflection_probability = available ? probability : 0;
        transmission_probability = available ? 1 - probability : 0;
    }
    let probability_sum : float = reflection_probability + transmission_probability;
    if (probability_sum == 0.0)
        return result;
    let microfacet_pdf : float = kernel::ggx_pdf_from_distribution(
                             microfacet, wi.z > 0.0 ? wi : -wi, half_vector, false, distribution);
    if (reflected)
        result.pdf =
            microfacet_pdf / (4.0 * abs(micro_incident)) * reflection_probability / probability_sum;
    else {
        let denominator : float = kernel::square(kernel::refraction_denominator(
                              eta_path, micro_incident, micro_outgoing));
        result.pdf = microfacet_pdf * abs(micro_outgoing) / denominator * transmission_probability /
                     probability_sum;
    }
    return result;
}

public func solid_sample<let balanced : bool = false>(wi: float3, random_value: float3,
                                                      alpha: float, ior: float)
    ->kernel::Proposal {
    if (wi.z == 0)
        return kernel::empty_proposal();
    let microfacet = kernel::Microfacet(float2(alpha));
    let fresnel : kernel::SpecularFresnel = dielectric_fresnel(ior);
    var half_vector = float3(0.0, 0.0, 1.0);
    var microfacet_pdf = 1.0;
    var distribution = 1.0;
    let sample_direction : float3 = wi.z > 0.0 ? wi : -wi;
    half_vector = kernel::sample_ggx_vndf(microfacet, sample_direction, random_value.xy, false);
    distribution = kernel::ggx_distribution<true>(microfacet, half_vector);
    microfacet_pdf = kernel::ggx_pdf_from_distribution(microfacet, sample_direction, half_vector,
                                                       false, distribution);
    let micro_incident : float = dot(half_vector, wi);
    let rt : kernel::InterfaceResponse =
                 kernel::specular_interface_response(fresnel, micro_incident);
    var reflection : float3 = rt.reflection;
    let transmission : float3 = rt.transmission;
    var reflection_probability : float = kernel::spectrum_to_weight(reflection);
    var transmission_probability : float = kernel::spectrum_to_weight(transmission);
    let refracted : kernel::RefractResult =
                        kernel::specular_fresnel_refract(fresnel, -wi, half_vector);
    if (refracted.valid == 0u) {
        transmission_probability = 0.0;
        reflection = float3(1.0);
    }
    if (balanced) {
        let available : bool = reflection_probability + transmission_probability > 0;
        let probability : float = openpbr::primary::reflection_probability(
                              reflection_probability, transmission_probability);
        reflection_probability = available ? probability : 0;
        transmission_probability = available ? 1 - probability : 0;
    }
    let probability_sum : float = reflection_probability + transmission_probability;
    if (probability_sum == 0.0) {
        return kernel::empty_proposal();
    }
    var result : kernel::Proposal = kernel::empty_proposal();
    if (random_value.z < transmission_probability / probability_sum) {
        var wo : float3 = refracted.wo;
        if (wi.z * wo.z >= 0.0) {
            return kernel::empty_proposal();
        }
        let eta_path : float = refracted.relative_ior;
        result.wo = wo;
        result.eta = eta_path;
        result.response.value = transmission;
        result.pdf = transmission_probability / probability_sum;
        let micro_outgoing : float = dot(wo, half_vector);
        let denominator : float = kernel::square(kernel::refraction_denominator(
                              eta_path, micro_incident, micro_outgoing));
        result.pdf *= microfacet_pdf * abs(micro_outgoing) / denominator;
        result.response.value = transmission * distribution *
                                kernel::ggx_transmission_masking(microfacet, wi, wo) *
                                abs(micro_incident * micro_outgoing / (wi.z * denominator));
        result.response.flags = kernel::LOBE_SPECULAR_TRANSMISSION;
        result.response.value *= 1.0 / kernel::square(eta_path);
    } else {
        var wo : float3 = reflect(-wi, half_vector);
        if (wi.z * wo.z < 0.0) {
            return kernel::empty_proposal();
        }
        result.wo = wo;
        result.response.value = reflection;
        result.pdf = reflection_probability / probability_sum;
        result.pdf *= microfacet_pdf / (4.0 * abs(micro_incident));
        result.response.value =
            reflection * distribution * kernel::reflection_factor(microfacet, wi, wo);
        result.response.flags = kernel::LOBE_SPECULAR_REFLECTION;
    }
    return result;
}

static const let transmission_is_thin : bool = (selected_flow & FLOW_THIN_TRANSMISSION) != 0;
static const let transmission_is_matched : bool = (selected_flow & FLOW_DIELECTRIC_REFLECTION) == 0;
static const let transmission_is_delta
    : bool = (selected_flow & (FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION)) == 0;
static const let thin_has_delta
    : bool = (selected_flow & (FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION)) != 0;
static const let solid_has_delta_reflection : bool = (selected_flow & FLOW_DELTA_REFLECTION) != 0;
static const let solid_has_matched_domain : bool = (selected_flow & FLOW_DELTA_TRANSMISSION) != 0;
typealias SelectedThinRough =
    TypeSelect<thin_has_delta, ThinRough<MixedMeasure>, ThinRough<Continuous>>.Type;
typealias ReflectingThin = TypeSelect<transmission_is_delta, ThinDelta, SelectedThinRough>.Type;
typealias SelectedThin = TypeSelect<transmission_is_matched, MatchedThin, ReflectingThin>.Type;
typealias SelectedSolidRough =
    TypeSelect<solid_has_matched_domain, SolidRoughMatched, SolidContinuous>.Type;
typealias SelectedSolidMeasure =
    TypeSelect<solid_has_delta_reflection, SolidMixed, SelectedSolidRough>.Type;
typealias ReflectingSolid =
    TypeSelect<transmission_is_delta, SolidDelta, SelectedSolidMeasure>.Type;
typealias SelectedSolid = TypeSelect<transmission_is_matched, MatchedSolid, ReflectingSolid>.Type;
public typealias Transmission = TypeSelect<transmission_is_thin, SelectedThin, SelectedSolid>.Type;
}
}
