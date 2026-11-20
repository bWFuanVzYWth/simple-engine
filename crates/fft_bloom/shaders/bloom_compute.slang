// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 原图分块线性卷积。每块在 x 仅存非负频率的 RG 和 B 两个平面。
module bloom_compute;
import fft;
import wide_fft;

struct Parameters {
    var radius : uint2;
    var size : uint;
    var optical : uint;
    var threshold : float;
    var intensity : float;
    var center_r : float;
    var center_g : float;
    var center_b : float;
    var first_y : uint;
    var first_x : uint;
    var accumulate : uint;
};

[[vk::binding(0, 0)]]
[[vk::image_format("rgba32f")]]
var spectrum : RWTexture2D<float4>;
[[vk::binding(1, 0)]]
var parameters : StructuredBuffer<Parameters>;
[[vk::binding(3, 0)]]
[[vk::image_format("rgba32f")]]
var tiles : RWTexture2D<float4>;
[[vk::binding(4, 0)]]
[[vk::image_format("rgba32f")]]
var output : RWTexture2D<float4>;

func extent(image: Texture2D<float4>)->uint2 {
    var w : uint;
    var h : uint;
    image.GetDimensions(w, h);
    return uint2(w, h);
}

[shader("compute")]
[numthreads(fft::lanes, 1, 1)]
func kernel_rows_main([[vk::binding(2, 0)]] uniform weights: StructuredBuffer<float>,
                      uint3 group: SV_GroupID, uint lane: SV_GroupIndex)
    ->void {
    let p = parameters[0];
    let side = 2 * p.radius + 1;
    for (var x = lane; x < p.size; x += fft::lanes) {
        let pixel = (uint2(x, group.x) + p.radius) & (p.size - 1);
        var value = float3(0);
        if (all(pixel < side) && (p.optical == 0 || any(pixel != p.radius))) {
            let i = 3 * (pixel.y * side.x + pixel.x);
            value = float3(weights[i], weights[i + 1], weights[i + 2]);
        }
        fft::values[x] = float4(value, 0);
    }
    fft::transform<false>(lane, p.size);
    let bins = p.size / 2 + 1;
    for (var k = lane; k < bins; k += fft::lanes) {
        var rg : float4;
        var blue : float4;
        fft::split_rgb(k, p.size, rg, blue);
        let at = uint2(k, group.x);
        spectrum[at] = rg;
        spectrum[at + uint2(bins, 0)] = blue;
    }
}

[shader("compute")]
[numthreads(fft::lanes, 1, 1)]
func kernel_columns_main(uint3 group: SV_GroupID, uint lane: SV_GroupIndex)->void {
    let size = parameters[0].size;
    for (var y = lane; y < size; y += fft::lanes)
        fft::values[y] = spectrum[uint2(group.x, y)];
    fft::transform<false>(lane, size);
    for (var y = lane; y < size; y += fft::lanes)
        spectrum[uint2(group.x, y)] = fft::values[y];
}

func highlight(Texture2D<float4> scene, pixel: int2, dims: uint2, p: Parameters)->float4 {
    if (any(pixel < 0) || any(uint2(pixel) >= dims))
        return 0;
    let v = scene.Load(int3(pixel, 0));
    if (v.w < 0 || !all(isfinite(v)))
        return 0;
    if (p.optical != 0)
        return float4(v.xyz, 0);
    let rgb = max(v.xyz, 0.0);
    let peak = max(rgb.x, max(rgb.y, rgb.z));
    if (peak <= p.threshold)
        return 0;
    return float4(rgb * ((peak - p.threshold) / peak), 0);
}

[shader("compute")]
[numthreads(fft::lanes, 1, 1)]
func forward_rows_main([[vk::binding(2, 0)]] uniform scene: Texture2D<float4>,
                       uint3 group: SV_GroupID, uint lane: SV_GroupIndex)
    ->void {
    let p = parameters[0];
    let tile = uint2(group.x, group.y / p.size);
    let origin =
        int2(tile * (p.size - 2 * p.radius) + uint2(p.first_x, p.first_y)) - int2(p.radius);
    let dims = extent(scene);
    for (var x = lane; x < p.size; x += fft::lanes)
        fft::values[x] = highlight(scene, origin + int2(x, group.y % p.size), dims, p);
    fft::transform<false>(lane, p.size);
    let bins = p.size / 2 + 1;
    for (var k = lane; k < bins; k += fft::lanes) {
        var rg : float4;
        var blue : float4;
        fft::split_rgb(k, p.size, rg, blue);
        let at = uint2(group.x * (2 * bins) + k, group.y);
        tiles[at] = rg;
        tiles[at + uint2(bins, 0)] = blue;
    }
}

