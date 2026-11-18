// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
// OpenPBR 支持子域的编译期入口；来源与许可见 crates/openpbr/assets/NOTICE.md。

/// 对已分类材料按编译期类别保留相应闭包，不读取应用 ID。
/// Material、方向和介质契约同 api；调用方在发布材料时保证类别匹配，不作运行时修复或回退。
/// 同一类别仍允许有效的 RGB、roughness、IOR 和 SSS 权重变化；不是预烘材质或完整 OpenPBR。
module specialized;

import openpbr.api;
import openpbr.compact_lobes;
import openpbr.contract.material;
import openpbr.contract.sample;
import openpbr.opaque.state;
import openpbr.opaque.lobes;
import openpbr.opaque.evaluate;
import openpbr.opaque.discrete;
import openpbr.dielectric.contract;
import openpbr.dielectric.state;
import openpbr.dielectric.evaluate;
import openpbr.dielectric.refract;
import openpbr.math.frame;
import openpbr.math.microfacet_energy;
import openpbr.math.fresnel;
import openpbr.math.delta;
import openpbr.math.spectrum;
import openpbr.contract.flags;
import openpbr.service.sample;
import openpbr.service.conversion;
import openpbr.service.sanitization;

namespace openpbr {
/// 同一命中的两个连续方向与一次散射；未请求的评估为零，未请求散射为规范的无事件结果。
public struct Batch {
    /// evaluation_mask bit0 对应的连续响应／PDF，未请求时为零。
    public var first : Evaluation;
    /// evaluation_mask bit1 对应的连续响应／PDF，未请求时为零。
    public var second : Evaluation;
    /// 可选散射；不包含沿路径段的 Beer 吸收。
    public var scattered : Sample;
};

/// 非 dielectric、metalness=0、SSS=0；粗糙度仍可产生光滑 specular 与连续 diffuse 混合。
public static const let CAP_OPAQUE_DIELECTRIC = 0u;
/// 非 dielectric、metalness=0；保留一般 0..1 薄层 SSS 权重及其反侧散射。
public static const let CAP_OPAQUE_SUBSURFACE = 1u;
/// 非 dielectric、metalness=1；粗糙度可连续变化，SSS 与原 conductor 一样不参与响应。
public static const let CAP_CONDUCTOR = 2u;
/// solid dielectric；仍保留 IOR 相等时的 delta 透射及粗糙度的光滑边界。
public static const let CAP_SOLID_DIELECTRIC = 3u;
/// thin dielectric；厚度由 Material 提供，仍保留有效粗糙度及 IOR 范围。
public static const let CAP_THIN_DIELECTRIC = 4u;
/// 光滑 conductor：roughness²<1e-4；无需能量表，不存在连续方向评估。
public static const let CAP_DELTA_CONDUCTOR = 5u;
/// 纯 delta solid：roughness²<1e-4 或两侧 IOR 相等；保留 Fresnel/TIR、eta 与 medium 更新。
public static const let CAP_DELTA_SOLID_DIELECTRIC = 6u;
/// 纯 delta thin：须满足 api.is_discrete_only 的缩放／clamp 阈值；保留薄片吸收，不更新 medium。
public static const let CAP_DELTA_THIN_DIELECTRIC = 7u;

internal func require_capability<let capability : uint>()->void {
    static_assert(capability <= CAP_DELTA_THIN_DIELECTRIC, "unknown OpenPBR capability");
}

internal func specialized_dielectric<let capability : uint>()->bool {
    return capability == CAP_SOLID_DIELECTRIC || capability == CAP_THIN_DIELECTRIC ||
           capability == CAP_DELTA_SOLID_DIELECTRIC || capability == CAP_DELTA_THIN_DIELECTRIC;
}

internal func specialized_thin<let capability : uint>()->bool {
    return capability == CAP_THIN_DIELECTRIC || capability == CAP_DELTA_THIN_DIELECTRIC;
}

public func specialized_frame<let capability : uint>(normal: float3, view: float3, incident: float4,
                                                     transmitted: float4)
    ->kernel::Frame {
    var normal = normal;
    normal = dot(normal, view) < 0.0 ? -normal : normal;
    var frame : kernel::Frame = kernel::frame_from_normal(normal);
    if (specialized_dielectric<capability>()) {
        let exiting : bool = !specialized_thin<capability>() && incident.x > transmitted.x;
        if ((dot(frame.normal, view) >= 0.0) == exiting) {
            frame.normal = -frame.normal;
            frame.bitangent = -frame.bitangent;
        }
    }
    return frame;
}

internal func specialized_ior<let capability : uint>(incident: float4, transmitted: float4)->float {
    let exiting : bool = !specialized_thin<capability>() && incident.x > transmitted.x;
    return (exiting ? incident.x : transmitted.x) / (exiting ? transmitted.x : incident.x);
}

public func specialized_opaque<let capability : uint, let continuous : bool, EnergyLut>(
    energy: EnergyLut, material: Material, wi: float3, incident: float4)
    ->kernel::OpaqueState
    where EnergyLut : IEnergyLut {
    // 编译期端点直接进入原状态构造，未经过 fractional-metal 通用分支。
    return kernel::opaque_state_impl<continuous>(
        energy, material.base_roughness.xyz, material.base_roughness.w,
        material.surface.x / incident.x,
        capability == CAP_OPAQUE_SUBSURFACE ? material.surface.z : 0.0, capability == CAP_CONDUCTOR,
        saturate(material.base_roughness.xyz), float3(1.0), wi);
}

internal func specialized_transmission<let capability : uint, EnergyLut>(
    energy: EnergyLut, material: Material, wi: float3, incident: float4, transmitted: float4)
    ->kernel::TransmissionState
    where EnergyLut : IEnergyLut {
    return kernel::transmission_state(
        energy, material.base_roughness.w, specialized_ior<capability>(incident, transmitted),
        specialized_thin<capability>(), transmitted.yzw * material.surface.w, wi);
}

public func specialized_opaque_evaluate<let capability : uint, let continuous : bool>(
    wi: float3, wo: float3, state: kernel::OpaqueState)
    ->kernel::LocalEvaluation {
    if (capability == CAP_CONDUCTOR)
        return kernel::conductor_evaluate_impl<continuous>(wi, wo, state, kernel::LOBE_ALL);
    return kernel::opaque_layer_evaluate_impl<continuous>(wi, wo, state);
}

public func specialized_opaque_sample<let capability : uint, let continuous : bool>(
    wi: float3, u: float3, state: kernel::OpaqueState)
    ->kernel::Proposal {
    if (capability == CAP_CONDUCTOR)
        return kernel::conductor_sample_impl<continuous>(wi, u, state, kernel::LOBE_ALL);
    var sampled : kernel::Proposal =
                      kernel::opaque_layer_sample_impl<continuous>(wi, u, state, false);
    if (kernel::has_sample(sampled) && (continuous || !kernel::is_delta(sampled.response.flags))) {
        let evaluated
            : kernel::LocalEvaluation =
                  specialized_opaque_evaluate<capability, continuous>(wi, sampled.wo, state);
        sampled.response.value = evaluated.response.value;
        sampled.pdf = evaluated.pdf;
    }
    return sampled;
}

internal func specialized_delta<let capability : uint>(material: Material, wi: float3, u: float3,
                                                       incident: float4, transmitted: float4)
    ->kernel::Proposal {
    if (capability == CAP_DELTA_CONDUCTOR) {
        var state : kernel::OpaqueDiscreteState = {};
        state.conductor_fresnel.f0 = saturate(material.base_roughness.xyz);
        state.conductor_fresnel.f82 = float3(1.0);
        state.conductor_fresnel.f90 = float3(1.0);
        state.conductor_fresnel.weight = 1.0;
        state.conductor_fresnel.energy_f0 = saturate(material.base_roughness.xyz);
        return kernel::conductor_discrete_sample(wi, state, kernel::LOBE_ALL);
    }
    var fresnel : kernel::SpecularFresnel;
    fresnel.ior = specialized_ior<capability>(incident, transmitted);
    fresnel.energy_ior = kernel::ior_adjustment(fresnel.ior, 1.0);
    fresnel.color = float3(1.0);
    if (capability == CAP_DELTA_THIN_DIELECTRIC)
        return kernel::thin_wall_delta_sample(
            wi, u.z, fresnel, float3(1.0), transmitted.yzw * material.surface.w, kernel::LOBE_ALL);
    return kernel::refractive_delta_sample(wi, u.z, fresnel, kernel::LOBE_ALL);
}

/// 转换窄核输出；UnitEta／KnownEvents 是调用方对成功 proposal 的额外证明，零旗标表示未知。
public func specialized_result<let capability : uint, let continuous : bool,
                               let unit_eta : bool = false, let known_events : uint = 0>(
    sampled: kernel::Proposal, frame: kernel::Frame, view: float3, incident: float4,
    transmitted: float4)
    ->Sample {
    var result : Sample = {};
    result.relative_eta = 1.0;
    result.medium = incident;
    if (!kernel::has_sample(sampled))
        return result;
    var checked : kernel::SurfaceSample = kernel::invalid_surface_sample();
    checked.direction = kernel::to_world(frame, sampled.wo);
    if (!continuous && (sampled.response.flags & kernel::LOBE_DELTA_REFLECTION) != 0u)
        checked.direction = normalize(reflect(-view, frame.normal));
    checked.response = sampled.response.value;
    checked.pdf = sampled.pdf;
    checked.relative_eta = sampled.eta;
    if (known_events != 0u)
        checked.event_flags = known_events;
    else if (continuous) {
        if (capability == CAP_SOLID_DIELECTRIC)
            checked.event_flags =
                kernel::EVENT_GLOSSY | (sampled.response.flags == kernel::LOBE_SPECULAR_TRANSMISSION
                                            ? kernel::EVENT_TRANSMISSION
                                            : kernel::EVENT_REFLECTION);
        else if (capability == CAP_CONDUCTOR)
            checked.event_flags = kernel::EVENT_GLOSSY | kernel::EVENT_REFLECTION;
        else
            checked.event_flags = kernel::to_bsdf_continuous_event_flags(sampled.response.flags);
    } else
        checked.event_flags = kernel::to_bsdf_event_flags(sampled.response.flags);
    if (continuous)
        checked = kernel::sanitize_surface_sample_with_events < unit_eta ||
                  (capability != CAP_SOLID_DIELECTRIC && capability != CAP_DELTA_SOLID_DIELECTRIC) >
                      (checked);
    else
        checked = kernel::sanitize_surface_sample < unit_eta ||
                  (capability != CAP_SOLID_DIELECTRIC && capability != CAP_DELTA_SOLID_DIELECTRIC) >
                      (checked);
    result.direction = checked.direction;
    result.response = checked.response;
    result.pdf = checked.pdf;
    result.relative_eta = checked.relative_eta;
    result.events = checked.event_flags;
    if (specialized_dielectric<capability>() && !specialized_thin<capability>() &&
        (result.events & TRANSMISSION) != 0u)
        result.medium = transmitted;
    return result;
}

/// Nonzero=true 保证 response flags 非零；数值 sanitize 仍保留。
public func specialized_evaluation<let nonzero : bool = false>(response: kernel::Response,
                                                               pdf: float)
    ->Evaluation {
    var checked : kernel::SurfaceEvaluation = kernel::invalid_surface_evaluation();
    if (nonzero) {
        checked.response = response.value;
        checked.pdf = pdf;
    } else if (response.flags != kernel::LOBE_NONE) {
        checked.response = response.value;
        checked.pdf = pdf;
    }
    checked = kernel::sanitize_surface_evaluation(checked);
    return { checked.response, checked.pdf };
}

internal func specialized_evaluate_prepared<let capability : uint>(wi: float3, wo: float3,
                                                                   state: kernel::OpaqueState)
    ->Evaluation {
    if (wo.z < 0.0 && (capability == CAP_OPAQUE_DIELECTRIC || capability == CAP_CONDUCTOR))
        return { float3(0.0), 0.0 };
    let evaluated : kernel::LocalEvaluation =
                        specialized_opaque_evaluate<capability, false>(wi, wo, state);
    return specialized_evaluation(evaluated.response, evaluated.pdf);
}

internal func specialized_evaluate_prepared<let capability : uint>(wi: float3, wo: float3,
                                                                   state: kernel::TransmissionState)
    ->Evaluation {
    let evaluated : kernel::TransmissionEvaluation =
                        kernel::transmission_evaluate(wi, wo, state, kernel::LOBE_ALL);
    return specialized_evaluation(evaluated.response, evaluated.pdf);
}

internal func specialized_sample_prepared<let capability : uint>(wi: float3, u: float3,
                                                                 state: kernel::OpaqueState)
    ->kernel::Proposal {
    return specialized_opaque_sample<capability, false>(wi, u, state);
}

internal func specialized_sample_prepared<let capability : uint>(wi: float3, u: float3,
                                                                 state: kernel::TransmissionState)
    ->kernel::Proposal {
    return kernel::transmission_sample_raw(wi, u, kernel::transmission_closure_state(state),
                                           kernel::LOBE_ALL);
}

/// 编译期类别的连续支持域；纯 delta 恒 false，其他类别保持保守拒绝语义。
/// capability 必须是本模块八个 CAP_* 常量之一，未知值导致编译错误；Material 须匹配该类别。
public func may_evaluate_specialized<let capability : uint>(material: Material, normal: float3,
                                                            view: float3, direction: float3,
                                                            incident: float4, transmitted: float4)
    ->bool {
    require_capability<capability>();
    if (capability >= CAP_DELTA_CONDUCTOR)
        return false;
    if (capability == CAP_OPAQUE_SUBSURFACE || specialized_dielectric<capability>())
        return true;
    let frame : kernel::Frame = specialized_frame<capability>(normal, view, incident, transmitted);
    return !(dot(frame.normal, direction) < 0.0);
}

/// 与 evaluate 相同的连续响应／marginal PDF，按编译期类别裁剪其他闭包。
/// 纯 delta 恒返回零且不消费 energy；非 delta 类别仍保留合法的光滑／IOR 相等边界。
/// 例如 evaluate_specialized<CAP_SOLID_DIELECTRIC>(energy, material, ...)，EnergyLut 由实参推断。
public func evaluate_specialized<let capability : uint, EnergyLut>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, direction: float3,
    incident: float4, transmitted: float4)
    ->Evaluation
    where EnergyLut : IEnergyLut {
    require_capability<capability>();
    if (capability >= CAP_DELTA_CONDUCTOR)
        return { float3(0.0), 0.0 };
    let frame : kernel::Frame = specialized_frame<capability>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let wo : float3 = kernel::to_local(frame, direction);
    if (wo.z < 0.0 && (capability == CAP_OPAQUE_DIELECTRIC || capability == CAP_CONDUCTOR))
        return { float3(0.0), 0.0 };
    if (specialized_dielectric<capability>()) {
        let state
            : kernel::TransmissionState =
                  specialized_transmission<capability>(energy, material, wi, incident, transmitted);
        return specialized_evaluate_prepared<capability>(wi, wo, state);
    }
    return specialized_evaluate_prepared<capability>(
        wi, wo, specialized_opaque<capability, false>(energy, material, wi, incident));
}

