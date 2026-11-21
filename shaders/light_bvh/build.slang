// 在 GPU 上按空间 Morton 序二分，叶记录保持原始 rank，与正反向 PDF 共用布局。
import light_sampling.area;
import light_bvh.world;
import light_bvh.order;

struct Build {
    var vertices : uint *;
    var leaves : uint *;
    var tree : uint *;
    var records : uint *;
    var count : uint;
    var level : uint;
    var first : uint;
    var write_records : uint;
    var anchor_x : int;
    var anchor_y : int;
    var anchor_z : int;
    var vertex_quads : uint;
    var sorted : uint4 *;
    var destination : uint4 *;
    var mode : uint;
    var width : uint;
    var jobs : light_bvh::LightRegion *;
    var world : light_bvh::LightWorld *;
    var job_first : uint;
    var shift : uint;
    var work : uint;
    // 0 为单树参考布局，1 为独占的区块子树。
    var layout : uint;
};
[[vk::push_constant]]
var launch : ConstantBuffer<Build>;

func metric_at(packet: uint)->uint {
    return (packet >> 2u) * 40u + (packet & 3u) * 8u;
}
func child_at(packet: uint, count: uint)->uint {
    let block : uint = packet >> 2u;
    return block * 40u + min(4u, count - 1u - block * 4u) * 8u + (packet & 3u) * 2u;
}
func region_delta(a: int, b: int)->float {
    return a >= b ? float(asuint(a) - asuint(b)) : -float(asuint(b) - asuint(a));
}
func point_at(row: uint *, i: uint)->float3 {
    return asfloat(uint3(row[i * 4u], row[i * 4u + 1u], row[i * 4u + 2u]));
}
groupshared var tile : uint4[64];
func spatial_key(p: Build, rank: uint)->uint4 {
    let q = p.leaves[4u * rank];
    if (q >= p.vertex_quads)
        return uint4(0xffffffffu, 0xffffffffu, 0xffffffffu, rank);
    let row : uint * = p.vertices + 16u * q;
    var center : float3 = point_at(row, 0) * 0.25 + point_at(row, 1) * 0.25 +
                          point_at(row, 2) * 0.25 + point_at(row, 3) * 0.25;
    center +=
        float3(region_delta(asint(row[3]), p.anchor_x), region_delta(asint(row[7]), p.anchor_y),
               region_delta(asint(row[11]), p.anchor_z)) *
        64.0;
    var bits : uint3 = asuint(center);
    bits ^= asuint(asint(bits) >> 31) | 0x80000000u;
    return uint4(light_bvh::morton10(bits >> 22u), light_bvh::morton10(bits >> 12u),
                 light_bvh::morton10(bits >> 2u), rank);
}
// 两级的拓扑都按各自的序列长度中分，局部响应保留原叶目录序号。
func locate(count: uint, level: uint, node: uint, out begin: uint, out packet: uint,
            out parent: uint, out side: uint, out path: uint)
    ->uint {
    var n = count;
    begin = 0;
    packet = 0;
    parent = 0;
    side = 0;
    path = 0;
    for (var depth = 0u; depth < level; ++depth) {
        if (n <= 1u)
            return 0u;
        let left = n / 2u;
        side = (node >> (level - 1u - depth)) & 1u;
        path |= side << depth;
        parent = packet;
        if (side == 0u) {
            packet += 1u;
            n = left;
        } else {
            packet += left;
            begin += left;
            n -= left;
        }
    }
    return n;
}

