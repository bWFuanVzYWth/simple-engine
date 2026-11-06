/// 初采随机域；只保存bounce起点，与逐维LCG消费在模2^32上逐位一致。
module initial_random;
import restir_pt.rng;

namespace restir_pt
{
public uint initialRandomState<let Dimension : uint>(uint seed)
{
    uint multiplier = 1u;
    uint increment = 0u;
    [unroll]
    for (uint dimension = 0; dimension < Dimension; ++dimension)
    {
        multiplier *= 1664525u;
        increment = 1664525u * increment + 1013904223u;
    }
    return multiplier * seed + increment;
}
public float initialRandomValue<let Dimension : uint>(uint seed)
{
    return float(initialRandomState<Dimension>(seed) >> 8) * 0x1p-24f;
}
public float3 initialLightRandom<let LightSamples : uint>(uint seed, uint index)
{
    if (LightSamples == 2)
        return float3(index == 0 ? initialRandomValue<4>(seed) : initialRandomValue<6>(seed),
                      index == 0 ? initialRandomValue<5>(seed) : initialRandomValue<7>(seed),
                      min((float(index) + initialRandomValue<3>(seed)) * 0.5, 0x1.fffffep-1f));
    return float3(initialRandomValue<4>(seed), initialRandomValue<5>(seed),
                  initialRandomValue<3>(seed));
}
public float initialLightRISRandom(uint seed, uint index)
{
    if (index == 0)
        return initialRandomValue<2>(seed);
    return float(block_cipher_tea(seed, 0x6e656532u).x >> 8) * 0x1p-24f;
}
public float3 initialBsdfRandom(uint seed, bool useRussianRoulette)
{
    uint selector = useRussianRoulette ? initialRandomState<9>(seed) : initialRandomState<8>(seed);
    TinyUniformSampleGenerator sampler = rng_from_seed(selector);
    float2 direction = sampler.next2D();
    return float3(direction, float(selector >> 8) * 0x1p-24f);
}
}
