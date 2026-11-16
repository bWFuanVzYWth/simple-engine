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
func expand_quads(dispatch_id: uint3: SV_DispatchThreadID)->void
{
    let slot = parameters.first + dispatch_id.x;
    if (slot >= parameters.count)
        return;
    let geometry_word = parameters.headers[slot * 2u];
    let surface_word = parameters.headers[slot * 2u + 1u];
    let surface_words =
        parameters.headers + parameters.count * 2u + (surface_word & 0x7fffffffu) * 16u;
    let payload_format = (geometry_word >> 29u) & 3u;
    var vertex_words : uint[12];
    var cell = int3(surface_words[1], surface_words[2], surface_words[3]);
    if ((surface_word >> 31u) != 0u)
    {
        let anchor = uint3(geometry_word, geometry_word >> 6u, geometry_word >> 12u) & 63u;
        let lod = (surface_words[0] >> 4u) & 31u;
        cell = int3(anchor << lod);
        [unroll]
        for (var i = 0u; i < 12; ++i)
            vertex_words[i] = asuint(float(anchor[i % 3u] + ((geometry_word >> (18u + i)) & 1u)) *
                                     float(1u << lod));
    }
    else if ((geometry_word >> 31u) == 0u)
    {
        let layout = surface_words[0];
        let axis = layout & 3u;
        let u = (axis + 1u) % 3u;
        let v = (axis + 2u) % 3u;
        var lower = uint3(geometry_word, geometry_word >> 6u, geometry_word >> 12u) & 63u;
        cell = int3(lower << ((layout >> 4u) & 31u));
        let positive = (geometry_word >> 30u) & 1u;
        lower[axis] += positive;
        var upper = lower;
        upper[u] += ((geometry_word >> 18u) & 63u) + 1u;
        upper[v] += ((geometry_word >> 24u) & 63u) + 1u;
        var a = lower;
        var c = upper;
        let flip_u = (layout & 4u) != 0;
        let flip_v = (layout & 8u) != 0;
        a[u] = flip_u ? upper[u] : lower[u];
        c[u] = flip_u ? lower[u] : upper[u];
        a[v] = flip_v ? upper[v] : lower[v];
        c[v] = flip_v ? lower[v] : upper[v];
        var b = a;
        var d = a;
        let along_u = ((positive != 0u) != flip_u) != flip_v;
        b[along_u ? u : v] = c[along_u ? u : v];
        d[along_u ? v : u] = c[along_u ? v : u];
        let scale = float(1u << ((layout >> 4u) & 31u));
        [unroll]
        for (var j = 0u; j < 3; ++j)
        {
            vertex_words[j] = asuint(float(a[j]) * scale);
            vertex_words[j + 3] = asuint(float(b[j]) * scale);
            vertex_words[j + 6] = asuint(float(c[j]) * scale);
            vertex_words[j + 9] = asuint(float(d[j]) * scale);
        }
    }
    else if (payload_format == 0u)
    {
        let payload_words = parameters.payload + (geometry_word & 0x1fffffffu);
        let a = payload_words[0];
        let c = payload_words[1];
        [unroll]
        for (var axis = 0u; axis < 3; ++axis)
        {
            let av = asuint(float((a >> (8u * axis)) & 255u));
            let cv = asuint(float((c >> (8u * axis)) & 255u));
            vertex_words[axis] = av;
            vertex_words[3 + axis] = ((a >> (24 + axis)) & 1u) != 0 ? cv : av;
            vertex_words[6 + axis] = cv;
            vertex_words[9 + axis] = ((c >> (24 + axis)) & 1u) != 0 ? cv : av;
        }
    }
    else
    {
        let payload_words = parameters.payload + (geometry_word & 0x1fffffffu);
        [unroll]
        for (var i = 0u; i < 12; ++i)
        {
            if (payload_format == 1u)
                vertex_words[i] = asuint(float((payload_words[i >> 2] >> ((i & 3u) * 8u)) & 255u));
            else if (payload_format == 2u)
                vertex_words[i] = asuint(
                    float(int(payload_words[i >> 1] << ((1u - (i & 1u)) * 16u)) >> 16) * 0.0625);
            else
                vertex_words[i] = payload_words[i];
        }
    }
    let region = int3(parameters.region_x, parameters.region_y, parameters.region_z);
    let tags = uint4(asuint(region), 0u);
    let vertex_output = parameters.vertices + slot * 16u;
    [unroll]
    for (var j = 0u; j < 4; ++j)
    {
        vertex_output[4u * j] = vertex_words[3u * j];
        vertex_output[4u * j + 1u] = vertex_words[3u * j + 1u];
        vertex_output[4u * j + 2u] = vertex_words[3u * j + 2u];
        vertex_output[4u * j + 3u] = tags[j];
    }
    var flags = surface_words[13];
    if ((surface_words[0] & 512u) != 0u)
        flags |= (surface_word >> 31u) != 0u ? 257u
                                             : (((geometry_word >> 18u) & 63u) + 1u) |
                                                   ((((geometry_word >> 24u) & 63u) + 1u) << 8u);
    if ((parameters.mask & 1u) != 0u)
    {
        let attribute_output = parameters.attributes + slot * 12u;
        if ((surface_words[0] & 512u) == 0u)
        {
            [unroll]
            for (var k = 0u; k < 12; ++k)
                attribute_output[k] = surface_words[4u + k];
        }
        else
        {
            let scale = float(1u << ((surface_words[0] >> 4u) & 31u));
            [unroll]
            for (var j = 0u; j < 2; ++j)
            {
                let code = (surface_words[0] >> (10u + 3u * j)) & 7u;
                let axis = code & 3u;
                let sign = (code & 4u) << 29u;
                [unroll]
                for (var i = 0u; i < 4; ++i)
                    attribute_output[2u * i + j] =
                        asuint(asfloat(vertex_words[3u * i + axis]) / scale) ^ sign;
                attribute_output[10u + j] = asuint(float(cell[axis]) / scale) ^ sign;
            }
            attribute_output[8] = surface_words[12];
            attribute_output[9] = flags;
        }
    }
    if ((parameters.mask & 2u) != 0u)
    {
        var id = parameters.base_id;
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
        let index_output = parameters.indices + slot * 6u;
        let first_vertex = slot * 4u;
        index_output[0] = first_vertex;
        index_output[1] = first_vertex + 1u;
        index_output[2] = first_vertex + 2u;
        index_output[3] = first_vertex;
        index_output[4] = first_vertex + 2u;
        index_output[5] = first_vertex + 3u;
    }
    if ((parameters.mask & 8u) != 0u)
    {
        let cell_output = parameters.cells + slot * 4u;
        let world = region * 64 + cell;
        cell_output[0] = asuint(world.x);
        cell_output[1] = asuint(world.y);
        cell_output[2] = asuint(world.z);
        cell_output[3] = flags;
    }
}
