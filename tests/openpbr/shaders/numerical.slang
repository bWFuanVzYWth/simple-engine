/// 无适配器拒绝的数值核；解析极限由 CPU f64 独立核对。
module numerical_test;
import openpbr.api;
import "openpbr/math/microfacet_distribution.slang";
import "openpbr/contract/material.slang";
import "openpbr/contract/sample.slang";
import "openpbr/math/fresnel.slang";
import "openpbr/math/microfacet_energy.slang";
struct Case {
    var material : openpbr::Material;
    var view : float4;
    var u : float4;
    var incident : float4;
    var transmitted : float4;
};
[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<Case>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;
[[vk::binding(2, 0)]]
var energy_texture : Texture3D<float4>;
[[vk::binding(3, 0)]]
var energy_sampler : SamplerState;
[shader("compute")]
[numthreads(64, 1, 1)]
func main(uint3 id: SV_DispatchThreadID)->void {
    var count : uint;
    var stride : uint;
    inputs.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    let c : Case = inputs[id.x];
    let m : openpbr::kernel::Microfacet = { float2(c.material.base_roughness.w *
                                                   c.material.base_roughness.w) };
    outputs[5 * id.x] =
        float4(c.u.w == 0 ? openpbr::kernel::ggx_transmission_masking(m, c.view.xyz, c.u.xyz) : 0,
               openpbr::kernel::reflection_factor(m, c.view.xyz, c.u.xyz), 0, 0);
    outputs[5 * id.x + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
}

[shader("compute")]
[numthreads(64, 1, 1)]
func math_main(uint3 id: SV_DispatchThreadID)->void {
    var count : uint;
    var stride : uint;
    inputs.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    let c = inputs[id.x];
    let m = openpbr::kernel::Microfacet(
        float2(c.material.base_roughness.w * c.material.base_roughness.w));
    let distribution = openpbr::kernel::ggx_distribution<true>(m, c.u.xyz);
    outputs[5 * id.x] = float4(
        distribution, openpbr::kernel::ggx_reflection_pdf(m, c.view.xyz, true, distribution),
        openpbr::kernel::dielectric_reflectance(c.incident.x / c.transmitted.x, c.view.z), 0);
    outputs[5 * id.x + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
}

[shader("compute")]
[numthreads(64, 1, 1)]
func purpose_main(uint3 id: SV_DispatchThreadID)->void {
    var count : uint;
    var stride : uint;
    inputs.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    let c = inputs[id.x];
    let roughness = c.material.base_roughness.w;
    let m = openpbr::kernel::Microfacet(float2(roughness * roughness));
    let energy = openpbr::Energy(energy_texture, energy_sampler);
    let ior = c.incident.x / c.transmitted.x;
    outputs[5 * id.x] = float4(openpbr::kernel::dielectric_energy(energy, m, c.view.z, ior), 0);
    outputs[5 * id.x + 1] = float4(openpbr::kernel::dielectric_energy<false, true, true, true>(
                                       energy, m, c.view.z, ior, roughness),
                                   0);
    let smooth = openpbr::kernel::Microfacet(float2(0));
    outputs[5 * id.x + 3] =
        float4(openpbr::kernel::dielectric_energy<true>(energy, smooth, c.view.z, ior).z,
               openpbr::kernel::dielectric_energy<true, false, false, true>(
                   energy, smooth, c.view.z, ior, roughness)
                   .z,
               0, 0);
    outputs[5 * id.x + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
}
