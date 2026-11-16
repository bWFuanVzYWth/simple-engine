// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
// 内核与能量表来源见 crates/openpbr/assets/NOTICE.md。

/// OpenPBR 能力子集，不支持 coat/fuzz/thin-film/dispersion 或厚体 SSS。
/// 不绑定场景或设备；输入为同一线性工作空间、有限单位法线／方向，随机数属于 [0,1)^3。
/// 材料／介质须遵循 Rust openpbr crate 的有效范围；调用方维护相机初值、边界两侧和几何支持。
/// 光学深度、eta 运算和响应的中间量须可由 f32 表示；不支持任意有限极端输入。
module api;
__exported import bsdf.openpbr.contract.surface;

import bsdf.openpbr.pt;
import bsdf.openpbr.contract.material;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.contract.flags;
import bsdf.openpbr.math.frame;
import bsdf.openpbr.math.fresnel;
import bsdf.openpbr.math.spectrum;
import bsdf.openpbr.math.microfacet_energy;
import bsdf.openpbr.dielectric.contract;
import bsdf.openpbr.dielectric.state;
import bsdf.openpbr.dielectric.evaluate;
import bsdf.openpbr.dielectric.refract;
import bsdf.openpbr.opaque.state;
import bsdf.openpbr.opaque.evaluate;
import bsdf.openpbr.service.contract.sample;
import bsdf.openpbr.service.support.conversion;
import bsdf.openpbr.service.support.sanitization;

