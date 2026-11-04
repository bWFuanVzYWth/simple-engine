/// 有限分片积分与双色 Bernoulli 历史；直接检查 CV 的积分和方差，不依赖应用场景。
module control_variate_test;
import restir_pt.control_variate;
import restir_pt.reservoir;
using namespace restir_pt;

[[vk::binding(0, 0)]]
StructuredBuffer<float4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> outputs;

PathReservoir source(float3 value, float weight, float mass)
{
    PathReservoir result = empty_reservoir();
    result.integrand = value;
    result.weight = weight;
    result.M = mass;
    return result;
}

PrecomputedShiftData shifted(float3 value, float Jacobian)
{
    return { value, Jacobian, 0 };
}

void boundaries()
{
    outputs[0] = float4(cv_alpha(float3(0, 0.5, 3), float3(0, 0.25, 1)), 0);
    CVAccumulator colors;
    colors.init(float3(3, 5, 7), 0);
    colors.add(float3(100), 2);
    outputs[1] = float4(colors.resolve(), colors.mass);
    colors.init(float3(3, 5, 7), 1);
    colors.add(float3(100), 0);
    outputs[2] = float4(colors.resolve(), colors.mass);

    // 零 target 的源是合法零贡献抽样，不等于缺失邻居；其颜色仍可承载历史。
    PathReservoir canonical = source(float3(0), 0, 1);
    PathReservoir neighbor = source(float3(0), 0, 2);
    PrecomputedShiftData none = shifted(float3(0), 0);
    colors.init(float3(3, 6, 9), canonical.M);
    colors.add(cv_from_neighbor(float3(0), canonical, neighbor, none, none, float3(1)), neighbor.M);
    outputs[3] = float4(colors.resolve(), colors.mass);

    canonical = source(float3(1), 1, 1);
    neighbor = source(float3(1), 4, 1);
    colors.init(float3(1), canonical.M);
    colors.add(cv_from_neighbor(float3(1), canonical, neighbor, none, none, float3(1)), neighbor.M);
    outputs[4] = float4(colors.resolve(), colors.mass);

    canonical = source(float3(2), 0.5, 1);
    neighbor = source(float3(1), 0.25, 100);
    outputs[5] = float4(cv_temporal(float3(1), float3(0.25), canonical, neighbor,
                                    shifted(float3(4), 2), shifted(float3(3), 0.5), float3(1), 20),
                        0);
    outputs[6] = float4(
        cv_temporal(float3(3, 5, 7), float3(100), canonical, neighbor, none, none, float3(1), 0),
        0);
}

float3 canonical_function(uint interval, bool zeroTarget)
{
    if (zeroTarget && interval == 0)
        return float3(0);
    return interval == 0 ? float3(4, 1, 0.5) : float3(0.5, 3, 2);
}

float3 neighbor_function(uint interval, bool zeroTarget)
{
    if (zeroTarget && interval == 1)
        return float3(0);
    return interval == 0 ? float3(0.25, 2, 3) : float3(3, 0.5, 0.25);
}

void integral(bool partial, bool zeroTarget = false)
{
    // x∈[0,1]，y∈[0,2]，y=2x；两段宽度分别是1/2和1，抽样概率刻意不同。
    // partial仅保留第一段映射，其余积分须由各自源独占域补足；不同抽样点允许单向失败。
    float3 difference = 0, estimate = 0, combined = 0, temporal = 0;
    float3 alpha = float3(0.5, 1.3, 2);
    float3 neighborIntegral = neighbor_function(0, zeroTarget) + neighbor_function(1, zeroTarget);
    for (uint i = 0; i < 2; i++)
        for (uint j = 0; j < 2; j++)
        {
            float pc = i == 0 ? 0.2 : 0.8;
            float pn = j == 0 ? 0.7 : 0.3;
            PathReservoir canonical = source(canonical_function(i, zeroTarget),
                                             zeroTarget && i == 0 ? 0 : 1 / (2 * pc), 2);
            PathReservoir neighbor =
                source(neighbor_function(j, zeroTarget), zeroTarget && j == 1 ? 0 : 1 / pn, 7);
            PrecomputedShiftData reverse =
                shifted(neighbor_function(i, zeroTarget), partial && i == 1 ? 0 : 2);
            PrecomputedShiftData forward =
                shifted(canonical_function(j, zeroTarget), partial && j == 1 ? 0 : 0.5);
            float probability = pc * pn;
            difference += probability * cv_difference(canonical, neighbor, forward, reverse, alpha);
            float3 fromNeighbor =
                cv_from_neighbor(neighborIntegral, canonical, neighbor, forward, reverse, alpha);
            estimate += probability * fromNeighbor;
            CVAccumulator colors;
            colors.init(canonical.integrand * canonical.weight, canonical.M);
            colors.add(fromNeighbor, neighbor.M);
            combined += probability * colors.resolve();
            // 历史原M=70，cap只改confidence，不缩放已归一化UCW。
            neighbor.M = 70;
            temporal +=
                probability * cv_temporal(canonical.integrand * canonical.weight, neighborIntegral,
                                          canonical, neighbor, reverse, forward, alpha, 20);
        }
    outputs[0] = float4(difference, 0);
    outputs[1] = float4(estimate, 0);
    outputs[2] = float4(combined, 0);
    outputs[3] = float4(temporal, 0);
}

void chroma_variance()
{
    // 两条等概率彩色路径，积分为(1,1,0)；history是16个独立抽样的平均。
    // 穷举Binomial(16,1/2)，不以有限随机样本的波动代替方差判断。
    float3 initialMean = 0, filteredMean = 0;
    float initialChroma2 = 0, filteredChroma2 = 0;
    float historyProbability = 1.f / 65536.f;
    PathReservoir canonical = source(float3(1, 0, 0), 2, 1);
    PathReservoir history = source(float3(0, 1, 0), 2, 16);
    PrecomputedShiftData reverse = shifted(canonical.integrand, 1);
    PrecomputedShiftData forward = shifted(history.integrand, 1);
    for (uint red = 0; red <= 16; red++)
    {
        float3 historyColor = float3(red, 16 - red, 0) / 8.f;
        for (uint current = 0; current < 2; current++)
        {
            float3 initial = current == 0 ? float3(2, 0, 0) : float3(0, 2, 0);
            float3 filtered =
                cv_temporal(initial, historyColor, canonical, history, reverse, forward, float3(1));
            float probability = 0.5 * historyProbability;
            initialMean += probability * initial;
            filteredMean += probability * filtered;
            initialChroma2 += probability * (initial.x - initial.y) * (initial.x - initial.y);
            filteredChroma2 += probability * (filtered.x - filtered.y) * (filtered.x - filtered.y);
        }
        historyProbability *= float(16 - red) / float(red + 1);
    }
    outputs[0] = float4(initialMean, initialChroma2);
    outputs[1] = float4(filteredMean, filteredChroma2);
}

[numthreads(64, 1, 1)]
void cv_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x != 0)
        return;
    uint mode = uint(inputs[0].x);
    if (mode == 0)
        boundaries();
    else if (mode == 1 || mode == 2)
        integral(mode == 2);
    else if (mode == 4)
        integral(false, true);
    else
        chroma_variance();
}
