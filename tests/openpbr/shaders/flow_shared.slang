module specialized_fixture;
import bsdf.openpbr.api;
import bsdf.openpbr.flow;

#ifndef OPENPBR_FLOW
#error OPENPBR_FLOW must select one compile-time OpenPBR capability
#endif

struct Case
{
    openpbr::Material material;
    float4 view;
    float4 u;
    float4 incident;
    float4 transmitted;
};
[[vk::binding(0, 0)]]
StructuredBuffer<Case> cases;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> output;
[[vk::binding(2, 0)]]
Texture3D<float4> energy_texture;
[[vk::binding(3, 0)]]
SamplerState energy_sampler;

[shader("compute")]
[numthreads(64, 1, 1)]
void main(uint3 id: SV_DispatchThreadID)
{
    uint count, stride;
    cases.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    Case c = cases[id.x];
    openpbr::Energy energy = { energy_texture, energy_sampler };
#if OPENPBR_SHARED
    openpbr::Batch shaded = openpbr::shade_flow<OPENPBR_FLOW>(
        energy, c.material, float3(0, 0, 1), c.view.xyz, float3(0.6, 0, 0.8), float3(0.6, 0, -0.8),
        3u, c.u.xyz, true, c.incident, c.transmitted);
#else
    openpbr::Batch shaded = {};
    shaded.first =
        openpbr::evaluate_flow<OPENPBR_FLOW>(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                             float3(0.6, 0, 0.8), c.incident, c.transmitted);
    shaded.second =
        openpbr::evaluate_flow<OPENPBR_FLOW>(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                             float3(0.6, 0, -0.8), c.incident, c.transmitted);
    shaded.scattered = openpbr::sample_flow<OPENPBR_FLOW>(
        energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident, c.transmitted);
#endif
    uint i = id.x * 5;
    output[i] = float4(shaded.scattered.direction, shaded.scattered.relative_eta);
    output[i + 1] = float4(shaded.scattered.response, shaded.scattered.pdf);
    output[i + 2] = asfloat(uint4(shaded.scattered.events, 0, id.x + 1, 0));
    output[i + 3] = float4(shaded.first.response + shaded.second.response,
                           shaded.first.pdf + shaded.second.pdf);
    output[i + 4] = shaded.scattered.medium;
}