func build_local(p: Build, node: uint, lane: uint)->void {
    if (p.mode == 1u) {
        var key = uint4(0xffffffffu);
        if (node < p.count)
            key = spatial_key(p, node);
        tile[lane] = key;
        GroupMemoryBarrierWithGroupSync();
        [unroll]
        for (var width: uint = 2; width <= 64; width *= 2) {
            [unroll]
            for (var stride: uint = width >> 1u; stride > 0; stride >>= 1u) {
                let peer : uint = lane ^ stride;
                if (lane < peer) {
                    let a : uint4 = tile[lane];
                    let b : uint4 = tile[peer];
                    if ((lane & width) == 0u ? light_bvh::less_key(b, a)
                                             : light_bvh::less_key(a, b)) {
                        tile[lane] = b;
                        tile[peer] = a;
                    }
                }
                GroupMemoryBarrierWithGroupSync();
            }
        }
        if (node < p.count)
            p.destination[node] = tile[lane];
        return;
    }
    if (p.mode == 2u) {
        if (node >= p.count)
            return;
        let begin = node / (2u * p.width) * (2u * p.width);
        let right : bool = node - begin >= p.width;
        let other : uint = right ? begin : begin + p.width;
        var lo : uint = min(other, p.count);
        var hi : uint = min(other + p.width, p.count);
        let key = p.sorted[node];
        while (lo < hi) {
            let mid : uint = lo + (hi - lo) / 2u;
            if (light_bvh::less_key(p.sorted[mid], key))
                lo = mid + 1u;
            else
                hi = mid;
        }
        p.destination[begin + (node - (right ? begin + p.width : begin)) +
                      (lo - min(other, p.count))] = key;
        return;
    }
    if (node >= (1u << p.level))
        return;
    let tree = p.tree + (p.layout == 0u ? 0u : 4u);
    var begin : uint;
    var packet : uint;
    var parent : uint;
    var side : uint;
    var path : uint;
    let n = locate(p.count, p.level, node, begin, packet, parent, side, path);
    if (n == 0u)
        return;
    var center : float3;
    var power : float;
    var child : uint;
    if (n == 1u) {
        let rank = p.count == 1u ? begin : p.sorted[begin].w;
        let leaf = p.leaves + 4u * rank;
        if (leaf[0] >= p.vertex_quads)
            return;
        let row : uint * = p.vertices + 16u * leaf[0];
        let region = asint(uint3(row[3], row[7], row[11]));
        let offset = float3(region_delta(region.x, p.anchor_x), region_delta(region.y, p.anchor_y),
                            region_delta(region.z, p.anchor_z)) *
                     64.0;
        let local : light_sampling::LightQuadVertices = { point_at(row, 0), point_at(row, 1),
                                                          point_at(row, 2), point_at(row, 3) };
        let a = local.a;
        let b = local.b;
        let c = local.c;
        let d = local.d;
        let area0 = length(cross(b - a, c - a));
        let area1 = length(cross(c - a, d - a));
        center = (a + c + lerp(d, b, area0 / (area0 + area1))) * (1.0 / 3.0) + offset;
        power = asfloat(leaf[1]);
        child = 0x80000000u | rank;
        let base = ((p.count - 1u) * 10u + 7u) & ~7u;
        let edge_squared = asuint(dot(b - a, b - a) + dot(d - a, d - a));
        let encoded = (p.level << 27u) | path;
        if (p.layout == 0u) {
            let at = base + 8u * rank;
            let lo = min(min(a, b), min(c, d)) + offset;
            let hi = max(max(a, b), max(c, d)) + offset;
            tree[at] = asuint(lo.x);
            tree[at + 1u] = asuint(lo.y);
            tree[at + 2u] = asuint(lo.z);
            tree[at + 3u] = edge_squared;
            tree[at + 4u] = asuint(hi.x);
            tree[at + 5u] = asuint(hi.y);
            tree[at + 6u] = asuint(hi.z);
            tree[at + 7u] = encoded;
        } else {
            let at = base + 2u * rank;
            tree[at] = edge_squared;
            tree[at + 1u] = encoded;
        }
        if (p.write_records != 0u) {
            var out_row : uint * = p.records + 16u * rank;
            let area = light_sampling::dynamic_quad_area(local);
            out_row[0] = area.x;
            out_row[1] = area.y;
            out_row[2] = area.z;
            out_row[3] = leaf[2];
            [unroll]
            for (var k: uint = 0; k < 4; ++k) {
                let v = point_at(row, k) + offset;
                out_row[4u + 3u * k] = asuint(v.x);
                out_row[5u + 3u * k] = asuint(v.y);
                out_row[6u + 3u * k] = asuint(v.z);
            }
        } else if (p.layout != 0u) {
            p.records[rank] = leaf[2];
        }
    } else {
        let at = metric_at(packet);
        let a = asfloat(uint3(tree[at], tree[at + 1u], tree[at + 2u]));
        let b = asfloat(uint3(tree[at + 4u], tree[at + 5u], tree[at + 6u]));
        let pa = asfloat(tree[at + 3u]);
        let pb = asfloat(tree[at + 7u]);
        power = pa + pb;
        center = lerp(a, b, pb / power);
        child = packet;
    }
    if (p.level != 0) {
        let at = metric_at(parent) + side * 4u;
        tree[at] = asuint(center.x);
        tree[at + 1u] = asuint(center.y);
        tree[at + 2u] = asuint(center.z);
        tree[at + 3u] = asuint(power);
        tree[child_at(parent, p.count) + side] = child;
    } else if (p.layout != 0u) {
        let summary = (float4 *)p.tree;
        *summary = float4(center, power);
    }
}

