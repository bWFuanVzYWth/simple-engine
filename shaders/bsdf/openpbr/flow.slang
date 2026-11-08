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
#if defined(OPENPBR_FLOW)
#define SELECTED_FLOW OPENPBR_FLOW
#else
#define SELECTED_FLOW GAME_FLOW
#endif
#if SELECTED_FLOW & 48
#define COMPOSED_TRANSMISSION 1
#else
#define COMPOSED_OPAQUE 1
#endif
#ifdef OPENPBR_RECIPE_VARIANT
#define SELECTED_VARIANT OPENPBR_VARIANT
#else
#define SELECTED_VARIANT 0
#endif

namespace openpbr
{
/// 按方向即时消费连续评估，不发起callable或主查询。
public interface IFlowEvaluationConsumer
{
    [mutating]
    void consume(uint direction, Evaluation evaluated);
};
#ifdef SELECTED_FLOW
static const bool OpaqueContinuous = (SELECTED_FLOW & 64) == 0;
#else
static const bool OpaqueContinuous = false;
#endif

/// 每次只持有一个实际闭包；二值金属在建立状态之前分流。
internal Evaluation evaluate_closure<C : openpbr_composition::IClosure, Provider>(
    Provider energy, Material material, float3 normal, float3 view, float3 direction,
    float4 incident)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    PrimeFullOpenPbrOnb frame =
        specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, incident);
    float3 wi = primeFullOpenPbrOnbToLocal(frame, view);
    C.State state = C.prepare(energy, material, wi, incident.x);
    openpbr_composition::Value v =
        C.evaluate(wi, primeFullOpenPbrOnbToLocal(frame, direction), state);
    return specialized_evaluation(v.throughput, v.pdf);
}
internal Sample sample_closure<C : openpbr_composition::IClosure, Provider>(
    Provider energy, Material material, float3 normal, float3 view, float3 u, float4 incident)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    PrimeFullOpenPbrOnb frame =
        specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, incident);
    float3 wi = primeFullOpenPbrOnbToLocal(frame, view);
    C.State state = C.prepare(energy, material, wi, incident.x);
    return specialized_result<CAP_OPAQUE_DIELECTRIC, OpaqueContinuous>(
        openpbr_composition::sample_closure<C>(wi, u, state), frame, view, incident, incident);
}
internal Sample
consume_closure<C : openpbr_composition::IClosure, Provider, Consumer, let Directions : uint>(
    Provider energy, Material material, float3 normal, float3 view, float3 first, float3 second,
    uint evaluation_mask, float3 u, bool continuation, float4 incident, inout Consumer consumer)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
    where Consumer : IFlowEvaluationConsumer
{
    Sample result = {};
    result.relative_eta = 1;
    result.medium = incident;
    if ((Directions & evaluation_mask) == 0 && !continuation)
        return result;
    PrimeFullOpenPbrOnb frame =
        specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, incident);
    float3 wi = primeFullOpenPbrOnbToLocal(frame, view);
    C.State state = C.prepare(energy, material, wi, incident.x);
    if ((Directions & evaluation_mask & 1u) != 0)
    {
        openpbr_composition::Value v =
            C.evaluate(wi, primeFullOpenPbrOnbToLocal(frame, first), state);
        consumer.consume(0, specialized_evaluation(v.throughput, v.pdf));
    }
    if ((Directions & evaluation_mask & 2u) != 0)
    {
        openpbr_composition::Value v =
            C.evaluate(wi, primeFullOpenPbrOnbToLocal(frame, second), state);
        consumer.consume(1, specialized_evaluation(v.throughput, v.pdf));
    }
    if (continuation)
        result = specialized_result<CAP_OPAQUE_DIELECTRIC, OpaqueContinuous>(
            openpbr_composition::sample_closure<C>(wi, u, state), frame, view, incident, incident);
    return result;
}

#ifdef COMPOSED_TRANSMISSION
static const uint TransmissionCapability =
    (SELECTED_FLOW & 32) != 0 ? CAP_THIN_DIELECTRIC : CAP_SOLID_DIELECTRIC;
