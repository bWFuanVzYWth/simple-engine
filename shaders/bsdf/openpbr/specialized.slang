// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
// OpenPBR 支持子域的编译期入口；来源与许可见 crates/openpbr/assets/NOTICE.md。

/// 对已分类材料按编译期类别保留相应闭包，不读取应用 ID。
/// Material、方向和介质契约同 api；调用方在发布材料时保证类别匹配，不作运行时修复或回退。
/// 同一类别仍允许有效的 RGB、roughness、IOR 和 SSS 权重变化；不是预烘材质或完整 OpenPBR。
module specialized;

import bsdf.openpbr.api;
import "bsdf/openpbr/pt.slang";
import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/opaque/state.slang";
import "bsdf/openpbr/opaque/lobes.slang";
import "bsdf/openpbr/opaque/evaluate.slang";
import "bsdf/openpbr/opaque/discrete.slang";
import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/dielectric/state.slang";
import "bsdf/openpbr/dielectric/evaluate.slang";
import "bsdf/openpbr/dielectric/refract.slang";
import "bsdf/openpbr/math/frame.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/delta.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/service/contract/sample.slang";
import "bsdf/openpbr/service/support/conversion.slang";
import "bsdf/openpbr/service/support/sanitization.slang";

namespace openpbr
{
/// 同一命中的两个连续方向与一次散射；未请求的评估为零，未请求散射为规范的无事件结果。
public struct Batch
{
    /// evaluation_mask bit0 对应的连续响应／PDF，未请求时为零。
    public Evaluation first;
    /// evaluation_mask bit1 对应的连续响应／PDF，未请求时为零。
    public Evaluation second;
    /// 可选散射；不包含沿路径段的 Beer 吸收。
    public Sample scattered;
};

/// 非 dielectric、metalness=0、SSS=0；粗糙度仍可产生光滑 specular 与连续 diffuse 混合。
public static const uint CAP_OPAQUE_DIELECTRIC = 0u;
/// 非 dielectric、metalness=0；保留一般 0..1 薄层 SSS 权重及其反侧散射。
public static const uint CAP_OPAQUE_SUBSURFACE = 1u;
/// 非 dielectric、metalness=1；粗糙度可连续变化，SSS 与原 conductor 一样不参与响应。
public static const uint CAP_CONDUCTOR = 2u;
/// solid dielectric；仍保留 IOR 相等时的 delta 透射及粗糙度的光滑边界。
public static const uint CAP_SOLID_DIELECTRIC = 3u;
/// thin dielectric；厚度由 Material 提供，仍保留有效粗糙度及 IOR 范围。
public static const uint CAP_THIN_DIELECTRIC = 4u;
/// 光滑 conductor：roughness²<1e-4；无需能量表，不存在连续方向评估。
public static const uint CAP_DELTA_CONDUCTOR = 5u;
/// 纯 delta solid：roughness²<1e-4 或两侧 IOR 相等；保留 Fresnel/TIR、eta 与 medium 更新。
public static const uint CAP_DELTA_SOLID_DIELECTRIC = 6u;
/// 纯 delta thin：须满足 api.is_discrete_only 的缩放／clamp 阈值；保留薄片吸收，不更新 medium。
public static const uint CAP_DELTA_THIN_DIELECTRIC = 7u;

internal void require_capability<let Capability : uint>()
{
    static_assert(Capability <= CAP_DELTA_THIN_DIELECTRIC, "unknown OpenPBR capability");
}

internal bool specialized_dielectric<let Capability : uint>()
{
    return Capability == CAP_SOLID_DIELECTRIC || Capability == CAP_THIN_DIELECTRIC ||
           Capability == CAP_DELTA_SOLID_DIELECTRIC || Capability == CAP_DELTA_THIN_DIELECTRIC;
}

internal bool specialized_thin<let Capability : uint>()
{
    return Capability == CAP_THIN_DIELECTRIC || Capability == CAP_DELTA_THIN_DIELECTRIC;
}

public PrimeFullOpenPbrOnb specialized_frame<let Capability : uint>(float3 normal, float3 view,
                                                                    float4 incident,
                                                                    float4 transmitted)
{
    normal = dot(normal, view) < 0.0 ? -normal : normal;
    PrimeFullOpenPbrOnb onb = primeFullOpenPbrOnbFromNormal(normal);
    if (specialized_dielectric<Capability>())
    {
        bool exiting = !specialized_thin<Capability>() && incident.x > transmitted.x;
        if ((dot(onb.normal, view) >= 0.0) == exiting)
        {
            onb.normal = -onb.normal;
            onb.bitangent = -onb.bitangent;
        }
    }
    return onb;
}

internal float specialized_ior<let Capability : uint>(float4 incident, float4 transmitted)
{
    bool exiting = !specialized_thin<Capability>() && incident.x > transmitted.x;
    return (exiting ? incident.x : transmitted.x) / (exiting ? transmitted.x : incident.x);
}

public PrimeFullCompactOpaqueState specialized_opaque<let Capability : uint, let Continuous : bool,
                                                      Provider>(Provider energy, Material material,
                                                                float3 wi, float4 incident)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    // 编译期端点直接进入原状态构造，未经过 fractional-metal 通用分支。
    return primeFullPtOpaqueStateImpl<Continuous>(
        energy, material.base_roughness.xyz, material.base_roughness.w,
        material.surface.x / incident.x,
        Capability == CAP_OPAQUE_SUBSURFACE ? material.surface.z : 0.0, Capability == CAP_CONDUCTOR,
        saturate(material.base_roughness.xyz), float3(1.0), wi);
}

