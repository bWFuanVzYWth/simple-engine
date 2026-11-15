/// 相关的两份无偏随机估计；生产输出选择只使用另一独立随机域。
module fixed_mixture_test;
import restir_pt.fixed_mixture;
import restir_pt.rng;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

[shader("compute")]
[numthreads(64, 1, 1)]
func fixed_mixture_main(uint3 id: SV_DispatchThreadID)->void
{
    let sample : uint = id.x;
    if (sample >= inputs[0].z)
        return;
    let pixel = uint2(9, 5);
    var source : restir_pt::TinyUniformSampleGenerator =
                     restir_pt::rng_for_pixel(pixel, 3u * (sample + inputs[0].y));
    let sign : float = source.next1D() < 0.5 ? -1.0 : 1.0;
    let delta : float3 = sign * float3(3.0, -4.0, 6.0);
    let mean = float3(1.0, 2.0, 3.0);
    let fresh
        : bool = restir_pt::fixed_mixture_fresh(pixel, sample + inputs[0].y, asfloat(inputs[0].x));
    outputs[sample] = float4(fresh ? mean + delta : mean - delta, fresh ? 1.0 : 0.0);
}
