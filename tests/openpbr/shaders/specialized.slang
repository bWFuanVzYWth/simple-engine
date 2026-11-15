module specialized_fixture;
import bsdf.openpbr.api;
import bsdf.openpbr.specialized;

#ifndef OPENPBR_CAPABILITY
#error OPENPBR_CAPABILITY must select one compile-time OpenPBR capability
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
    let i : uint = id.x * 5;
    if (c.u.w == 2.0)
    {
        let evaluated : openpbr::Evaluation = openpbr::evaluate_specialized<OPENPBR_CAPABILITY>(
                            energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident,
                            c.transmitted);
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
        let control = uint(c.view.w);
        let shaded : openpbr::Batch = openpbr::shade_specialized<OPENPBR_CAPABILITY>(
                         energy, c.material, float3(0, 0, 1), c.view.xyz, float3(0.6, 0, 0.8),
                         float3(0.6, 0, -0.8), control & 3u, c.u.xyz, (control & 4u) != 0u,
                         c.incident, c.transmitted);
        let evaluated : openpbr::Evaluation = c.u.w == 3.0 ? shaded.first : shaded.second;
        output[i] = float4(shaded.scattered.direction, shaded.scattered.relative_eta);
        output[i + 1] = float4(shaded.scattered.response, shaded.scattered.pdf);
        output[i + 2] = asfloat(uint4(shaded.scattered.events, 0, id.x + 1, 0));
        output[i + 3] = float4(evaluated.response, evaluated.pdf);
        output[i + 4] = shaded.scattered.medium;
        return;
    }
    let sampled
        : openpbr::Sample = openpbr::sample_specialized<OPENPBR_CAPABILITY>(
              energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident, c.transmitted);
    let evaluated : openpbr::Evaluation = openpbr::evaluate_specialized<OPENPBR_CAPABILITY>(
                        energy, c.material, float3(0, 0, 1), c.view.xyz, sampled.direction,
                        c.incident, c.transmitted);
    output[i] = float4(sampled.direction, sampled.relative_eta);
    output[i + 1] = float4(sampled.response, sampled.pdf);
    output[i + 2] = asfloat(uint4(sampled.events, 0, id.x + 1, 0));
    output[i + 3] = float4(evaluated.response, evaluated.pdf);
    output[i + 4] = sampled.medium;
}
