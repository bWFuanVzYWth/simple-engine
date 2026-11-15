// 来源：Sky Tracer 9e9430d；见 crates/atmosphere/data/README.md。
import atmosphere.parameters;
import atmosphere.sampling;
using namespace atmosphere;
static const let SKIP_INACTIVE_SPECIES = true;
static const let CACHE_PHASE_COORDINATE = false;
struct AerialView
{
    column_major var matrix : float4x4;
    var settings : float4;
};
import atmosphere_bindings;
[[vk::binding(B0, 0)]]
var p : ConstantBuffer<Params>;
[[vk::binding(B1, 0)]]
var data : StructuredBuffer<float4>;
[[vk::binding(B2, 0)]]
var tau_read : Texture2D<float4>;
[[vk::binding(B3, 0)]]
var source_read : Texture2D<float4>;
[[vk::binding(B4, 0)]]
var mean_read : Texture2D<float4>;
[[vk::binding(B5, 0)]]
var ground_read : Texture2D<float4>;
[[vk::binding(B6, 0)]]
var linear_sampler : SamplerState;
[[vk::binding(B7, 0)]]
var incoming : RWStructuredBuffer<float4>;
[vk::image_format("rgba16f")]
[[vk::binding(B8, 0)]]
var source_write : RWTexture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B9, 0)]]
var mean_write : RWTexture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B10, 0)]]
var ground_write : RWTexture2D<float4>;
[vk::image_format("rgba16f")]
[[vk::binding(B11, 0)]]
var tau_write : RWTexture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B12, 0)]]
var output : RWTexture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B13, 0)]]
var output_t : RWTexture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B14, 0)]]
var sky_write : RWTexture2D<float4>;
[[vk::binding(B15, 0)]]
var sky_read : Texture2D<float4>;
[[vk::binding(B16, 0)]]
var directions : RWStructuredBuffer<float4>;
[[vk::binding(B17, 0)]]
var moments : RWStructuredBuffer<float4>;
[[vk::binding(B18, 0)]]
var high_read : Texture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B19, 0)]]
var high_write : RWTexture2D<float4>;
[[vk::binding(B20, 0)]]
var sky_mapping : StructuredBuffer<float4>;
[[vk::binding(B21, 0)]]
var aerial_view : ConstantBuffer<AerialView>;
[vk::image_format("rgba16f")]
[[vk::binding(B22, 0)]]
var aerial_light : RWTexture3D<float4>;
[vk::image_format("rgba16f")]
[[vk::binding(B23, 0)]]
var aerial_t : RWTexture3D<float4>;
func dimensions(t: Texture2D<float4>)->uint2
{
    var w : uint;
    var h : uint;
    t.GetDimensions(w, h);
    return uint2(w, h);
}
func dimensions(t: RWTexture2D<float4>)->uint2
{
    var w : uint;
    var h : uint;
    t.GetDimensions(w, h);
    return uint2(w, h);
}
func dimensions(t: RWTexture3D<float4>)->uint3
{
    var w : uint;
    var h : uint;
    var d : uint;
    t.GetDimensions(w, h, d);
    return uint3(w, h, d);
}