static const bool TransmissionContinuous = (SELECTED_FLOW & 320) == 0;
internal float interface_ior(float4 incident, float4 transmitted)
{
#if SELECTED_FLOW & 32
    return transmitted.x / incident.x;
#else
    return incident.x > transmitted.x ? incident.x / transmitted.x : transmitted.x / incident.x;
#endif
}
internal Evaluation evaluate_transmission<Provider>(Provider energy, Material material,
                                                    float3 normal, float3 view, float3 direction,
                                                    float4 incident, float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    PrimeFullOpenPbrOnb frame =
        specialized_frame<TransmissionCapability>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(frame, view);
    openpbr_composition::Transmission.State state = openpbr_composition::Transmission.prepare(
        energy, material.base_roughness.w, interface_ior(incident, transmitted),
        transmitted.yzw * material.surface.w, wi);
    openpbr_composition::Value v = openpbr_composition::Transmission.evaluate(
        wi, primeFullOpenPbrOnbToLocal(frame, direction), state);
    return specialized_evaluation(v.throughput, v.pdf);
}
internal Sample
consume_transmission<Provider, Consumer, let Directions : uint, let Primary : bool = false>(
    Provider energy, Material material, float3 normal, float3 view, float3 first, float3 second,
    uint evaluation_mask, float3 u, bool continuation, float4 incident, float4 transmitted,
    inout Consumer consumer)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
    where Consumer : IFlowEvaluationConsumer
{
    Sample result = {};
    result.relative_eta = 1;
    result.medium = incident;
    if ((Directions & evaluation_mask) == 0 && !continuation)
        return result;
    PrimeFullOpenPbrOnb frame =
        specialized_frame<TransmissionCapability>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(frame, view);
    openpbr_composition::Transmission.State state = openpbr_composition::Transmission.prepare(
        energy, material.base_roughness.w, interface_ior(incident, transmitted),
        transmitted.yzw * material.surface.w, wi);
    if ((Directions & evaluation_mask & 1u) != 0)
    {
        openpbr_composition::Value v = openpbr_composition::Transmission.evaluate<Primary>(
            wi, primeFullOpenPbrOnbToLocal(frame, first), state);
        consumer.consume(0, specialized_evaluation(v.throughput, v.pdf));
    }
    if ((Directions & evaluation_mask & 2u) != 0)
    {
        openpbr_composition::Value v = openpbr_composition::Transmission.evaluate<Primary>(
            wi, primeFullOpenPbrOnbToLocal(frame, second), state);
        consumer.consume(1, specialized_evaluation(v.throughput, v.pdf));
    }
    if (continuation)
        result = specialized_result<TransmissionCapability, TransmissionContinuous>(
            openpbr_composition::Transmission.sample<Primary>(wi, u, state), frame, view, incident,
            transmitted);
    return result;
}
#endif

