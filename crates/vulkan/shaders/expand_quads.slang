// 与 scene_cache::PackedQuads 的连续 word 流对应。地址与范围由本批所有者建立并保活。
struct Expansion
{
    uint *headers;
    uint *payload;
    uint *vertices;
    uint *attributes;
    uint *identities;
    uint *indices;
    uint *cells;
    uint *id_input;
    uint first;
    uint count;
    uint2 base_id;
    int region_x;
    int region_y;
    int region_z;
    uint mask;
};
[[vk::push_constant]]
ConstantBuffer<Expansion> parameters;

[shader("compute")]
[numthreads(64, 1, 1)]
void expand_quads(uint3 tid: SV_DispatchThreadID)
{
    uint slot = parameters.first + tid.x;
    if (slot >= parameters.count)
        return;
    uint header = parameters.headers[slot * 2u];
    uint selected = parameters.headers[slot * 2u + 1u];
    uint *surface = parameters.headers + parameters.count * 2u + (selected & 0x7fffffffu) * 16u;
    uint format = (header >> 29u) & 3u;
    uint vertices[12];
    int3 cell = int3(surface[1], surface[2], surface[3]);
    if ((selected >> 31u) != 0u)
    {
        uint3 anchor = uint3(header, header >> 6u, header >> 12u) & 63u;
        uint lod = (surface[0] >> 4u) & 31u;
        cell = int3(anchor << lod);
        [unroll]
        for (uint i = 0; i < 12; ++i)
            vertices[i] =
                asuint(float(anchor[i % 3u] + ((header >> (18u + i)) & 1u)) * float(1u << lod));
    }
    else if ((header >> 31u) == 0u)
    {
        uint layout = surface[0], axis = layout & 3u, u = (axis + 1u) % 3u, v = (axis + 2u) % 3u;
        uint3 lo = uint3(header, header >> 6u, header >> 12u) & 63u;
        cell = int3(lo << ((layout >> 4u) & 31u));
        uint positive = (header >> 30u) & 1u;
        lo[axis] += positive;
        uint3 hi = lo;
        hi[u] += ((header >> 18u) & 63u) + 1u;
        hi[v] += ((header >> 24u) & 63u) + 1u;
        uint3 a = lo, c = hi;
        bool fu = (layout & 4u) != 0, fv = (layout & 8u) != 0;
        a[u] = fu ? hi[u] : lo[u];
        c[u] = fu ? lo[u] : hi[u];
        a[v] = fv ? hi[v] : lo[v];
        c[v] = fv ? lo[v] : hi[v];
        uint3 b = a, d = a;
        bool along_u = ((positive != 0u) != fu) != fv;
        b[along_u ? u : v] = c[along_u ? u : v];
        d[along_u ? v : u] = c[along_u ? v : u];
        float scale = float(1u << ((layout >> 4u) & 31u));
        [unroll]
        for (uint j = 0; j < 3; ++j)
        {
            vertices[j] = asuint(float(a[j]) * scale);
            vertices[j + 3] = asuint(float(b[j]) * scale);
            vertices[j + 6] = asuint(float(c[j]) * scale);
            vertices[j + 9] = asuint(float(d[j]) * scale);
        }
    }
    else if (format == 0u)
    {
        uint *src = parameters.payload + (header & 0x1fffffffu);
        uint a = src[0], c = src[1];
        [unroll]
        for (uint axis = 0; axis < 3; ++axis)
        {
            uint av = asuint(float((a >> (8u * axis)) & 255u));
            uint cv = asuint(float((c >> (8u * axis)) & 255u));
            vertices[axis] = av;
            vertices[3 + axis] = ((a >> (24 + axis)) & 1u) != 0 ? cv : av;
            vertices[6 + axis] = cv;
            vertices[9 + axis] = ((c >> (24 + axis)) & 1u) != 0 ? cv : av;
        }
    }
    else
    {
        uint *src = parameters.payload + (header & 0x1fffffffu);
        [unroll]
        for (uint i = 0; i < 12; ++i)
        {
            if (format == 1u)
                vertices[i] = asuint(float((src[i >> 2] >> ((i & 3u) * 8u)) & 255u));
            else if (format == 2u)
                vertices[i] =
                    asuint(float(int(src[i >> 1] << ((1u - (i & 1u)) * 16u)) >> 16) * 0.0625);
            else
                vertices[i] = src[i];
        }
    }
    int3 region = int3(parameters.region_x, parameters.region_y, parameters.region_z);
    uint4 tags = uint4(asuint(region), 0u);
    uint *dst = parameters.vertices + slot * 16u;
    [unroll]
    for (uint j = 0; j < 4; ++j)
    {
        dst[4u * j] = vertices[3u * j];
        dst[4u * j + 1u] = vertices[3u * j + 1u];
        dst[4u * j + 2u] = vertices[3u * j + 2u];
        dst[4u * j + 3u] = tags[j];
    }
    uint flags = surface[13];
    if ((surface[0] & 512u) != 0u)
        flags |= (selected >> 31u) != 0u
                     ? 257u
                     : (((header >> 18u) & 63u) + 1u) | ((((header >> 24u) & 63u) + 1u) << 8u);
    if ((parameters.mask & 1u) != 0u)
    {
        uint *dst_attr = parameters.attributes + slot * 12u;
        if ((surface[0] & 512u) == 0u)
        {
            [unroll]
            for (uint k = 0; k < 12; ++k)
                dst_attr[k] = surface[4u + k];
        }
        else
        {
            float scale = float(1u << ((surface[0] >> 4u) & 31u));
            [unroll]
            for (uint j = 0; j < 2; ++j)
            {
                uint code = (surface[0] >> (10u + 3u * j)) & 7u, axis = code & 3u,
                     sign = (code & 4u) << 29u;
                [unroll]
                for (uint i = 0; i < 4; ++i)
                    dst_attr[2u * i + j] = asuint(asfloat(vertices[3u * i + axis]) / scale) ^ sign;
                dst_attr[10u + j] = asuint(float(cell[axis]) / scale) ^ sign;
            }
            dst_attr[8] = surface[12];
            dst_attr[9] = flags;
        }
    }
    if ((parameters.mask & 2u) != 0u)
    {
        uint2 id = parameters.base_id;
        if ((parameters.mask & 16u) != 0u)
            id = uint2(parameters.id_input[slot * 2u], parameters.id_input[slot * 2u + 1u]);
        else
        {
            id.x += slot;
            id.y += uint(id.x < slot);
        }
        parameters.identities[slot * 2u] = id.x;
        parameters.identities[slot * 2u + 1u] = id.y;
    }
    if ((parameters.mask & 4u) != 0u)
    {
        uint *row = parameters.indices + slot * 6u;
        uint at = slot * 4u;
        row[0] = at;
        row[1] = at + 1u;
        row[2] = at + 2u;
        row[3] = at;
        row[4] = at + 2u;
        row[5] = at + 3u;
    }
    if ((parameters.mask & 8u) != 0u)
    {
        uint *row = parameters.cells + slot * 4u;
        int3 world = region * 64 + cell;
        row[0] = asuint(world.x);
        row[1] = asuint(world.y);
        row[2] = asuint(world.z);
        row[3] = flags;
    }
}
