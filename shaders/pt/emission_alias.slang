/// 资源期EmitterAlias的静态裁剪texel分布；不写表或查询宿主。
module emission_alias;
__exported import pt.light_sampling;

namespace pt
{
/// offset为资源世代大表内的header word偏移，非零；sample属于[0,1)^2。
/// 格点的条件残量、patch选择与面积密度共享同一CPU准备表。
public QuadAreaSample sample_emitter_alias(StructuredBuffer<uint> words, uint offset, float2 sample)
{
    uint target = min(uint(sample.x * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
    uint shift = words[offset + 3u];
    uint capacity = 1u << shift;
    uint column = target >> shift;
    uint local = target & (capacity - 1u);
    uint entries = offset + words[offset + 5u];
    uint entry = entries + column * 8u;
    uint threshold = words[entry];
    float u;
    if (local < threshold)
        u = (float(local) + 0.5) / float(threshold);
    else
    {
        u = (float(local - threshold) + 0.5) / float(capacity - threshold);
        entry = entries + words[entry + 1u] * 8u;
    }
    u = min(u, 0.9999999403953552);
    uint patch = offset + words[offset + 6u] + words[entry + 3u] * 8u;
    float target_area = u * asfloat(words[entry + 6u]);
    uint count = words[entry + 4u];
    for (uint i = 0; i + 1u < count; ++i)
    {
        float patch_area = asfloat(words[patch + 1u]);
        if (target_area < patch_area)
            break;
        target_area -= patch_area;
        patch += 8u;
    }
    float root = sqrt(min(target_area / asfloat(words[patch + 1u]), 0.9999999403953552));
    float2 bary = float2(root * (1.0 - sample.y), root * sample.y);
    float2 a = asfloat(uint2(words[patch + 2u], words[patch + 3u]));
    float2 b = asfloat(uint2(words[patch + 4u], words[patch + 5u]));
    float2 c = asfloat(uint2(words[patch + 6u], words[patch + 7u]));
    return { words[patch], a + bary.x * (b - a) + bary.y * (c - a), asfloat(words[entry + 5u]) };
}

/// UV为对应source sprite局部坐标；任一被采样patch均反查到同一cell面积密度。
public float emitter_alias_density(StructuredBuffer<uint> words, uint offset, float2 uv)
{
    uint2 extent = uint2(words[offset], words[offset + 1u]);
    uint2 cell = min(uint2(saturate(uv) * float2(extent)), extent - 1u);
    uint index = words[offset + words[offset + 4u] + cell.y * extent.x + cell.x];
    return index == 0xffffffffu ? 0.0
                                : asfloat(words[offset + words[offset + 5u] + index * 8u + 5u]);
}
}
