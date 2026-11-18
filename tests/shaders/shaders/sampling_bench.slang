/// PT 在一个 epoch 内的 Binary Sobol S8 二维、三维与标量消费。
module sampling_bench;
import sampling.binary_sobol;

[[vk::binding(0, 0)]]
var seed : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

[shader("compute")]
[numthreads(8, 8, 1)]
func sobol_4k_main(id: uint3: SV_DispatchThreadID)->void {
    var sum = float4(0.0);
    [loop]
    for (var draw = 0u; draw < 16; ++draw) {
        let sample = sampling::binary_sobol_s8(id.xy, 32 + draw, 12, seed[0].x);
        let pair = sample.sample_2d(0);
        let light = sample.sample_3d(11, 10);
        let scatter = sample.sample_3d(12, 13);
        sum += float4(light + scatter + float3(pair, 0.0), sample.sample_1d(14));
    }
    outputs[id.y * 3840 + id.x] = sum;
}