/// 已分类 opaque 的连续响应与完整 marginal PDF；capability 仅接受0、1、2。
/// 材料须满足对应 CAP_OPAQUE_DIELECTRIC／CAP_OPAQUE_SUBSURFACE／CAP_CONDUCTOR，
/// 调用方另外保证 float32 roughness²>=1e-4；IOR 相等仍合法，不检查或回退光滑边界。
/// 保留 diffuse／薄层 SSS、Fresnel、能量补偿和无效方向；通用入口仍支持光滑材料。
public func evaluate_opaque_continuous<let capability : uint, EnergyLut>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, direction: float3,
    incident: float4, transmitted: float4)
    ->Evaluation
    where EnergyLut : IEnergyLut {
    static_assert(capability <= CAP_CONDUCTOR, "continuous opaque capability must be 0, 1 or 2");
    let frame : kernel::Frame = specialized_frame<capability>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let wo : float3 = kernel::to_local(frame, direction);
    if (wo.z < 0.0 && capability != CAP_OPAQUE_SUBSURFACE)
        return { float3(0.0), 0.0 };
    let state : kernel::OpaqueState =
                    specialized_opaque<capability, true>(energy, material, wi, incident);
    let evaluated : kernel::LocalEvaluation =
                        specialized_opaque_evaluate<capability, true>(wi, wo, state);
    return specialized_evaluation(evaluated.response, evaluated.pdf);
}

