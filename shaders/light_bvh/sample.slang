/// 与接收点相关的 quad 光源离散采样；不决定辐射功率、法线、遮挡、面积采样或积分策略。
/// 所有位置须在同一坐标系，运算处于 float32 范围。任意朝向、非平面 quad 均合法。
/// 区块树和参考单树按 sibling pair 保存，L 个灯有 L-1 对。每至多四对一块，先存每对左右
/// center.xyz/power（32B），再存 child0/child1（8B）。child 高位标记叶，低位为局部灯序号。
/// 叶目录在 base+roundup(10*(L-1),8)/4，base 以 uint4 为单位。两级树每灯保留 edge²/path，
/// 参考单树另保留包围盒。path 低 27 位保存左右选择，高 5 位为深度，最大 24。
/// 以功率/距离平方选择分支，在 24-bit 随机格点上量化，且为每个后代保留至少一个格点。
/// 叶子分母为质心距离²+(|v1-v0|²+|v3-v0|²)/4，平滑近灯与盒内查询；内部节点不平滑。
module "light_bvh/sample.slang";

import light_bvh.world;

namespace light_bvh {
static const let LIGHT_BVH_LEAF = 0x80000000u;
static const let LIGHT_BVH_SELECTORS = 16777216u;
static const let LIGHT_BVH_INVERSE_SELECTORS = 5.9604644775390625e-8;

/// 一次离散选灯的结果；不包含灯内面积或立体角 PDF。
public struct LightBvhSample {
    /// 原始输入灯索引；空树为 0xffffffff。
    public var light : uint;
    /// 实际量化分布中该灯的选择概率；空树为零，单叶为一。
    public var pmf : float;
};

struct LightBvhNode {
    var center : float3;
    var power : float;
    var child : uint;
    var leaves : uint;
};

interface ILightWords {
    func at(index: uint)->uint4;
}

struct DescriptorWords : ILightWords {
    var buffer : StructuredBuffer<uint4>;
    func at(index: uint)->uint4 {
        return buffer[index];
    }
}

struct AddressWords : ILightWords {
    var address : uint4 *;
    func at(index: uint)->uint4 {
        return address[index];
    }
}

func light_bvh_pair<Words : ILightWords>(words: Words, base: uint, packets: uint, at: uint,
                                         leaves: uint, out first: LightBvhNode,
                                         out second: LightBvhNode)
    ->void {
    let block : uint = at >> 2u;
    let within = at & 3u;
    let block_base = base + 10u * block;
    let block_count = min(4u, packets - 4u * block);
    let a : float4 = asfloat(words.at(block_base + 2u * within));
    let b : float4 = asfloat(words.at(block_base + 2u * within + 1u));
    let topology = words.at(block_base + 2u * block_count + (within >> 1u));
    let children = select((within & 1u) == 0, topology.xy, topology.zw);
    let left_leaves = leaves >> 1u;
    first = { a.xyz, a.w, children.x, left_leaves };
    second = { b.xyz, b.w, children.y, leaves - left_leaves };
}

func packed_pair<Words : ILightWords>(words: Words, base: uint, index: uint)->uint2 {
    let packed = words.at(base + (index >> 1u));
    return select((index & 1u) == 0, packed.xy, packed.zw);
}

func light_bvh_metrics<Words : ILightWords, let smooth_leaf : bool, let compact : bool>(
    words: Words, leaf_base: uint, node: LightBvhNode, point_value: float3)
    ->float2 {
    let delta = node.center - point_value;
    var distance_squared : float = dot(delta, delta);
    if (smooth_leaf && (node.child & LIGHT_BVH_LEAF) != 0) {
        let at : uint = node.child & ~LIGHT_BVH_LEAF;
        let edge_squared =
            compact ? packed_pair(words, leaf_base, at).x : words.at(leaf_base + 2u * at).w;
        distance_squared += 0.25 * asfloat(edge_squared);
    }
    return float2(distance_squared, node.power);
}

/// 两个非空子树的距离平方和正功率转换为左侧的 24-bit 选择区间。
/// count 至少为两侧有效叶数之和。正向选择和逆向 PDF 必须使用同一接收点。
public func split_selectors(a: float2, b: float2, left_leaves: uint, right_leaves: uint,
                            count: uint)
    ->uint {
    let power = float2(a.y, b.y) / max(a.y, b.y);
    let scale = max(a.x, b.x);
    var probability : float;
    if (scale == 0)
        probability = power.x / (power.x + power.y);
    else {
        let score = power * (float2(b.x, a.x) / scale);
        let total = score.x + score.y;
        probability = total > 0 ? score.x / total : -1.0;
    }
    probability = clamp(probability, 1.0e-6, 1.0 - 1.0e-6);
    precise let scaled = float(count) * probability;
    let integral = uint(scaled);
    let fraction = scaled - float(integral);
    let odd_tie = (fraction == 0.5) & ((integral & 1u) != 0);
    let left = integral + uint((fraction > 0.5) | odd_tie);
    return clamp(left, left_leaves, count - right_leaves);
}

func light_bvh_left_count<Words : ILightWords, let smooth_leaf : bool, let compact : bool>(
    words: Words, leaf_base: uint, first: LightBvhNode, second: LightBvhNode, point_value: float3,
    count: uint)
    ->uint {
    let a = light_bvh_metrics<Words, smooth_leaf, compact>(words, leaf_base, first, point_value);
    let b = light_bvh_metrics<Words, smooth_leaf, compact>(words, leaf_base, second, point_value);
    return split_selectors(a, b, first.leaves, second.leaves, count);
}

/// 叶索引、叶区间内的剩余格点以及该区间的实际格点数。
public struct SelectedInterval {
    public var index : uint;
    public var selector : uint;
    public var count : uint;
};

func select_pair<Words : ILightWords, let upper : bool>(words: Words, base: uint, packets: uint,
                                                        at: uint, leaves: uint,
                                                        out first: LightBvhNode,
                                                        out second: LightBvhNode)
    ->void {
    if (upper) {
        let address = base + 3u * at;
        let a = asfloat(words.at(address));
        let b = asfloat(words.at(address + 1u));
        let topology = words.at(address + 2u);
        first = { a.xyz, a.w, topology.x, topology.z };
        second = { b.xyz, b.w, topology.y, topology.w };
    } else {
        light_bvh_pair(words, base, packets, at, leaves, first, second);
    }
}

func select_interval<Words : ILightWords, let upper : bool, let compact : bool>(
    words: Words, base: uint, entries: uint, light_count: uint, point_value: float3, selector: uint,
    count: uint)
    ->SelectedInterval {
    if (entries <= 1u)
        return { 0u, selector, count };
    let packets = entries - 1u;
    let leaf_base = base + (((10u * packets + 7u) & ~7u) >> 2u);
    var result : SelectedInterval = { 0u, selector, count };
    var leaves = light_count;
    while (true) {
        var a : LightBvhNode;
        var b : LightBvhNode;
        select_pair<Words, upper>(words, base, packets, result.index, leaves, a, b);
        let left = light_bvh_left_count<Words, !upper, compact>(words, leaf_base, a, b, point_value,
                                                                result.count);
        let right = result.selector >= left;
        result.selector -= select(right, left, 0u);
        result.count = select(right, result.count - left, left);
        let child = select(right, b.child, a.child);
        leaves = select(right, b.leaves, a.leaves);
        result.index = child & ~LIGHT_BVH_LEAF;
        if ((child & LIGHT_BVH_LEAF) != 0)
            return result;
    }
}

func interval_count<Words : ILightWords, let upper : bool, let compact : bool>(
    words: Words, base: uint, entries: uint, light_count: uint, point_value: float3, path: uint,
    count: uint)
    ->uint {
    let packets = entries - 1u;
    let leaf_base = base + (((10u * packets + 7u) & ~7u) >> 2u);
    var count = count;
    var at = 0u;
    var leaves = light_count;
    for (var depth: uint = 0; depth < (path >> 27u); ++depth) {
        var a : LightBvhNode;
        var b : LightBvhNode;
        select_pair<Words, upper>(words, base, packets, at, leaves, a, b);
        let left = light_bvh_left_count<Words, !upper, compact>(words, leaf_base, a, b, point_value,
                                                                count);
        let second = ((path >> depth) & 1u) != 0;
        count = select(second, count - left, left);
        at = select(second, b.child, a.child);
        leaves = select(second, b.leaves, a.leaves);
    }
    return count;
}

/// 用 [0,1) 中的一个均匀数选择灯，返回真实格点 PMF；不消费额外随机数。
/// words/base/light_count 须来自同一有效树，point 有限；树与输入不在 shader 内重验。
/// 分支格点数为 round-to-nearest-even(count*p)，再按后代叶数夹紧；空树返回无灯。
public func sample_light_bvh(words: StructuredBuffer<uint4>, base: uint, light_count: uint,
                             point_value: float3, sample_value: float)
    ->LightBvhSample {
    if (light_count == 0)
        return { 0xffffffffu, 0.0 };
    let selector = uint(sample_value * float(LIGHT_BVH_SELECTORS));
    let input : DescriptorWords = { words };
    let result = select_interval<DescriptorWords, false, false>(
        input, base, light_count, light_count, point_value, selector, LIGHT_BVH_SELECTORS);
    return { result.index, float(result.count) * LIGHT_BVH_INVERSE_SELECTORS };
}

/// 在相同接收点重算指定灯的真实选择 PMF；空树及不在输入目录中的索引返回零。
/// 返回离散概率；转换为面积或立体角 PDF 的几何项由调用方组合。
public func light_bvh_pmf(words: StructuredBuffer<uint4>, base: uint, light_count: uint,
                          point_value: float3, light: uint)
    ->float {
    if (light >= light_count)
        return 0;
    if (light_count == 1)
        return 1;
    let leaf_base = base + (((10u * (light_count - 1u) + 7u) & ~7u) >> 2u);
    let path = words[leaf_base + 2u * light + 1u].w;
    let input : DescriptorWords = { words };
    let count = interval_count<DescriptorWords, false, false>(
        input, base, light_count, light_count, point_value, path, LIGHT_BVH_SELECTORS);
    return float(count) * LIGHT_BVH_INVERSE_SELECTORS;
}

/// 与同一发布事务的树和原始灯顺序绑定；不拥有GPU缓冲。
public struct LightBvhProposal {
    public var words : StructuredBuffer<uint4>;
    public var base : uint;
    public var count : uint;
    public func sample(receiver: float3, selector: float)->LightBvhSample {
        return sample_light_bvh(words, base, count, receiver, selector);
    }
    public func pmf(receiver: float3, light: uint)->float {
        return light_bvh_pmf(words, base, count, receiver, light);
    }
};

/// 两级抽样的当前世界目录项与区块内灯序号。pmf 为两级共享格点的实际概率。
public struct LightWorldSample {
    public var region : uint;
    public var local : uint;
    public var pmf : float;
};

/// 在已选区块内继续划分格点。count 至少为区块灯数，selector 小于 count。
public func select_light_region(entry: LightRegion, receiver: float3, viewer_anchor: int3,
                                selector: uint, count: uint)
    ->SelectedInterval {
    let subtree : AddressWords = { entry.words };
    let point_value = receiver + light_grid_offset(viewer_anchor, entry.grid.xyz);
    return select_interval<AddressWords, false, true>(subtree, 1u, entry.count, entry.count,
                                                      point_value, selector, count);
}

/// 反查区块内指定灯的格点数，使用与正向遍历相同的区间和接收点。
public func light_region_interval_count(entry: LightRegion, local: uint, receiver: float3,
                                        viewer_anchor: int3, count: uint)
    ->uint {
    if (entry.count == 1u)
        return count;
    let leaf_base = 1u + (((10u * (entry.count - 1u) + 7u) & ~7u) >> 2u);
    let subtree : AddressWords = { entry.words };
    let path = packed_pair(subtree, leaf_base, local).y;
    let point_value = receiver + light_grid_offset(viewer_anchor, entry.grid.xyz);
    return interval_count<AddressWords, false, true>(subtree, 1u, entry.count, entry.count,
                                                     point_value, path, count);
}

/// 接收点使用 viewer_anchor 对应的局部坐标；空树返回 PMF 为零。
public func sample_light_world(world: LightWorld, receiver: float3, viewer_anchor: int3,
                               sample_value: float)
    ->LightWorldSample {
    if (world.counts.x == 0u)
        return { 0u, 0u, 0.0 };
    let origin = receiver + light_grid_offset(viewer_anchor, world.anchor.xyz);
    let top : AddressWords = { world.topology };
    let region = select_interval<AddressWords, true, false>(
        top, 0u, world.counts.y, world.counts.x, origin,
        uint(sample_value * float(LIGHT_BVH_SELECTORS)), LIGHT_BVH_SELECTORS);
    let light = select_light_region(world.regions[region.index], receiver, viewer_anchor,
                                    region.selector, region.count);
    return { region.index, light.index, float(light.count) * LIGHT_BVH_INVERSE_SELECTORS };
}

/// 从同一发布版本反查区块内灯的实际 PMF。world 中的区块路径和子树路径独立更新。
public func light_world_pmf(world: LightWorld, region: uint, local: uint, receiver: float3,
                            viewer_anchor: int3)
    ->float {
    let entry = world.regions[region];
    let top : AddressWords = { world.topology };
    let origin = receiver + light_grid_offset(viewer_anchor, world.anchor.xyz);
    let region_count = interval_count<AddressWords, true, false>(
        top, 0u, world.counts.y, world.counts.x, origin, asuint(entry.grid.w), LIGHT_BVH_SELECTORS);
    let slots = light_region_interval_count(entry, local, receiver, viewer_anchor, region_count);
    return float(slots) * LIGHT_BVH_INVERSE_SELECTORS;
}

/// 从世界内连续灯号定位区块。rank 须属于 world.counts.x 范围。
public func light_world_region_at(world: LightWorld, rank: uint)->uint {
    var lo = 0u;
    var hi = world.counts.y;
    while (lo + 1u < hi) {
        let mid = lo + (hi - lo) / 2u;
        if (world.regions[mid].first <= rank)
            lo = mid;
        else
            hi = mid;
    }
    return lo;
}

func grid_before(a: int3, b: int3)->bool {
    return a.x < b.x || (a.x == b.x && (a.y < b.y || (a.y == b.y && a.z < b.z)));
}

/// 查找使用唯一空间键发布的区块。有效灯的 grid 由同一发布版本提供。
public func light_world_find_region(world: LightWorld, grid: int3)->uint {
    var lo = 0u;
    var hi = world.counts.y;
    while (lo < hi) {
        let mid = lo + (hi - lo) / 2u;
        if (grid_before(world.regions[mid].grid.xyz, grid))
            lo = mid + 1u;
        else
            hi = mid;
    }
    return lo;
}
}
