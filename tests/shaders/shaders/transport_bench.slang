/// 两策略 MIS 权重，覆盖普通、零值以及跨指数范围的 PDF。
module transport_bench;
import sampling.pcg;
import transport.weights;

[[vk::binding(0, 0)]]
var seed : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

func pdf(bits: uint, wide: bool)->float {
    let mantissa = bits & 0x007fffffu;
    let exponent = (bits >> 23) % 255u;
    return wide ? asfloat((exponent << 23) | mantissa) : asfloat(0x3f800000u | mantissa) - 1.0;
}

[shader("compute")]
[numthreads(8, 8, 1)]
func mis_main(id: uint3: SV_DispatchThreadID)->void {
    let pixel = id.y * 3840 + id.x;
    var state = pixel ^ seed[0].x;
    var sum = float2(0.0);
    [loop]
    for (var draw = 0u; draw < 32; ++draw) {
        state = sampling::pcg(state);
        let first = pdf(state, draw >= 24);
        state = sampling::pcg(state);
        let second = pdf(state, draw >= 24);
        sum += float2(transport::power_heuristic(first, second),
                      transport::power_heuristic(second, first));
    }
    outputs[pixel] = float4(sum, 0.0, 1.0);
}
