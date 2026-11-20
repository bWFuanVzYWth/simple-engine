// SPDX-License-Identifier: GPL-3.0-only
module pupil_integral;
import math.constants;
import pupil_grid;
struct SpectralNode {
    var scale : float;
    var r : float;
    var g : float;
    var b : float;
    var terms : float4;
};
[[vk::binding(0, 0)]]
var config : StructuredBuffer<OpticalGrid>;
[[vk::binding(1, 0)]]
var pupil : StructuredBuffer<float>;
[[vk::binding(2, 0)]]
var nodes : StructuredBuffer<SpectralNode>;
[[vk::binding(3, 0)]]
[[vk::image_format("rg32f")]]
var field : RWTexture2D<float2>;
[[vk::binding(4, 0)]]
var integral : RWStructuredBuffer<float4>;
[[vk::binding(5, 0)]]
var output : RWStructuredBuffer<float>;
[[vk::binding(6, 0)]]
var weights : StructuredBuffer<float>;
[[vk::binding(7, 0)]]
var horizontal : RWStructuredBuffer<float4>;
func sinc(x: float)->float {
    let t = math::pi * x;
    return abs(t) < 0.0001 ? 1 : sin(t) / t;
}
static const let taps : uint = 8;
func coeff(fr: float)->uint {
    return uint(round(fr * 256)) * taps;
}
[shader("compute")]
[numthreads(16, 16, 1)]
func interpolate_rows(uint3 tid: SV_DispatchThreadID)->void {
    let p = config[0];
    let id = tid.xy;
    if (id.x >= p.cells.x || id.y >= p.fft_rows)
        return;
    let u = (float(p.origin.x) + (float(id.x) + 0.5) / float(p.sampling)) * float(p.fft_side) /
            p.density;
    let ix = int(floor(u)) - 3;
    let w = coeff(frac(u));
    let mask = p.fft_side - 1;
    let row = uint(p.first_fft_row + int(id.y)) & mask;
    var z = float2(0);
    var opposite = float2(0);
    [ForceUnroll]
    for (var t = 0u; t < taps; t++) {
        let at = uint2(uint(ix + int(t)), row);
        let wgt = weights[w + t];
        z += field[at & mask] * wgt;
        if (p.fiber_count > 0)
            opposite += field[(-at) & mask] * wgt;
    }
    var pair = float4(z, 0, 0);
    if (p.fiber_count > 0) {
        let conjugate = opposite * float2(1, -1);
        let diff = z - conjugate;
        pair = 0.5 * float4(z + conjugate, diff.y, -diff.x);
    }
    horizontal[id.y * p.cells.x + id.x] = pair;
}

groupshared var summed : float4[256];
[shader("compute")]
[numthreads(16, 16, 1)]
func integrate_cells(uint3 tid: SV_DispatchThreadID, uint lane: SV_GroupIndex)->void {
    let p = config[0];
    let id = tid.xy;
    let uv = float2(p.origin) + (float2(id) + 0.5) / float(p.sampling);
    var power = float4(0);
    if (all(id < p.cells)) {
        let k = uv.y * float(p.fft_side) / p.density;
        let iy = int(floor(k)) - 3;
        let w = coeff(frac(k));
        var field_pair = float4(0);
        [ForceUnroll]
        for (var t = 0u; t < taps; t++) {
            let row = uint(iy + int(t) - p.first_fft_row) & (p.fft_side - 1);
            field_pair += horizontal[row * p.cells.x + id.x] * weights[w + t];
        }
        // 输入像素是面积均匀的方孔。周期频谱乘单个方孔的连续频响，跨 Nyquist 连续。
        let box = sinc(uv.x / p.density) * sinc(uv.y / p.density);
        let a = field_pair.xy * box;
        let b = field_pair.zw * box;
        power = float4(dot(a, a), dot(b, b), dot(a, b), a.x * b.y - a.y * b.x);
    }
    summed[lane] = power / float(p.sampling * p.sampling);
    [ForceUnroll]
    for (var d = 1u; d < 16; d <<= 1) {
        GroupMemoryBarrierWithGroupSync();
        let a = summed[lane] + ((lane & 15) >= d ? summed[lane - d] : float4(0));
        GroupMemoryBarrierWithGroupSync();
        summed[lane] = a;
    }
    [ForceUnroll]
    for (var d = 1u; d < 16; d <<= 1) {
        GroupMemoryBarrierWithGroupSync();
        let a = summed[lane] + (lane >= 16 * d ? summed[lane - 16 * d] : float4(0));
        GroupMemoryBarrierWithGroupSync();
        summed[lane] = a;
    }
    GroupMemoryBarrierWithGroupSync();
    if (all(id < p.cells))
        integral[id.y * p.cells.x + id.x] = summed[lane];
}