func phase_coord(nu: float)->float
{
    let a = select((mapping_flags(p) & 4u) != 0u, p.angular_mapping.x, 0.5);
    return a * acos(clamp(nu, -1.0, 1.0)) / atmosphere::PI +
           (1.0 - a) * pow(max((1.0 - nu) * 0.5, 0.0), 1.0 / 3.0);
}
func shape(hi: uint, si: uint, uv: float2)->float4
{
    let pixel = float2(float(si * p.solve.y), float(hi * p.solve.z)) +
                uv * float2(float(p.solve.y - 1u), float(p.solve.z - 1u)) + 0.5;
    return source_read.SampleLevel(linear_sampler, pixel / float2(dimensions(source_read)), 0.0);
}
func log_mean(hi: uint, si: uint)->float4
{
    return mean_read.Load(int3(int2(int(si), int(hi)), 0));
}
func high_basis(mu: float, mu_s: float, nu: float)->float4
{
    let y = clamp(mu, -1.0, 1.0);
    let radial = sqrt(max(1.0 - y * y, 0.0));

    let z = clamp((nu - y * mu_s) / sqrt(max(1.0 - mu_s * mu_s, 1e-20)), -radial, radial);
    return float4(1.0, y * y, z * z, y * z);
}
func high_shape(hi: uint, si: uint, b: float4)->float4
{
    var m : float4[5] = { (float4)0, (float4)0, (float4)0, (float4)0, (float4)0 };
    for (var k = 0u; k < 5u; k++)
    {
        m[k] = high_read.Load(int3(int2(int(si * 5u + k), int(hi + 1u - p.offsets1.y)), 0));
    }
    return (m[0] + m[1] * b.y + m[2] * b.z + m[3] * b.w) / max(dot(m[4], b), 1e-20);
}
func indirect(h: float, mu: float, mu_s: float, nu: float, cached_phase: float, c: Medium)->float4
{
    if ((p.size_steps.w & 1u) == 0u)
    {
        return float4(0.0);
    }
    var lo = 0u;
    var hi = p.dims.x - 1u;
    while (hi - lo > 1u)
    {
        let mid = (lo + hi) / 2u;
        if (data[mid].x <= h)
        {
            lo = mid;
        }
        else
        {
            hi = mid;
        }
    }
    let h0 = data[lo].x;
    let h1 = data[hi].x;
    let th = clamp((h - h0) / (h1 - h0), 0.0, 1.0);
    let elevation = asin(clamp(mu_s, -1.0, 1.0));
    let s0 = solar_layer(lo, elevation) * float(p.dims.y - 1u);
    let s1 = solar_layer(hi, elevation) * float(p.dims.y - 1u);
    let i0 = min(uint(s0), p.dims.y - 2u);
    let i1 = min(uint(s1), p.dims.y - 2u);
    let t0 = s0 - float(i0);
    let t1 = s1 - float(i1);
    let brightness = exp(lerp(lerp(log_mean(lo, i0), log_mean(lo, i0 + 1u), t0),
                              lerp(log_mean(hi, i1), log_mean(hi, i1 + 1u), t1), th));
    if (h >= 35.0 && p.offsets1.y < p.dims.x)
    {
        let b = high_basis(mu, mu_s, nu);
        let v = lerp(lerp(high_shape(lo, i0, b), high_shape(lo, i0 + 1u, b), t0),
                     lerp(high_shape(hi, i1, b), high_shape(hi, i1 + 1u, b), t1), th);
        return max(v * brightness * c.scattering[0], float4(0.0));
    }
    let cp = clamp((mu - mu_s * nu) / sqrt(max((1.0 - mu_s * mu_s) * (1.0 - nu * nu), 1e-20)), -1.0,
                   1.0);
    var angle = 0.0;
    if ((mapping_flags(p) & 8u) != 0u)
    {
        let t = (1.0 - cp) * 0.5;
        angle = t + p.angular_mapping.y * t * (1.0 - t) * (2.0 * t - 1.0);
    }
    else
    {
        angle = acos(cp) / atmosphere::PI;
    }
    var phase = cached_phase;
    if (!CACHE_PHASE_COORDINATE)
    {
        phase = phase_coord(nu);
    }
    let uv = float2(phase, angle);
    let v = lerp(lerp(shape(lo, i0, uv), shape(lo, i0 + 1u, uv), t0),
                 lerp(shape(hi, i1, uv), shape(hi, i1 + 1u, uv), t1), th);
    var sigma = float4(0.0);
    for (var k = 0u; k < 5u; k++)
    {
        sigma += c.scattering[k];
    }
    return max(v * brightness * sigma, float4(0.0));
}
func ground_light(mu_s: float)->float4
{
    let x = solar_layer(0u, asin(clamp(mu_s, -1.0, 1.0))) * float(p.dims.y - 1u);
    let i = min(uint(x), p.dims.y - 2u);
    return lerp(ground_read.Load(int3(int2(int(i), 0), 0)),
                ground_read.Load(int3(int2(int(i + 1u), 0), 0)), x - float(i));
}
func write_optical_depth(id: uint3)->void
{
    if (id.x >= p.dims.w || id.y >= p.dims.z)
    {
        return;
    }
    let h = data[p.mapping.z + id.y].x;
    let v = float(id.x) / float(p.dims.w - 1u);
    let a = v * v * v;
    var x = a;
    if ((mapping_flags(p) & 16u) == 0u)
    {
        let b = (1.0 - v) * (1.0 - v) * (1.0 - v);
        x = a / max(a + b, 1e-20);
    }
    let mu = horizon(p, h) + (1.0 - horizon(p, h)) * x;
    let length = boundary(p, h, mu, false);
    let dt = length / float(p.extra.w);
    var tau = float4(0.0);
    for (var j = 0u; j < p.extra.w; j++)
    {
        tau += medium(height_at(p, h, mu, (float(j) + 0.5) * dt)).extinction * dt;
    }
    tau_write[int2(id.xy)] = min(tau, float4(80.0));
}
[shader("compute")]
[numthreads(8, 8, 1)]
func optical_depth(uint3 id: SV_DispatchThreadID)->void
{
    write_optical_depth(id);
}

