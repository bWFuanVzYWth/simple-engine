/// PT 后处理实际的一、二样本累计和 HDR 归一，混入少量已失败、非有限和溢出路径。
module pt_bench;
import pt.accumulation;

[[vk::binding(0, 0)]]
var seed : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

func accumulate<let paths : uint>(id: uint2)->void {
    let pixel = id.y * 3840 + id.x;
    let variant = (pixel ^ seed[0].x) & 1023u;
    let screen = float2(id) / float2(3840, 2160);
    var count = 1u + (variant & 15u);
    var sum = float4(float3(screen, 0.5) * float(count), variant == 0u ? -1.0 : 0.0);
    var radiance = float3(0.25 + screen.y, screen.x * screen.y, screen.x + 0.1);
    if (variant == 2u)
        radiance.y = asfloat(0x7f800000u);
    if (variant == 3u) {
        sum.x = 3e38;
        radiance.x = 3e38;
    }
    let error = variant == 1u ? -3.0 : 0.0;
    [unroll]
    for (var lane = 0u; lane < paths; ++lane)
        pt::accumulate_sample(sum, count, radiance, error);
    let resolved = pt::resolve_linear_hdr(sum, count);
    outputs[pixel] = float4(resolved.xyz, resolved.w + float(count) * 0.0625);
}

[shader("compute")]
[numthreads(8, 8, 1)]
func single_sample_main(id: uint3: SV_DispatchThreadID)->void {
    accumulate<1u>(id.xy);
}

[shader("compute")]
[numthreads(8, 8, 1)]
func dual_sample_main(id: uint3: SV_DispatchThreadID)->void {
    accumulate<2u>(id.xy);
}
