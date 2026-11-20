// SPDX-License-Identifier: GPL-3.0-only
module pupil_fourier;
import fft;
import math.constants;

import pupil_grid;
[[vk::binding(0, 0)]]
var config : StructuredBuffer<OpticalGrid>;
[[vk::binding(1, 0)]]
var pupil : StructuredBuffer<float>;
[[vk::binding(3, 0)]]
[[vk::image_format("rg32f")]]
var field : RWTexture2D<float2>;

[[vk::binding(8, 0)]]
var particles : StructuredBuffer<float>;
[[vk::binding(9, 0)]]
var bins : StructuredBuffer<uint>;

func mask_pixel(at: int2, extent: uint2)->float {
    if (any(at < 0) || any(at >= int2(extent)))
        return 0.;
    return pupil[uint(at.y) * extent.x + uint(at.x)];
}
// 栅格间距不大于源 PNG 的一个像素。对实际覆盖的最多四格作面积平均。
func mask_amplitude(xy: float2, p: OpticalGrid)->float {
    let source_density = float(max(p.extent.x, p.extent.y));
    let size = source_density / p.density;
    let lo = xy * source_density + float2(p.extent) * 0.5 - size * 0.5;
    let corner = int2(floor(lo));
    let mix = saturate(1. - (1. - frac(lo)) / size);
    let a = mask_pixel(corner, p.extent);
    if (all(mix == 0.))
        return a;
    let b = mask_pixel(corner + int2(1, 0), p.extent);
    let c = mask_pixel(corner + int2(0, 1), p.extent);
    let d = mask_pixel(corner + 1, p.extent);
    return lerp(lerp(a, b, mix.x), lerp(c, d, mix.x), mix.y);
}

// A 是开口乘灰尘振幅，B 是 A 中被纤维覆盖的部分，FFT 的实部和虚部分别装入两张实掩模。
func sample_layers(xy: float2, p: OpticalGrid)->float2 {
    if (any(abs(xy) > 0.75))
        return float2(0);
    var a = 0.;
    if (p.kind == PupilKind::radial) {
        let theta = frac(atan2(xy.y, xy.x) / (2 * math::pi) + 1) * float(p.extent.x);
        let i = uint(theta) & (p.extent.x - 1);
        let r = lerp(pupil[i], pupil[(i + 1) & (p.extent.x - 1)], frac(theta));
        a = saturate(0.5 + (r - length(xy)) * p.density);
    } else {
        a = mask_amplitude(xy, p);
    }
    if (a == 0 || p.dust_count + p.fiber_count == 0)
        return float2(a, 0);
    let cell = uint2(clamp(floor((xy / 1.5 + 0.5) * 32), 0., 31.));
    let bin = cell.y * 32 + cell.x;
    var dust = 0.;
    var fiber = 0.;
    for (var j = bins[bin]; j < bins[bin + 1]; j++) {
        let id = bins[j];
        let k = id * 6;
        let delta = xy - float2(particles[k], particles[k + 1]);
        let l = particles[k + 2];
        if (id < p.dust_count) {
            let size = l * p.density;
            dust = max(
                dust, min(saturate(0.5 + (l - length(delta)) * p.density), math::pi * size * size));
        } else {
            let w = particles[k + 3];
            let axis = float2(particles[k + 4], particles[k + 5]);
            let dist = abs(float2(dot(axis, delta), dot(float2(-axis.y, axis.x), delta)));
            let half_sizes = float2(l, w);
            let edge =
                min(saturate(0.5 + (half_sizes - dist) * p.density), 2 * half_sizes * p.density);
            fiber = max(fiber, edge.x * edge.y);
        }
    }
    a *= lerp(1., p.dust_transmission, dust);
    return float2(a, a * fiber);
}

[shader("compute")]
[numthreads(16, 16, 1)]
func rasterize_pupil(uint3 tid: SV_DispatchThreadID)->void {
    let p = config[0];
    if (any(tid.xy > 2 * p.raster_radius))
        return;
    let xy = int2(tid.xy) - int(p.raster_radius);
    field[uint2(xy) & (p.fft_side - 1)] = sample_layers(float2(xy) / p.density, p);
}

#ifndef PUPIL_FFT_SIZE
#define PUPIL_FFT_SIZE 1024
#endif
static const let n : uint = PUPIL_FFT_SIZE;
static const let lanes : uint = 128;
static const let slots : uint = n / lanes;
func fft_line<let column : bool>(axis_line: uint, lane: uint)->void {
    let p = config[0];
    if (!column) {
        let y = int(axis_line) - int(axis_line >= n / 2 ? n : 0);
        let empty = p.kind == PupilKind::mask_native ? uint(y + int(p.extent.y / 2)) >= p.extent.y
                                                     : abs(y) > int(p.raster_radius);
        if (empty) {
            for (var x = lane; x < n; x += lanes)
                field[uint2(x, axis_line)] = float2(0);
            return;
        }
    }
    var v : float4[slots];
    [ForceUnroll]
    for (var j = 0u; j < slots; j++) {
        let k = lane + j * lanes;
        var z = float2(0);
        if (column)
            z = field[uint2(axis_line, k)];
        else {
            let xy = int2(k, axis_line) - int2(k >= n / 2 ? n : 0, axis_line >= n / 2 ? n : 0);
            if (p.kind == PupilKind::mask_native) {
                let q = xy + int2(p.extent / 2);
                if (all(q >= 0) && all(q < int2(p.extent)))
                    z.x = pupil[q.y * p.extent.x + q.x];
            } else if (abs(xy.x) <= int(p.raster_radius)) {
                z = field[uint2(k, axis_line)];
            }
            z /= p.density * p.density;
        }
        v[j] = float4(z, 0, 0);
    }
    // 每组独占一行或一列，组内先读完原位数据，再写回变换结果。
    if (column || p.kind != PupilKind::mask_native)
        DeviceMemoryBarrierWithGroupSync();
    [ForceUnroll]
    for (var span = slots / 2; span > 0; span >>= 1) {
        [ForceUnroll]
        for (var pair = 0u; pair < slots / 2; pair++) {
            let t = pair & (span - 1);
            let k = 2 * pair - t;
            fft::butterfly<false>(v[k], v[k + span],
                                  math::pi * float(t * lanes + lane) / float(span * lanes));
        }
    }
    static const let batch : uint = slots < 8 ? slots : 8;
    [ForceUnroll]
    for (var base = 0u; base < slots; base += batch) {
        GroupMemoryBarrierWithGroupSync();
        [ForceUnroll]
        for (var j = 0u; j < batch; j++)
            fft::values[j * lanes + lane] = v[base + j];
        fft::transform_blocks<false, lanes>(lane, lanes, batch * lanes);
        [ForceUnroll]
        for (var j = 0u; j < batch; j++) {
            let k = fft::frequency((base + j) * lanes + lane, n);
            let z = fft::values[j * lanes + lane].xy;
            if (column)
                field[uint2(axis_line, k)] = z;
            else
                field[uint2(k, axis_line)] = z;
        }
    }
}
[shader("compute")]
[numthreads(128, 1, 1)]
func pupil_rows(uint3 g: SV_GroupID, uint lane: SV_GroupIndex)->void {
    fft_line<false>(g.x, lane);
}
[shader("compute")]
[numthreads(128, 1, 1)]
func pupil_columns(uint3 g: SV_GroupID, uint lane: SV_GroupIndex)->void {
    fft_line<true>(g.x, lane);
}
