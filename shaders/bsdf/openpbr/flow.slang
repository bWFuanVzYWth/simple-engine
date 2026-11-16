// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 已绑定材质键的组合数值入口；未绑定键的独立参考保留原算法。
module flow;
#if !defined(OPENPBR_FLOW) && !defined(GAME_FLOW)
__exported import bsdf.openpbr.flow_reference;
#else
__exported import bsdf.openpbr.flow_contract;
import bsdf.openpbr.composition;
import bsdf.openpbr.primary_composition;
import bsdf.openpbr.primary_policy;
import bsdf.openpbr.transmission_composition;
import bsdf.openpbr.math.frame;
import bsdf.openpbr.math.microfacet_energy;
import bsdf.openpbr.contract.material;
import bsdf.openpbr.selection;

namespace openpbr
{
/// 按方向即时消费连续评估，不发起callable或主查询。
public interface IFlowEvaluationConsumer
{
    [mutating]
    func consume(direction: uint, evaluated: Evaluation)->void;
};
static const let opaque_continuous : bool = (closure::selected_flow & FLOW_DELTA_REFLECTION) == 0;

/// 每次只持有一个实际闭包；二值金属在建立状态之前分流。
internal func evaluate_closure<Closure : closure::IClosure, EnergyLut>(energy: EnergyLut,
                                                                       material: Material,
                                                                       normal: float3, view: float3,
                                                                       direction: float3,
                                                                       incident: float4)
    ->Evaluation
    where EnergyLut : IEnergyLut
{
    let frame : kernel::Frame =
                    specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, incident);
    let wi : float3 = kernel::to_local(frame, view);
    let state : Closure.State = Closure.prepare(energy, material, wi, incident.x);
    let evaluated : closure::LocalEvaluation =
                        Closure.evaluate(wi, kernel::to_local(frame, direction), state);
    return specialized_evaluation(evaluated.response, evaluated.pdf);
}
internal func sample_closure<Closure : closure::IClosure, EnergyLut>(energy: EnergyLut,
                                                                     material: Material,
                                                                     normal: float3, view: float3,
                                                                     random: float3,
                                                                     incident: float4)
    ->Sample
    where EnergyLut : IEnergyLut
{
    let frame : kernel::Frame =
                    specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, incident);
    let wi : float3 = kernel::to_local(frame, view);
    let state : Closure.State = Closure.prepare(energy, material, wi, incident.x);
    return specialized_result<CAP_OPAQUE_DIELECTRIC, opaque_continuous>(
        closure::sample_closure<Closure>(wi, random, state), frame, view, incident, incident);
}
internal func
consume_closure<Closure : closure::IClosure, EnergyLut, Consumer, let directions : uint>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, random: float3, continuation: bool, incident: float4,
    inout consumer: Consumer)
    ->Sample
    where EnergyLut : IEnergyLut
    where Consumer : IFlowEvaluationConsumer
{
    var result : Sample = {};
    result.relative_eta = 1;
    result.medium = incident;
    if ((directions & evaluation_mask) == 0 && !continuation)
        return result;
    let frame : kernel::Frame =
                    specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, incident);
    let wi : float3 = kernel::to_local(frame, view);
    let state : Closure.State = Closure.prepare(energy, material, wi, incident.x);
    if ((directions & evaluation_mask & 1u) != 0)
    {
        let evaluated : closure::LocalEvaluation =
                            Closure.evaluate(wi, kernel::to_local(frame, first), state);
        consumer.consume(0, specialized_evaluation(evaluated.response, evaluated.pdf));
    }
    if ((directions & evaluation_mask & 2u) != 0)
    {
        let evaluated : closure::LocalEvaluation =
                            Closure.evaluate(wi, kernel::to_local(frame, second), state);
        consumer.consume(1, specialized_evaluation(evaluated.response, evaluated.pdf));
    }
    if (continuation)
        result = specialized_result<CAP_OPAQUE_DIELECTRIC, opaque_continuous>(
            closure::sample_closure<Closure>(wi, random, state), frame, view, incident, incident);
    return result;
}

static const let transmission_capability
    : uint = (closure::selected_flow & FLOW_THIN_TRANSMISSION) != 0 ? CAP_THIN_DIELECTRIC
                                                                    : CAP_SOLID_DIELECTRIC;
static const let transmission_continuous
    : bool = (closure::selected_flow & (FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION)) == 0;
