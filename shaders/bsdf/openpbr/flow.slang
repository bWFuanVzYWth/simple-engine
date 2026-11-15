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
import "bsdf/openpbr/math/frame.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/contract/material.slang";
import bsdf.openpbr.selection;

namespace openpbr
{
/// 按方向即时消费连续评估，不发起callable或主查询。
public interface IFlowEvaluationConsumer
{
    [mutating]
    func consume(direction: uint, evaluated: Evaluation)->void;
};
static const let OpaqueContinuous : bool = (openpbr_composition::SelectedFlow & 64) == 0;

/// 每次只持有一个实际闭包；二值金属在建立状态之前分流。
internal func evaluate_closure<C : openpbr_composition::IClosure, Provider>(
    energy: Provider, material: Material, normal: float3, view: float3, direction: float3,
    incident: float4)
    ->Evaluation
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    let frame : PrimeFullOpenPbrOnb =
                    specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, incident);
    let wi : float3 = primeFullOpenPbrOnbToLocal(frame, view);
    let state : C.State = C.prepare(energy, material, wi, incident.x);
    let v : openpbr_composition::Value =
                C.evaluate(wi, primeFullOpenPbrOnbToLocal(frame, direction), state);
    return specialized_evaluation(v.throughput, v.pdf);
}
internal func sample_closure<C : openpbr_composition::IClosure, Provider>(
    energy: Provider, material: Material, normal: float3, view: float3, u: float3, incident: float4)
    ->Sample
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    let frame : PrimeFullOpenPbrOnb =
                    specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, incident);
    let wi : float3 = primeFullOpenPbrOnbToLocal(frame, view);
    let state : C.State = C.prepare(energy, material, wi, incident.x);
    return specialized_result<CAP_OPAQUE_DIELECTRIC, OpaqueContinuous>(
        openpbr_composition::sample_closure<C>(wi, u, state), frame, view, incident, incident);
}
internal func
consume_closure<C : openpbr_composition::IClosure, Provider, Consumer, let Directions : uint>(
    energy: Provider, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    inout consumer: Consumer)
    ->Sample
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
    where Consumer : IFlowEvaluationConsumer
{
    var result : Sample = {};
    result.relative_eta = 1;
    result.medium = incident;
    if ((Directions & evaluation_mask) == 0 && !continuation)
        return result;
    let frame : PrimeFullOpenPbrOnb =
                    specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, incident);
    let wi : float3 = primeFullOpenPbrOnbToLocal(frame, view);
    let state : C.State = C.prepare(energy, material, wi, incident.x);
    if ((Directions & evaluation_mask & 1u) != 0)
    {
        let v : openpbr_composition::Value =
                    C.evaluate(wi, primeFullOpenPbrOnbToLocal(frame, first), state);
        consumer.consume(0, specialized_evaluation(v.throughput, v.pdf));
    }
    if ((Directions & evaluation_mask & 2u) != 0)
    {
        let v : openpbr_composition::Value =
                    C.evaluate(wi, primeFullOpenPbrOnbToLocal(frame, second), state);
        consumer.consume(1, specialized_evaluation(v.throughput, v.pdf));
    }
    if (continuation)
        result = specialized_result<CAP_OPAQUE_DIELECTRIC, OpaqueContinuous>(
            openpbr_composition::sample_closure<C>(wi, u, state), frame, view, incident, incident);
    return result;
}

static const let TransmissionCapability : uint = (openpbr_composition::SelectedFlow & 32) != 0
                                                     ? CAP_THIN_DIELECTRIC
                                                     : CAP_SOLID_DIELECTRIC;