[shader("compute")]
[numthreads(8, 8, 1)]
func optical_depth_rows(uint3 id: SV_DispatchThreadID)->void
{
    if (id.y >= p.batch.w)
    {
        return;
    }
    write_optical_depth(uint3(id.x, id.y + p.batch.z, 0u));
}
[shader("compute")]
[numthreads(64, 1, 1)]
func prepare_directions(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x >= p.solve.x || id.y >= p.batch.y * p.dims.y)
    {
        return;
    }
    let state = data[p.extra.x + p.batch.x * p.dims.y + id.y];
    let h = state.x;
    let mu_s = state.y;
    let sun = float3(0.0, mu_s, state.z);
    let n = p.solve.x / 3u;
    var v : float3 = (float3)0;
    var w : float = (float)0;
    let sample_index = select(id.x < n, id.x, n + id.x % n);
    let q = data[p.extra.y + sample_index];
    if (q.w == 0.0)
    {
        directions[id.y * p.solve.x + id.x] = float4(0.0);
        return;
    }
    if (id.x < n)
    {
        let tangent = float3(0.0, sun.z, -sun.y);
        v = q.x * tangent + q.y * float3(1.0, 0.0, 0.0) + q.z * sun;
        w = q.w;
    }
    else
    {
        let hor = horizon(p, h);
        var low = -1.0;
        var high = hor;
        if (id.x >= 2u * n)
        {
            low = hor;
            high = 1.0;
        }
        let u = pow(q.x, p.medium.w);
        var mu = lerp(hor, low, u);
        if (id.x >= 2u * n)
        {
            mu = lerp(hor, high, u);
        }
        let r = sqrt(max(1.0 - mu * mu, 0.0));
        v = float3(r * q.z, mu, r * q.y);
        w = q.w * (high - low) * p.medium.w * pow(q.x, p.medium.w - 1.0);
    }
    let d = 0.0004 + max(1.0 - dot(v, sun), 0.0);
    let sw = p.importance.x / (p.importance.x + d * d) *
             lerp(1.0, p.importance.z, smoothstep(12.0, 35.0, h));
    w *= select(id.x < n, sw, 1.0 - sw);

    directions[id.y * p.solve.x + id.x] = float4(v, w);
}
[shader("compute")]
[numthreads(64, 1, 1)]
func trace_incident(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x >= p.solve.x || id.y >= p.batch.y * p.dims.y)
    {
        return;
    }
    let state_index = p.batch.x * p.dims.y + id.y;
    let state = data[p.extra.x + state_index];
    let q = directions[id.y * p.solve.x + id.x];
    if (q.w == 0.0)
    {
        incoming[id.y * p.solve.x + id.x] = float4(0.0);
        return;
    }
    let sun = float3(0.0, state.y, state.z);
    let index = id.y * p.solve.x + id.x;
    let result = integrate_at(q.xyz, sun, state.x, 0.0, p.size_steps.z);
    let light = result.light;
    incoming[index] = light;
}
[shader("compute")]
[numthreads(64, 1, 1)]
func incident_moments(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x >= p.batch.y * p.dims.y)
    {
        return;
    }
    let state_index = p.batch.x * p.dims.y + id.x;
    var m : float4[7] = { (float4)0, (float4)0, (float4)0, (float4)0,
                          (float4)0, (float4)0, (float4)0 };
    for (var j = 0u; j < p.solve.x; j++)
    {
        let q = directions[id.x * p.solve.x + j];
        let l = incoming[id.x * p.solve.x + j] * q.w;
        m[0] += l;
        m[1] += l * q.x * q.x;
        m[2] += l * q.y * q.y;
        m[3] += l * q.z * q.z;
        m[4] += l * q.y * q.z;
        m[5] += q.w * float4(1.0, q.x * q.x, q.y * q.y, q.z * q.z);
        m[6].x += q.w * q.y * q.z;
    }
    for (var k = 0u; k < 7u; k++)
    {
        moments[id.x * 7u + k] = m[k];
    }
    let hi = state_index / p.dims.y;
    if (hi + 1u >= p.offsets1.y)
    {
        let scale = max(m[0] / (4.0 * atmosphere::PI), float4(1e-30));

        let packed : float4[5] = {
            (m[0] + m[1]) / scale, (m[2] - m[1]) / scale, (m[3] - m[1]) / scale, 2.0 * m[4] / scale,
            float4(m[5].x + m[5].y, m[5].z - m[5].y, m[5].w - m[5].y, 2.0 * m[6].x)
        };
        for (var k = 0u; k < 5u; k++)
        {
            high_write[int2(int((id.x % p.dims.y) * 5u + k), int(hi + 1u - p.offsets1.y))] =
                packed[k];
        }
    }
    mean_write[int2(int(state_index % p.dims.y), int(hi))] =
        log(max(m[0] / (4.0 * atmosphere::PI), float4(1e-30)));
}
[shader("compute")]
[numthreads(8, 8, 1)]
func scattering_source(uint3 id: SV_DispatchThreadID)->void
{
    let pixel = id.xy + uint2(0u, p.batch.x * p.solve.z);
    if (any(pixel >= dimensions(source_write)) || id.y >= p.batch.y * p.solve.z)
    {
        return;
    }
    let hi = pixel.y / p.solve.z;
    let si = id.x / p.solve.y;
    let state_index = hi * p.dims.y + si;
    let local_state = (hi - p.batch.x) * p.dims.y + si;
    let state = data[p.extra.x + state_index];
    let h = state.x;
    let mu_s = state.y;
    let nu = data[p.extra.z + id.x % p.solve.y].x;
    let cone = data[p.mapping.w + id.y % p.solve.z];
    let sun = float3(0.0, mu_s, state.z);
    let tangent = float3(0.0, sun.z, -sun.y);
    let v =
        sun * nu + sqrt(max(1.0 - nu * nu, 0.0)) * (tangent * cone.x + float3(cone.y, 0.0, 0.0));
    let coeff = p.offsets0.x + hi * 6u;
    let mi = local_state * 7u;
    let ray = moments[mi] + moments[mi + 1u] * v.x * v.x + moments[mi + 2u] * v.y * v.y +
              moments[mi + 3u] * v.z * v.z + 2.0 * moments[mi + 4u] * v.y * v.z;
    let weights = moments[mi + 5u];
    let normalization = weights.x + weights.y * v.x * v.x + weights.z * v.y * v.y +
                        weights.w * v.z * v.z + 2.0 * moments[mi + 6u].x * v.y * v.z;
    var source = data[coeff] * ray / max(normalization, 1e-20);

    if (h < 35.0)
    {
        var sums : float4[4] = { (float4)0, (float4)0, (float4)0, (float4)0 };
        var norms : float4[4] = { (float4)0, (float4)0, (float4)0, (float4)0 };

        var species_enabled : bool[4] = { (bool)0, (bool)0, (bool)0, (bool)0 };
        for (var species = 0u; species < 4u; species++)
        {
            species_enabled[species] =
                !SKIP_INACTIVE_SPECIES || any(data[coeff + species + 1u] != float4(0.0));
        }
        for (var j = 0u; j < p.solve.x; j++)
        {
            let q = directions[local_state * p.solve.x + j];
            let l = incoming[local_state * p.solve.x + j];
            let a = q.y * v.y + q.z * v.z;
            let b = q.x * v.x;

            let positive = phase_stencil(clamp(a + b, -1.0, 1.0));
            let negative = phase_stencil(clamp(a - b, -1.0, 1.0));
            for (var species = 0u; species < 4u; species++)
            {
                if (!species_enabled[species])
                {
                    continue;
                }
                let weight =
                    (tabulated_phase(positive, species) + tabulated_phase(negative, species)) *
                    (0.5 * q.w);
                sums[species] += l * weight;
                norms[species] += weight;
            }
        }

        for (var species = 0u; species < 4u; species++)
        {
            source += data[coeff + species + 1u] * data[p.offsets1.x + species] * sums[species] /
                      max(norms[species], float4(1e-20));
        }
    }
    let sigma = data[coeff + 5u];
    let mean = exp(log_mean(hi, si));
    source_write[int2(pixel)] = max(source / max(mean * sigma, float4(1e-30)), float4(0.0));
}
[shader("compute")]
[numthreads(64, 1, 1)]
func ground_irradiance(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x >= p.dims.y || p.batch.x != 0u)
    {
        return;
    }
    let mu_s = data[p.extra.x + id.x].y;
    var irradiance =
        sun_transmittance(p, tau_read, linear_sampler, 0.0, mu_s) * max(mu_s, 0.0) * p.solar;
    if (p.solve.w > 0u)
    {
        for (var j = 0u; j < p.solve.x; j++)
        {
            let q = directions[id.x * p.solve.x + j];
            irradiance += incoming[id.x * p.solve.x + j] * (q.w * max(q.y, 0.0));
        }
    }
    ground_write[int2(int(id.x), 0)] = irradiance * (p.medium.y / atmosphere::PI);
}

