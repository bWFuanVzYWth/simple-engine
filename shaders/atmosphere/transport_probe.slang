#include "compute.slang"
[[vk::binding(8, 0)]]
var probe_queries : StructuredBuffer<float4>;
[[vk::binding(9, 0)]]
var probe_outputs : RWStructuredBuffer<float4>;

// 独立测试入口：每查询两行 ray.xyz/segmentKm、sun.xyz/initialHeightKm；输出 L4、T4。
[shader("compute")]
[numthreads(64, 1, 1)]
func transport_probe(uint3 id: SV_DispatchThreadID)->void
{
    var rows : uint;
    var stride : uint;
    probe_queries.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let ray : float4 = probe_queries[2 * id.x];
    let sun : float4 = probe_queries[2 * id.x + 1];
    let result : Transport = integrate_at(ray.xyz, sun.xyz, sun.w, ray.w, p.size_steps.z);
    probe_outputs[2 * id.x] = result.light;
    probe_outputs[2 * id.x + 1] = result.transmittance;
}
