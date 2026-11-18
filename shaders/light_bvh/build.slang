// 在 GPU 上按空间 Morton 序二分，叶记录保持原始 rank，与正反向 PDF 共用布局。
import light_sampling.area;

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
};
[[vk::push_constant]]
var p : ConstantBuffer<Build>;

func metric_at(packet: uint)->uint {
    return (packet >> 2u) * 40u + (packet & 3u) * 8u;
}
func child_at(packet: uint)->uint {
    let block : uint = packet >> 2u;
    return block * 40u + min(4u, p.count - 1u - block * 4u) * 8u + (packet & 3u) * 2u;
}
func region_delta(a: int, b: int)->float {
    return a >= b ? float(asuint(a) - asuint(b)) : -float(asuint(b) - asuint(a));
}
func point_at(row: uint *, i: uint)->float3 {
    return asfloat(uint3(row[i * 4u], row[i * 4u + 1u], row[i * 4u + 2u]));
}
func spread10(v: uint)->uint {
    var v = v;
    v &= 1023u;
    v = (v | (v << 16u)) & 0x30000ffu;
    v = (v | (v << 8u)) & 0x300f00fu;
    v = (v | (v << 4u)) & 0x30c30c3u;
    return (v | (v << 2u)) & 0x9249249u;
}
func digit(v: uint3)->uint {
    return spread10(v.x) | (spread10(v.y) << 1u) | (spread10(v.z) << 2u);
}
func less_key(a: uint4, b: uint4)->bool {
    return a.x < b.x ||
           (a.x == b.x && (a.y < b.y || (a.y == b.y && (a.z < b.z || (a.z == b.z && a.w < b.w)))));
}
groupshared var tile : uint4[64];
func spatial_key(index: uint)->uint4 {
    let q = p.leaves[4u * index];
    if (q >= p.vertex_quads)
        return uint4(0xffffffffu, 0xffffffffu, 0xffffffffu, index);
    let row : uint * = p.vertices + 16u * q;
    var center : float3 = point_at(row, 0) * 0.25 + point_at(row, 1) * 0.25 +
                          point_at(row, 2) * 0.25 + point_at(row, 3) * 0.25;
    center +=
        float3(region_delta(asint(row[3]), p.anchor_x), region_delta(asint(row[7]), p.anchor_y),
               region_delta(asint(row[11]), p.anchor_z)) *
        64.0;
    var bits : uint3 = asuint(center);
    bits ^= asuint(asint(bits) >> 31) | 0x80000000u;
    return uint4(digit(bits >> 22u), digit(bits >> 12u), digit(bits >> 2u), index);
}
[shader("compute")]
[numthreads(64, 1, 1)]
func build_level(uint3 thread: SV_DispatchThreadID, uint lane: SV_GroupIndex)->void {
    let node : uint = p.first + thread.x;
    if (p.mode == 1u) {
        var key = uint4(0xffffffffu);
        if (node < p.count)
            key = spatial_key(node);
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
                    if ((lane & width) == 0u ? less_key(b, a) : less_key(a, b)) {
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
            if (less_key(p.sorted[mid], key))
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
    var n : uint = p.count;
    var begin : uint = 0;
    var packet : uint = 0;
    var parent : uint = 0;
    var side : uint = 0;
    var path : uint = 0;
    for (var depth: uint = 0; depth < p.level; ++depth) {
        if (n <= 1u)
            return;
        let left = n / 2u;
        side = (node >> (p.level - 1u - depth)) & 1u;
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
    var center : float3;
    var power : float;
    var child : uint;
    if (n == 1u) {
        let rank : uint = p.count == 1u ? begin : p.sorted[begin].w;
        let leaf : uint * = p.leaves + 4u * rank;
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
        let at : uint = (((p.count - 1u) * 10u + 7u) & ~7u) + rank * 8u;
        let lo : float3 = min(min(a, b), min(c, d)) + offset;
        let hi : float3 = max(max(a, b), max(c, d)) + offset;
        p.tree[at] = asuint(lo.x);
        p.tree[at + 1u] = asuint(lo.y);
        p.tree[at + 2u] = asuint(lo.z);
        p.tree[at + 3u] = asuint(dot(b - a, b - a) + dot(d - a, d - a));
        p.tree[at + 4u] = asuint(hi.x);
        p.tree[at + 5u] = asuint(hi.y);
        p.tree[at + 6u] = asuint(hi.z);
        p.tree[at + 7u] = (p.level << 27u) | path;
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
        }
    } else {
        let at = metric_at(packet);
        let a = asfloat(uint3(p.tree[at], p.tree[at + 1u], p.tree[at + 2u]));
        let b = asfloat(uint3(p.tree[at + 4u], p.tree[at + 5u], p.tree[at + 6u]));
        let pa = asfloat(p.tree[at + 3u]);
        let pb = asfloat(p.tree[at + 7u]);
        power = pa + pb;
        center = lerp(a, b, pb / power);
        child = packet;
    }
    if (p.level != 0) {
        let at = metric_at(parent) + side * 4u;
        p.tree[at] = asuint(center.x);
        p.tree[at + 1u] = asuint(center.y);
        p.tree[at + 2u] = asuint(center.z);
        p.tree[at + 3u] = asuint(power);
        p.tree[child_at(parent) + side] = child;
    }
}