func solar_layer(layer: uint, e: float)->float
{
    if ((mapping_flags(p) & 2u) == 0u)
    {
        return solar_coord(p, data[layer].x, sin(e));
    }
    let offset = p.mapping.y + 4u * layer;
    let base = data[offset];
    let d = max(atmosphere::PI * 0.5 - e, 0.0);
    var y = base.x * e + base.y - base.z * sqrt(d / (d + 0.0872664626));
    for (var j = 1u; j <= 3u; j++)
    {
        let c = data[offset + j];
        let x = e - c.x;
        y += c.z * x / (abs(x) + c.y);
    }
    return clamp(y, 0.0, 1.0);
}
struct Medium
{
    var extinction : float4;
    var scattering : float4[5];
};
func medium(h: float)->Medium
{
    var lo = 0u;
    var hi = p.offsets1.z - 1u;
    while (hi - lo > 1u)
    {
        let mid = (lo + hi) / 2u;
        if (data[p.offsets0.z + 7u * mid].x <= h)
        {
            lo = mid;
        }
        else
        {
            hi = mid;
        }
    }
    let a = p.offsets0.z + lo * 7u;
    let b = p.offsets0.z + hi * 7u;
    let t = clamp((h - data[a].x) / (data[b].x - data[a].x), 0.0, 1.0);
    var result : Medium = (Medium)0;
    result.extinction = lerp(data[a + 1u], data[b + 1u], t);
    for (var j = 0u; j < 5u; j++)
    {
        result.scattering[j] = lerp(data[a + 2u + j], data[b + 2u + j], t);
    }
    return result;
}
struct PhaseStencil
{
    var index : uint;
    var fraction : float;
};
func phase_stencil(mu: float)->PhaseStencil
{
    let x = clamp(pow(max((1.0 - mu) * 0.5, 0.0), 1.0 / 3.0) * 1024.0 - 0.5, 0.0, 1023.0);
    let i = min(uint(x), 1022u);
    return PhaseStencil(i, x - float(i));
}
func tabulated_phase(stencil: PhaseStencil, species: uint)->float4
{
    let offset = p.offsets0.w + species * 1024u + stencil.index;
    return lerp(data[offset], data[offset + 1u], stencil.fraction);
}
struct Transport
{
    var light : float4;
    var transmittance : float4;
};
// 四波长共同积分核；include_ground 仅控制末端理想行星表面的边界辐射。
// Aerial 组合显式场景表面，因此传 false；求解器与 SkyView 保留源边界条件 true。
func integrate_core(ray: float3, sun: float3, initial_height: float, segment: float, steps: uint,
                    include_ground: bool)
    ->Transport
{
    var result : Transport = (Transport)0;
    result.light = float4(0.0);
    result.transmittance = float4(1.0);
    var h = initial_height;
    var mu = ray.y;
    var mu_s = sun.y;
    let nu = dot(ray, sun);
    var entry = 0.0;
    var up = float3(0.0, 1.0, 0.0);
    if (h > p.medium.x)
    {
        let radius = p.sun.w + h;
        let b = radius * mu;
        let c = (h - p.medium.x) * (radius + p.sun.w + p.medium.x);
        let disc = b * b - c;
        if (b >= 0.0 || disc <= 0.0)
        {
            return result;
        }
        entry = c / (-b + sqrt(disc));
        if (segment > 0.0 && entry >= segment)
        {
            return result;
        }
        up = normalize(up * radius + ray * entry);
        h = p.medium.x;
        mu = dot(ray, up);
        mu_s = dot(sun, up);
    }
    let hit = ground(p, h, mu);
    let full_length = boundary(p, h, mu, hit);
    var length = full_length;
    if (segment > 0.0)
    {
        length = min(length, max(segment - entry, 0.0));
    }
    let count = steps;

    let b = (p.sun.w + h) * mu;
    let middle = clamp(-b, 0.0, length);
    let hmin = height_at(p, h, mu, middle);
    let hend = height_at(p, h, mu, length);
    let scale = max(p.importance.y, 1e-10);
    var before = 0.0;
    var after = 0.0;
    if (p.importance.y > 0.0)
    {
        before = log(1.0 + max(h - hmin, 0.0) / scale);
        after = log(1.0 + max(hend - hmin, 0.0) / scale);
    }
    var split = uint(round(float(count) * before / max(before + after, 1e-20)));
    if (middle <= 0.0)
    {
        split = 0u;
    }
    else if (middle >= length)
    {
        split = count;
    }
    else
    {
        split = clamp(split, 1u, count - 1u);
    }
    var solar_phase : float4[5] = { (float4)0, (float4)0, (float4)0, (float4)0, (float4)0 };
    let cosine = clamp(nu, -1.0, 1.0);
    let stencil = phase_stencil(cosine);
    solar_phase[0] = float4(3.0 * (1.0 + cosine * cosine) / (16.0 * atmosphere::PI));
    for (var k = 0u; k < 4u; k++)
    {
        solar_phase[k + 1u] = tabulated_phase(stencil, k);
    }
    var previous = 0.0;
    var cached_phase = 0.0;
    var phase_cached = false;
    for (var i = 1u; i <= count; i++)
    {
        var edge = length * float(i) / float(count);
        if (before + after >= 1e-5 && i < count)
        {
            if (i == split)
            {
                edge = middle;
            }
            else
            {
                let incoming = i < split;
                var lh = after * float(i - split) / max(float(count - split), 1.0);
                if (incoming)
                {
                    lh = before * (1.0 - float(i) / float(split));
                }
                let hh = hmin + scale * (exp(lh) - 1.0);
                let c = (h - hh) * (2.0 * p.sun.w + h + hh);
                let root = sqrt(max(b * b - c, 0.0));
                if (incoming)
                {
                    edge = c / max(-b + root, 1e-20);
                }
                else if (b > 0.0)
                {
                    edge = -c / max(root + b, 1e-20);
                }
                else
                {
                    edge = -b + root;
                }
                edge = clamp(edge, previous, length);
            }
        }
        let dt = edge - previous;
        let d = (edge + previous) * 0.5;
        previous = edge;
        let hp = height_at(p, h, mu, d);
        let local_up = (up * (p.sun.w + h) + ray * d) / (p.sun.w + hp);
        let c = medium(hp);
        var direct = float4(0.0);
        for (var k = 0u; k < 5u; k++)
        {
            direct += c.scattering[k] * solar_phase[k];
        }
        direct *= sun_transmittance(p, tau_read, linear_sampler, hp, dot(local_up, sun));
        direct *= p.solar;
        var source = direct;

        if (p.solve.w != 1u)
        {
            if (CACHE_PHASE_COORDINATE && !phase_cached && hp < 35.0 && (p.size_steps.w & 1u) != 0u)
            {
                cached_phase = phase_coord(nu);
                phase_cached = true;
            }
            source += indirect(hp, dot(local_up, ray), dot(local_up, sun), nu, cached_phase, c);
        }
        let tau = c.extinction * dt;
        let trans = exp(-tau);
        let integral = segment_integral(c.extinction, dt);
        result.light += result.transmittance * source * integral;
        result.transmittance *= trans;
    }
    if (include_ground && hit && length >= full_length)
    {
        let end_up = (up * (p.sun.w + h) + ray * length) / p.sun.w;
        result.light += result.transmittance * ground_light(dot(end_up, sun));
    }
    return result;
}

