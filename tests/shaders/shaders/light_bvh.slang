module light_bvh_test;

import light_bvh.sample;
import light_bvh.world;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;
[[vk::binding(2, 0)]]
var worlds : StructuredBuffer<light_bvh::LightWorld>;

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

[numthreads(64, 1, 1)]
func light_world_main(uint3 id: SV_DispatchThreadID)->void {
    let world = worlds[0];
    let width = inputs[0].x;
    let point_value = asfloat(inputs[id.y + 1u].xyz);
    let selector = (float(id.x) + 0.5) / float(width);
    let selected = light_bvh::sample_light_world(world, point_value, int3(0), selector);
    let entry = world.regions[selected.region];
    let selected_pmf =
        light_bvh::light_world_pmf(world, selected.region, selected.local, point_value, int3(0));
    var inverse = 0.0;
    if (id.x < world.counts.x) {
        let group = light_bvh::light_world_region_at(world, id.x);
        let region = world.regions[group];
        inverse =
            light_bvh::light_world_pmf(world, group, id.x - region.first, point_value, int3(0));
    }
    outputs[id.x + id.y * width] = uint4(entry.first + selected.local, asuint(selected.pmf),
                                         asuint(selected_pmf), asuint(inverse));
}

[numthreads(1, 1, 1)]
func light_world_power()->void {
    let world = worlds[0];
    let power = asfloat(world.topology[0].w) + asfloat(world.topology[1].w);
    outputs[0] = uint4(asuint(power), world.counts.xy, 0u);
}
