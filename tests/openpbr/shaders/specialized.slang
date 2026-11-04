module specialized_fixture;
import bsdf.openpbr.api;
import bsdf.openpbr.specialized;

#ifndef OPENPBR_CAPABILITY
#error OPENPBR_CAPABILITY must select one compile-time OpenPBR capability
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
    uint i = id.x * 5;
    if (c.u.w == 2.0)
    {
        openpbr::Evaluation evaluated = openpbr::evaluate_specialized<OPENPBR_CAPABILITY>(
            energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident, c.transmitted);
        output[i + 2] = asfloat(
            uint4(0,
                  openpbr::may_evaluate_specialized<OPENPBR_CAPABILITY>(
                      c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident, c.transmitted),
                  id.x + 1, 0));
        output[i + 3] = float4(evaluated.response, evaluated.pdf);
        return;
    }
    if (c.u.w == 3.0 || c.u.w == 4.0)
    {
        uint control = uint(c.view.w);
        openpbr::Batch shaded = openpbr::shade_specialized<OPENPBR_CAPABILITY>(
            energy, c.material, float3(0, 0, 1), c.view.xyz, float3(0.6, 0, 0.8),
            float3(0.6, 0, -0.8), control & 3u, c.u.xyz, (control & 4u) != 0u, c.incident,
            c.transmitted);
        openpbr::Evaluation evaluated = c.u.w == 3.0 ? shaded.first : shaded.second;
        output[i] = float4(shaded.scattered.direction, shaded.scattered.relative_eta);
        output[i + 1] = float4(shaded.scattered.response, shaded.scattered.pdf);
        output[i + 2] = asfloat(uint4(shaded.scattered.events, 0, id.x + 1, 0));
        output[i + 3] = float4(evaluated.response, evaluated.pdf);
        output[i + 4] = shaded.scattered.medium;
        return;
    }
    openpbr::Sample sampled = openpbr::sample_specialized<OPENPBR_CAPABILITY>(
        energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident, c.transmitted);
    openpbr::Evaluation evaluated = openpbr::evaluate_specialized<OPENPBR_CAPABILITY>(
        energy, c.material, float3(0, 0, 1), c.view.xyz, sampled.direction, c.incident,
        c.transmitted);
    output[i] = float4(sampled.direction, sampled.relative_eta);
    output[i + 1] = float4(sampled.response, sampled.pdf);
    output[i + 2] = asfloat(uint4(sampled.events, 0, id.x + 1, 0));
    output[i + 3] = float4(evaluated.response, evaluated.pdf);
    output[i + 4] = sampled.medium;
}
