module light_bvh_bench;

import pt.light_bvh;

[[vk::binding(0, 0)]]
StructuredBuffer<uint4> tree;
[[vk::binding(1, 0)]]
StructuredBuffer<uint> queries;
[[vk::binding(2, 0)]]
RWStructuredBuffer<uint4> outputs;

// 4-word 头为 count/light_count/tree_base/reserved；base 仍为 word 单位，调用时换算 uint4。
// 每项8 words，尾项为原查询序号。
// 两入口只调用生产算法；分 buffer 使树布局变化不影响查询工作集。
[numthreads(64, 1, 1)]
void light_bvh_sample_bench(uint3 id: SV_DispatchThreadID)
{
    if (id.x >= queries[0])
        return;
    uint at = 4u + id.x * 8u;
    float3 point = asfloat(uint3(queries[at], queries[at + 1u], queries[at + 2u]));
    pt::LightBvhSample selected =
        pt::sample_light_bvh(tree, queries[2] / 4u, queries[1], point, asfloat(queries[at + 3u]));
    outputs[id.x] = uint4(selected.light, asuint(selected.pmf), 0u, queries[at + 7u] + 1u);
}

[numthreads(64, 1, 1)]
void light_bvh_reverse_bench(uint3 id: SV_DispatchThreadID)
{
    if (id.x >= queries[0])
        return;
    uint at = 4u + id.x * 8u;
    float3 point = asfloat(uint3(queries[at], queries[at + 1u], queries[at + 2u]));
    uint light = queries[at + 4u];
    float pmf = pt::light_bvh_pmf(tree, queries[2] / 4u, queries[1], point, light);
    outputs[id.x] = uint4(light, asuint(pmf), 0u, queries[at + 7u] + 1u);
}