internal func interface_ior(incident: float4, transmitted: float4)->float
{
    if ((closure::selected_flow & FLOW_THIN_TRANSMISSION) != 0)
    {
        return transmitted.x / incident.x;
    }
    else
    {
        return incident.x > transmitted.x ? incident.x / transmitted.x : transmitted.x / incident.x;
    }
}
internal func evaluate_transmission<EnergyLut>(energy: EnergyLut, material: Material,
                                               normal: float3, view: float3, direction: float3,
                                               incident: float4, transmitted: float4)
    ->Evaluation
    where EnergyLut : IEnergyLut
{
    let frame : kernel::Frame =
                    specialized_frame<transmission_capability>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let state : closure::Transmission.State = closure::Transmission.prepare(
        energy, material.base_roughness.w, interface_ior(incident, transmitted),
        transmitted.yzw * material.surface.w, wi);
    let evaluated
        : closure::LocalEvaluation =
              closure::Transmission.evaluate(wi, kernel::to_local(frame, direction), state);
    return specialized_evaluation(evaluated.response, evaluated.pdf);
}
internal func
consume_transmission<EnergyLut, Consumer, let directions : uint, let primary : bool = false>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, random: float3, continuation: bool, incident: float4,
    transmitted: float4, inout consumer: Consumer)
    ->Sample
    where EnergyLut : IEnergyLut
    where Consumer : IFlowEvaluationConsumer
{
    var result : Sample = {};
    result.relative_eta = 1;
    result.medium = incident;
    if ((directions & evaluation_mask) == 0 && !continuation)
        return result;
    let frame : kernel::Frame =
                    specialized_frame<transmission_capability>(normal, view, incident, transmitted);
    let wi : float3 = kernel::to_local(frame, view);
    let state : closure::Transmission.State = closure::Transmission.prepare(
        energy, material.base_roughness.w, interface_ior(incident, transmitted),
        transmitted.yzw * material.surface.w, wi);
    if ((directions & evaluation_mask & 1u) != 0)
    {
        let evaluated : closure::LocalEvaluation = closure::Transmission.evaluate<primary>(
                            wi, kernel::to_local(frame, first), state);
        consumer.consume(0, specialized_evaluation(evaluated.response, evaluated.pdf));
    }
    if ((directions & evaluation_mask & 2u) != 0)
    {
        let evaluated : closure::LocalEvaluation = closure::Transmission.evaluate<primary>(
                            wi, kernel::to_local(frame, second), state);
        consumer.consume(1, specialized_evaluation(evaluated.response, evaluated.pdf));
    }
    if (continuation)
        result = specialized_result<transmission_capability, transmission_continuous>(
            closure::Transmission.sample<primary>(wi, random, state), frame, view, incident,
            transmitted);
    return result;
}

