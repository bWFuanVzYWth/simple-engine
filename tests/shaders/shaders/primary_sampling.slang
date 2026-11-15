/// 实际首面闭包：提案与求值一致性、物理响应和混合估计量。
module primary_sampling_test;
import bsdf.openpbr.api;
import bsdf.openpbr.flow;
import bsdf.openpbr.primary_policy;
import pt.path_sampler;
import pt.integrator;
import pt.scene;
import pt.transport;
import "bsdf/openpbr/math/microfacet_energy.slang";

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

public struct Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    public func sampleLevel(uvw: float3)->float4
    {
        return float4(0.2, 0.3, 0.4, 0.5);
    }
};

func weight(s: openpbr::Sample)->float3
{
    return s.events != 0 ? s.response / s.pdf : float3(0);
}

struct RadianceConsumer : openpbr::IFlowEvaluationConsumer
{
    var radiance : float3;
    var continuation : bool;
    var sequence : uint;

    [mutating]
    func consume(direction: uint, evaluated: openpbr::Evaluation)->void
    {
        sequence = sequence * 4u + direction + 1u;
        let light = direction == 0 ? float4(0.3, 1.2, 0.7, 0.19) : float4(2.1, 0.4, 0.5, 0.37);
        if (any(evaluated.response > 0))
            radiance += float3(0.7, 1.3, 0.9) *
                        pt::triple_product(evaluated.response, light.xyz,
                                           pt::power_heuristic_over_pdf(
                                               light.w, continuation ? evaluated.pdf : 0));
    }
};

[numthreads(64, 1, 1)]
func primary_selection_main(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x >= uint(inputs[4].x))
        return;
    let sampler = pt::path_sampler(uint2(id.x & 255u, id.x >> 8u), 0, 8, 3721);
    let choice = (id.x >> 2u) & 3u;
    let random = choice == 0   ? 0.0
                 : choice == 1 ? 0.5
                 : choice == 2 ? asfloat(0x3f7fffffu)
                               : sampler.bsdf(0).z;
    let samples = 1u + (id.x & 1u);
    let lane = samples == 2u ? (id.x >> 1u) & 1u : 0u;
    outputs[id.x * 5] = float4(openpbr_primary::selection(random, samples, lane), random,
                               float(samples), float(lane));
}

// 掩码、最后一跳和介质限额均保留原来的评估、MIS和散射契约。
[numthreads(64, 1, 1)]
func primary_consumption_main(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x >= uint(inputs[4].x))
        return;
    let m : openpbr::Material = { inputs[0], inputs[1], uint4(uint(inputs[4].y), 0, 0, 0) };
    let view = normalize(float3(inputs[4].z, 0, inputs[4].w));
    let first = float3(0.6, 0, 0.8);
    let second = float3(0.6, 0, -0.8);
    let mask = id.x & 3u;
    let continuation = (id.x & 8u) != 0;
    let scatter = continuation && (id.x & 4u) != 0;
    let samples = (id.x & 16u) != 0 ? 2u : 1u;
    let lane = samples == 2 ? (id.x >> 5u) & 1u : 0u;
    let sampler = pt::path_sampler(uint2(id.x & 255u, id.x >> 8u), 0, 8, 3721);
    let u = scatter ? sampler.bsdf(lane) : float3(0);
    var consumer : RadianceConsumer = { float3(0.13, 0.17, 0.23), continuation, 0 };
    let actual = openpbr::shade_primary_flow_consume<OPENPBR_FLOW, Energy, RadianceConsumer>(
        Energy(), m, float3(0, 0, 1), view, first, second, mask, u, scatter, inputs[2], inputs[3],
        samples, lane, consumer);
    let expected = openpbr::shade_primary_flow<OPENPBR_FLOW>(Energy(), m, float3(0, 0, 1), view,
                                                             first, second, mask, u, scatter,
                                                             inputs[2], inputs[3], samples, lane);
    var radiance = float3(0.13, 0.17, 0.23);
    if ((mask & 1u) != 0 && any(expected.first.response > 0))
        radiance += float3(0.7, 1.3, 0.9) *
                    pt::triple_product(
                        expected.first.response, float3(0.3, 1.2, 0.7),
                        pt::power_heuristic_over_pdf(0.19, continuation ? expected.first.pdf : 0));
    if ((mask & 2u) != 0 && any(expected.second.response > 0))
        radiance += float3(0.7, 1.3, 0.9) *
                    pt::triple_product(
                        expected.second.response, float3(2.1, 0.4, 0.5),
                        pt::power_heuristic_over_pdf(0.37, continuation ? expected.second.pdf : 0));
    let expectedSequence = mask == 3u ? 6u : mask;
    outputs[id.x * 5] = float4(actual.direction - expected.scattered.direction,
                               actual.relative_eta - expected.scattered.relative_eta);
    outputs[id.x * 5 + 1] =
        float4(actual.response - expected.scattered.response, actual.pdf - expected.scattered.pdf);
    outputs[id.x * 5 + 2] = actual.medium - expected.scattered.medium;
    outputs[id.x * 5 + 3] =
        float4(consumer.radiance, float(actual.events != expected.scattered.events));
    outputs[id.x * 5 + 4] = float4(radiance, float(consumer.sequence != expectedSequence));
}

