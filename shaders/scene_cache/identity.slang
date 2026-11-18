/// scene_cache::directory的活动身份查询；比较完整token，hash碰撞只能延长探测。
/// base按uint4计，目录和真实GPU恢复数据必须在同一发布事务中生效。
module identity;
namespace scene_cache {
public func identity_index(words: StructuredBuffer<uint4>, base: uint, token: uint)->uint {
    if (token == 0u)
        return 0xffffffffu;
    let header = words[base];
    var hash : uint = token;
    hash ^= hash >> 16u;
    hash *= 0x7feb352du;
    hash ^= hash >> 15u;
    hash *= 0x846ca68bu;
    hash ^= hash >> 16u;
    var slot : uint = hash & header.y;
    for (var attempt: uint = 0; attempt <= header.y; ++attempt) {
        let packed = words[base + header.z + slot / 2u];
        let entry = (slot & 1u) == 0u ? packed.xy : packed.zw;
        if (entry.x == token)
            return entry.y - 1u;
        if (entry.x == 0u)
            return 0xffffffffu;
        slot = (slot + 1u) & header.y;
    }
    return 0xffffffffu;
}
}
