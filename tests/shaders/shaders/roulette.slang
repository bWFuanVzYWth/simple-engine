module roulette_test;

import pt.roulette;

[[vk::binding(0, 0)]]
StructuredBuffer<uint4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint4> outputs;

// 每 case 两行：float bits beta.xyz/eta_scale；relative_eta/传输标记/completed/minimum。
[numthreads(64, 1, 1)]
void roulette_probability_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    float4 a = asfloat(inputs[2 * id.x]);
    uint4 b = inputs[2 * id.x + 1];
    float probability = pt::roulette_probability(a.xyz, a.w);
    float scale = pt::eta_scale_after_scatter(a.w, asfloat(b.x), b.y != 0);
    outputs[id.x] =
        uint4(asuint(probability), id.x + 1, asuint(scale), uint(pt::roulette_applies(b.z, b.w)));
}

// 第二行 x 为均匀离散积分的格数 N；中点样本精确积分 N 格上的 Bernoulli 期望。
[numthreads(64, 1, 1)]
void roulette_mean_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    float4 a = asfloat(inputs[2 * id.x]);
    uint count = inputs[2 * id.x + 1].x;
    float probability = pt::roulette_probability(a.xyz, a.w);
    float3 mean = 0;
    if (probability == 1)
        mean = a.xyz;
    else if (probability > 0)
    {
        for (uint i = 0; i < count; ++i)
        {
            float u = (float(i) + 0.5) / float(count);
            if (u < probability)
                mean += pt::roulette_reweight(a.xyz, probability);
        }
        mean /= float(count);
    }
    outputs[id.x] = uint4(asuint(mean.x), id.x + 1, asuint(mean.y), asuint(mean.z));
}

// 第一行是空气中的 beta.xyz 与 eta_t/eta_i；第二行未用。
// 理想透射的 radiance 因子 1/eta² 应被 RR eta_scale 抵消，出介质后恢复原 metric。
[numthreads(64, 1, 1)]
void roulette_eta_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    float4 a = asfloat(inputs[2 * id.x]);
    float eta = a.w;
    float square = eta * eta;
    float scale = pt::eta_scale_after_scatter(1, eta, true);
    float3 inside = a.xyz / square;
    float inside_probability = pt::roulette_probability(inside, scale);
    float restored = pt::eta_scale_after_scatter(scale, 1.0 / eta, true);
    float restored_probability = pt::roulette_probability(inside * square, restored);
    float reflected = pt::eta_scale_after_scatter(1, eta, false);
    outputs[id.x] = uint4(asuint(inside_probability), id.x + 1, asuint(restored_probability),
                          asuint(reflected));
}
