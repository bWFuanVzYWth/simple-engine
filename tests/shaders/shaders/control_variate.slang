/// 有限分片积分与双色 Bernoulli 历史；直接检查 CV 的积分和方差，不依赖应用场景。
module control_variate_test;
import restir_pt.control_variate;
import restir_pt.reservoir;
using namespace restir_pt;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

func source(value: float3, weight: float, mass: float)->PathReservoir {
    var result : PathReservoir = empty_reservoir();
    result.integrand = value;
    result.weight = weight;
    result.sample_mass = mass;
    return result;
}

func shifted(value: float3, jacobian: float)->PrecomputedShiftData {
    return { value, jacobian, 0 };
}

func boundaries()->void {
    outputs[0] = float4(cv_alpha(float3(0, 0.5, 3), float3(0, 0.25, 1)), 0);
    var colors : CVAccumulator;
    colors.init(float3(3, 5, 7), 0);
    colors.add(float3(100), 2);
    outputs[1] = float4(colors.resolve(), colors.mass);
    colors.init(float3(3, 5, 7), 1);
    colors.add(float3(100), 0);
    outputs[2] = float4(colors.resolve(), colors.mass);

    // 零 target 的源是合法零贡献抽样，不等于缺失邻居；其颜色仍可承载历史。
    var canonical : PathReservoir = source(float3(0), 0, 1);
    var neighbor : PathReservoir = source(float3(0), 0, 2);
    let none : PrecomputedShiftData = shifted(float3(0), 0);
    colors.init(float3(3, 6, 9), canonical.sample_mass);
    colors.add(cv_from_neighbor(float3(0), canonical, neighbor, none, none, float3(1)),
               neighbor.sample_mass);
    outputs[3] = float4(colors.resolve(), colors.mass);

    canonical = source(float3(1), 1, 1);
    neighbor = source(float3(1), 4, 1);
    colors.init(float3(1), canonical.sample_mass);
    colors.add(cv_from_neighbor(float3(1), canonical, neighbor, none, none, float3(1)),
               neighbor.sample_mass);
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

func canonical_function(interval: uint, zeroTarget: bool)->float3 {
    if (zeroTarget && interval == 0)
        return float3(0);
    return interval == 0 ? float3(4, 1, 0.5) : float3(0.5, 3, 2);
}

func neighbor_function(interval: uint, zeroTarget: bool)->float3 {
    if (zeroTarget && interval == 1)
        return float3(0);
    return interval == 0 ? float3(0.25, 2, 3) : float3(3, 0.5, 0.25);
}

func integral(partial: bool, zeroTarget: bool = false)->void {
    // x∈[0,1]，y∈[0,2]，y=2x；两段宽度分别是1/2和1，抽样概率刻意不同。
    // partial仅保留第一段映射，其余积分须由各自源独占域补足；不同抽样点允许单向失败。
    var difference : float3 = 0;
    var estimate : float3 = 0;
    var combined : float3 = 0;
    var temporal : float3 = 0;
    let alpha = float3(0.5, 1.3, 2);
    let neighborIntegral : float3 =
                               neighbor_function(0, zeroTarget) + neighbor_function(1, zeroTarget);
    for (var i: uint = 0; i < 2; i++)
        for (var j: uint = 0; j < 2; j++) {
            let pc : float = i == 0 ? 0.2 : 0.8;
            let pn : float = j == 0 ? 0.7 : 0.3;
            let canonical : PathReservoir = source(canonical_function(i, zeroTarget),
                                                   zeroTarget && i == 0 ? 0 : 1 / (2 * pc), 2);
            var neighbor : PathReservoir = source(neighbor_function(j, zeroTarget),
                                                  zeroTarget && j == 1 ? 0 : 1 / pn, 7);
            let reverse : PrecomputedShiftData =
                              shifted(neighbor_function(i, zeroTarget), partial && i == 1 ? 0 : 2);
            let forward : PrecomputedShiftData = shifted(canonical_function(j, zeroTarget),
                                                         partial && j == 1 ? 0 : 0.5);
            let probability : float = pc * pn;
            difference += probability * cv_difference(canonical, neighbor, forward, reverse, alpha);
            let fromNeighbor : float3 = cv_from_neighbor(neighborIntegral, canonical, neighbor,
                                                         forward, reverse, alpha);
            estimate += probability * fromNeighbor;
            var colors : CVAccumulator;
            colors.init(canonical.integrand * canonical.weight, canonical.sample_mass);
            colors.add(fromNeighbor, neighbor.sample_mass);
            combined += probability * colors.resolve();
            // 历史原M=70，cap只改confidence，不缩放已归一化UCW。
            neighbor.sample_mass = 70;
            temporal +=
                probability * cv_temporal(canonical.integrand * canonical.weight, neighborIntegral,
                                          canonical, neighbor, reverse, forward, alpha, 20);
        }
    outputs[0] = float4(difference, 0);
    outputs[1] = float4(estimate, 0);
    outputs[2] = float4(combined, 0);
    outputs[3] = float4(temporal, 0);
}

func chroma_variance()->void {
    // 两条等概率彩色路径，积分为(1,1,0)；history是16个独立抽样的平均。
    // 穷举Binomial(16,1/2)，不以有限随机样本的波动代替方差判断。
    var initialMean : float3 = 0;
    var filteredMean : float3 = 0;
    var initialChroma2 : float = 0;
    var filteredChroma2 : float = 0;
    var historyProbability : float = 1.f / 65536.f;
    var canonical : PathReservoir = source(float3(1, 0, 0), 2, 1);
    var history : PathReservoir = source(float3(0, 1, 0), 2, 16);
    let reverse : PrecomputedShiftData = shifted(canonical.integrand, 1);
    let forward : PrecomputedShiftData = shifted(history.integrand, 1);
    for (var red: uint = 0; red <= 16; red++) {
        let history_color : float3 = float3(red, 16 - red, 0) / 8.f;
        for (var current: uint = 0; current < 2; current++) {
            let initial : float3 = current == 0 ? float3(2, 0, 0) : float3(0, 2, 0);
            let filtered : float3 = cv_temporal(initial, history_color, canonical, history, reverse,
                                                forward, float3(1));
            let probability : float = 0.5 * historyProbability;
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
func cv_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x != 0)
        return;
    let mode = uint(inputs[0].x);
    if (mode == 0)
        boundaries();
    else if (mode == 1 || mode == 2)
        integral(mode == 2);
    else if (mode == 4)
        integral(false, true);
    else
        chroma_variance();
}
