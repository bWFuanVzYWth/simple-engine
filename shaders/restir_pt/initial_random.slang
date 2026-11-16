/// 初采随机域；只保存bounce起点，与逐维LCG消费在模2^32上逐位一致。
module initial_random;
import restir_pt.rng;

namespace restir_pt
{
public func initial_random_state<let dimension : uint>(seed: uint)->uint
{
    var multiplier = 1u;
    var increment = 0u;
    [unroll]
    for (var advance_index: uint = 0; advance_index < dimension; ++advance_index)
    {
        multiplier *= 1664525u;
        increment = 1664525u * increment + 1013904223u;
    }
    return multiplier * seed + increment;
}
public func initial_random_value<let dimension : uint>(seed: uint)->float
{
    return float(initial_random_state<dimension>(seed) >> 8) * 0x1p-24f;
}
public func initial_light_random<let light_samples : uint>(seed: uint, index: uint)->float3
{
    if (light_samples == 2)
        return float3(index == 0 ? initial_random_value<4>(seed) : initial_random_value<6>(seed),
                      index == 0 ? initial_random_value<5>(seed) : initial_random_value<7>(seed),
                      min((float(index) + initial_random_value<3>(seed)) * 0.5, 0x1.fffffep-1f));
    return float3(initial_random_value<4>(seed), initial_random_value<5>(seed),
                  initial_random_value<3>(seed));
}
public func initial_light_ris_random(seed: uint, index: uint)->float
{
    if (index == 0)
        return initial_random_value<2>(seed);
    return float(block_cipher_tea(seed, 0x6e656532u).x >> 8) * 0x1p-24f;
}
public func initial_bsdf_random(seed: uint, use_russian_roulette: bool)->float3
{
    let selector : uint = use_russian_roulette ? initial_random_state<9>(seed)
                                               : initial_random_state<8>(seed);
    var sampler : TinyUniformSampleGenerator = rng_from_seed(selector);
    let direction = sampler.next_2d();
    return float3(direction, float(selector >> 8) * 0x1p-24f);
}
}
