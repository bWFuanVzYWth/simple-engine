module light_bvh_test;

import pt.light_bvh;

[[vk::binding(0, 0)]]
StructuredBuffer<uint4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint4> outputs;

// uint4 头、双 uint4 queries、32B对齐的 CPU packet；不在 wrapper 中实现选灯或 PMF 数学。
[numthreads(64, 1, 1)]
void light_bvh_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x >= inputs[0].x)
        return;
    uint at = 1u + id.x * 2u;
    uint4 a = inputs[at], b = inputs[at + 1u];
    uint base = a.x, lights = a.y;
    float3 point = asfloat(uint3(a.zw, b.x));
    pt::LightBvhSample selected = pt::sample_light_bvh(inputs, base, lights, point, asfloat(b.y));
    float reverse = pt::light_bvh_pmf(inputs, base, lights, point, b.z);
    outputs[id.x] = uint4(selected.light, asuint(selected.pmf), asuint(reverse), id.x + 1u);
}