public Evaluation evaluate_flow<let Flow : uint, Provider>(Provider energy, Material material,
                                                           float3 normal, float3 view,
                                                           float3 direction, float4 incident,
                                                           float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    if (!flow_continuous<Flow>())
        return { float3(0), 0 };
    static_assert(Flow == SELECTED_FLOW, "material recipe must match its SBT entry");
#ifdef COMPOSED_OPAQUE
#if (SELECTED_VARIANT & 2048) && (SELECTED_FLOW & 11)
    if (material.surface.y == 1)
        return evaluate_closure<openpbr_composition::Metal>(energy, material, normal, view,
                                                            direction, incident);
    return evaluate_closure<openpbr_composition::Dielectric>(energy, material, normal, view,
                                                             direction, incident);
#else
    return evaluate_closure<openpbr_composition::Opaque>(energy, material, normal, view, direction,
                                                         incident);
#endif
#elif defined(COMPOSED_TRANSMISSION)
    return evaluate_transmission(energy, material, normal, view, direction, incident, transmitted);
#endif
}
public Sample sample_flow<let Flow : uint, Provider>(Provider energy, Material material,
                                                     float3 normal, float3 view, float3 u,
                                                     float4 incident, float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    static_assert(Flow == SELECTED_FLOW, "material recipe must match its SBT entry");
#ifdef COMPOSED_OPAQUE
#if (SELECTED_VARIANT & 2048) && (SELECTED_FLOW & 11)
    if (material.surface.y == 1)
        return sample_closure<openpbr_composition::Metal>(energy, material, normal, view, u,
                                                          incident);
    return sample_closure<openpbr_composition::Dielectric>(energy, material, normal, view, u,
                                                           incident);
#else
    return sample_closure<openpbr_composition::Opaque>(energy, material, normal, view, u, incident);
#endif
#elif defined(COMPOSED_TRANSMISSION)
    BatchConsumer consumer = {};
    return consume_transmission<Provider, BatchConsumer, 0>(energy, material, normal, view,
                                                            float3(0), float3(0), 0, u, true,
                                                            incident, transmitted, consumer);
#endif
}
public Sample shade_flow_consume<let Flow : uint, Provider, Consumer, let Directions : uint = 3>(
    Provider energy, Material material, float3 normal, float3 view, float3 first, float3 second,
    uint evaluation_mask, float3 u, bool continuation, float4 incident, float4 transmitted,
    inout Consumer consumer)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
    where Consumer : IFlowEvaluationConsumer
{
    static_assert(Flow == SELECTED_FLOW, "material recipe must match its SBT entry");
#ifdef COMPOSED_OPAQUE
#if (SELECTED_VARIANT & 2048) && (SELECTED_FLOW & 11)
    if (material.surface.y == 1)
        return consume_closure<openpbr_composition::Metal, Provider, Consumer, Directions>(
            energy, material, normal, view, first, second, evaluation_mask, u, continuation,
            incident, consumer);
    return consume_closure<openpbr_composition::Dielectric, Provider, Consumer, Directions>(
        energy, material, normal, view, first, second, evaluation_mask, u, continuation, incident,
        consumer);
#else
    return consume_closure<openpbr_composition::Opaque, Provider, Consumer, Directions>(
        energy, material, normal, view, first, second, evaluation_mask, u, continuation, incident,
        consumer);
#endif
#elif defined(COMPOSED_TRANSMISSION)
    return consume_transmission<Provider, Consumer, Directions>(
        energy, material, normal, view, first, second, evaluation_mask, u, continuation, incident,
        transmitted, consumer);
#endif
}
/// 首表面独立提案；光源MIS使用同一混合密度，2 spp 对提案选择分层后取平均。
public Batch shade_primary_flow<let Flow : uint, Provider, let Directions : uint = 3>(
    Provider energy, Material material, float3 normal, float3 view, float3 first, float3 second,
    uint evaluation_mask, float3 u, bool continuation, float4 incident, float4 transmitted,
    uint samples, uint lane)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    static_assert(Flow == SELECTED_FLOW, "primary recipe must match its callable entry");
    u.z = openpbr_primary::selection(u.z, samples, lane);
    BatchConsumer consumer = {};
    Sample scattered;
#ifdef COMPOSED_OPAQUE
#if (SELECTED_VARIANT & 2048) && (SELECTED_FLOW & 11)
    if (material.surface.y == 1)
        scattered =
            consume_closure<openpbr_composition::Metal, Provider, BatchConsumer, Directions>(
                energy, material, normal, view, first, second, evaluation_mask, u, continuation,
                incident, consumer);
    else
        scattered =
            consume_closure<openpbr_composition::PrimaryDielectric, Provider, BatchConsumer,
                            Directions>(energy, material, normal, view, first, second,
                                        evaluation_mask, u, continuation, incident, consumer);
#else
    scattered =
        consume_closure<openpbr_composition::PrimaryOpaque, Provider, BatchConsumer, Directions>(
            energy, material, normal, view, first, second, evaluation_mask, u, continuation,
            incident, consumer);
#endif
#else
    scattered = consume_transmission<Provider, BatchConsumer, Directions, true>(
        energy, material, normal, view, first, second, evaluation_mask, u, continuation, incident,
        transmitted, consumer);
#endif
    return { consumer.first, consumer.second, scattered };
}
struct BatchConsumer : IFlowEvaluationConsumer
{
    Evaluation first;
    Evaluation second;
    [mutating]
    void consume(uint direction, Evaluation value)
    {
        if (direction == 0)
            first = value;
        else
            second = value;
    }
};
public Batch shade_flow<let Flow : uint, Provider, let Directions : uint = 3>(
    Provider energy, Material material, float3 normal, float3 view, float3 first, float3 second,
    uint evaluation_mask, float3 u, bool continuation, float4 incident, float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    BatchConsumer consumer = {};
    Sample sample = shade_flow_consume<Flow, Provider, BatchConsumer, Directions>(
        energy, material, normal, view, first, second, evaluation_mask, u, continuation, incident,
        transmitted, consumer);
    return { consumer.first, consumer.second, sample };
}
}

#endif
