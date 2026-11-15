/// 无适配器拒绝的数值核；解析极限由 CPU f64 独立核对。
module numerical_test;
import bsdf.openpbr.api;
import "bsdf/openpbr/math/microfacet_distribution.slang";
import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";
struct Case
{
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
[shader("compute")]
[numthreads(64, 1, 1)]
func main(uint3 id: SV_DispatchThreadID)->void
{
    var count : uint;
    var stride : uint;
    inputs.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    let c : Case = inputs[id.x];
    let m : PrimeFullOpenPbrMicrofacet = { float2(c.material.base_roughness.w *
                                                  c.material.base_roughness.w) };
    outputs[5 * id.x] =
        float4(c.u.w == 0 ? primeFullOpenPbrMicrofacetG2t(m, c.view.xyz, c.u.xyz) : 0,
               primeFullOpenPbrReflectionFactor(m, c.view.xyz, c.u.xyz), 0, 0);
    outputs[5 * id.x + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
}