public func evaluate_flow<let flow : uint, EnergyLut>(energy: EnergyLut, material: Material,
                                                      normal: float3, view: float3,
                                                      direction: float3, incident: float4,
                                                      transmitted: float4)
    ->Evaluation
    where EnergyLut : IEnergyLut
{
    if (!flow_continuous<flow>())
        return { float3(0), 0 };
    static_assert(flow == closure::selected_flow, "material recipe must match its SBT entry");
    if ((closure::selected_flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) == 0)
    {
        if ((closure::selected_variant & closure::variant_metal_switch) != 0 &&
            (closure::selected_flow &
             (FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_SUBSURFACE)) != 0)
        {
            if (material.surface.y == 1)
                return evaluate_closure<closure::Metal>(energy, material, normal, view, direction,
                                                        incident);
            return evaluate_closure<closure::Dielectric>(energy, material, normal, view, direction,
                                                         incident);
        }
        else
        {
            return evaluate_closure<closure::Opaque>(energy, material, normal, view, direction,
                                                     incident);
        }
    }
    else
    {
        return evaluate_transmission(energy, material, normal, view, direction, incident,
                                     transmitted);
    }
}
public func sample_flow<let flow : uint, EnergyLut>(energy: EnergyLut, material: Material,
                                                    normal: float3, view: float3, random: float3,
                                                    incident: float4, transmitted: float4)
    ->Sample
    where EnergyLut : IEnergyLut
{
    static_assert(flow == closure::selected_flow, "material recipe must match its SBT entry");
    if ((closure::selected_flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) == 0)
    {
        if ((closure::selected_variant & closure::variant_metal_switch) != 0 &&
            (closure::selected_flow &
             (FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_SUBSURFACE)) != 0)
        {
            if (material.surface.y == 1)
                return sample_closure<closure::Metal>(energy, material, normal, view, random,
                                                      incident);
            return sample_closure<closure::Dielectric>(energy, material, normal, view, random,
                                                       incident);
        }
        else
        {
            return sample_closure<closure::Opaque>(energy, material, normal, view, random,
                                                   incident);
        }
    }
    else
    {
        var consumer : BatchConsumer = {};
        return consume_transmission<EnergyLut, BatchConsumer, 0>(
            energy, material, normal, view, float3(0), float3(0), 0, random, true, incident,
            transmitted, consumer);
    }
}
public func shade_flow_consume<let flow : uint, EnergyLut, Consumer, let directions : uint = 3>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, random: float3, continuation: bool, incident: float4,
    transmitted: float4, inout consumer: Consumer)
    ->Sample
    where EnergyLut : IEnergyLut
    where Consumer : IFlowEvaluationConsumer
{
    static_assert(flow == closure::selected_flow, "material recipe must match its SBT entry");
    if ((closure::selected_flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) == 0)
    {
        if ((closure::selected_variant & closure::variant_metal_switch) != 0 &&
            (closure::selected_flow &
             (FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_SUBSURFACE)) != 0)
        {
            if (material.surface.y == 1)
                return consume_closure<closure::Metal, EnergyLut, Consumer, directions>(
                    energy, material, normal, view, first, second, evaluation_mask, random,
                    continuation, incident, consumer);
            return consume_closure<closure::Dielectric, EnergyLut, Consumer, directions>(
                energy, material, normal, view, first, second, evaluation_mask, random,
                continuation, incident, consumer);
        }
        else
        {
            return consume_closure<closure::Opaque, EnergyLut, Consumer, directions>(
                energy, material, normal, view, first, second, evaluation_mask, random,
                continuation, incident, consumer);
        }
    }
    else
    {
        return consume_transmission<EnergyLut, Consumer, directions>(
            energy, material, normal, view, first, second, evaluation_mask, random, continuation,
            incident, transmitted, consumer);
    }
}
/// 首表面按 first、second 顺序即时消费所请求的评估，再采样一次散射。
/// 共享同一闭包准备与首面提案；回调约束同 IFlowEvaluationConsumer。
/// 调用方保证 samples 为 1 或 2，且 lane<samples。
public func shade_primary_flow_consume<let flow : uint, EnergyLut, Consumer,
                                       let directions : uint = 3>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, random: float3, continuation: bool, incident: float4,
    transmitted: float4, samples: uint, lane: uint, inout consumer: Consumer)
    ->Sample
    where EnergyLut : IEnergyLut
    where Consumer : IFlowEvaluationConsumer
{
    var random = random;
    static_assert(flow == closure::selected_flow, "primary recipe must match its callable entry");
    random.z = openpbr::primary::selection(random.z, samples, lane);
    var scattered : Sample;
    if ((closure::selected_flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) == 0)
    {
        if ((closure::selected_variant & closure::variant_metal_switch) != 0 &&
            (closure::selected_flow &
             (FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_SUBSURFACE)) != 0)
        {
            if (material.surface.y == 1)
                scattered = consume_closure<closure::Metal, EnergyLut, Consumer, directions>(
                    energy, material, normal, view, first, second, evaluation_mask, random,
                    continuation, incident, consumer);
            else
                scattered =
                    consume_closure<closure::PrimaryDielectric, EnergyLut, Consumer, directions>(
                        energy, material, normal, view, first, second, evaluation_mask, random,
                        continuation, incident, consumer);
        }
        else
        {
            scattered = consume_closure<closure::PrimaryOpaque, EnergyLut, Consumer, directions>(
                energy, material, normal, view, first, second, evaluation_mask, random,
                continuation, incident, consumer);
        }
    }
    else
    {
        scattered = consume_transmission<EnergyLut, Consumer, directions, true>(
            energy, material, normal, view, first, second, evaluation_mask, random, continuation,
            incident, transmitted, consumer);
    }
    return scattered;
}
/// 首表面独立提案；光源MIS使用同一混合密度，2 spp 对提案选择分层后取平均。
/// 调用方保证 samples 为 1 或 2，且 lane<samples。
public func shade_primary_flow<let flow : uint, EnergyLut, let directions : uint = 3>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, random: float3, continuation: bool, incident: float4,
    transmitted: float4, samples: uint, lane: uint)
    ->Batch
    where EnergyLut : IEnergyLut
{
    var consumer : BatchConsumer = {};
    let scattered = shade_primary_flow_consume<flow, EnergyLut, BatchConsumer, directions>(
        energy, material, normal, view, first, second, evaluation_mask, random, continuation,
        incident, transmitted, samples, lane, consumer);
    return { consumer.first, consumer.second, scattered };
}
struct BatchConsumer : IFlowEvaluationConsumer
{
    var first : Evaluation;
    var second : Evaluation;
    [mutating]
    func consume(direction: uint, value: Evaluation)->void
    {
        if (direction == 0)
            first = value;
        else
            second = value;
    }
};
public func shade_flow<let flow : uint, EnergyLut, let directions : uint = 3>(
    energy: EnergyLut, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, random: float3, continuation: bool, incident: float4,
    transmitted: float4)
    ->Batch
    where EnergyLut : IEnergyLut
{
    var consumer : BatchConsumer = {};
    let sample : Sample = shade_flow_consume<flow, EnergyLut, BatchConsumer, directions>(
                     energy, material, normal, view, first, second, evaluation_mask, random,
                     continuation, incident, transmitted, consumer);
    return { consumer.first, consumer.second, sample };
}
}

#endif
