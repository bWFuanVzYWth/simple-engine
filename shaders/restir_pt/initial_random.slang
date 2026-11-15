/// 初采随机域；只保存bounce起点，与逐维LCG消费在模2^32上逐位一致。
module initial_random;
import restir_pt.rng;

namespace restir_pt
{
public func initialRandomState<let Dimension : uint>(seed: uint)->uint
{
    var multiplier = 1u;
    var increment = 0u;
    [unroll]
    for (var dimension: uint = 0; dimension < Dimension; ++dimension)
    {
        multiplier *= 1664525u;
        increment = 1664525u * increment + 1013904223u;
    }
    return multiplier * seed + increment;
}
public func initialRandomValue<let Dimension : uint>(seed: uint)->float
{
    return float(initialRandomState<Dimension>(seed) >> 8) * 0x1p-24f;
}
public func initialLightRandom<let LightSamples : uint>(seed: uint, index: uint)->float3
{
    if (LightSamples == 2)
        return float3(index == 0 ? initialRandomValue<4>(seed) : initialRandomValue<6>(seed),
                      index == 0 ? initialRandomValue<5>(seed) : initialRandomValue<7>(seed),
                      min((float(index) + initialRandomValue<3>(seed)) * 0.5, 0x1.fffffep-1f));
    return float3(initialRandomValue<4>(seed), initialRandomValue<5>(seed),
                  initialRandomValue<3>(seed));
}
public func initialLightRISRandom(seed: uint, index: uint)->float
{
    if (index == 0)
        return initialRandomValue<2>(seed);
    return float(block_cipher_tea(seed, 0x6e656532u).x >> 8) * 0x1p-24f;
}
public func initialBsdfRandom(seed: uint, useRussianRoulette: bool)->float3
{
    let selector : uint = useRussianRoulette ? initialRandomState<9>(seed)
                                             : initialRandomState<8>(seed);
    var sampler : TinyUniformSampleGenerator = rng_from_seed(selector);
    let direction : float2 = sampler.next2D();
    return float3(direction, float(selector >> 8) * 0x1p-24f);
}
}
