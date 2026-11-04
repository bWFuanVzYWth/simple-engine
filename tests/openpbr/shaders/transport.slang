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