func integrate_at(ray: float3, sun: float3, initial_height: float, segment: float, steps: uint)
    ->Transport
{
    return integrate_core(ray, sun, initial_height, segment, steps, true);
}

func integrate(ray: float3, sun: float3)->Transport
{
    return integrate_at(ray, sun, p.view.w, p.medium.z, p.size_steps.z);
}
func camera_ray(id: uint2)->float3
{
    let uv = (float2(id) + 0.5) / float2(p.size_steps.xy) * 2.0 - 1.0;
    let sy = sin(p.view.x);
    let cy = cos(p.view.x);
    let sp = sin(p.view.y);
    let cp = cos(p.view.y);
    let forward = float3(sy * cp, sp, cy * cp);
    let right = float3(cy, 0.0, -sy);
    let up = cross(forward, right);
    return normalize(forward +
                     right * uv.x * tan(p.view.z * 0.5) * float(p.size_steps.x) /
                         float(p.size_steps.y) -
                     up * uv.y * tan(p.view.z * 0.5));
}
func world_sun()->float3
{
    return float3(sin(p.sun.x) * cos(p.sun.y), sin(p.sun.y), cos(p.sun.x) * cos(p.sun.y));
}
[shader("compute")]
[numthreads(8, 8, 1)]
func render(uint3 id: SV_DispatchThreadID)->void
{
    if (any(id.xy >= p.size_steps.xy))
    {
        return;
    }
    let ray = camera_ray(id.xy);
    let sun = world_sun();
    let result = integrate(ray, sun);
    var light = result.light;
    var rgb = float3(0.0);
    let half_radius = sin(p.sun.z * 0.5);
    if ((p.size_steps.w & 2u) != 0u && p.medium.z <= 0.0 && !ground(p, p.view.w, ray.y) &&
        dot(ray - sun, ray - sun) <= 4.0 * half_radius * half_radius)
    {
        light +=
            result.transmittance * p.solar / (4.0 * atmosphere::PI * half_radius * half_radius);
    }
    for (var i = 0u; i < 4u; i++)
    {
        rgb += light[i] * p.rgb[i].xyz;
    }
    var value = float4(rgb, 1.0);
    if ((p.size_steps.w & 4u) != 0u)
    {
        value = light;
    }
    output[int2(id.xy)] = value;
    output_t[int2(id.xy)] = result.transmittance;
}

