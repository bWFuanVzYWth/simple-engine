module production;
import bsdf.openpbr.api;
import bsdf.openpbr.denoise;

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
    if (c.u.w == 5.0)
    {
        let value : openpbr::DenoiseAlbedos = openpbr::denoise_albedos(
                        energy, c.material, float3(0, 0, 1), c.view.xyz, c.incident, c.transmitted);
        let i : uint = id.x * 5;
        output[i] = float4(value.diffuse, 0);
        output[i + 1] = float4(value.specular, 0);
        output[i + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
        return;
    }
    if (c.u.w == 1.0)
    {
        let i : uint = id.x * 5;
        output[i] = energy.sampleLevel(c.view.xyz);
        output[i + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
        return;
    }
    if (c.u.w == 2.0)
    {
        let i : uint = id.x * 5;
        let evaluated : openpbr::Evaluation =
                            openpbr::evaluate(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                              c.u.xyz, c.incident, c.transmitted);
        output[i + 2] = asfloat(uint4(0,
                                      openpbr::may_evaluate(c.material, float3(0, 0, 1), c.view.xyz,
                                                            c.u.xyz, c.incident, c.transmitted),
                                      id.x + 1, 0));
        output[i + 3] = float4(evaluated.response, evaluated.pdf);
        return;
    }
    let s : openpbr::Sample = openpbr::sample(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                              c.u.xyz, c.incident, c.transmitted);
    let e : openpbr::Evaluation = openpbr::evaluate(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                                    s.direction, c.incident, c.transmitted);
    let i : uint = id.x * 5;
    output[i] = float4(s.direction, s.relative_eta);
    output[i + 1] = float4(s.response, s.pdf);
    output[i + 2] = asfloat(uint4(
        s.events, openpbr::is_discrete_only(c.material, c.incident, c.transmitted), id.x + 1, 0));
    output[i + 3] = float4(e.response, e.pdf);
    output[i + 4] = s.medium;
}
