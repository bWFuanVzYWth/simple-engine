module transport_test;
import pt.transport;
[[vk::binding(0, 0)]]
StructuredBuffer<float4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint4> outputs;
[shader("compute")]
[numthreads(64, 1, 1)]
void main(uint3 id: SV_DispatchThreadID)
{
    if (id.x != 0)
        return;
    float4 p = inputs[0];
    float contribution = pt::power_heuristic_over_pdf(p.x, p.y);
    outputs[0] = uint4(asuint(contribution), asuint(pt::product_over(p.z, p.z, p.z)),
                       asuint(pt::triple_product(float3(p.z), float3(p.z), p.w).x), 1u);
}

[shader("compute")]
[numthreads(64, 1, 1)]
void vectors(uint3 id: SV_DispatchThreadID)
{
    uint count, stride;
    inputs.GetDimensions(count, stride);
    if (id.x >= count / 3u)
        return;
    float4 first = inputs[id.x * 3u];
    float4 second = inputs[id.x * 3u + 1u];
    float2 pdf = inputs[id.x * 3u + 2u].xy;
    outputs[id.x * 2u] = asuint(float4(pt::product_over(first.xyz, second.xyz, first.w),
                                       pt::product_over(first.x, second.x, first.w)));
    outputs[id.x * 2u + 1u] = asuint(float4(pt::triple_product(first.xyz, second.xyz, second.w),
                                            pt::power_heuristic_over_pdf(pdf.x, pdf.y)));
}
