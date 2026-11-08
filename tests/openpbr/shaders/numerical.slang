/// 无适配器拒绝的数值核；解析极限由 CPU f64 独立核对。
module numerical_test;
import bsdf.openpbr.api;
import "bsdf/openpbr/math/microfacet_distribution.slang";
import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";
struct Case
{
    openpbr::Material material;
    float4 view;
    float4 u;
    float4 incident;
    float4 transmitted;
};
[[vk::binding(0, 0)]]
StructuredBuffer<Case> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> outputs;
[shader("compute")]
[numthreads(64, 1, 1)]
void main(uint3 id: SV_DispatchThreadID)
{
    uint count, stride;
    inputs.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    Case c = inputs[id.x];
    PrimeFullOpenPbrMicrofacet m = { float2(c.material.base_roughness.w *
                                            c.material.base_roughness.w) };
    outputs[5 * id.x] =
        float4(c.u.w == 0 ? primeFullOpenPbrMicrofacetG2t(m, c.view.xyz, c.u.xyz) : 0,
               primeFullOpenPbrReflectionFactor(m, c.view.xyz, c.u.xyz), 0, 0);
    outputs[5 * id.x + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
}
