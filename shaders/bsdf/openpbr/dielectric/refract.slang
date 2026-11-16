// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/refract.slang";

import bsdf.openpbr.dielectric.contract;
import bsdf.openpbr.dielectric.discrete;
import bsdf.openpbr.dielectric.thin_wall;
import bsdf.openpbr.math.fresnel;
import bsdf.openpbr.math.microfacet_distribution;
import bsdf.openpbr.math.microfacet_energy;
import bsdf.openpbr.math.microfacet_measure;
import bsdf.openpbr.contract.flags;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.math.spectrum;

// 连续 core 仅供已证明 max(alpha)>=1e-4 且实际界面 ior!=1 的 solid 入口。
// 通用入口保留原光滑／匹配 IOR 分流；连续物理公式与后处理共用。
namespace openpbr
{
namespace kernel
{
public func refractive_eval(wi: float3, wo: float3, state: TransmissionState, sampling_flags: uint)
    ->Response
{
    let result : Response = empty_response();
    let microfacet : Microfacet = state.specular_microfacet;
    let fresnel : SpecularFresnel = state.specular_fresnel;
    if (microfacet_effectively_smooth(microfacet) || fresnel.ior == 1.0)
    {
        return result;
    }
    return refractive_eval_continuous(wi, wo, state, sampling_flags);
}

public func refractive_eval_continuous(wi: float3, wo: float3, state: TransmissionState,
                                       sampling_flags: uint)
    ->Response
{

    var result : Response = empty_response();
    let microfacet : Microfacet = state.specular_microfacet;
    let fresnel : SpecularFresnel = state.specular_fresnel;
    let reflected : bool = wi.z * wo.z > 0.0;
    let eta_path : float = reflected ? 1.0 : (wi.z > 0.0 ? fresnel.ior : 1.0 / fresnel.ior);
    var half_vector : float3 = wi + eta_path * wo;
    let length_squared : float = dot(half_vector, half_vector);
    if (wo.z == 0.0 || wi.z == 0.0 || length_squared == 0.0)
    {
        return result;
    }
    half_vector *= rsqrt(length_squared);
    if (half_vector.z <= 0.0)
    {
        half_vector = -half_vector;
    }
    let micro_incident : float = dot(wi, half_vector);
    let micro_outgoing : float = dot(wo, half_vector);
    if (micro_incident * wi.z < 0.0 || micro_outgoing * wo.z < 0.0)
    {
        return result;
    }
    let rt : InterfaceResponse = specular_interface_response(fresnel, micro_incident);
    let distribution : float = ggx_distribution(microfacet, half_vector);
    if (reflected && (sampling_flags & LOBE_SPECULAR_REFLECTION) != 0u)
    {
        result.value = rt.reflection * distribution * reflection_factor(microfacet, wi, wo);
        result.flags = LOBE_SPECULAR_REFLECTION;
    }
    else if (!reflected && (sampling_flags & LOBE_SPECULAR_TRANSMISSION) != 0u)
    {
        let denominator
            : float =
                  square(refraction_denominator(eta_path, micro_incident, micro_outgoing)) * wi.z;
        result.value = rt.transmission * distribution *
                       ggx_transmission_masking(microfacet, wi, wo) *
                       abs(micro_incident * micro_outgoing / denominator);
        result.value *= 1.0 / square(eta_path);
        result.flags = LOBE_SPECULAR_TRANSMISSION;
    }
    return result;
}

/// 连续 solid 同方向共享半向量、Fresnel 和 D，保留响应与密度各自算序。
public func refractive_evaluate_continuous(wi: float3, wo: float3, state: TransmissionState,
                                           sampling_flags: uint)
    ->TransmissionEvaluation
{

    var result : TransmissionEvaluation = {};
    if (!is_specular(sampling_flags))
        return result;
    let microfacet : Microfacet = state.specular_microfacet;
    let fresnel : SpecularFresnel = state.specular_fresnel;
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
    let rt : InterfaceResponse = specular_interface_response(fresnel, micro_incident);
    let distribution : float = ggx_distribution(microfacet, half_vector);
    if (reflected && (sampling_flags & LOBE_SPECULAR_REFLECTION) != 0u)
    {
        result.response.value =
            rt.reflection * distribution * reflection_factor(microfacet, wi, wo);
        result.response.flags = LOBE_SPECULAR_REFLECTION;
    }
    else if (!reflected && (sampling_flags & LOBE_SPECULAR_TRANSMISSION) != 0u)
    {
        let denominator
            : float =
                  square(refraction_denominator(eta_path, micro_incident, micro_outgoing)) * wi.z;
        result.response.value = rt.transmission * distribution *
                                ggx_transmission_masking(microfacet, wi, wo) *
                                abs(micro_incident * micro_outgoing / denominator);
        result.response.value *= 1.0 / square(eta_path);
        result.response.flags = LOBE_SPECULAR_TRANSMISSION;
    }
    var reflection_probability : float = spectrum_to_weight(rt.reflection);
    var transmission_probability : float = spectrum_to_weight(rt.transmission);
    if (!is_reflective(sampling_flags))
        reflection_probability = 0.0;
    if (!is_transmissive(sampling_flags))
        transmission_probability = 0.0;
    let probability_sum : float = reflection_probability + transmission_probability;
    if (probability_sum == 0.0)
        return result;
    let microfacet_pdf : float = ggx_pdf_from_distribution(microfacet, wi.z > 0.0 ? wi : -wi,
                                                           half_vector, false, distribution);
    if (reflected)
        result.pdf =
            microfacet_pdf / (4.0 * abs(micro_incident)) * reflection_probability / probability_sum;
    else
    {
        let denominator
            : float = square(refraction_denominator(eta_path, micro_incident, micro_outgoing));
        result.pdf = microfacet_pdf * abs(micro_outgoing) / denominator * transmission_probability /
                     probability_sum;
    }
    return result;
}

public func refractive_sample(wi: float3, random_value: float3, state: TransmissionState,
                              sampling_flags: uint)
    ->Proposal
{
    let microfacet : Microfacet = state.specular_microfacet;
    let fresnel : SpecularFresnel = state.specular_fresnel;
    if (microfacet_effectively_smooth(microfacet) || fresnel.ior == 1.0)
    {
        return refractive_discrete_sample(wi, random_value.z, state, sampling_flags);
    }
    return refractive_sample_continuous(wi, random_value, state, sampling_flags);
}

public func refractive_sample_continuous(wi: float3, random_value: float3, state: TransmissionState,
                                         sampling_flags: uint)
    ->Proposal
{
    let microfacet : Microfacet = state.specular_microfacet;
    let fresnel : SpecularFresnel = state.specular_fresnel;
    var half_vector = float3(0.0, 0.0, 1.0);
    var microfacet_pdf = 1.0;
    var distribution = 1.0;
    if (!is_specular(sampling_flags))
    {
        return empty_proposal();
    }
    let sample_direction : float3 = wi.z > 0.0 ? wi : -wi;
    half_vector = sample_ggx_vndf(microfacet, sample_direction, random_value.xy, false);
    microfacet_pdf = ggx_pdf(microfacet, sample_direction, half_vector, false);
    distribution = ggx_distribution(microfacet, half_vector);
    let micro_incident : float = dot(half_vector, wi);
    let rt : InterfaceResponse = specular_interface_response(fresnel, micro_incident);
    var reflection : float3 = rt.reflection;
    let transmission : float3 = rt.transmission;
    var reflection_probability : float = spectrum_to_weight(reflection);
    var transmission_probability : float = spectrum_to_weight(transmission);
    if (!is_reflective(sampling_flags))
    {
        reflection_probability = 0.0;
    }
    if (!is_transmissive(sampling_flags))
    {
        transmission_probability = 0.0;
    }
    let refracted : RefractResult = specular_fresnel_refract(fresnel, -wi, half_vector);
    if (refracted.valid == 0u)
    {
        transmission_probability = 0.0;
        reflection = float3(1.0);
    }
    let probability_sum : float = reflection_probability + transmission_probability;
    if (probability_sum == 0.0)
    {
        return empty_proposal();
    }
    var result : Proposal = empty_proposal();
    if (random_value.z < transmission_probability / probability_sum)
    {
        var wo : float3 = refracted.wo;
        if (wi.z * wo.z >= 0.0)
        {
            return empty_proposal();
        }
        let eta_path : float = refracted.relative_ior;
        result.wo = wo;
        result.eta = eta_path;
        result.response.value = transmission;
        result.pdf = transmission_probability / probability_sum;
        let micro_outgoing : float = dot(wo, half_vector);
        let denominator
            : float = square(refraction_denominator(eta_path, micro_incident, micro_outgoing));
        result.pdf *= microfacet_pdf * abs(micro_outgoing) / denominator;
        result.response.value = transmission * distribution *
                                ggx_transmission_masking(microfacet, wi, wo) *
                                abs(micro_incident * micro_outgoing / (wi.z * denominator));
        result.response.flags = LOBE_SPECULAR_TRANSMISSION;
        result.response.value *= 1.0 / square(eta_path);
    }
    else
    {
        var wo : float3 = reflect(-wi, half_vector);
        if (wi.z * wo.z < 0.0)
        {
            return empty_proposal();
        }
        result.wo = wo;
        result.response.value = reflection;
        result.pdf = reflection_probability / probability_sum;
        result.pdf *= microfacet_pdf / (4.0 * abs(micro_incident));
        result.response.value = reflection * distribution * reflection_factor(microfacet, wi, wo);
        result.response.flags = LOBE_SPECULAR_REFLECTION;
    }
    return result;
}

public func refractive_pdf(wi: float3, wo: float3, state: TransmissionState, sampling_flags: uint)
    ->float
{
    if (microfacet_effectively_smooth(state.specular_microfacet) ||
        state.specular_fresnel.ior == 1.0)
        return 0.0;
    return refractive_pdf_continuous(wi, wo, state, sampling_flags);
}

public func refractive_pdf_continuous(wi: float3, wo: float3, state: TransmissionState,
                                      sampling_flags: uint)
    ->float
{

    let microfacet : Microfacet = state.specular_microfacet;
    let fresnel : SpecularFresnel = state.specular_fresnel;
    if (!is_specular(sampling_flags))
    {
        return 0.0;
    }
    let reflected : bool = wi.z * wo.z > 0.0;
    let eta_path : float = reflected ? 1.0 : (wi.z > 0.0 ? fresnel.ior : 1.0 / fresnel.ior);
    var half_vector : float3 = wi + eta_path * wo;
    let length_squared : float = dot(half_vector, half_vector);
    if (wo.z == 0.0 || wi.z == 0.0 || length_squared == 0.0)
    {
        return 0.0;
    }
    half_vector *= rsqrt(length_squared);
    if (half_vector.z <= 0.0)
    {
        half_vector = -half_vector;
    }
    let micro_incident : float = dot(wi, half_vector);
    let micro_outgoing : float = dot(wo, half_vector);
    if (micro_incident * wi.z < 0.0 || micro_outgoing * wo.z < 0.0)
    {
        return 0.0;
    }
    let rt : InterfaceResponse = specular_interface_response(fresnel, micro_incident);
    var reflection_probability : float = spectrum_to_weight(rt.reflection);
    var transmission_probability : float = spectrum_to_weight(rt.transmission);
    if (!is_reflective(sampling_flags))
    {
        reflection_probability = 0.0;
    }
    if (!is_transmissive(sampling_flags))
    {
        transmission_probability = 0.0;
    }
    let probability_sum : float = reflection_probability + transmission_probability;
    if (probability_sum == 0.0)
    {
        return 0.0;
    }
    let microfacet_pdf : float = ggx_pdf(microfacet, wi.z > 0.0 ? wi : -wi, half_vector, false);
    if (reflected)
    {
        return microfacet_pdf / (4.0 * abs(micro_incident)) * reflection_probability /
               probability_sum;
    }
    let denominator : float =
                          square(refraction_denominator(eta_path, micro_incident, micro_outgoing));
    return microfacet_pdf * abs(micro_outgoing) / denominator * transmission_probability /
           probability_sum;
}

public func finish_solid_evaluation(result: Response, state: TransmissionState, continuous: bool)
    ->Response
{
    var result = result;
    if (result.flags != LOBE_NONE)
    {
        if (is_transmissive(result.flags))
        {
            result.value *= state.transmission_tint;
        }
        if (continuous || is_non_delta(result.flags))
        {
            result.value *= is_transmissive(result.flags)
                                ? state.transmission_multiple_scattering.y
                                : state.transmission_multiple_scattering.x;
        }
    }
    return result;
}

public func transmission_eval_raw<let continuous : bool = false>(wi: float3, input_wo: float3,
                                                                 state: TransmissionState,
                                                                 sampling_flags: uint)
    ->Response
{
    if (state.geometry_thin_walled != 0u)
    {
        var wo : float3 = input_wo;
        var transmitted = false;
        if (wo.z < 0.0)
        {
            wo.z = -wo.z;
            transmitted = true;
            if (!is_transmissive(sampling_flags))
            {
                return empty_response();
            }
        }
        else if (!is_reflective(sampling_flags))
        {
            return empty_response();
        }
        let base : ReflectionEvaluation = evaluate_reflection_impl<continuous>(
                       wi, wo, state.transmission_microfacet, sampling_flags, LOBE_ALL);
        var result : Response;
        result.value = float3(base.factor);
        result.flags = base.flags;
        if (result.flags != LOBE_NONE)
        {
            let rt : InterfaceResponse = transmission_thin_wall_rt(base.micro_cosine, state);
            if (transmitted)
            {
                result.value *= rt.transmission;
                result.flags =
                    is_delta(result.flags) ? LOBE_DELTA_TRANSMISSION : LOBE_SPECULAR_TRANSMISSION;
            }
            else
            {
                result.value *= rt.reflection;
            }
            if (continuous || is_non_delta(result.flags))
            {
                result.value /=
                    microfacet_directional_albedo_unity(state.transmission_microfacet, wi.z);
            }
        }
        return result;
    }
    let material_wi : float3 = transmission_material_frame(wi, state);
    let material_wo : float3 = transmission_material_frame(input_wo, state);
    let result : Response = refractive_eval(material_wi, material_wo, state, sampling_flags);
    return finish_solid_evaluation(result, state, false);
}

public func transmission_eval_raw_solid_continuous(wi: float3, input_wo: float3,
                                                   state: TransmissionState, sampling_flags: uint)
    ->Response
{
    let material_wi : float3 = transmission_material_frame(wi, state);
    let material_wo : float3 = transmission_material_frame(input_wo, state);
    let result : Response =
                     refractive_eval_continuous(material_wi, material_wo, state, sampling_flags);
    return finish_solid_evaluation(result, state, true);
}

/// 显式边界状态的联合评估；光滑／匹配 IOR、thin 的数值拒绝保持原入口约定。
public func transmission_evaluate_raw<let continuous : bool = false>(wi: float3, input_wo: float3,
                                                                     state: TransmissionState,
                                                                     sampling_flags: uint)
    ->TransmissionEvaluation
{
    if (state.geometry_thin_walled == 0u)
    {
        var result : TransmissionEvaluation = {};
        if (microfacet_effectively_smooth(state.specular_microfacet) ||
            state.specular_fresnel.ior == 1.0)
            return result;
        result = refractive_evaluate_continuous(transmission_material_frame(wi, state),
                                                transmission_material_frame(input_wo, state), state,
                                                sampling_flags);
        result.response = finish_solid_evaluation(result.response, state, false);
        return result;
    }
    var wo : float3 = input_wo;
    let transmitted : bool = wo.z < 0.0;
    if (transmitted)
        wo.z = -wo.z;
    let base : ReflectionEvaluation = evaluate_reflection_impl<continuous>(
                   wi, wo, state.transmission_microfacet, sampling_flags, LOBE_ALL);
    var result : TransmissionEvaluation = {};
    if (transmitted ? is_transmissive(sampling_flags) : is_reflective(sampling_flags))
    {
        result.response.value = float3(base.factor);
        result.response.flags = base.flags;
        if (base.flags != LOBE_NONE)
        {
            let rt : InterfaceResponse = transmission_thin_wall_rt(base.micro_cosine, state);
            if (transmitted)
            {
                result.response.value *= rt.transmission;
                result.response.flags =
                    is_delta(base.flags) ? LOBE_DELTA_TRANSMISSION : LOBE_SPECULAR_TRANSMISSION;
            }
            else
                result.response.value *= rt.reflection;
            if (continuous || is_non_delta(result.response.flags))
                result.response.value /=
                    microfacet_directional_albedo_unity(state.transmission_microfacet, wi.z);
        }
    }
    // PDF 的 Fresnel 原入口使用 normalize，保留该数值路径及无效输入边界。
    let half_vector : float3 = normalize(wi + wo);
    let rt : InterfaceResponse = transmission_thin_wall_rt(dot(wi, half_vector), state);
    var reflection_probability : float = spectrum_to_weight(rt.reflection);
    var transmission_probability : float = spectrum_to_weight(rt.transmission);
    if (!is_reflective(sampling_flags))
        reflection_probability = 0.0;
    if (!is_transmissive(sampling_flags))
        transmission_probability = 0.0;
    let probability_sum : float = reflection_probability + transmission_probability;
    if (probability_sum != 0.0)
        result.pdf = transmitted ? base.pdf * transmission_probability / probability_sum
                                 : base.pdf * reflection_probability / probability_sum;
    return result;
}

/// 已证明连续且非匹配 IOR 的 solid 联合评估，补偿仍在几何响应之后应用。
public func transmission_evaluate_raw_solid_continuous(wi: float3, input_wo: float3,
                                                       state: TransmissionState,
                                                       sampling_flags: uint)
    ->TransmissionEvaluation
{
    var result : TransmissionEvaluation = refractive_evaluate_continuous(
                     transmission_material_frame(wi, state),
                     transmission_material_frame(input_wo, state), state, sampling_flags);
    result.response = finish_solid_evaluation(result.response, state, true);
    return result;
}

public func finish_solid_sample(result: Proposal, state: TransmissionState, continuous: bool)
    ->Proposal
{
    var result = result;
    if (result.response.flags == LOBE_NONE)
    {
        return result;
    }
    result.wo = transmission_material_frame(result.wo, state);
    if (is_transmissive(result.response.flags))
    {
        result.response.value *= state.transmission_tint;
    }
    if (continuous || is_non_delta(result.response.flags))
    {
        result.response.value *= is_transmissive(result.response.flags)
                                     ? state.transmission_multiple_scattering.y
                                     : state.transmission_multiple_scattering.x;
    }
    return result;
}

public func transmission_sample_raw<let continuous : bool = false>(wi: float3, random_value: float3,
                                                                   state: TransmissionState,
                                                                   sampling_flags: uint)
    ->Proposal
{
    var result : Proposal = empty_proposal();
    if (state.geometry_thin_walled != 0u)
    {
        if (!continuous && microfacet_effectively_smooth(state.transmission_microfacet))
        {
            return thin_wall_discrete_sample_raw(wi, random_value.z, state, sampling_flags);
        }
        let base : ReflectionProposal = sample_reflection_impl<continuous>(
                       wi, random_value, state.transmission_microfacet, sampling_flags, LOBE_ALL);
        result = base.proposal;
        if (result.response.flags != LOBE_NONE)
        {
            let rt : InterfaceResponse = transmission_thin_wall_rt(base.micro_cosine, state);
            var reflection_probability : float = spectrum_to_weight(rt.reflection);
            var transmission_probability : float = spectrum_to_weight(rt.transmission);
            if (!is_reflective(sampling_flags))
            {
                reflection_probability = 0.0;
            }
            if (!is_transmissive(sampling_flags))
            {
                transmission_probability = 0.0;
            }
            let probability_sum : float = reflection_probability + transmission_probability;
            if (probability_sum == 0.0)
            {
                result = empty_proposal();
                return result;
            }
            if (random_value.z < transmission_probability / probability_sum)
            {
                result.response.value *= rt.transmission;
                result.pdf *= transmission_probability / probability_sum;
                result.response.flags = (!continuous && is_delta(result.response.flags))
                                            ? LOBE_DELTA_TRANSMISSION
                                            : LOBE_SPECULAR_TRANSMISSION;
                result.wo.z = -result.wo.z;
            }
            else
            {
                result.response.value *= rt.reflection;
                result.pdf *= reflection_probability / probability_sum;
            }
            if (continuous || is_non_delta(result.response.flags))
            {
                result.response.value /=
                    microfacet_directional_albedo_unity(state.transmission_microfacet, wi.z);
            }
        }
        return result;
    }

    let material_wi : float3 = transmission_material_frame(wi, state);
    result = refractive_sample(material_wi, random_value, state, sampling_flags);
    return finish_solid_sample(result, state, false);
}

public func transmission_sample_raw_solid_continuous(wi: float3, random_value: float3,
                                                     state: TransmissionState, sampling_flags: uint)
    ->Proposal
{
    let material_wi : float3 = transmission_material_frame(wi, state);
    let result : Proposal =
                     refractive_sample_continuous(material_wi, random_value, state, sampling_flags);
    return finish_solid_sample(result, state, true);
}

public func transmission_pdf_raw<let continuous : bool = false>(wi: float3, input_wo: float3,
                                                                state: TransmissionState,
                                                                sampling_flags: uint)
    ->float
{
    if (state.geometry_thin_walled != 0u)
    {
        var wo : float3 = input_wo;
        var transmitted = false;
        if (wo.z < 0.0)
        {
            wo.z = -wo.z;
            transmitted = true;
        }
        let half_vector : float3 = normalize(wi + wo);
        let rt : InterfaceResponse = transmission_thin_wall_rt(dot(wi, half_vector), state);
        var reflection_probability : float = spectrum_to_weight(rt.reflection);
        var transmission_probability : float = spectrum_to_weight(rt.transmission);
        if (!is_reflective(sampling_flags))
        {
            reflection_probability = 0.0;
        }
        if (!is_transmissive(sampling_flags))
        {
            transmission_probability = 0.0;
        }
        let probability_sum : float = reflection_probability + transmission_probability;
        if (probability_sum == 0.0)
        {
            return 0.0;
        }
        let microfacet_pdf : float = reflection_pdf_impl<continuous>(
                                 wi, wo, state.transmission_microfacet, sampling_flags, LOBE_ALL);
        return transmitted ? microfacet_pdf * transmission_probability / probability_sum
                           : microfacet_pdf * reflection_probability / probability_sum;
    }
    return refractive_pdf(transmission_material_frame(wi, state),
                          transmission_material_frame(input_wo, state), state, sampling_flags);
}

public func transmission_pdf_raw_solid_continuous(wi: float3, input_wo: float3,
                                                  state: TransmissionState, sampling_flags: uint)
    ->float
{
    return refractive_pdf_continuous(transmission_material_frame(wi, state),
                                     transmission_material_frame(input_wo, state), state,
                                     sampling_flags);
}
}
}
