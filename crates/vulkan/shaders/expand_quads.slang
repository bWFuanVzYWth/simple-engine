// 与 scene_cache::PackedQuads 的连续 word 流对应。地址与范围由本批所有者建立并保活。
struct Expansion
{
    var headers : uint *;
    var payload : uint *;
    var vertices : uint *;
    var attributes : uint *;
    var identities : uint *;
    var indices : uint *;
    var cells : uint *;
    var id_input : uint *;
    var first : uint;
    var count : uint;
    var base_id : uint2;
    var region_x : int;
    var region_y : int;
    var region_z : int;
    var mask : uint;
};
[[vk::push_constant]]
var parameters : ConstantBuffer<Expansion>;

[shader("compute")]
[numthreads(64, 1, 1)]
func expand_quads(uint3 tid: SV_DispatchThreadID)->void
{
    let slot : uint = parameters.first + tid.x;
    if (slot >= parameters.count)
        return;
    let header : uint = parameters.headers[slot * 2u];
    let selected : uint = parameters.headers[slot * 2u + 1u];
    let surface : uint * =
                      parameters.headers + parameters.count * 2u + (selected & 0x7fffffffu) * 16u;
    let format : uint = (header >> 29u) & 3u;
    var vertices : uint[12];
    var cell = int3(surface[1], surface[2], surface[3]);
    if ((selected >> 31u) != 0u)
    {
        let anchor : uint3 = uint3(header, header >> 6u, header >> 12u) & 63u;
        let lod : uint = (surface[0] >> 4u) & 31u;
        cell = int3(anchor << lod);
        [unroll]
        for (var i: uint = 0; i < 12; ++i)
            vertices[i] =
                asuint(float(anchor[i % 3u] + ((header >> (18u + i)) & 1u)) * float(1u << lod));
    }
    else if ((header >> 31u) == 0u)
    {
        let layout : uint = surface[0];
        let axis : uint = layout & 3u;
        let u : uint = (axis + 1u) % 3u;
        let v : uint = (axis + 2u) % 3u;
        var lo : uint3 = uint3(header, header >> 6u, header >> 12u) & 63u;
        cell = int3(lo << ((layout >> 4u) & 31u));
        let positive : uint = (header >> 30u) & 1u;
        lo[axis] += positive;
        var hi : uint3 = lo;
        hi[u] += ((header >> 18u) & 63u) + 1u;
        hi[v] += ((header >> 24u) & 63u) + 1u;
        var a : uint3 = lo;
        var c : uint3 = hi;
        let fu : bool = (layout & 4u) != 0;
        let fv : bool = (layout & 8u) != 0;
        a[u] = fu ? hi[u] : lo[u];
        c[u] = fu ? lo[u] : hi[u];
        a[v] = fv ? hi[v] : lo[v];
        c[v] = fv ? lo[v] : hi[v];
        var b : uint3 = a;
        var d : uint3 = a;
        let along_u : bool = ((positive != 0u) != fu) != fv;
        b[along_u ? u : v] = c[along_u ? u : v];
        d[along_u ? v : u] = c[along_u ? v : u];
        let scale = float(1u << ((layout >> 4u) & 31u));
        [unroll]
        for (var j: uint = 0; j < 3; ++j)
        {
            vertices[j] = asuint(float(a[j]) * scale);
            vertices[j + 3] = asuint(float(b[j]) * scale);
            vertices[j + 6] = asuint(float(c[j]) * scale);
            vertices[j + 9] = asuint(float(d[j]) * scale);
        }
    }
    else if (format == 0u)
    {
        let src : uint * = parameters.payload + (header & 0x1fffffffu);
        let a : uint = src[0];
        let c : uint = src[1];
        [unroll]
        for (var axis: uint = 0; axis < 3; ++axis)
        {
            let av : uint = asuint(float((a >> (8u * axis)) & 255u));
            let cv : uint = asuint(float((c >> (8u * axis)) & 255u));
            vertices[axis] = av;
            vertices[3 + axis] = ((a >> (24 + axis)) & 1u) != 0 ? cv : av;
            vertices[6 + axis] = cv;
            vertices[9 + axis] = ((c >> (24 + axis)) & 1u) != 0 ? cv : av;
        }
    }
    else
    {
        let src : uint * = parameters.payload + (header & 0x1fffffffu);
        [unroll]
        for (var i: uint = 0; i < 12; ++i)
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
    let region = int3(parameters.region_x, parameters.region_y, parameters.region_z);
    let tags = uint4(asuint(region), 0u);
    var dst : uint * = parameters.vertices + slot * 16u;
    [unroll]
    for (var j: uint = 0; j < 4; ++j)
    {
        dst[4u * j] = vertices[3u * j];
        dst[4u * j + 1u] = vertices[3u * j + 1u];
        dst[4u * j + 2u] = vertices[3u * j + 2u];
        dst[4u * j + 3u] = tags[j];
    }
    var flags : uint = surface[13];
    if ((surface[0] & 512u) != 0u)
        flags |= (selected >> 31u) != 0u
                     ? 257u
                     : (((header >> 18u) & 63u) + 1u) | ((((header >> 24u) & 63u) + 1u) << 8u);
    if ((parameters.mask & 1u) != 0u)
    {
        var dst_attr : uint * = parameters.attributes + slot * 12u;
        if ((surface[0] & 512u) == 0u)
        {
            [unroll]
            for (var k: uint = 0; k < 12; ++k)
                dst_attr[k] = surface[4u + k];
        }
        else
        {
            let scale = float(1u << ((surface[0] >> 4u) & 31u));
            [unroll]
            for (var j: uint = 0; j < 2; ++j)
            {
                let code : uint = (surface[0] >> (10u + 3u * j)) & 7u;
                let axis : uint = code & 3u;
                let sign : uint = (code & 4u) << 29u;
                [unroll]
                for (var i: uint = 0; i < 4; ++i)
                    dst_attr[2u * i + j] = asuint(asfloat(vertices[3u * i + axis]) / scale) ^ sign;
                dst_attr[10u + j] = asuint(float(cell[axis]) / scale) ^ sign;
            }
            dst_attr[8] = surface[12];
            dst_attr[9] = flags;
        }
    }
    if ((parameters.mask & 2u) != 0u)
    {
        var id : uint2 = parameters.base_id;
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
        var row : uint * = parameters.indices + slot * 6u;
        let at : uint = slot * 4u;
        row[0] = at;
        row[1] = at + 1u;
        row[2] = at + 2u;
        row[3] = at;
        row[4] = at + 2u;
        row[5] = at + 3u;
    }
    if ((parameters.mask & 8u) != 0u)
    {
        var row : uint * = parameters.cells + slot * 4u;
        let world : int3 = region * 64 + cell;
        row[0] = asuint(world.x);
        row[1] = asuint(world.y);
        row[2] = asuint(world.z);
        row[3] = flags;
    }
}
