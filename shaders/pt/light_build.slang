// 在 GPU 上按空间 Morton 序二分，叶记录保持原始 rank，与正反向 PDF 共用布局。
import pt.light_area;

struct Build
{
    uint *vertices;
    uint *leaves;
    uint *tree;
    uint *records;
    uint count;
    uint level;
    uint first;
    uint write_records;
    int anchor_x;
    int anchor_y;
    int anchor_z;
    uint vertex_quads;
    uint4 *sorted;
    uint4 *destination;
    uint mode;
    uint width;
};
[[vk::push_constant]]
ConstantBuffer<Build> p;

uint metric_at(uint packet)
{
    return (packet >> 2u) * 40u + (packet & 3u) * 8u;
}
uint child_at(uint packet)
{
    uint block = packet >> 2u;
    return block * 40u + min(4u, p.count - 1u - block * 4u) * 8u + (packet & 3u) * 2u;
}
float region_delta(int a, int b)
{
    return a >= b ? float(asuint(a) - asuint(b)) : -float(asuint(b) - asuint(a));
}
float3 point_at(uint *row, uint i)
{
    return asfloat(uint3(row[i * 4u], row[i * 4u + 1u], row[i * 4u + 2u]));
}
uint spread10(uint v)
{
    v &= 1023u;
    v = (v | (v << 16u)) & 0x30000ffu;
    v = (v | (v << 8u)) & 0x300f00fu;
    v = (v | (v << 4u)) & 0x30c30c3u;
    return (v | (v << 2u)) & 0x9249249u;
}
uint digit(uint3 v)
{
    return spread10(v.x) | (spread10(v.y) << 1u) | (spread10(v.z) << 2u);
}
bool less_key(uint4 a, uint4 b)
{
    return a.x < b.x ||
           (a.x == b.x && (a.y < b.y || (a.y == b.y && (a.z < b.z || (a.z == b.z && a.w < b.w)))));
}
groupshared uint4 tile[64];
uint4 spatial_key(uint index)
{
    uint q = p.leaves[4u * index];
    if (q >= p.vertex_quads)
        return uint4(0xffffffffu, 0xffffffffu, 0xffffffffu, index);
    uint *row = p.vertices + 16u * q;
    float3 center = point_at(row, 0) * 0.25 + point_at(row, 1) * 0.25 + point_at(row, 2) * 0.25 +
                    point_at(row, 3) * 0.25;
    center +=
        float3(region_delta(asint(row[3]), p.anchor_x), region_delta(asint(row[7]), p.anchor_y),
               region_delta(asint(row[11]), p.anchor_z)) *
        64.0;
    uint3 bits = asuint(center);
    bits ^= asuint(asint(bits) >> 31) | 0x80000000u;
    return uint4(digit(bits >> 22u), digit(bits >> 12u), digit(bits >> 2u), index);
}
[shader("compute")]
[numthreads(64, 1, 1)]
void build_level(uint3 thread: SV_DispatchThreadID, uint lane: SV_GroupIndex)
{
    uint node = p.first + thread.x;
    if (p.mode == 1u)
    {
        uint4 key = uint4(0xffffffffu);
        if (node < p.count)
            key = spatial_key(node);
        tile[lane] = key;
        GroupMemoryBarrierWithGroupSync();
        [unroll]
        for (uint width = 2; width <= 64; width *= 2)
        {
            [unroll]
            for (uint stride = width / 2; stride > 0; stride /= 2)
            {
                uint peer = lane ^ stride;
                if (lane < peer)
                {
                    uint4 a = tile[lane], b = tile[peer];
                    if ((lane & width) == 0u ? less_key(b, a) : less_key(a, b))
                    {
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
    if (p.mode == 2u)
    {
        if (node >= p.count)
            return;
        uint begin = node / (2u * p.width) * (2u * p.width);
        bool right = node - begin >= p.width;
        uint other = right ? begin : begin + p.width;
        uint lo = min(other, p.count), hi = min(other + p.width, p.count);
        uint4 key = p.sorted[node];
        while (lo < hi)
        {
            uint mid = lo + (hi - lo) / 2u;
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
    uint n = p.count, begin = 0, packet = 0, parent = 0, side = 0, path = 0;
    for (uint depth = 0; depth < p.level; ++depth)
    {
        if (n <= 1u)
            return;
        uint left = n / 2u;
        side = (node >> (p.level - 1u - depth)) & 1u;
        path |= side << depth;
        parent = packet;
        if (side == 0u)
        {
            packet += 1u;
            n = left;
        }
        else
        {
            packet += left;
            begin += left;
            n -= left;
        }
    }
    float3 center;
    float power;
    uint child;
    if (n == 1u)
    {
        uint rank = p.count == 1u ? begin : p.sorted[begin].w;
        uint *leaf = p.leaves + 4u * rank;
        if (leaf[0] >= p.vertex_quads)
            return;
        uint *row = p.vertices + 16u * leaf[0];
        int3 region = asint(uint3(row[3], row[7], row[11]));
        float3 offset =
            float3(region_delta(region.x, p.anchor_x), region_delta(region.y, p.anchor_y),
                   region_delta(region.z, p.anchor_z)) *
            64.0;
        pt::LightQuadVertices local = { point_at(row, 0), point_at(row, 1), point_at(row, 2),
                                        point_at(row, 3) };
        float3 a = local.a, b = local.b, c = local.c, d = local.d;
        float area0 = length(cross(b - a, c - a)), area1 = length(cross(c - a, d - a));
        center = (a + c + lerp(d, b, area0 / (area0 + area1))) / 3.0 + offset;
        power = asfloat(leaf[1]);
        child = 0x80000000u | rank;
        uint at = (((p.count - 1u) * 10u + 7u) & ~7u) + rank * 8u;
        float3 lo = min(min(a, b), min(c, d)) + offset, hi = max(max(a, b), max(c, d)) + offset;
        p.tree[at] = asuint(lo.x);
        p.tree[at + 1u] = asuint(lo.y);
        p.tree[at + 2u] = asuint(lo.z);
        p.tree[at + 3u] = asuint(dot(b - a, b - a) + dot(d - a, d - a));
        p.tree[at + 4u] = asuint(hi.x);
        p.tree[at + 5u] = asuint(hi.y);
        p.tree[at + 6u] = asuint(hi.z);
        p.tree[at + 7u] = (p.level << 27u) | path;
        if (p.write_records != 0u)
        {
            uint *out_row = p.records + 16u * rank;
            uint4 area = pt::dynamic_quad_area(local);
            out_row[0] = area.x;
            out_row[1] = area.y;
            out_row[2] = area.z;
            out_row[3] = leaf[2];
            [unroll]
            for (uint k = 0; k < 4; ++k)
            {
                float3 v = point_at(row, k) + offset;
                out_row[4u + 3u * k] = asuint(v.x);
                out_row[5u + 3u * k] = asuint(v.y);
                out_row[6u + 3u * k] = asuint(v.z);
            }
        }
    }
    else
    {
        uint at = metric_at(packet);
        float3 a = asfloat(uint3(p.tree[at], p.tree[at + 1u], p.tree[at + 2u]));
        float3 b = asfloat(uint3(p.tree[at + 4u], p.tree[at + 5u], p.tree[at + 6u]));
        float pa = asfloat(p.tree[at + 3u]), pb = asfloat(p.tree[at + 7u]);
        power = pa + pb;
        center = lerp(a, b, pb / power);
        child = packet;
    }
    if (p.level != 0)
    {
        uint at = metric_at(parent) + side * 4u;
        p.tree[at] = asuint(center.x);
        p.tree[at + 1u] = asuint(center.y);
        p.tree[at + 2u] = asuint(center.z);
        p.tree[at + 3u] = asuint(power);
        p.tree[child_at(parent) + side] = child;
    }
}
