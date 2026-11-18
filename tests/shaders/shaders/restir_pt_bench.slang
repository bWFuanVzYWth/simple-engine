/// 单像素的 20 帧历史上限与三邻居双向复用，含不可逆 shift。
module restir_pt_bench;
import restir_pt.resampling;
import restir_pt.reservoir;
import restir_pt.types;
import sampling.pcg;

[[vk::binding(0, 0)]]
var seed : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

func reservoir(inout state: uint, mass: float)->restir_pt::PathReservoir {
    var result = restir_pt::empty_reservoir();
    result.sample_mass = mass;
    result.init_random_seed = state;
    let energy = 0.001 + 4.0 * sampling::random(state);
    result.integrand = energy * float3(0.5 + sampling::random(state), 1.0, 0.25);
    result.weight = 0.25 + sampling::random(state);
    return result;
}

func shift(inout state: uint, source: restir_pt::PathReservoir)->restir_pt::PrecomputedShiftData {
    let jacobian = sampling::random(state);
    let factor = sampling::random(state);
    if (jacobian < 0.125)
        return { float3(0.0), 0.0, 0.0 };
    return { source.integrand * factor, jacobian * 2.0, 0.5 + factor };
}

[shader("compute")]
[numthreads(8, 8, 1)]
func reuse_main(id: uint3: SV_DispatchThreadID)->void {
    let pixel = id.y * 3840 + id.x;
    var state = pixel ^ seed[0].x;
    let center = reservoir(state, 1.0);
    let history = reservoir(state, float(1u + (id.y & 31u)));
    let into_history = shift(state, center);
    let into_center = shift(state, history);
    let temporal_u = float2(sampling::random(state), sampling::random(state));
    let canonical =
        restir_pt::temporal_resample(center, history, into_history, into_center, temporal_u, 20.0);
    let neighbor_mass = float(1u + (id.x & 15u));
    let factor = restir_pt::PairwiseFactor(canonical.sample_mass + 3.0 * neighbor_mass,
                                           canonical.sample_mass);
    var canonical_weight = restir_pt::spatial_begin(factor);
    var result = restir_pt::empty_reservoir();
    [unroll]
    for (var slot = 0u; slot < 3u; ++slot) {
        let neighbor = reservoir(state, neighbor_mass);
        let forward = shift(state, neighbor);
        let reverse = shift(state, canonical);
        restir_pt::spatial_merge_neighbor(result, canonical, neighbor, forward, reverse, factor,
                                          canonical_weight, sampling::random(state));
    }
    restir_pt::spatial_finish(result, canonical, canonical_weight, sampling::random(state));
    outputs[pixel] = float4(result.integrand * result.weight,
                            float(result.init_random_seed & 1023u) + result.sample_mass);
}