[shader("compute")]
[numthreads(fft::lanes, 1, 1)]
func filter_columns_main(uint3 group: SV_GroupID, uint lane: SV_GroupIndex)->void {
    let size = parameters[0].size;
    let stride = size + 2;
    let origin = uint2(group.x, group.y * size);
    for (var y = lane; y < size; y += fft::lanes)
        fft::values[y] = tiles[origin + uint2(0, y)];
    fft::transform<false>(lane, size);
    for (var ky = lane; ky < size; ky += fft::lanes)
        fft::values[ky] = fft::multiply(fft::values[ky], spectrum[uint2(group.x % stride, ky)]);
    fft::transform<true>(lane, size);
    for (var y = lane; y < size; y += fft::lanes)
        tiles[origin + uint2(0, y)] = fft::values[y] / float(size);
}

[shader("compute")]
[numthreads(fft::lanes, 1, 1)]
func composite_rows_main([[vk::binding(2, 0)]] uniform scene: Texture2D<float4>,
                         uint3 group: SV_GroupID, uint lane: SV_GroupIndex)
    ->void {
    let p = parameters[0];
    let stride = p.size - 2 * p.radius;
    let row = group.y / stride.y * p.size + group.y % stride.y + p.radius.y;
    let bins = p.size / 2 + 1;
    for (var k = lane; k < p.size; k += fft::lanes) {
        let at = uint2(group.x * (2 * bins) + min(k, p.size - k), row);
        var rg = tiles[at];
        var b = tiles[at + uint2(bins, 0)].xy;
        if (k >= bins) {
            rg.yw = -rg.yw;
            b.y = -b.y;
        }
        fft::values[fft::frequency(k, p.size)] = float4(rg.x - rg.w, rg.y + rg.z, b);
    }
    fft::transform<true>(lane, p.size);
    let dims = extent(scene);
    for (var x = p.radius.x + lane; x < p.size - p.radius.x; x += fft::lanes) {
        let pixel = uint2(p.first_x + group.x * stride.x + x - p.radius.x, p.first_y + group.y);
        if (pixel.x >= dims.x)
            continue;
        let base = scene.Load(int3(pixel, 0));
        let filtered = fft::values[x].xyz / float(p.size);
        var rgb = p.optical != 0 ? base.xyz * float3(p.center_r, p.center_g, p.center_b) + filtered
                                 : base.xyz + p.intensity * max(filtered, 0.0);
        if (p.accumulate != 0)
            rgb += output[pixel].xyz;
        output[pixel] = base.w >= 0 && all(isfinite(base)) ? float4(rgb, base.w) : base;
    }
}

// 大核仍按原像素分辨率卷积。每次复用一个 4096² 工作块，频谱的负频率由共轭恢复。
static const let wide_bins : uint = wide_fft::size / 2 + 1;

func store_wide_bins<let kernel : bool>(uint2 at, float4 rg, float4 blue)->void {
    if (kernel) {
        spectrum[at] = rg;
        spectrum[at + uint2(wide_bins, 0)] = blue;
    } else {
        tiles[at] = rg;
        tiles[at + uint2(wide_bins, 0)] = blue;
    }
}

func wide_row_spectrum<let kernel : bool>(inout samples: wide_fft::Line, row: uint, lane: uint)
    ->void {
    samples.transform<false>(lane);
    [ForceUnroll]
    for (var base = 0u; base < wide_fft::slots; base += wide_fft::batch) {
        GroupMemoryBarrierWithGroupSync();
        [ForceUnroll]
        for (var j = 0u; j < wide_fft::batch; j++) {
            let slot = base + j;
            // 位反转的 -k 在最高置位以下逐位取反。高位选寄存器，低位选共享槽。
            let opposite_slot = slot == 0 ? 0 : slot ^ ((1u << firstbithigh(slot)) - 1);
            fft::values[j * wide_fft::threads + lane] = samples.v[opposite_slot];
        }
        GroupMemoryBarrierWithGroupSync();
        [ForceUnroll]
        for (var j = 0u; j < wide_fft::batch; j++) {
            let slot = base + j;
            let k = fft::frequency(slot * wide_fft::threads + lane, wide_fft::size);
            if (k < wide_bins) {
                let opposite = fft::frequency((-k) & (wide_fft::size - 1), wide_fft::size);
                var rg : float4;
                var blue : float4;
                fft::unpack_rgb(samples.v[slot],
                                fft::values[j * wide_fft::threads + opposite % wide_fft::threads],
                                rg, blue);
                store_wide_bins<kernel>(uint2(k, row), rg, blue);
            }
        }
    }
}