[shader("compute")]
[numthreads(8, 8, 1)]
func build_sky(uint3 id: SV_DispatchThreadID)->void
{
    if (any(id.xy >= p.sky.xy))
    {
        return;
    }
    let lower_rows = p.sky.z;
    var chart = 0u;
    var start = 0u;
    var rows = lower_rows;
    if (id.y >= p.sky.w)
    {
        chart = 2u;
        start = p.sky.w;
        rows = p.sky.y - p.sky.w;
    }
    else if (id.y >= lower_rows)
    {
        chart = 1u;
        start = lower_rows;
        rows = p.sky.w - lower_rows;
    }
    if (chart == 1u && p.view.w >= p.medium.x)
    {
        sky_write[int2(id.xy)] = float4(log(float3(1e-30)), 0.0);
        return;
    }
    let hit = chart == 2u;
    let u = float(id.x) / float(p.sky.x - 1u);
    var a = u * u;
    if ((mapping_flags(p) & 32u) != 0u)
    {
        a *= a;
    }
    let cp = 1.0 - 2.0 * a;

    var mu = sky_mapping[12u + id.y].x;
    let hor = horizon(p, p.view.w);
    if (hit)
    {
        mu = min(mu, hor - 1e-7);
    }
    else
    {
        mu = max(mu, hor + 1e-7);
    }
    let r = sqrt(max(1.0 - mu * mu, 0.0));
    let ray = float3(r * 2.0 * sqrt(max(a * (1.0 - a), 0.0)), mu, r * cp);
    let sun = float3(0.0, sin(p.sun.y), cos(p.sun.y));
    let result = integrate(ray, sun);
    var rgb = float3(0.0);
    for (var k = 0u; k < 4u; k++)
    {
        rgb += result.light[k] * p.rgb[k].xyz;
    }
    sky_write[int2(id.xy)] = float4(log(max(rgb, float3(1e-30))), 0.0);
}