/// evaluate_opaque_continuous 同一前置条件下的采样；保留原方向／随机映射和完整 marginal PDF。
/// diffuse、SSS 透射及 glossy 标签分别保留，不产生 delta；无效 proposal 与 sanitize 契约不变。
public func sample_opaque_continuous<let capability : uint, EnergyLut>(energy: EnergyLut,
                                                                       material: Material,
                                                                       normal: float3, view: float3,
                                                                       u: float3, incident: float4,
                                                                       transmitted: float4)
    ->Sample
    where EnergyLut : IEnergyLut {
    static_assert(capability <= CAP_CONDUCTOR, "continuous opaque capability must be 0, 1 or 2");
    let frame : kernel::Frame = specialized_frame<capability>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let state : kernel::OpaqueState =
                    specialized_opaque<capability, true>(energy, material, wi, incident);
    let sampled : kernel::Proposal = specialized_opaque_sample<capability, true>(wi, u, state);
    return specialized_result<capability, true>(sampled, frame, view, incident, transmitted);
}

/// 连续 solid 的完整响应／marginal PDF；Material 必须满足 CAP_SOLID_DIELECTRIC。
/// 调用方另外保证 float32 roughness²>=1e-4，且 specialized_ior 的实际除法结果不等于1；
/// 只比较输入 IOR 不相等不足以代替该保证。不检查、修复或回退光滑／匹配边界。
/// 保留原能量补偿、局部无效方向及数值契约；通用 evaluate_specialized 不受此限制。
public func evaluate_solid_continuous<EnergyLut>(energy: EnergyLut, material: Material,
                                                 normal: float3, view: float3, direction: float3,
                                                 incident: float4, transmitted: float4)
    ->Evaluation
    where EnergyLut : IEnergyLut {
    let frame : kernel::Frame =
                    specialized_frame<CAP_SOLID_DIELECTRIC>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let wo : float3 = kernel::to_local(frame, direction);
    let state : kernel::TransmissionState = specialized_transmission<CAP_SOLID_DIELECTRIC>(
                    energy, material, wi, incident, transmitted);
    let evaluated
        : kernel::TransmissionEvaluation =
              kernel::transmission_evaluate_solid_continuous(wi, wo, state, kernel::LOBE_ALL);
    return specialized_evaluation(evaluated.response, evaluated.pdf);
}

