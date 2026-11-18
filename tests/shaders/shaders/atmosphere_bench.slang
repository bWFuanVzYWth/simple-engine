/// 对四波长输运分别测近地短步与天空长步，范围取自 Earth 剖面和运行时积分步长。
module atmosphere_bench;
import atmosphere.sampling;
import atmosphere.terrain_frame;

[[vk::binding(0, 0)]]
var seed : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

func integrate_segments<let terrain : bool>(id: uint2)->float4 {
    let range = atmosphere::TERRAIN_RANGE_M * 0.001;
    let depth = atmosphere::TERRAIN_DEPTH;
    let layer = (id.x + (seed[0].x & 15u)) % (depth - 15u);
    let height = float(id.y) / 2160.0;
    // 对应介质表 0、1 和 24 km 的消光系数，保留四个通道跨近零阈值的情况。
    let dense = float4(0.10414, 0.08079, 0.06316, 0.05078);
    let sigma = terrain ? lerp(dense, float4(0.09277, 0.07184, 0.05610, 0.04505), height)
                        : lerp(dense, float4(0.00147, 0.00160, 0.00262, 0.00148), height);
    var light = float4(0.0);
    var transmission = float4(1.0);
    [loop]
    for (var step = 0u; step < 16u; ++step) {
        let z = float(layer + step);
        let dt = terrain ? (2.0 * z + 1.0) * (0.5 * range / float(depth * depth))
                         : lerp(0.01, 2.0, z / float(depth));
        let segment = atmosphere::segment_transport(sigma, dt);
        light += transmission * segment.integral;
        transmission *= segment.transmittance;
    }
    return light + transmission;
}

[shader("compute")]
[numthreads(8, 8, 1)]
func terrain_segments_main(id: uint3: SV_DispatchThreadID)->void {
    outputs[id.y * 3840 + id.x] = integrate_segments<true>(id.xy);
}

[shader("compute")]
[numthreads(8, 8, 1)]
func sky_segments_main(id: uint3: SV_DispatchThreadID)->void {
    outputs[id.y * 3840 + id.x] = integrate_segments<false>(id.xy);
}