internal PrimeFullCompactTransmissionState
specialized_transmission<let Capability : uint, Provider>(Provider energy, Material material,
                                                          float3 wi, float4 incident,
                                                          float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    return primeFullPtTransmissionState(
        energy, material.base_roughness.w, specialized_ior<Capability>(incident, transmitted),
        specialized_thin<Capability>(), transmitted.yzw * material.surface.w, wi);
}

public PrimeFullCompactOpaqueEvaluation specialized_opaque_evaluate<
    let Capability : uint, let Continuous : bool>(float3 wi, float3 wo,
                                                  PrimeFullCompactOpaqueState state)
{
    if (Capability == CAP_CONDUCTOR)
        return primeFullCompactConductorEvaluateImpl<Continuous>(wi, wo, state,
                                                                 PRIME_FULL_OPENPBR_FLAG_ALL);
    return primeFullCompactOpaqueLayerEvaluateImpl<Continuous>(wi, wo, state);
}

public PrimeFullOpenPbrSample specialized_opaque_sample<
    let Capability : uint, let Continuous : bool>(float3 wi, float3 u,
                                                  PrimeFullCompactOpaqueState state)
{
    if (Capability == CAP_CONDUCTOR)
        return primeFullCompactConductorSampleImpl<Continuous>(wi, u, state,
                                                               PRIME_FULL_OPENPBR_FLAG_ALL);
    PrimeFullOpenPbrSample sampled =
        primeFullCompactOpaqueLayerSampleImpl<Continuous>(wi, u, state, false);
    if (primeFullOpenPbrHasSample(sampled) &&
        (Continuous || !primeFullOpenPbrIsDelta(sampled.throughput.flags)))
    {
        PrimeFullCompactOpaqueEvaluation evaluated =
            specialized_opaque_evaluate<Capability, Continuous>(wi, sampled.wo, state);
        sampled.throughput.value = evaluated.throughput.value;
        sampled.pdf = evaluated.pdf;
    }
    return sampled;
}