[numthreads(64, 1, 1)]
func primary_sampling_main(uint3 id: SV_DispatchThreadID)->void
{
    let n = uint(inputs[4].x);
    if (id.x >= n)
        return;
    let m : openpbr::Material = { inputs[0], inputs[1], uint4(uint(inputs[4].y), 0, 0, 0) };
    let incident : float4 = inputs[2];
    let transmitted : float4 = inputs[3];
    let view : float3 = normalize(float3(inputs[4].z, 0, inputs[4].w));
    let normal = float3(0, 0, 1);
    let random : pt::PathSampler = pt::path_sampler(uint2(id.x & 255u, id.x >> 8u), 0, 8, 3721);
    let u : float3 = random.bsdf(0);
    let one : openpbr::Sample =
                  openpbr::shade_primary_flow<OPENPBR_FLOW>(Energy(), m, normal, view, 0, 0, 0, u,
                                                            true, incident, transmitted, 1, 0)
                      .scattered;
    let a : openpbr::Sample =
                openpbr::shade_primary_flow<OPENPBR_FLOW>(Energy(), m, normal, view, 0, 0, 0, u,
                                                          true, incident, transmitted, 2, 0)
                    .scattered;
    let b : openpbr::Sample = openpbr::shade_primary_flow<OPENPBR_FLOW>(
                                  Energy(), m, normal, view, 0, 0, 0, random.bsdf(1), true,
                                  incident, transmitted, 2, 1)
                                  .scattered;
    let reference : openpbr::Sample = openpbr::sample_flow<OPENPBR_FLOW>(Energy(), m, normal, view,
                                                                         u, incident, transmitted);
    let proposed
        : openpbr::Evaluation =
              openpbr::shade_primary_flow<OPENPBR_FLOW>(Energy(), m, normal, view, one.direction, 0,
                                                        1, 0, false, incident, transmitted, 1, 0)
                  .first;
    let physical : openpbr::Evaluation = openpbr::evaluate_flow<OPENPBR_FLOW>(
                       Energy(), m, normal, view, one.direction, incident, transmitted);
    let responseError : float = length(proposed.response - physical.response) /
                                max(1.0, length(physical.response));
    let pdfError : float = one.events != 0 && (one.events & openpbr::DISCRETE) == 0
                               ? abs(proposed.pdf - one.pdf) / max(1.0, one.pdf)
                               : 0;
    // 连续MIS的互补性同时消费新提案PDF；不混入delta质量。
    let mis : float = proposed.pdf > 0 ? pt::power_heuristic(proposed.pdf, 0.37) +
                                             pt::power_heuristic(0.37, proposed.pdf)
                                       : 1;
    outputs[id.x * 5] = float4(weight(one), responseError);
    outputs[id.x * 5 + 1] = float4((weight(a) + weight(b)) * 0.5, pdfError);
    outputs[id.x * 5 + 2] = float4(weight(reference), abs(mis - 1));
    outputs[id.x * 5 + 3] = float4(float(a.events), float(b.events), a.pdf, b.pdf);
    outputs[id.x * 5 + 4] = float4(a.direction - b.direction, float(one.events));
}
