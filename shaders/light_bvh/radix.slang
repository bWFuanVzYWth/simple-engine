/// 在 GPU 上排序光源并构建 Morton 基数树。距离、坐标须在同一坐标系下，功率非负。
module "light_bvh/radix.slang";

import light_bvh.order;

namespace light_bvh {
static const let radix_leaf = 0x80000000u;
groupshared uint4 keys[128];

/// 以 128 个线程排序一块互异的字典序键。末块的空槽传入全一哨兵，所有线程都须调用。
public func sort_tile(key: uint4, lane: uint)->uint4 {
    keys[lane] = key;
    GroupMemoryBarrierWithGroupSync();
    [unroll]
    for (var width = 2u; width <= 128u; width <<= 1u) {
        [unroll]
        for (var step = width >> 1u; step > 0u; step >>= 1u) {
            if (lane < 64u) {
                let first = 2u * lane - (lane & (step - 1u));
                let a = keys[first];
                let b = keys[first + step];
                if (less_key(a, b) != ((first & width) == 0u)) {
                    keys[first] = b;
                    keys[first + step] = a;
                }
            }
            GroupMemoryBarrierWithGroupSync();
        }
    }
    return keys[lane];
}
// 在两条有序序列中定位一个输出边界。每块只做两次全局二分，组内归并读取共享内存。
func left_cut(tree: RWStructuredBuffer<uint4>, src: uint, base: uint, left: uint, right: uint,
              total: uint)
    ->uint {
    var lo = total - min(total, right);
    var hi = min(total, left);
    while (lo < hi) {
        let m = (lo + hi) >> 1u;
        let j = total - m;
        if (less_key(tree[src + base + m], tree[src + base + left + j - 1u]))
            lo = m + 1u;
        else
            hi = m;
    }
    return lo;
}
groupshared uint cuts[2];
/// 以 128 个线程从两段相邻的有序键归并一个输出块。src、dest 为同一缓冲的不相交工作区。
public func merge_tile(tree: RWStructuredBuffer<uint4>, src: uint, dest: uint, n: uint, width: uint,
                       block: uint, lane: uint)
    ->void {
    if (block >= n)
        return;
    let base = block & ~(2u * width - 1u);
    let left = min(width, n - base);
    let right = min(width, n - base - left);
    let size = min(128u, n - block);
    if (right == 0u) {
        if (lane < size)
            tree[dest + block + lane] = tree[src + block + lane];
        return;
    }
    let relative = block - base;
    if (lane < 2u)
        cuts[lane] = left_cut(tree, src, base, left, right, relative + lane * size);
    GroupMemoryBarrierWithGroupSync();
    let count_a = cuts[1] - cuts[0];
    let in_a = lane < count_a;
    var key = uint4(0xffffffffu);
    if (lane < size) {
        let index = in_a ? cuts[0] + lane : left + relative - cuts[0] + lane - count_a;
        key = tree[src + base + index];
        keys[lane] = key;
    }
    GroupMemoryBarrierWithGroupSync();
    if (lane >= size)
        return;
    var lo = in_a ? count_a : 0u;
    var hi = in_a ? size : count_a;
    while (lo < hi) {
        let mid = (lo + hi) >> 1u;
        if (less_key(keys[mid], key))
            lo = mid + 1u;
        else
            hi = mid;
    }
    tree[dest + block + lane + lo - count_a] = key;
}
/// 合并两个光源区间的质心和总功率。零功率的区间不影响质心。
public func merge_nodes(a: float4, b: float4)->float4 {
    let power = a.w + b.w;
    return power > 0.0 ? float4(lerp(a.xyz, b.xyz, b.w / power), power) : float4(0);
}

struct Interval {
    float4 metric;
    uint support;
};
func accumulate(tree: RWStructuredBuffer<uint4>, inout interval: Interval, node: uint, leaves: uint,
                sorted: uint, supports: uint)
    ->void {
    interval.metric = merge_nodes(interval.metric, asfloat(tree[node]));
    interval.support += node >= leaves ? tree[sorted + node - leaves].z : tree[supports + node].x;
}
func interval_value(tree: RWStructuredBuffer<uint4>, begin: uint, end: uint, leaves: uint,
                    sorted: uint, supports: uint)
    ->Interval {
    var lo = begin + leaves;
    var hi = end + leaves;
    var interval : Interval = { float4(0), 0u };
    while (lo < hi) {
        if ((lo & 1u) != 0u) {
            accumulate(tree, interval, lo, leaves, sorted, supports);
            lo++;
        }
        if ((hi & 1u) != 0u) {
            hi--;
            accumulate(tree, interval, hi, leaves, sorted, supports);
        }
        lo >>= 1u;
        hi >>= 1u;
    }
    return interval;
}
// 共同前缀决定 radix 子树；空间码相同时补入唯一有序叶号。
func prefix(diff: uint2, a: uint, b: uint)->int {
    if (diff.x != 0u)
        return 31 - firstbithigh(diff.x);
    if (diff.y != 0u)
        return 63 - firstbithigh(diff.y);
    return a == b ? 96 : 95 - firstbithigh(a ^ b);
}
func child(tree: RWStructuredBuffer<uint4>, sorted: uint, lo: uint, hi: uint)->uint {
    if (hi == lo + 1u)
        return radix_leaf | lo;
    let first = tree[sorted + lo].xy;
    let depth = prefix(first ^ tree[sorted + hi - 1u].xy, lo, hi - 1u);
    var a = lo + 1u;
    var b = hi - 1u;
    while (a < b) {
        let m = (a + b) >> 1u;
        if (prefix(first ^ tree[sorted + m].xy, lo, m) > depth)
            a = m + 1u;
        else
            b = m;
    }
    return a;
}
/// 每个相邻分界对应唯一内节点。输入为按 Morton 键排序的叶与完全二叉树上的区间聚合。
/// leaves 为覆盖 n 的二次幂。头在第 0 行，分支起点为 4×leaves，左右摘要及子引用共占三行。
/// 超过 24 位选择器容量时按顶层叶数分摊区间，叶内采样另取选择器。跨组无需等待。
public func build_radix_branch(tree: RWStructuredBuffer<uint4>, mid: uint, n: uint, leaves: uint,
                               sorted: uint, supports: uint, total: uint)
    ->void {
    if (mid == 0u || mid >= n)
        return;
    let left_key = tree[sorted + mid - 1u].xy;
    let right_key = tree[sorted + mid].xy;
    let depth = prefix(left_key ^ right_key, mid - 1u, mid);
    var step = 2u;
    while (step <= mid &&
           prefix(tree[sorted + mid - step].xy ^ right_key, mid - step, mid) >= depth)
        step *= 2u;
    var a = mid - min(mid, step - 1u);
    var b = mid - step / 2u;
    while (a < b) {
        let m = (a + b) >> 1u;
        if (prefix(tree[sorted + m].xy ^ right_key, m, mid) >= depth)
            b = m;
        else
            a = m + 1u;
    }
    let lo = a;
    step = 2u;
    while (step <= n - mid &&
           prefix(left_key ^ tree[sorted + mid + step - 1u].xy, mid - 1u, mid + step - 1u) >= depth)
        step *= 2u;
    a = mid + step / 2u;
    b = mid + min(n - mid, step - 1u);
    while (a < b) {
        let m = (a + b + 1u) >> 1u;
        if (prefix(left_key ^ tree[sorted + m - 1u].xy, mid - 1u, m - 1u) >= depth)
            a = m;
        else
            b = m - 1u;
    }
    let hi = a;
    let at = 4u * leaves + 3u * mid;
    let left = interval_value(tree, lo, mid, leaves, sorted, supports);
    let right = interval_value(tree, mid, hi, leaves, sorted, supports);
    let large = total > (1u << 24u);
    tree[at] = asuint(left.metric);
    tree[at + 1u] = asuint(right.metric);
    tree[at + 2u] = uint4(child(tree, sorted, lo, mid), child(tree, sorted, mid, hi),
                          large ? mid - lo : left.support, large ? hi - mid : right.support);
    if (lo == 0u && hi == n)
        tree[0] = uint4(n, leaves, mid, total);
}
}