/// evaluate_solid_continuous 同一前置条件下的连续 solid 采样；不产生离散事件。
/// 方向、随机映射、完整 PDF、微面 TIR、能量补偿、eta／medium 及无效 proposal 与
/// sample_specialized<CAP_SOLID_DIELECTRIC> 相同；沿路径段 Beer 仍由调用方消费。
public func sample_solid_continuous<EnergyLut>(energy: EnergyLut, material: Material,
                                               normal: float3, view: float3, u: float3,
                                               incident: float4, transmitted: float4)
    ->Sample
    where EnergyLut : IEnergyLut {
    let frame : kernel::Frame =
                    specialized_frame<CAP_SOLID_DIELECTRIC>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let state : kernel::TransmissionState = specialized_transmission<CAP_SOLID_DIELECTRIC>(
                    energy, material, wi, incident, transmitted);
    let sampled : kernel::Proposal = kernel::transmission_sample_raw_solid_continuous(
                      wi, u, kernel::transmission_closure_state(state), kernel::LOBE_ALL);
    return specialized_result<CAP_SOLID_DIELECTRIC, true>(sampled, frame, view, incident,
                                                          transmitted);
}

/// 纯 delta 类别的窄采样入口；不接收 Energy 或准备连续闭包状态。
/// capability 仅接受 CAP_DELTA_CONDUCTOR、CAP_DELTA_SOLID_DIELECTRIC、CAP_DELTA_THIN_DIELECTRIC，
/// 调用方保证整个材料闭包满足对应类别；非 delta 类别编译失败，不作运行时分类或回退。
/// u 属于 [0,1)，对应 sample_specialized 的 u.z；conductor 不消费该随机数。
/// 其余输入、概率质量、事件、方向、eta、medium 和无效 proposal 语义与 sample_specialized 相同。
public func sample_delta_specialized<let capability : uint>(material: Material, normal: float3,
                                                            view: float3, u: float,
                                                            incident: float4, transmitted: float4)
    ->Sample {
    static_assert(capability >= CAP_DELTA_CONDUCTOR && capability <= CAP_DELTA_THIN_DIELECTRIC,
                  "sample_delta_specialized requires a delta capability");
    let frame : kernel::Frame = specialized_frame<capability>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    return specialized_result<capability, false>(
        specialized_delta<capability>(material, wi, float3(0.0, 0.0, u), incident, transmitted),
        frame, view, incident, transmitted);
}

