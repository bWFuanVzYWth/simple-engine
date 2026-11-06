/// scene_cache::directory的活动身份查询；比较完整token，hash碰撞只能延长探测。
/// base按uint4计，目录和真实GPU恢复数据必须在同一发布事务中生效。
module identity;
namespace geometry
{
public uint identity_index(StructuredBuffer<uint4> words, uint base, uint token)
{
    if (token == 0u)
        return 0xffffffffu;
    uint4 header = words[base];
    uint hash = token;
    hash ^= hash >> 16u;
    hash *= 0x7feb352du;
    hash ^= hash >> 15u;
    hash *= 0x846ca68bu;
    hash ^= hash >> 16u;
    uint slot = hash & header.y;
    for (uint attempt = 0; attempt <= header.y; ++attempt)
    {
        uint4 packed = words[base + header.z + slot / 2u];
        uint2 entry = (slot & 1u) == 0u ? packed.xy : packed.zw;
        if (entry.x == token)
            return entry.y - 1u;
        if (entry.x == 0u)
            return 0xffffffffu;
        slot = (slot + 1u) & header.y;
    }
    return 0xffffffffu;
}
}