[shader("compute")]
[numthreads(4, 4, 4)]
func build_aerial(uint3 id: SV_DispatchThreadID)->void
{
    let dims : uint3 = dimensions(aerial_light);
    if (any(id >= dims))
        return;
    let uv : float2 = (float2(id.xy) + .5) / float2(dims.xy);
    let ray
        : float3 =
              normalize(mul(aerial_view.matrix, float4(uv.x * 2. - 1., 1. - uv.y * 2., 0, 1)).xyz);
    let z : float = (float(id.z) + .5) / float(dims.z);
    let result : Transport = integrate_core(ray, world_sun(), p.view.w,
                                            aerial_view.settings.x * z * z, p.size_steps.z, false);
    var light : float3 = 0;
    var clear : float3 = 0;
    var filtered : float3 = 0;
    for (var k: uint = 0; k < 4; k++)
    {
        light += result.light[k] * p.rgb[k].xyz;
        clear += p.solar[k] * p.rgb[k].xyz;
        filtered += result.transmittance[k] * p.solar[k] * p.rgb[k].xyz;
    }
    aerial_light[id] = float4(max(light * aerial_view.settings.yzw, 0), 1);
    aerial_t[id] = float4(clamp(filtered / max(clear, 1e-20), 0, 1), 1);
}