/// 与 sample 相同的随机映射、完整 marginal PDF、event、eta 与 medium 转移。
/// 类别是编译期参数；纯 delta 直接调用原 delta 内核，不构造连续能量状态或读取能量纹理。
/// 随机数、局部无效 proposal 归零及沿路径段 Beer 的责任与 api 完全相同。
public func sample_specialized<let capability : uint, EnergyLut>(energy: EnergyLut,
                                                                 material: Material, normal: float3,
                                                                 view: float3, u: float3,
                                                                 incident: float4,
                                                                 transmitted: float4)
    ->Sample
    where EnergyLut : IEnergyLut {
    require_capability<capability>();
    let frame : kernel::Frame = specialized_frame<capability>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    var sampled : kernel::Proposal;
    if (capability >= CAP_DELTA_CONDUCTOR)
        sampled = specialized_delta<capability>(material, wi, u, incident, transmitted);
    else if (specialized_dielectric<capability>()) {
        let state
            : kernel::TransmissionState =
                  specialized_transmission<capability>(energy, material, wi, incident, transmitted);
        sampled = specialized_sample_prepared<capability>(wi, u, state);
    } else
        sampled = specialized_sample_prepared<capability>(
            wi, u, specialized_opaque<capability, false>(energy, material, wi, incident));
    return specialized_result<capability, false>(sampled, frame, view, incident, transmitted);
}

