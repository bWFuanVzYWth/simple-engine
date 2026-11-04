/// 相关的两份无偏随机估计；生产输出选择只使用另一独立随机域。
module fixed_mixture_test;
import restir_pt.fixed_mixture;
import restir_pt.rng;

[[vk::binding(0, 0)]]
StructuredBuffer<uint4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> outputs;

[shader("compute")]
[numthreads(64, 1, 1)]
void fixed_mixture_main(uint3 id: SV_DispatchThreadID)
{
    uint sample = id.x;
    if (sample >= inputs[0].z)
        return;
    uint2 pixel = uint2(9, 5);
    restir_pt::TinyUniformSampleGenerator source =
        restir_pt::rng_for_pixel(pixel, 3u * (sample + inputs[0].y));
    float sign = source.next1D() < 0.5 ? -1.0 : 1.0;
    float3 delta = sign * float3(3.0, -4.0, 6.0);
    float3 mean = float3(1.0, 2.0, 3.0);
    bool fresh = restir_pt::fixed_mixture_fresh(pixel, sample + inputs[0].y, asfloat(inputs[0].x));
    outputs[sample] = float4(fresh ? mean + delta : mean - delta, fresh ? 1.0 : 0.0);
}
