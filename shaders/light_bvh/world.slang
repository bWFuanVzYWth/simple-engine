/// 世界目录和区块表与 Vulkan 构建器共享。地址由 Context 发出，均随绑定版本保活。
module "light_bvh/world.slang";

namespace light_bvh {
/// 区块独占的功率摘要、树节点和发光记录。first 是当前世界版本中的首个灯序号。
public struct LightRegion {
    public var words : uint4 *;
    public var count : uint;
    public var first : uint;
    /// xyz 是 64m 网格坐标，w 是世界树中的路径位。
    public var grid : int4;
};

/// 世界树版本。counts.xy 为总灯数和非空区块数。
public struct LightWorld {
    public var topology : uint4 *;
    public var regions : LightRegion *;
    public var counts : uint4;
    public var anchor : int4;
};

/// 以无符号差求有符号网格距离，涵盖 i32 全域。
public func light_grid_offset(a: int3, b: int3)->float3 {
    let delta = asuint(a) - asuint(b);
    return select(a >= b, float3(delta), -float3(0u - delta)) * 64.0;
}

/// 已发布非空区块的功率中心（相对 viewer_anchor）与总功率。
public func region_summary(entry: LightRegion, viewer_anchor: int3)->float4 {
    let root = asfloat(entry.words[0]);
    return float4(root.xyz + light_grid_offset(entry.grid.xyz, viewer_anchor), root.w);
}

/// 区块摘要和紧凑叶目录之后是应用记录。
public func region_payload(count: uint)->uint {
    return 1u + (((count - 1u) * 10u + 7u) & ~7u) / 4u + (count + 1u) / 2u;
}
}
