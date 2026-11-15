// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
// 内核与能量表来源见 crates/openpbr/assets/NOTICE.md。

/// OpenPBR 能力子集，不支持 coat/fuzz/thin-film/dispersion 或厚体 SSS。
/// 不绑定场景或设备；输入为同一线性工作空间、有限单位法线／方向，随机数属于 [0,1)^3。
/// 材料／介质须遵循 Rust openpbr crate 的有效范围；调用方维护相机初值、边界两侧和几何支持。
/// 光学深度、eta 运算和响应的中间量须可由 f32 表示；不支持任意有限极端输入。
module api;

import "bsdf/openpbr/pt.slang";
import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/math/frame.slang";
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/dielectric/state.slang";
import "bsdf/openpbr/dielectric/evaluate.slang";
import "bsdf/openpbr/dielectric/refract.slang";
import "bsdf/openpbr/opaque/state.slang";
import "bsdf/openpbr/opaque/evaluate.slang";
import "bsdf/openpbr/service/contract/sample.slang";
import "bsdf/openpbr/service/support/conversion.slang";
import "bsdf/openpbr/service/support/sanitization.slang";

namespace openpbr
{
/// 48B 材料，CPU 内容见 Rust Material::to_words；命中贴图可替换有效 baseRGB。
public struct Material
{
    /// RGB 与线性 roughness，均属于 [0,1]。
    public var base_roughness : float4;
    /// opaque IOR、metalness、薄层 SSS weight、thin dielectric thickness。
    public var surface : float4;
    /// bit0=dielectric，bit1=thin dielectric；其余内容为零。
    public var control : uint4;
};

/// 原作者能量表的真实 Rgba16Float 3D 纹理及 normalized linear clamp LOD0 sampler。
/// 纹理为 44x32x159，不是颜色；资源所有权与绑定由调用方维持。
public struct Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    public var texture : Texture3D<float4>;
    public var sampler : SamplerState;
    public func sampleLevel(uvw: float3)->float4
    {
        return texture.SampleLevel(sampler, uvw, 0.0);
    }
};

/// 反射事件；与 TRANSMISSION 互斥。
public static const let REFLECTION = 1u;
/// 透射事件；solid dielectric 更新 medium，thin 和薄层 SSS 不更新。
public static const let TRANSMISSION = 2u;
/// 漫反射／薄层 SSS 分量。
public static const let DIFFUSE = 4u;
/// 有限宽度微表面分量。
public static const let GLOSSY = 8u;
/// 离散 delta 事件，pdf 是概率质量；不能与立体角密度直接用于 MIS。
public static const let DISCRETE = 16u;

/// 完整闭包的连续方向响应与边缘 proposal PDF；response 已包含 |cos|，不是裸 BRDF。
public struct Evaluation
{
    public var response : float3;
    public var pdf : float;
};

/// 拒绝 proposal 时 events=0，无可消费的散射；其余内容规范为零／incident medium。
/// 有效 proposal 以 beta*=response/pdf 更新；不要再次乘 cos 或额外重复乘 eta²。
public struct Sample
{
    public var direction : float3;
    public var response : float3;
    public var pdf : float;
    /// eta_t/eta_i，供 eta-aware RR；响应已包含参考实现的 radiance eta 补偿。
    public var relative_eta : float;
    public var events : uint;
    /// 后续段的 float4(IOR,RGB extinction)，thin dielectric 透射仍保留入射介质。
    public var medium : float4;
};

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

internal func frame(material: Material, normal: float3, view: float3, incident: float4,
                    transmitted: float4)
    ->PrimeFullOpenPbrOnb
{
    var normal = normal;
    normal = dot(normal, view) < 0.0 ? -normal : normal;
    var onb : PrimeFullOpenPbrOnb = primeFullOpenPbrOnbFromNormal(normal);
    if (dielectric(material))
    {
        let exiting : bool = !thin(material) && incident.x > transmitted.x;
        if ((dot(onb.normal, view) >= 0.0) == exiting)
        {
            onb.normal = -onb.normal;
            onb.bitangent = -onb.bitangent;
        }
    }
    return onb;
}

internal func opaque_state<Provider>(energy: Provider, material: Material, wi: float3,
                                     incident: float4)
    ->PrimeFullCompactOpaqueState
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    return primeFullPtMetallicState(energy, material.base_roughness.xyz, material.base_roughness.w,
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
    let onb : PrimeFullOpenPbrOnb = frame(material, normal, view, incident, transmitted);
    return !(dot(onb.normal, direction) < 0.0);
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
            return clamp(alpha * primeFullOpenPbrThinDielectricRoughnessScaler2(ior), 0.0, 1.0) <
                   1.0e-4;
        return alpha < 1.0e-4 || ior == 1.0;
    }
    return material.surface.y == 1.0 && alpha < 1.0e-4;
}