/// 同一命中只构造一次 ONB、local wi 与所选闭包状态，不将完整 prepared state 交给调用方。
/// evaluation_mask 的低两位分别启用 first／second；未启用的方向无需有效，高位不参与评估。
/// continuation=false 时不采样，scattered 保持 events=0、relative_eta=1 和 incident medium。
/// 纯 delta 忽略两个连续方向，直接沿用原 delta 核且不消费 energy；状态只在本次调用内有效。
public func shade_specialized<let capability : uint, EnergyLut>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    transmitted: float4)
    ->Batch
    where EnergyLut : IEnergyLut {
    require_capability<capability>();
    var result : Batch = {};
    result.scattered.relative_eta = 1.0;
    result.scattered.medium = incident;
    if (capability >= CAP_DELTA_CONDUCTOR) {
        if (continuation) {
            let frame : kernel::Frame =
                            specialized_frame<capability>(normal, view, incident, transmitted);
            let wi : float3 = kernel::to_local(frame, view);
            result.scattered = specialized_result<capability, false>(
                specialized_delta<capability>(material, wi, u, incident, transmitted), frame, view,
                incident, transmitted);
        }
        return result;
    }
    if ((evaluation_mask & 3u) == 0u && !continuation)
        return result;
    let frame : kernel::Frame = specialized_frame<capability>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    if (specialized_dielectric<capability>()) {
        let state
            : kernel::TransmissionState =
                  specialized_transmission<capability>(energy, material, wi, incident, transmitted);
        if ((evaluation_mask & 1u) != 0u)
            result.first = specialized_evaluate_prepared<capability>(
                wi, kernel::to_local(frame, first), state);
        if ((evaluation_mask & 2u) != 0u)
            result.second = specialized_evaluate_prepared<capability>(
                wi, kernel::to_local(frame, second), state);
        if (continuation)
            result.scattered = specialized_result<capability, false>(
                specialized_sample_prepared<capability>(wi, u, state), frame, view, incident,
                transmitted);
    } else {
        let state : kernel::OpaqueState =
                        specialized_opaque<capability, false>(energy, material, wi, incident);
        if ((evaluation_mask & 1u) != 0u)
            result.first = specialized_evaluate_prepared<capability>(
                wi, kernel::to_local(frame, first), state);
        if ((evaluation_mask & 2u) != 0u)
            result.second = specialized_evaluate_prepared<capability>(
                wi, kernel::to_local(frame, second), state);
        if (continuation)
            result.scattered = specialized_result<capability, false>(
                specialized_sample_prepared<capability>(wi, u, state), frame, view, incident,
                transmitted);
    }
    return result;
}
}
