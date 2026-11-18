/// 软件光追的细体素推进与均匀区域跳跃，包含平行轴和等时边界。
module voxel_bench;
import voxel.dag;

[[vk::binding(0, 0)]]
var seed : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

[shader("compute")]
[numthreads(8, 8, 1)]
func dda_main(id: uint3: SV_DispatchThreadID)->void {
    let pixel = id.y * 3840 + id.x;
    let screen = float2(id.xy) / float2(3840, 2160);
    let origin =
        split_origin(float3(256.5 + floor(screen.x * 64.0), 256.5, 256.5 + floor(screen.y * 64.0)));
    let variant = (id.x ^ id.y ^ seed[0].x) & 31u;
    var dir = float3(screen.x * 2.0 - 1.0, 1.0, screen.y * 2.0 - 1.0);
    if (variant == 0)
        dir = float3(1.0, 1.0, 0.0);
    let inverse = select(dir != 0.0, 1.0 / select(dir != 0.0, dir, float3(1.0)), float3(0.0));
    var state : DdaState;
    var end : float;
    if (!begin_dda(origin, dir, int3(0), int3(1024), 512.0, state, end)) {
        outputs[pixel] = float4(0.0);
        return;
    }
    var total = 0.0;
    [loop]
    for (var step = 0u; step < 24u && state.t < end; ++step) {
        total += next_t(state, inverse, origin);
        let lod = (step + variant) & 7u;
        if (lod < 2u) {
            let shift = (lod + 1u) * 2u;
            let region_min = (state.voxel >> shift) << shift;
            state = skip_region(state, inverse, origin, region_min, 1u << shift);
        } else
            state = advance_one(state, inverse, origin);
    }
    outputs[pixel] = float4(float3(state.voxel) + float(state.axis) * 0.25, total);
}
