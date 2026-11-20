module roulette_test;

import transport.roulette;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;

// 每 case 两行：float bits beta.xyz/eta_scale；relative_eta/标记/completed/minimum。
// 标记非零表示透射。
[numthreads(64, 1, 1)]
func roulette_probability_main(uint3 id: SV_DispatchThreadID)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let a : float4 = asfloat(inputs[2 * id.x]);
    let b : uint4 = inputs[2 * id.x + 1];
    let probability : float = transport::roulette_probability(a.xyz, a.w);
    let scale : float = transport::eta_scale_after_scatter(a.w, asfloat(b.x), b.y != 0);
    outputs[id.x] = uint4(asuint(probability), id.x + 1, asuint(scale),
                          uint(transport::roulette_applies(b.z, b.w)));
}

// 第二行 x 为均匀离散积分的格数 N；中点样本精确积分 N 格上的 Bernoulli 期望。
[numthreads(64, 1, 1)]
func roulette_mean_main(uint3 id: SV_DispatchThreadID)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let a : float4 = asfloat(inputs[2 * id.x]);
    let count : uint = inputs[2 * id.x + 1].x;
    let probability = transport::roulette_probability(a.xyz, a.w);
    var mean : float3 = 0;
    if (probability == 1)
        mean = a.xyz;
    else if (probability > 0) {
        for (var i: uint = 0; i < count; ++i) {
            let u : float = (float(i) + 0.5) / float(count);
            if (u < probability)
                mean += transport::roulette_reweight(a.xyz, probability);
        }
        mean /= float(count);
    }
    outputs[id.x] = uint4(asuint(mean.x), id.x + 1, asuint(mean.y), asuint(mean.z));
}

// 第一行是空气中的 beta.xyz 与 eta_t/eta_i。
// 理想透射的 radiance 因子 1/eta² 应被 RR eta_scale 抵消，出介质后恢复原 metric。
[numthreads(64, 1, 1)]
func roulette_eta_main(uint3 id: SV_DispatchThreadID)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let a : float4 = asfloat(inputs[2 * id.x]);
    let eta : float = a.w;
    let square : float = eta * eta;
    let scale : float = transport::eta_scale_after_scatter(1, eta, true);
    let inside : float3 = a.xyz / square;
    let inside_probability : float = transport::roulette_probability(inside, scale);
    let restored : float = transport::eta_scale_after_scatter(scale, 1.0 / eta, true);
    let restored_probability : float = transport::roulette_probability(inside * square, restored);
    let reflected : float = transport::eta_scale_after_scatter(1, eta, false);
    outputs[id.x] = uint4(asuint(inside_probability), id.x + 1, asuint(restored_probability),
                          asuint(reflected));
}