func at_corner(base: uint2, xy: int2, width: uint)->float4 {
    if (any(xy <= 0))
        return float4(0);
    return integral[(base.y + uint(xy.y) - 1) * width + base.x + uint(xy.x) - 1];
}
func cumulative(base: uint2, xy: float2, width: uint)->float4 {
    let a = int2(floor(xy));
    let b = min(a + 1, 16);
    let f = frac(xy);
    let low = lerp(at_corner(base, a, width), at_corner(base, int2(b.x, a.y), width), f.x);
    let high = lerp(at_corner(base, int2(a.x, b.y), width), at_corner(base, b, width), f.x);
    return lerp(low, high, f.y);
}
[shader("compute")]
[numthreads(8, 8, 1)]
func project_pixels(uint3 tid: SV_DispatchThreadID)->void {
    let p = config[0];
    let pixel = p.screen + tid.xy;
    if (any(pixel >= p.end_screen))
        return;
    let pos = float2(int2(pixel) - int(p.side / 2));
    var color = float3(0);
    for (var n = 0u; n < p.node_count; n++) {
        let node = nodes[n];
        let lo = max(((pos - 0.5) * node.scale - float2(p.origin)) * float(p.sampling), 0);
        let hi =
            min(((pos + 0.5) * node.scale - float2(p.origin)) * float(p.sampling), float2(p.cells));
        if (any(lo >= hi))
            continue;
        var value = float4(0);
        for (var y = uint(lo.y) & ~15u; y < uint(ceil(hi.y)); y += 16) {
            for (var x = uint(lo.x) & ~15u; x < uint(ceil(hi.x)); x += 16) {
                let b = uint2(x, y);
                let l = clamp(lo - float2(b), 0., 16.);
                let h = clamp(hi - float2(b), 0., 16.);
                value +=
                    (cumulative(b, h, p.cells.x) - cumulative(b, float2(h.x, l.y), p.cells.x)) -
                    (cumulative(b, float2(l.x, h.y), p.cells.x) - cumulative(b, l, p.cells.x));
            }
        }
        color += float3(node.r, node.g, node.b) * max(0., dot(value, node.terms));
    }
    let index = 3 * (pixel.y * p.side + pixel.x);
    output[index] += color.x;
    output[index + 1] += color.y;
    output[index + 2] += color.z;
}
[shader("compute")]
[numthreads(16, 16, 1)]
func square_pixels(uint3 tid: SV_DispatchThreadID)->void {
    let p = config[0];
    if (any(tid.xy >= p.side))
        return;
    let radius = p.side / 2;
    let delta = uint2(abs(int2(tid.xy) - int(radius)));
    var rgb = float3(0);
    for (var i = 0u; i < p.node_count; i++) {
        let base = i * (radius + 1);
        let intensity = pupil[base + delta.x] * pupil[base + delta.y];
        let node = nodes[i];
        rgb += float3(node.r, node.g, node.b) * intensity;
    }
    let k = 3 * (tid.y * p.side + tid.x);
    output[k] = rgb.x;
    output[k + 1] = rgb.y;
    output[k + 2] = rgb.z;
}
