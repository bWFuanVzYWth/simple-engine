/// 资源期EmitterAlias的静态裁剪texel分布；不写表或查询宿主。
module emission_alias;
__exported import pt.light_area;

namespace pt
{
/// offset为资源世代大表内的header word偏移，非零；sample属于[0,1)^2。
/// 格点的条件残量、patch选择与面积密度共享同一CPU准备表。
public QuadAreaSample sample_emitter_alias(ByteAddressBuffer words, uint offset, float2 sample)
{
    uint4 header0 = words.Load4(4u * offset);
    uint4 header1 = words.Load4(4u * (offset + 4u));
    uint target = min(uint(sample.x * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
    uint shift = header0.w;
    uint capacity = 1u << shift;
    uint column = target >> shift;
    uint local = target & (capacity - 1u);
    uint entries = offset + header1.y;
    uint entry = entries + column * 8u;
    uint4 entry0 = words.Load4(4u * entry);
    uint threshold = entry0.x;
    float u;
    if (local < threshold)
        u = (float(local) + 0.5) / float(threshold);
    else
    {
        u = (float(local - threshold) + 0.5) / float(capacity - threshold);
        entry = entries + entry0.y * 8u;
        entry0 = words.Load4(4u * entry);
    }
    u = min(u, 0.9999999403953552);
    uint4 entry1 = words.Load4(4u * (entry + 4u));
    uint patch = offset + header1.z + entry0.w * 8u;
    float target_area = u * asfloat(entry1.z);
    uint count = entry1.x;
    uint4 patch0 = words.Load4(4u * patch);
    for (uint i = 0; i + 1u < count; ++i)
    {
        float patch_area = asfloat(patch0.y);
        if (target_area < patch_area)
            break;
        target_area -= patch_area;
        patch += 8u;
        patch0 = words.Load4(4u * patch);
    }
    uint4 patch1 = words.Load4(4u * (patch + 4u));
    float root = sqrt(min(target_area / asfloat(patch0.y), 0.9999999403953552));
    float2 bary = float2(root * (1.0 - sample.y), root * sample.y);
    float2 a = asfloat(patch0.zw), b = asfloat(patch1.xy), c = asfloat(patch1.zw);
    return { patch0.x, a + bary.x * (b - a) + bary.y * (c - a), asfloat(entry1.y) };
}

/// UV为对应source sprite局部坐标；任一被采样patch均反查到同一cell面积密度。
public float emitter_alias_density(ByteAddressBuffer words, uint offset, float2 uv)
{
    uint4 header0 = words.Load4(4u * offset);
    uint4 header1 = words.Load4(4u * (offset + 4u));
    uint2 extent = header0.xy;
    uint2 cell = min(uint2(saturate(uv) * float2(extent)), extent - 1u);
    uint index = words.Load(4u * (offset + header1.x + cell.y * extent.x + cell.x));
    return index == 0xffffffffu ? 0.0
                                : asfloat(words.Load(4u * (offset + header1.y + index * 8u + 5u)));
}
}
