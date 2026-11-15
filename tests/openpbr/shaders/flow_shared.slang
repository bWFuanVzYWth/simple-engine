module specialized_fixture;
import bsdf.openpbr.api;
import bsdf.openpbr.flow;

#ifndef OPENPBR_FLOW
#error OPENPBR_FLOW must select one compile-time OpenPBR capability
#endif

struct Case
{
    var material : openpbr::Material;
    var view : float4;
    var u : float4;
    var incident : float4;
    var transmitted : float4;
};
[[vk::binding(0, 0)]]
var cases : StructuredBuffer<Case>;
[[vk::binding(1, 0)]]
var output : RWStructuredBuffer<float4>;
[[vk::binding(2, 0)]]
var energy_texture : Texture3D<float4>;
[[vk::binding(3, 0)]]
var energy_sampler : SamplerState;

struct EvaluationSum : openpbr::IFlowEvaluationConsumer
{
    var evaluated : openpbr::Evaluation;

    [mutating]
    func consume(direction: uint, value: openpbr::Evaluation)->void
    {
        evaluated.response += value.response;
        evaluated.pdf += value.pdf;
    }
};

func shade_case<let Shared : uint>(energy: openpbr::Energy, c: Case)->openpbr::Batch
{
    if (Shared == 1)
        return openpbr::shade_flow<OPENPBR_FLOW>(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                                 float3(0.6, 0, 0.8), float3(0.6, 0, -0.8), 3u,
                                                 c.u.xyz, true, c.incident, c.transmitted);
    if (Shared == 2)
    {
        var consumer : EvaluationSum = {};
        var shaded : openpbr::Batch = {};
        shaded.scattered =
            openpbr::shade_flow_consume<OPENPBR_FLOW, openpbr::Energy, EvaluationSum>(
                energy, c.material, float3(0, 0, 1), c.view.xyz, float3(0.6, 0, 0.8),
                float3(0.6, 0, -0.8), 3u, c.u.xyz, true, c.incident, c.transmitted, consumer);
        shaded.first = consumer.evaluated;
        return shaded;
    }
    var shaded : openpbr::Batch = {};
    shaded.first =
        openpbr::evaluate_flow<OPENPBR_FLOW>(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                             float3(0.6, 0, 0.8), c.incident, c.transmitted);
    shaded.second =
        openpbr::evaluate_flow<OPENPBR_FLOW>(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                             float3(0.6, 0, -0.8), c.incident, c.transmitted);
    shaded.scattered = openpbr::sample_flow<OPENPBR_FLOW>(
        energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident, c.transmitted);
    return shaded;
}

[shader("compute")]
[numthreads(64, 1, 1)]
func main(uint3 id: SV_DispatchThreadID)->void
{
    var count : uint;
    var stride : uint;
    cases.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    let c : Case = cases[id.x];
    let energy : openpbr::Energy = { energy_texture, energy_sampler };
    let shaded = shade_case<OPENPBR_SHARED>(energy, c);
    let i : uint = id.x * 5;
    output[i] = float4(shaded.scattered.direction, shaded.scattered.relative_eta);
    output[i + 1] = float4(shaded.scattered.response, shaded.scattered.pdf);
    output[i + 2] = asfloat(uint4(shaded.scattered.events, 0, id.x + 1, 0));
    output[i + 3] = float4(shaded.first.response + shaded.second.response,
                           shaded.first.pdf + shaded.second.pdf);
    output[i + 4] = shaded.scattered.medium;
}
