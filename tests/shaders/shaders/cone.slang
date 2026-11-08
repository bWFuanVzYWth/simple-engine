module cone_test;
import geometry.cone;
[[vk::binding(0, 0)]]
StructuredBuffer<float4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> outputs;
[shader("compute")]
[numthreads(64, 1, 1)]
void cone_main(uint3 id: SV_DispatchThreadID)
{
    uint count, stride;
    inputs.GetDimensions(count, stride);
    if (2 * id.x >= count)
        return;
    float4 cone = inputs[2 * id.x];
    float3 d = geometry::sample_uniform_cone(cone.xyz, cone.w, inputs[2 * id.x + 1].xy);
    outputs[2 * id.x] = float4(d, geometry::uniform_cone_pdf(cone.w));
    outputs[2 * id.x + 1] = float4(geometry::cone_contains(cone.xyz, cone.w, d), 0, 0, 0);
}
