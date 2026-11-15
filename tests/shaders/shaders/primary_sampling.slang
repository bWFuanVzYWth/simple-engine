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
