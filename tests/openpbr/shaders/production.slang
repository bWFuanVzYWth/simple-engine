module production;
import bsdf.openpbr.api;
import bsdf.openpbr.denoise;

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
    if (c.u.w == 5.0)
    {
        openpbr::DenoiseAlbedos value = openpbr::denoise_albedos(
            energy, c.material, float3(0, 0, 1), c.view.xyz, c.incident, c.transmitted);
        uint i = id.x * 5;
        output[i] = float4(value.diffuse, 0);
        output[i + 1] = float4(value.specular, 0);
        output[i + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
        return;
    }
    if (c.u.w == 1.0)
    {
        uint i = id.x * 5;
        output[i] = energy.sampleLevel(c.view.xyz);
        output[i + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
        return;
    }
    if (c.u.w == 2.0)
    {
        uint i = id.x * 5;
        openpbr::Evaluation evaluated = openpbr::evaluate(
            energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident, c.transmitted);
        output[i + 2] = asfloat(uint4(0,
                                      openpbr::may_evaluate(c.material, float3(0, 0, 1), c.view.xyz,
                                                            c.u.xyz, c.incident, c.transmitted),
                                      id.x + 1, 0));
        output[i + 3] = float4(evaluated.response, evaluated.pdf);
        return;
    }
    openpbr::Sample s = openpbr::sample(energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz,
                                        c.incident, c.transmitted);
    openpbr::Evaluation e = openpbr::evaluate(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                              s.direction, c.incident, c.transmitted);
    uint i = id.x * 5;
    output[i] = float4(s.direction, s.relative_eta);
    output[i + 1] = float4(s.response, s.pdf);
    output[i + 2] = asfloat(uint4(
        s.events, openpbr::is_discrete_only(c.material, c.incident, c.transmitted), id.x + 1, 0));
    output[i + 3] = float4(e.response, e.pdf);
    output[i + 4] = s.medium;
}