// 世界树只读取各区块独立发布的功率摘要。
func build_upper(p: Build, node: uint)->void {
    let world = *p.world;
    var begin : uint;
    var packet : uint;
    var parent : uint;
    var side : uint;
    var path : uint;
    let n = locate(world.counts.y, p.level, node, begin, packet, parent, side, path);
    if (n == 0u)
        return;
    var metric : float4;
    var child : uint;
    var count : uint;
    if (n == 1u) {
        let region = world.regions[begin];
        let summary = asfloat(region.words[0]);
        metric =
            float4(summary.xyz + light_bvh::light_grid_offset(region.grid.xyz, world.anchor.xyz),
                   summary.w);
        child = 0x80000000u | begin;
        count = region.count;
        world.regions[begin].grid.w = asint((p.level << 27u) | path);
    } else {
        let at = 3u * packet;
        let a = asfloat(world.topology[at]);
        let b = asfloat(world.topology[at + 1u]);
        let power = a.w + b.w;
        metric = float4(lerp(a.xyz, b.xyz, b.w / power), power);
        child = packet;
        count = world.topology[at + 2u].z + world.topology[at + 2u].w;
    }
    if (p.level != 0u) {
        let at = 3u * parent;
        world.topology[at + side] = asuint(metric);
        // 两个子节点分别写不同的标量，不可对完整 uint4 做读改写。
        let topology = (uint *)(world.topology + at + 2u);
        topology[side] = child;
        topology[2u + side] = count;
    }
}

[shader("compute")]
[numthreads(64, 1, 1)]
func build_level(uint3 thread: SV_DispatchThreadID, uint lane: SV_GroupIndex)->void {
    let id = launch.first + thread.x;
    if (id >= launch.work)
        return;
    var p : Build = launch;
    if (p.mode == 3u) {
        build_upper(p, id);
        return;
    }
    if (p.layout == 0u) {
        build_local(p, id, lane);
        return;
    }
    let job = p.jobs[p.job_first + (id >> p.shift)];
    p.count = job.count;
    p.leaves = p.leaves + 4u * job.first;
    p.tree = (uint *)job.words;
    p.records = p.tree + 4u * light_bvh::region_payload(job.count);
    p.anchor_x = job.grid.x;
    p.anchor_y = job.grid.y;
    p.anchor_z = job.grid.z;
    if (p.count > 1u) {
        p.sorted = p.sorted + job.first;
        p.destination = p.destination + job.first;
    }
    build_local(p, id & ((1u << p.shift) - 1u), lane);
}