[shader("compute")]
[numthreads(wide_fft::threads, 1, 1)]
func wide_kernel_rows_main([[vk::binding(2, 0)]] uniform weights: StructuredBuffer<float>,
                           uint3 group: SV_GroupID, uint lane: SV_GroupIndex)
    ->void {
    let p = parameters[0];
    let side = 2 * p.radius + 1;
    let y = (group.x + p.radius.y) & (wide_fft::size - 1);
    var samples : wide_fft::Line;
    [ForceUnroll]
    for (var j = 0u; j < wide_fft::slots; j++) {
        let x = (j * wide_fft::threads + lane + p.radius.x) & (wide_fft::size - 1);
        var rgb = float3(0);
        if (x < side.x && y < side.y) {
            let i = 3 * (y * side.x + x);
            rgb = float3(weights[i], weights[i + 1], weights[i + 2]);
        }
        samples.v[j] = float4(rgb, 0);
    }
    wide_row_spectrum<true>(samples, group.x, lane);
}

[shader("compute")]
[numthreads(wide_fft::threads, 1, 1)]
func wide_forward_rows_main([[vk::binding(2, 0)]] uniform scene: Texture2D<float4>,
                            uint3 group: SV_GroupID, uint lane: SV_GroupIndex)
    ->void {
    let p = parameters[0];
    let origin = int2(p.first_x, p.first_y) - int2(p.radius);
    let y = origin.y + int(group.y);
    let dims = extent(scene);
    if (y < 0 || uint(y) >= dims.y) {
        for (var k = lane; k < wide_bins; k += wide_fft::threads)
            store_wide_bins<false>(uint2(k, group.y), 0, 0);
        return;
    }
    var samples : wide_fft::Line;
    [ForceUnroll]
    for (var j = 0u; j < wide_fft::slots; j++)
        samples.v[j] =
            highlight(scene, int2(origin.x + int(j * wide_fft::threads + lane), y), dims, p);
    wide_row_spectrum<false>(samples, group.y, lane);
}

func wide_column<let kernel : bool>(x: uint, lane: uint)->void {
    var samples : wide_fft::Line;
    [ForceUnroll]
    for (var j = 0u; j < wide_fft::slots; j++) {
        let at = uint2(x, j * wide_fft::threads + lane);
        if (kernel)
            samples.v[j] = spectrum[at];
        else
            samples.v[j] = tiles[at];
    }
    samples.transform<false>(lane);
    [ForceUnroll]
    for (var j = 0u; j < wide_fft::slots; j++) {
        let at = uint2(x, j * wide_fft::threads + lane);
        if (kernel)
            spectrum[at] = samples.v[j];
        else
            samples.v[j] = fft::multiply(samples.v[j], spectrum[at]);
    }
    if (!kernel) {
        samples.transform<true>(lane);
        [ForceUnroll]
        for (var j = 0u; j < wide_fft::slots; j++)
            tiles[uint2(x, j * wide_fft::threads + lane)] = samples.v[j] / float(wide_fft::size);
    }
}

[shader("compute")]
[numthreads(wide_fft::threads, 1, 1)]
func wide_kernel_columns_main(uint3 group: SV_GroupID, uint lane: SV_GroupIndex)->void {
    wide_column<true>(group.x, lane);
}

[shader("compute")]
[numthreads(wide_fft::threads, 1, 1)]
func wide_filter_columns_main(uint3 group: SV_GroupID, uint lane: SV_GroupIndex)->void {
    wide_column<false>(group.x, lane);
}

[shader("compute")]
[numthreads(wide_fft::threads, 1, 1)]
func wide_composite_rows_main([[vk::binding(2, 0)]] uniform scene: Texture2D<float4>,
                              uint3 group: SV_GroupID, uint lane: SV_GroupIndex)
    ->void {
    let p = parameters[0];
    let row = group.y + p.radius.y;
    var samples : wide_fft::Line;
    [ForceUnroll]
    for (var j = 0u; j < wide_fft::slots; j++) {
        let k = fft::frequency(j * wide_fft::threads + lane, wide_fft::size);
        let at = uint2(min(k, wide_fft::size - k), row);
        var rg = tiles[at];
        var blue = tiles[at + uint2(wide_bins, 0)].xy;
        if (k >= wide_bins) {
            rg.yw = -rg.yw;
            blue.y = -blue.y;
        }
        samples.v[j] = float4(rg.x - rg.w, rg.y + rg.z, blue);
    }
    samples.transform<true>(lane);
    let dims = extent(scene);
    [ForceUnroll]
    for (var j = 0u; j < wide_fft::slots; j++) {
        let x = j * wide_fft::threads + lane;
        let pixel = uint2(p.first_x + x - p.radius.x, p.first_y + group.y);
        if (x >= p.radius.x && x < wide_fft::size - p.radius.x && pixel.x < dims.x) {
            let base = scene.Load(int3(pixel, 0));
            let filtered = samples.v[j].xyz / float(wide_fft::size);
            let rgb = p.optical != 0 ? filtered : base.xyz + p.intensity * max(filtered, 0.0);
            output[pixel] = base.w >= 0 && all(isfinite(base)) ? float4(rgb, base.w) : base;
        }
    }
}