/// 连续方向的完整闭包响应与边缘 PDF；incident/transmitted 为显式的边界两侧介质。
/// normal、view、direction 同世界空间，方向均离开顶点；normal 可未 face-forward。
/// delta 质量不属于连续评估；无效结果归零，不增加任意输入修复。
public func evaluate<Provider>(energy: Provider, material: Material, normal: float3, view: float3,
                               direction: float3, incident: float4, transmitted: float4)
    ->Evaluation
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    let onb : PrimeFullOpenPbrOnb = frame(material, normal, view, incident, transmitted);
    let wi : float3 = primeFullOpenPbrOnbToLocal(onb, view);
    let wo : float3 = primeFullOpenPbrOnbToLocal(onb, direction);
    if (wo.z < 0.0 && reflection_only(material))
        return { float3(0.0), 0.0 };
    var response : PrimeFullOpenPbrThroughput;
    var pdf : float;
    if (dielectric(material))
    {
        let state
            : PrimeFullCompactTransmissionState = primeFullPtTransmissionState(
                  energy, material.base_roughness.w, interface_ior(material, incident, transmitted),
                  thin(material), transmitted.yzw * material.surface.w, wi);
        let evaluated
            : PrimeFullCompactTransmissionEvaluation =
                  primeFullCompactTransmissionEvaluate(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
        response = evaluated.throughput;
        pdf = evaluated.pdf;
    }
    else
    {
        let evaluated : PrimeFullCompactOpaqueEvaluation = primeFullCompactOpaqueEvaluate(
                            wi, wo, opaque_state(energy, material, wi, incident));
        response = evaluated.throughput;
        pdf = evaluated.pdf;
    }
    var checked : BsdfEvaluation = primeInvalidBsdfEvaluation();
    if (response.flags != PRIME_FULL_OPENPBR_FLAG_NONE)
    {
        checked.response = response.value;
        checked.pdf = pdf;
    }
    checked = primeSanitizeBsdfEvaluation(checked);
    return { checked.response, checked.pdf };
}

/// 按分量选择、方向映射及完整边缘 PDF 采样；u 的分量独立，不能复用 RR 随机数。
/// solid transmission 更新 medium；薄片光学深度为 transmitted.sigma * 显式 thickness。
/// 采样只处理界面，沿路径段的 Beer 吸收由调用方另以当前 medium 与距离计算。
public func sample<Provider>(energy: Provider, material: Material, normal: float3, view: float3,
                             u: float3, incident: float4, transmitted: float4)
    ->Sample
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    var result : Sample = {};
    result.relative_eta = 1.0;
    result.medium = incident;
    let onb : PrimeFullOpenPbrOnb = frame(material, normal, view, incident, transmitted);
    let wi : float3 = primeFullOpenPbrOnbToLocal(onb, view);
    var sampled : PrimeFullOpenPbrSample;
    if (dielectric(material))
    {
        let state
            : PrimeFullCompactTransmissionState = primeFullPtTransmissionState(
                  energy, material.base_roughness.w, interface_ior(material, incident, transmitted),
                  thin(material), transmitted.yzw * material.surface.w, wi);
        sampled = primeFullCompactTransmissionSampleRaw(
            wi, u, primeFullCompactTransmissionClosureState(state), PRIME_FULL_OPENPBR_FLAG_ALL);
    }
    else
        sampled = primeFullPtOpaqueSample(wi, u, opaque_state(energy, material, wi, incident));
    if (!primeFullOpenPbrHasSample(sampled))
        return result;
    var checked : BsdfSample = primeInvalidBsdfSample();
    checked.direction = primeFullOpenPbrOnbToWorld(onb, sampled.wo);
    if ((sampled.throughput.flags & PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION) != 0u)
        checked.direction = normalize(reflect(-view, onb.normal));
    checked.response = sampled.throughput.value;
    checked.pdf = sampled.pdf;
    checked.relativeEta = sampled.eta;
    checked.eventFlags = primeFullOpenPbrToBsdfEventFlags(sampled.throughput.flags);
    checked = primeSanitizeBsdfSample(checked);
    result.direction = checked.direction;
    result.response = checked.response;
    result.pdf = checked.pdf;
    result.relative_eta = checked.relativeEta;
    result.events = checked.eventFlags;
    if (dielectric(material) && !thin(material) && (result.events & TRANSMISSION) != 0u)
        result.medium = transmitted;
    return result;
}
}