namespace openpbr
{
internal func dielectric(material: Material)->bool
{
    return (material.control.x & 1u) != 0;
}
internal func thin(material: Material)->bool
{
    return (material.control.x & 2u) != 0;
}

internal func interface_ior(material: Material, incident: float4, transmitted: float4)->float
{
    let exiting : bool = !thin(material) && incident.x > transmitted.x;
    return (exiting ? incident.x : transmitted.x) / (exiting ? transmitted.x : incident.x);
}

internal func material_frame(material: Material, normal: float3, view: float3, incident: float4,
                             transmitted: float4)
    ->kernel::Frame
{
    var normal = normal;
    normal = dot(normal, view) < 0.0 ? -normal : normal;
    var frame : kernel::Frame = kernel::frame_from_normal(normal);
    if (dielectric(material))
    {
        let exiting : bool = !thin(material) && incident.x > transmitted.x;
        if ((dot(frame.normal, view) >= 0.0) == exiting)
        {
            frame.normal = -frame.normal;
            frame.bitangent = -frame.bitangent;
        }
    }
    return frame;
}

internal func opaque_state<EnergyLut>(energy: EnergyLut, material: Material, wi: float3,
                                      incident: float4)
    ->kernel::OpaqueState
    where EnergyLut : IEnergyLut
{
    return kernel::metallic_state(energy, material.base_roughness.xyz, material.base_roughness.w,
                                  material.surface.x / incident.x, material.surface.z,
                                  material.surface.y, wi);
}

internal func reflection_only(material: Material)->bool
{
    return !dielectric(material) && (material.surface.y == 1.0 || material.surface.z == 0.0);
}

/// 连续方向的保守支持域快判，不读取能量表或准备闭包。
/// false 保证 evaluate 的 response／pdf 为零；true 不保证非零，不替代完整评估。
/// 输入及世界空间约定同 evaluate；保留 tangent、dielectric 和薄层 SSS 的反侧方向。
public func may_evaluate(material: Material, normal: float3, view: float3, direction: float3,
                         incident: float4, transmitted: float4)
    ->bool
{
    if (!reflection_only(material))
        return true;
    let frame : kernel::Frame = material_frame(material, normal, view, incident, transmitted);
    return !(dot(frame.normal, direction) < 0.0);
}

/// 整个闭包是否仅有离散事件；光滑 dielectric／conductor 成立，含漫反射的光滑 opaque 不成立。
/// continuous NEE 可跳过此闭包，但不能丢掉它的延续和发光命中。
public func is_discrete_only(material: Material, incident: float4, transmitted: float4)->bool
{
    let alpha : float = material.base_roughness.w * material.base_roughness.w;
    if (dielectric(material))
    {
        let ior : float = interface_ior(material, incident, transmitted);
        if (thin(material))
            return clamp(alpha * kernel::thin_dielectric_roughness_scaler2(ior), 0.0, 1.0) < 1.0e-4;
        return alpha < 1.0e-4 || ior == 1.0;
    }
    return material.surface.y == 1.0 && alpha < 1.0e-4;
}

/// 连续方向的完整闭包响应与边缘 PDF；incident/transmitted 为显式的边界两侧介质。
/// normal、view、direction 同世界空间，方向均离开顶点；normal 可未 face-forward。
/// delta 质量不属于连续评估；无效结果归零，不增加任意输入修复。
public func evaluate<EnergyLut>(energy: EnergyLut, material: Material, normal: float3, view: float3,
                                direction: float3, incident: float4, transmitted: float4)
    ->Evaluation
    where EnergyLut : IEnergyLut
{
    let frame : kernel::Frame = material_frame(material, normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let wo : float3 = kernel::to_local(frame, direction);
    if (wo.z < 0.0 && reflection_only(material))
        return { float3(0.0), 0.0 };
    var response : kernel::Response;
    var pdf : float;
    if (dielectric(material))
    {
        let state
            : kernel::TransmissionState = kernel::transmission_state(
                  energy, material.base_roughness.w, interface_ior(material, incident, transmitted),
                  thin(material), transmitted.yzw * material.surface.w, wi);
        let evaluated : kernel::TransmissionEvaluation =
                            kernel::transmission_evaluate(wi, wo, state, kernel::LOBE_ALL);
        response = evaluated.response;
        pdf = evaluated.pdf;
    }
    else
    {
        let evaluated
            : kernel::LocalEvaluation =
                  kernel::opaque_evaluate(wi, wo, opaque_state(energy, material, wi, incident));
        response = evaluated.response;
        pdf = evaluated.pdf;
    }
    var checked : kernel::SurfaceEvaluation = kernel::invalid_surface_evaluation();
    if (response.flags != kernel::LOBE_NONE)
    {
        checked.response = response.value;
        checked.pdf = pdf;
    }
    checked = kernel::sanitize_surface_evaluation(checked);
    return { checked.response, checked.pdf };
}

/// 按分量选择、方向映射及完整边缘 PDF 采样；u 的分量独立，不能复用 RR 随机数。
/// solid transmission 更新 medium；薄片光学深度为 transmitted.sigma * 显式 thickness。
/// 采样只处理界面，沿路径段的 Beer 吸收由调用方另以当前 medium 与距离计算。
public func sample<EnergyLut>(energy: EnergyLut, material: Material, normal: float3, view: float3,
                              u: float3, incident: float4, transmitted: float4)
    ->Sample
    where EnergyLut : IEnergyLut
{
    var result : Sample = {};
    result.relative_eta = 1.0;
    result.medium = incident;
    let frame : kernel::Frame = material_frame(material, normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    var sampled : kernel::Proposal;
    if (dielectric(material))
    {
        let state
            : kernel::TransmissionState = kernel::transmission_state(
                  energy, material.base_roughness.w, interface_ior(material, incident, transmitted),
                  thin(material), transmitted.yzw * material.surface.w, wi);
        sampled = kernel::transmission_sample_raw(wi, u, kernel::transmission_closure_state(state),
                                                  kernel::LOBE_ALL);
    }
    else
        sampled =
            kernel::sample_opaque_marginal(wi, u, opaque_state(energy, material, wi, incident));
    if (!kernel::has_sample(sampled))
        return result;
    var checked : kernel::SurfaceSample = kernel::invalid_surface_sample();
    checked.direction = kernel::to_world(frame, sampled.wo);
    if ((sampled.response.flags & kernel::LOBE_DELTA_REFLECTION) != 0u)
        checked.direction = normalize(reflect(-view, frame.normal));
    checked.response = sampled.response.value;
    checked.pdf = sampled.pdf;
    checked.relative_eta = sampled.eta;
    checked.event_flags = kernel::to_bsdf_event_flags(sampled.response.flags);
    checked = kernel::sanitize_surface_sample(checked);
    result.direction = checked.direction;
    result.response = checked.response;
    result.pdf = checked.pdf;
    result.relative_eta = checked.relative_eta;
    result.events = checked.event_flags;
    if (dielectric(material) && !thin(material) && (result.events & TRANSMISSION) != 0u)
        result.medium = transmitted;
    return result;
}
}
