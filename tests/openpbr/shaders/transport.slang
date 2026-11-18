module transport_test;
import transport.weights;
[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;
[shader("compute")]
[numthreads(64, 1, 1)]
func main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x != 0)
        return;
    let p : float4 = inputs[0];
    let contribution : float = transport::power_heuristic_over_pdf(p.x, p.y);
    outputs[0] = uint4(asuint(contribution), asuint(transport::product_over(p.z, p.z, p.z)),
                       asuint(transport::triple_product(float3(p.z), float3(p.z), p.w).x), 1u);
}

[shader("compute")]
[numthreads(64, 1, 1)]
func vectors(uint3 id: SV_DispatchThreadID)->void {
    var count : uint;
    var stride : uint;
    inputs.GetDimensions(count, stride);
    if (id.x >= count / 3u)
        return;
    let first : float4 = inputs[id.x * 3u];
    let second : float4 = inputs[id.x * 3u + 1u];
    let pdf : float2 = inputs[id.x * 3u + 2u].xy;
    outputs[id.x * 2u] = asuint(float4(transport::product_over(first.xyz, second.xyz, first.w),
                                       transport::product_over(first.x, second.x, first.w)));
    outputs[id.x * 2u + 1u] =
        asuint(float4(transport::triple_product(first.xyz, second.xyz, second.w),
                      transport::power_heuristic_over_pdf(pdf.x, pdf.y)));
}
