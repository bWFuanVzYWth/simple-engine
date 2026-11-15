module cone_test;
import geometry.cone;
[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;
[shader("compute")]
[numthreads(64, 1, 1)]
func cone_main(uint3 id: SV_DispatchThreadID)->void
{
    var count : uint;
    var stride : uint;
    inputs.GetDimensions(count, stride);
    if (2 * id.x >= count)
        return;
    let cone : float4 = inputs[2 * id.x];
    let d : float3 = geometry::sample_uniform_cone(cone.xyz, cone.w, inputs[2 * id.x + 1].xy);
    outputs[2 * id.x] = float4(d, geometry::uniform_cone_pdf(cone.w));
    outputs[2 * id.x + 1] = float4(geometry::cone_contains(cone.xyz, cone.w, d), 0, 0, 0);
}
