/// 资源期EmitterAlias的静态裁剪texel分布；不写表或查询宿主。
module emission_alias;
__exported import pt.light_area;

namespace pt
{
/// offset为资源世代大表内的header word偏移，非零；sample属于[0,1)^2。
/// 格点的条件残量、patch选择与面积密度共享同一CPU准备表。
public func sample_emitter_alias(words: ByteAddressBuffer, offset: uint, sample_value: float2)
    ->QuadAreaSample
{
    let header0 : uint4 = words.Load4(4u * offset);
    let header1 : uint4 = words.Load4(4u * (offset + 4u));
    let target : uint = min(uint(sample_value.x * float(LIGHT_SELECTORS)), LIGHT_SELECTORS - 1u);
    let shift : uint = header0.w;
    let capacity : uint = 1u << shift;
    let column : uint = target >> shift;
    let local : uint = target & (capacity - 1u);
    let entries : uint = offset + header1.y;
    var entry : uint = entries + column * 8u;
    var entry0 : uint4 = words.Load4(4u * entry);
    let threshold : uint = entry0.x;
    var u : float;
    if (local < threshold)
        u = (float(local) + 0.5) / float(threshold);
    else
    {
        u = (float(local - threshold) + 0.5) / float(capacity - threshold);
        entry = entries + entry0.y * 8u;
        entry0 = words.Load4(4u * entry);
    }
    u = min(u, 0.9999999403953552);
    let entry1 : uint4 = words.Load4(4u * (entry + 4u));
    var patch : uint = offset + header1.z + entry0.w * 8u;
    var target_area : float = u * asfloat(entry1.z);
    let count : uint = entry1.x;
    var patch0 : uint4 = words.Load4(4u * patch);
    for (var i: uint = 0; i + 1u < count; ++i)
    {
        let patch_area : float = asfloat(patch0.y);
        if (target_area < patch_area)
            break;
        target_area -= patch_area;
        patch += 8u;
        patch0 = words.Load4(4u * patch);
    }
    let patch1 : uint4 = words.Load4(4u * (patch + 4u));
    let root : float = sqrt(min(target_area / asfloat(patch0.y), 0.9999999403953552));
    let bary = float2(root * (1.0 - sample_value.y), root * sample_value.y);
    let a : float2 = asfloat(patch0.zw);
    let b : float2 = asfloat(patch1.xy);
    let c : float2 = asfloat(patch1.zw);
    return { patch0.x, a + bary.x * (b - a) + bary.y * (c - a), asfloat(entry1.y) };
}

/// UV为对应source sprite局部坐标；任一被采样patch均反查到同一cell面积密度。
public func emitter_alias_density(words: ByteAddressBuffer, offset: uint, uv: float2)->float
{
    let header0 : uint4 = words.Load4(4u * offset);
    let header1 : uint4 = words.Load4(4u * (offset + 4u));
    let extent : uint2 = header0.xy;
    let cell : uint2 = min(uint2(saturate(uv) * float2(extent)), extent - 1u);
    let index : uint = words.Load(4u * (offset + header1.x + cell.y * extent.x + cell.x));
    return index == 0xffffffffu ? 0.0
                                : asfloat(words.Load(4u * (offset + header1.y + index * 8u + 5u)));
}
}
