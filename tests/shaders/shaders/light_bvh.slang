module light_bvh_test;

import light_bvh.sample;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;

// uint4 头、双 uint4 queries、32B对齐的 CPU packet；不在 wrapper 中实现选灯或 PMF 数学。
[numthreads(64, 1, 1)]
func light_bvh_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x >= inputs[0].x)
        return;
    let at : uint = 1u + id.x * 2u;
    let a : uint4 = inputs[at];
    let b : uint4 = inputs[at + 1u];
    let base : uint = a.x;
    let lights : uint = a.y;
    let point : float3 = asfloat(uint3(a.zw, b.x));
    let selected : light_bvh::LightBvhSample =
                       light_bvh::sample_light_bvh(inputs, base, lights, point, asfloat(b.y));
    let reverse : float = light_bvh::light_bvh_pmf(inputs, base, lights, point, b.z);
    outputs[id.x] = uint4(selected.light, asuint(selected.pmf), asuint(reverse), id.x + 1u);
}