static const let TransmissionContinuous : bool = (openpbr_composition::SelectedFlow & 320) == 0;
internal func interface_ior(incident: float4, transmitted: float4)->float
{
    if ((openpbr_composition::SelectedFlow & 32) != 0)
    {
        return transmitted.x / incident.x;
    }
    else
    {
        return incident.x > transmitted.x ? incident.x / transmitted.x : transmitted.x / incident.x;
    }
}
internal func evaluate_transmission<Provider>(energy: Provider, material: Material, normal: float3,
                                              view: float3, direction: float3, incident: float4,
                                              transmitted: float4)
    ->Evaluation
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    let frame : PrimeFullOpenPbrOnb =
                    specialized_frame<TransmissionCapability>(normal, view, incident, transmitted);
    let wi : float3 = primeFullOpenPbrOnbToLocal(frame, view);
    let state : openpbr_composition::Transmission.State = openpbr_composition::Transmission.prepare(
        energy, material.base_roughness.w, interface_ior(incident, transmitted),
        transmitted.yzw * material.surface.w, wi);
    let v : openpbr_composition::Value = openpbr_composition::Transmission.evaluate(
                wi, primeFullOpenPbrOnbToLocal(frame, direction), state);
    return specialized_evaluation(v.throughput, v.pdf);
}
internal func
consume_transmission<Provider, Consumer, let Directions : uint, let Primary : bool = false>(
    energy: Provider, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    transmitted: float4, inout consumer: Consumer)
    ->Sample
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
    where Consumer : IFlowEvaluationConsumer
{
    var result : Sample = {};
    result.relative_eta = 1;
    result.medium = incident;
    if ((Directions & evaluation_mask) == 0 && !continuation)
        return result;
    let frame : PrimeFullOpenPbrOnb =
                    specialized_frame<TransmissionCapability>(normal, view, incident, transmitted);
    let wi : float3 = primeFullOpenPbrOnbToLocal(frame, view);
    let state : openpbr_composition::Transmission.State = openpbr_composition::Transmission.prepare(
        energy, material.base_roughness.w, interface_ior(incident, transmitted),
        transmitted.yzw * material.surface.w, wi);
    if ((Directions & evaluation_mask & 1u) != 0)
    {
        let v : openpbr_composition::Value = openpbr_composition::Transmission.evaluate<Primary>(
                    wi, primeFullOpenPbrOnbToLocal(frame, first), state);
        consumer.consume(0, specialized_evaluation(v.throughput, v.pdf));
    }
    if ((Directions & evaluation_mask & 2u) != 0)
    {
        let v : openpbr_composition::Value = openpbr_composition::Transmission.evaluate<Primary>(
                    wi, primeFullOpenPbrOnbToLocal(frame, second), state);
        consumer.consume(1, specialized_evaluation(v.throughput, v.pdf));
    }
    if (continuation)
        result = specialized_result<TransmissionCapability, TransmissionContinuous>(
            openpbr_composition::Transmission.sample<Primary>(wi, u, state), frame, view, incident,
            transmitted);
    return result;
}