internal PrimeFullOpenPbrSample specialized_delta<let Capability : uint>(Material material,
                                                                         float3 wi, float3 u,
                                                                         float4 incident,
                                                                         float4 transmitted)
{
    if (Capability == CAP_DELTA_CONDUCTOR)
    {
        PrimeFullCompactOpaqueDiscreteState state = {};
        state.conductorFresnel.f0 = saturate(material.base_roughness.xyz);
        state.conductorFresnel.f82 = float3(1.0);
        state.conductorFresnel.f90 = float3(1.0);
        state.conductorFresnel.weight = 1.0;
        state.conductorFresnel.energyF0 = saturate(material.base_roughness.xyz);
        return primeFullCompactConductorDiscreteSample(wi, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    }
    PrimeFullOpenPbrSpecularFresnel fresnel;
    fresnel.ior = specialized_ior<Capability>(incident, transmitted);
    fresnel.energyIor = primeFullOpenPbrIorAdjustment(fresnel.ior, 1.0);
    fresnel.color = float3(1.0);
    if (Capability == CAP_DELTA_THIN_DIELECTRIC)
        return primeFullThinWallDeltaSample(wi, u.z, fresnel, float3(1.0),
                                            transmitted.yzw * material.surface.w,
                                            PRIME_FULL_OPENPBR_FLAG_ALL);
    return primeFullRefractiveDeltaSample(wi, u.z, fresnel, PRIME_FULL_OPENPBR_FLAG_ALL);
}

/// 转换窄核输出；UnitEta／KnownEvents 是调用方对成功 proposal 的额外证明，零旗标表示未知。
public Sample specialized_result<let Capability : uint, let Continuous : bool,
                                 let UnitEta : bool = false, let KnownEvents : uint = 0>(
    PrimeFullOpenPbrSample sampled, PrimeFullOpenPbrOnb onb, float3 view, float4 incident,
    float4 transmitted)
{
    Sample result = {};
    result.relative_eta = 1.0;
    result.medium = incident;
    if (!primeFullOpenPbrHasSample(sampled))
        return result;
    BsdfSample checked = primeInvalidBsdfSample();
    checked.direction = primeFullOpenPbrOnbToWorld(onb, sampled.wo);
    if (!Continuous && (sampled.throughput.flags & PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION) != 0u)
        checked.direction = normalize(reflect(-view, onb.normal));
    checked.response = sampled.throughput.value;
    checked.pdf = sampled.pdf;
    checked.relativeEta = sampled.eta;
    if (KnownEvents != 0u)
        checked.eventFlags = KnownEvents;
    else if (Continuous)
    {
        if (Capability == CAP_SOLID_DIELECTRIC)
            checked.eventFlags =
                PRIME_BSDF_EVENT_GLOSSY |
                (sampled.throughput.flags == PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION
                     ? PRIME_BSDF_EVENT_TRANSMISSION
                     : PRIME_BSDF_EVENT_REFLECTION);
        else if (Capability == CAP_CONDUCTOR)
            checked.eventFlags = PRIME_BSDF_EVENT_GLOSSY | PRIME_BSDF_EVENT_REFLECTION;
        else
            checked.eventFlags =
                primeFullOpenPbrToBsdfContinuousEventFlags(sampled.throughput.flags);
    }
    else
        checked.eventFlags = primeFullOpenPbrToBsdfEventFlags(sampled.throughput.flags);
    if (Continuous)
        checked = primeSanitizeBsdfSampleWithEvents < UnitEta ||
                  (Capability != CAP_SOLID_DIELECTRIC && Capability != CAP_DELTA_SOLID_DIELECTRIC) >
                      (checked);
    else
        checked = primeSanitizeBsdfSample < UnitEta ||
                  (Capability != CAP_SOLID_DIELECTRIC && Capability != CAP_DELTA_SOLID_DIELECTRIC) >
                      (checked);
    result.direction = checked.direction;
    result.response = checked.response;
    result.pdf = checked.pdf;
    result.relative_eta = checked.relativeEta;
    result.events = checked.eventFlags;
    if (specialized_dielectric<Capability>() && !specialized_thin<Capability>() &&
        (result.events & TRANSMISSION) != 0u)
        result.medium = transmitted;
    return result;
}

/// Nonzero=true 保证 throughput flags 非零；数值 sanitize 仍保留。
public Evaluation specialized_evaluation<let Nonzero : bool = false>(
    PrimeFullOpenPbrThroughput response, float pdf)
{
    BsdfEvaluation checked = primeInvalidBsdfEvaluation();
    if (Nonzero)
    {
        checked.response = response.value;
        checked.pdf = pdf;
    }
    else if (response.flags != PRIME_FULL_OPENPBR_FLAG_NONE)
    {
        checked.response = response.value;
        checked.pdf = pdf;
    }
    checked = primeSanitizeBsdfEvaluation(checked);
    return { checked.response, checked.pdf };
}

internal Evaluation specialized_evaluate_prepared<let Capability : uint>(
    float3 wi, float3 wo, PrimeFullCompactOpaqueState state)
{
    if (wo.z < 0.0 && (Capability == CAP_OPAQUE_DIELECTRIC || Capability == CAP_CONDUCTOR))
        return { float3(0.0), 0.0 };
    PrimeFullCompactOpaqueEvaluation evaluated =
        specialized_opaque_evaluate<Capability, false>(wi, wo, state);
    return specialized_evaluation(evaluated.throughput, evaluated.pdf);
}

internal Evaluation specialized_evaluate_prepared<let Capability : uint>(
    float3 wi, float3 wo, PrimeFullCompactTransmissionState state)
{
    PrimeFullCompactTransmissionEvaluation evaluated =
        primeFullCompactTransmissionEvaluate(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    return specialized_evaluation(evaluated.throughput, evaluated.pdf);
}

internal PrimeFullOpenPbrSample specialized_sample_prepared<let Capability : uint>(
    float3 wi, float3 u, PrimeFullCompactOpaqueState state)
{
    return specialized_opaque_sample<Capability, false>(wi, u, state);
}

internal PrimeFullOpenPbrSample specialized_sample_prepared<let Capability : uint>(
    float3 wi, float3 u, PrimeFullCompactTransmissionState state)
{
    return primeFullCompactTransmissionSampleRaw(
        wi, u, primeFullCompactTransmissionClosureState(state), PRIME_FULL_OPENPBR_FLAG_ALL);
}

/// 编译期类别的连续支持域；纯 delta 恒 false，其他类别保持保守拒绝语义。
/// Capability 必须是本模块八个 CAP_* 常量之一，未知值导致编译错误；Material 须匹配该类别。
public bool may_evaluate_specialized<let Capability : uint>(Material material, float3 normal,
                                                            float3 view, float3 direction,
                                                            float4 incident, float4 transmitted)
{
    require_capability<Capability>();
    if (Capability >= CAP_DELTA_CONDUCTOR)
        return false;
    if (Capability == CAP_OPAQUE_SUBSURFACE || specialized_dielectric<Capability>())
        return true;
    PrimeFullOpenPbrOnb onb = specialized_frame<Capability>(normal, view, incident, transmitted);
    return !(dot(onb.normal, direction) < 0.0);
}

/// 与 evaluate 相同的连续响应／marginal PDF，按编译期类别裁剪其他闭包。
/// 纯 delta 恒返回零且不消费 energy；非 delta 类别仍保留合法的光滑／IOR 相等边界。
/// 例如 evaluate_specialized<CAP_SOLID_DIELECTRIC>(energy, material, ...)，Provider 由实参推断。
public Evaluation evaluate_specialized<let Capability : uint, Provider>(
    Provider energy, Material material, float3 normal, float3 view, float3 direction,
    float4 incident, float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    require_capability<Capability>();
    if (Capability >= CAP_DELTA_CONDUCTOR)
        return { float3(0.0), 0.0 };
    PrimeFullOpenPbrOnb onb = specialized_frame<Capability>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    float3 wo = primeFullOpenPbrOnbToLocal(onb, direction);
    if (wo.z < 0.0 && (Capability == CAP_OPAQUE_DIELECTRIC || Capability == CAP_CONDUCTOR))
        return { float3(0.0), 0.0 };
    if (specialized_dielectric<Capability>())
    {
        PrimeFullCompactTransmissionState state =
            specialized_transmission<Capability>(energy, material, wi, incident, transmitted);
        return specialized_evaluate_prepared<Capability>(wi, wo, state);
    }
    return specialized_evaluate_prepared<Capability>(
        wi, wo, specialized_opaque<Capability, false>(energy, material, wi, incident));
}

/// 已分类 opaque 的连续响应与完整 marginal PDF；Capability 仅接受0、1、2。
/// 材料须满足对应 CAP_OPAQUE_DIELECTRIC／CAP_OPAQUE_SUBSURFACE／CAP_CONDUCTOR，
/// 调用方另外保证 float32 roughness²>=1e-4；IOR 相等仍合法，不检查或回退光滑边界。
/// 保留 diffuse／薄层 SSS、Fresnel、能量补偿和无效方向；通用入口仍支持光滑材料。
public Evaluation evaluate_opaque_continuous<let Capability : uint, Provider>(
    Provider energy, Material material, float3 normal, float3 view, float3 direction,
    float4 incident, float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    static_assert(Capability <= CAP_CONDUCTOR, "continuous opaque capability must be 0, 1 or 2");
    PrimeFullOpenPbrOnb onb = specialized_frame<Capability>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    float3 wo = primeFullOpenPbrOnbToLocal(onb, direction);
    if (wo.z < 0.0 && Capability != CAP_OPAQUE_SUBSURFACE)
        return { float3(0.0), 0.0 };
    PrimeFullCompactOpaqueState state =
        specialized_opaque<Capability, true>(energy, material, wi, incident);
    PrimeFullCompactOpaqueEvaluation evaluated =
        specialized_opaque_evaluate<Capability, true>(wi, wo, state);
    return specialized_evaluation(evaluated.throughput, evaluated.pdf);
}

/// evaluate_opaque_continuous 同一前置条件下的采样；保留原方向／随机映射和完整 marginal PDF。
/// diffuse、SSS 透射及 glossy 标签分别保留，不产生 delta；无效 proposal 与 sanitize 契约不变。
public Sample sample_opaque_continuous<let Capability : uint, Provider>(Provider energy,
                                                                        Material material,
                                                                        float3 normal, float3 view,
                                                                        float3 u, float4 incident,
                                                                        float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    static_assert(Capability <= CAP_CONDUCTOR, "continuous opaque capability must be 0, 1 or 2");
    PrimeFullOpenPbrOnb onb = specialized_frame<Capability>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    PrimeFullCompactOpaqueState state =
        specialized_opaque<Capability, true>(energy, material, wi, incident);
    PrimeFullOpenPbrSample sampled = specialized_opaque_sample<Capability, true>(wi, u, state);
    return specialized_result<Capability, true>(sampled, onb, view, incident, transmitted);
}

/// 连续 solid 的完整响应／marginal PDF；Material 必须满足 CAP_SOLID_DIELECTRIC。
/// 调用方另外保证 float32 roughness²>=1e-4，且 specialized_ior 的实际除法结果不等于1；
/// 只比较输入 IOR 不相等不足以代替该保证。不检查、修复或回退光滑／匹配边界。
/// 保留原能量补偿、局部无效方向及数值契约；通用 evaluate_specialized 不受此限制。
public Evaluation evaluate_solid_continuous<Provider>(Provider energy, Material material,
                                                      float3 normal, float3 view, float3 direction,
                                                      float4 incident, float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    PrimeFullOpenPbrOnb onb =
        specialized_frame<CAP_SOLID_DIELECTRIC>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    float3 wo = primeFullOpenPbrOnbToLocal(onb, direction);
    PrimeFullCompactTransmissionState state =
        specialized_transmission<CAP_SOLID_DIELECTRIC>(energy, material, wi, incident, transmitted);
    PrimeFullCompactTransmissionEvaluation evaluated =
        primeFullCompactTransmissionEvaluateSolidContinuous(wi, wo, state,
                                                            PRIME_FULL_OPENPBR_FLAG_ALL);
    return specialized_evaluation(evaluated.throughput, evaluated.pdf);
}

/// evaluate_solid_continuous 同一前置条件下的连续 solid 采样；不产生离散事件。
/// 方向、随机映射、完整 PDF、微面 TIR、能量补偿、eta／medium 及无效 proposal 与
/// sample_specialized<CAP_SOLID_DIELECTRIC> 相同；沿路径段 Beer 仍由调用方消费。
public Sample sample_solid_continuous<Provider>(Provider energy, Material material, float3 normal,
                                                float3 view, float3 u, float4 incident,
                                                float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    PrimeFullOpenPbrOnb onb =
        specialized_frame<CAP_SOLID_DIELECTRIC>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    PrimeFullCompactTransmissionState state =
        specialized_transmission<CAP_SOLID_DIELECTRIC>(energy, material, wi, incident, transmitted);
    PrimeFullOpenPbrSample sampled = primeFullCompactTransmissionSampleRawSolidContinuous(
        wi, u, primeFullCompactTransmissionClosureState(state), PRIME_FULL_OPENPBR_FLAG_ALL);
    return specialized_result<CAP_SOLID_DIELECTRIC, true>(sampled, onb, view, incident,
                                                          transmitted);
}

/// 纯 delta 类别的窄采样入口；不接收 Energy 或准备连续闭包状态。
/// Capability 仅接受 CAP_DELTA_CONDUCTOR、CAP_DELTA_SOLID_DIELECTRIC、CAP_DELTA_THIN_DIELECTRIC，
/// 调用方保证整个材料闭包满足对应类别；非 delta 类别编译失败，不作运行时分类或回退。
/// u 属于 [0,1)，对应 sample_specialized 的 u.z；conductor 不消费该随机数。
/// 其余输入、概率质量、事件、方向、eta、medium 和无效 proposal 语义与 sample_specialized 相同。
public Sample sample_delta_specialized<let Capability : uint>(Material material, float3 normal,
                                                              float3 view, float u, float4 incident,
                                                              float4 transmitted)
{
    static_assert(Capability >= CAP_DELTA_CONDUCTOR && Capability <= CAP_DELTA_THIN_DIELECTRIC,
                  "sample_delta_specialized requires a delta capability");
    PrimeFullOpenPbrOnb onb = specialized_frame<Capability>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    return specialized_result<Capability, false>(
        specialized_delta<Capability>(material, wi, float3(0.0, 0.0, u), incident, transmitted),
        onb, view, incident, transmitted);
}

/// 与 sample 相同的随机映射、完整 marginal PDF、event、eta 与 medium 转移。
/// 类别是编译期参数；纯 delta 直接调用原 delta 内核，不构造连续能量状态或读取能量纹理。
/// 随机数、局部无效 proposal 归零及沿路径段 Beer 的责任与 api 完全相同。
public Sample sample_specialized<let Capability : uint, Provider>(Provider energy,
                                                                  Material material, float3 normal,
                                                                  float3 view, float3 u,
                                                                  float4 incident,
                                                                  float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    require_capability<Capability>();
    PrimeFullOpenPbrOnb onb = specialized_frame<Capability>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    PrimeFullOpenPbrSample sampled;
    if (Capability >= CAP_DELTA_CONDUCTOR)
        sampled = specialized_delta<Capability>(material, wi, u, incident, transmitted);
    else if (specialized_dielectric<Capability>())
    {
        PrimeFullCompactTransmissionState state =
            specialized_transmission<Capability>(energy, material, wi, incident, transmitted);
        sampled = specialized_sample_prepared<Capability>(wi, u, state);
    }
    else
        sampled = specialized_sample_prepared<Capability>(
            wi, u, specialized_opaque<Capability, false>(energy, material, wi, incident));
    return specialized_result<Capability, false>(sampled, onb, view, incident, transmitted);
}

/// 同一命中只构造一次 ONB、local wi 与所选闭包状态，不将完整 prepared state 交给调用方。
/// evaluation_mask 的低两位分别启用 first／second；未启用的方向无需有效，高位不参与评估。
/// continuation=false 时不采样，scattered 保持 events=0、relative_eta=1 和 incident medium。
/// 纯 delta 忽略两个连续方向，直接沿用原 delta 核且不消费 energy；状态只在本次调用内有效。
public Batch shade_specialized<let Capability : uint, Provider>(
    Provider energy, Material material, float3 normal, float3 view, float3 first, float3 second,
    uint evaluation_mask, float3 u, bool continuation, float4 incident, float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    require_capability<Capability>();
    Batch result = {};
    result.scattered.relative_eta = 1.0;
    result.scattered.medium = incident;
    if (Capability >= CAP_DELTA_CONDUCTOR)
    {
        if (continuation)
        {
            PrimeFullOpenPbrOnb onb =
                specialized_frame<Capability>(normal, view, incident, transmitted);
            float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
            result.scattered = specialized_result<Capability, false>(
                specialized_delta<Capability>(material, wi, u, incident, transmitted), onb, view,
                incident, transmitted);
        }
        return result;
    }
    if ((evaluation_mask & 3u) == 0u && !continuation)
        return result;
    PrimeFullOpenPbrOnb onb = specialized_frame<Capability>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    if (specialized_dielectric<Capability>())
    {
        PrimeFullCompactTransmissionState state =
            specialized_transmission<Capability>(energy, material, wi, incident, transmitted);
        if ((evaluation_mask & 1u) != 0u)
            result.first = specialized_evaluate_prepared<Capability>(
                wi, primeFullOpenPbrOnbToLocal(onb, first), state);
        if ((evaluation_mask & 2u) != 0u)
            result.second = specialized_evaluate_prepared<Capability>(
                wi, primeFullOpenPbrOnbToLocal(onb, second), state);
        if (continuation)
            result.scattered = specialized_result<Capability, false>(
                specialized_sample_prepared<Capability>(wi, u, state), onb, view, incident,
                transmitted);
    }
    else
    {
        PrimeFullCompactOpaqueState state =
            specialized_opaque<Capability, false>(energy, material, wi, incident);
        if ((evaluation_mask & 1u) != 0u)
            result.first = specialized_evaluate_prepared<Capability>(
                wi, primeFullOpenPbrOnbToLocal(onb, first), state);
        if ((evaluation_mask & 2u) != 0u)
            result.second = specialized_evaluate_prepared<Capability>(
                wi, primeFullOpenPbrOnbToLocal(onb, second), state);
        if (continuation)
            result.scattered = specialized_result<Capability, false>(
                specialized_sample_prepared<Capability>(wi, u, state), onb, view, incident,
                transmitted);
    }
    return result;
}
}