public func evaluate_flow<let Flow : uint, Provider>(energy: Provider, material: Material,
                                                     normal: float3, view: float3,
                                                     direction: float3, incident: float4,
                                                     transmitted: float4)
    ->Evaluation
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    if (!flow_continuous<Flow>())
        return { float3(0), 0 };
    static_assert(Flow == openpbr_composition::SelectedFlow,
                  "material recipe must match its SBT entry");
    if ((openpbr_composition::SelectedFlow & 48) == 0)
    {
        if ((openpbr_composition::SelectedVariant & 2048) != 0 &&
            (openpbr_composition::SelectedFlow & 11) != 0)
        {
            if (material.surface.y == 1)
                return evaluate_closure<openpbr_composition::Metal>(energy, material, normal, view,
                                                                    direction, incident);
            return evaluate_closure<openpbr_composition::Dielectric>(energy, material, normal, view,
                                                                     direction, incident);
        }
        else
        {
            return evaluate_closure<openpbr_composition::Opaque>(energy, material, normal, view,
                                                                 direction, incident);
        }
    }
    else
    {
        return evaluate_transmission(energy, material, normal, view, direction, incident,
                                     transmitted);
    }
}
public func sample_flow<let Flow : uint, Provider>(energy: Provider, material: Material,
                                                   normal: float3, view: float3, u: float3,
                                                   incident: float4, transmitted: float4)
    ->Sample
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    static_assert(Flow == openpbr_composition::SelectedFlow,
                  "material recipe must match its SBT entry");
    if ((openpbr_composition::SelectedFlow & 48) == 0)
    {
        if ((openpbr_composition::SelectedVariant & 2048) != 0 &&
            (openpbr_composition::SelectedFlow & 11) != 0)
        {
            if (material.surface.y == 1)
                return sample_closure<openpbr_composition::Metal>(energy, material, normal, view, u,
                                                                  incident);
            return sample_closure<openpbr_composition::Dielectric>(energy, material, normal, view,
                                                                   u, incident);
        }
        else
        {
            return sample_closure<openpbr_composition::Opaque>(energy, material, normal, view, u,
                                                               incident);
        }
    }
    else
    {
        var consumer : BatchConsumer = {};
        return consume_transmission<Provider, BatchConsumer, 0>(energy, material, normal, view,
                                                                float3(0), float3(0), 0, u, true,
                                                                incident, transmitted, consumer);
    }
}
public func shade_flow_consume<let Flow : uint, Provider, Consumer, let Directions : uint = 3>(
    energy: Provider, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    transmitted: float4, inout consumer: Consumer)
    ->Sample
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
    where Consumer : IFlowEvaluationConsumer
{
    static_assert(Flow == openpbr_composition::SelectedFlow,
                  "material recipe must match its SBT entry");
    if ((openpbr_composition::SelectedFlow & 48) == 0)
    {
        if ((openpbr_composition::SelectedVariant & 2048) != 0 &&
            (openpbr_composition::SelectedFlow & 11) != 0)
        {
            if (material.surface.y == 1)
                return consume_closure<openpbr_composition::Metal, Provider, Consumer, Directions>(
                    energy, material, normal, view, first, second, evaluation_mask, u, continuation,
                    incident, consumer);
            return consume_closure<openpbr_composition::Dielectric, Provider, Consumer, Directions>(
                energy, material, normal, view, first, second, evaluation_mask, u, continuation,
                incident, consumer);
        }
        else
        {
            return consume_closure<openpbr_composition::Opaque, Provider, Consumer, Directions>(
                energy, material, normal, view, first, second, evaluation_mask, u, continuation,
                incident, consumer);
        }
    }
    else
    {
        return consume_transmission<Provider, Consumer, Directions>(
            energy, material, normal, view, first, second, evaluation_mask, u, continuation,
            incident, transmitted, consumer);
    }
}
/// 首表面按 first、second 顺序即时消费所请求的评估，再采样一次散射。
/// 共享同一闭包准备与首面提案；回调约束同 IFlowEvaluationConsumer。
/// 调用方保证 samples 为 1 或 2，且 lane<samples。
public func shade_primary_flow_consume<let Flow : uint, Provider, Consumer,
                                       let Directions : uint = 3>(
    energy: Provider, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    transmitted: float4, samples: uint, lane: uint, inout consumer: Consumer)
    ->Sample
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
    where Consumer : IFlowEvaluationConsumer
{
    var u = u;
    static_assert(Flow == openpbr_composition::SelectedFlow,
                  "primary recipe must match its callable entry");
    u.z = openpbr_primary::selection(u.z, samples, lane);
    var scattered : Sample;
    if ((openpbr_composition::SelectedFlow & 48) == 0)
    {
        if ((openpbr_composition::SelectedVariant & 2048) != 0 &&
            (openpbr_composition::SelectedFlow & 11) != 0)
        {
            if (material.surface.y == 1)
                scattered =
                    consume_closure<openpbr_composition::Metal, Provider, Consumer, Directions>(
                        energy, material, normal, view, first, second, evaluation_mask, u,
                        continuation, incident, consumer);
            else
                scattered = consume_closure<openpbr_composition::PrimaryDielectric, Provider,
                                            Consumer, Directions>(energy, material, normal, view,
                                                                  first, second, evaluation_mask, u,
                                                                  continuation, incident, consumer);
        }
        else
        {
            scattered =
                consume_closure<openpbr_composition::PrimaryOpaque, Provider, Consumer, Directions>(
                    energy, material, normal, view, first, second, evaluation_mask, u, continuation,
                    incident, consumer);
        }
    }
    else
    {
        scattered = consume_transmission<Provider, Consumer, Directions, true>(
            energy, material, normal, view, first, second, evaluation_mask, u, continuation,
            incident, transmitted, consumer);
    }
    return scattered;
}
/// 首表面独立提案；光源MIS使用同一混合密度，2 spp 对提案选择分层后取平均。
/// 调用方保证 samples 为 1 或 2，且 lane<samples。
public func shade_primary_flow<let Flow : uint, Provider, let Directions : uint = 3>(
    energy: Provider, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    transmitted: float4, samples: uint, lane: uint)
    ->Batch
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    var consumer : BatchConsumer = {};
    let scattered = shade_primary_flow_consume<Flow, Provider, BatchConsumer, Directions>(
        energy, material, normal, view, first, second, evaluation_mask, u, continuation, incident,
        transmitted, samples, lane, consumer);
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
public func shade_flow<let Flow : uint, Provider, let Directions : uint = 3>(
    energy: Provider, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    transmitted: float4)
    ->Batch
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    var consumer : BatchConsumer = {};
    let sample : Sample = shade_flow_consume<Flow, Provider, BatchConsumer, Directions>(
                     energy, material, normal, view, first, second, evaluation_mask, u,
                     continuation, incident, transmitted, consumer);
    return { consumer.first, consumer.second, sample };
}
}

#endif
