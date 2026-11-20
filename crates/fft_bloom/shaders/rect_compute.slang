// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 实值光学核。横向实输入的共轭半谱完整保留非对称光斑的复数相位。
module rect_compute;
import mixed_fft;
import fft;
using namespace mixed_fft;

struct Parameters {
    var extent : uint2; // FFT 矩形
    var radius : uint2; // CPU 校验的完整光斑半径
    var origin : uint2; // 本块在输出图的起点
    var halo : uint2;   // 分块轴裁去的前缘，整轴为零
    var valid_extent : uint2;
    var read_rows : uint2; // 含有效输入的 FFT 行区间 [begin, end)
    var core_radius : uint;
};
[[vk::binding(0, 0)]]
var parameters : StructuredBuffer<Parameters>;
[[vk::binding(1, 0)]]
[[vk::image_format("rgba32f")]]
var work : RWTexture2D<float4>;
// 与工作图一样，R/G 两路复数配对，相邻两个 B 频点配对。
[[vk::binding(2, 0)]]
[[vk::image_format("rgba32f")]]
var transfer : RWTexture2D<float4>;
[[vk::binding(3, 0)]]
var scene : Texture2D<float4>;
[[vk::binding(4, 0)]]
[[vk::image_format("rgba32f")]]
var output : RWTexture2D<float4>;
[[vk::binding(5, 0)]]
var weights : StructuredBuffer<float>;

func valid(v: float4)->bool {
    return v.w >= 0 && all(isfinite(v));
}

func store_row(inout a: Line, y: uint, lane: uint)->void {
    const uint bins = size / 2 + 1;
    let low = lane_frequency(lane);
    [ForceUnroll]
    for (var base = 0u; base < slots; base += batch) {
        let count = min(batch, slots - base);
        GroupMemoryBarrierWithGroupSync();
        [ForceUnroll]
        for (var j = 0u; j < batch; j++) {
            if (j >= count)
                continue;
            let q = (base + j) % slots;
            fft::values[j * lanes + lane] = float4(a.v[(slots - q) % slots].xy, a.v[q].zw);
        }
        GroupMemoryBarrierWithGroupSync();
        [ForceUnroll]
        for (var j = 0u; j < batch; j++) {
            if (j >= count)
                continue;
            let q = (base + j) % slots;
            let k = low * slots + q;
            if (k < bins) {
                let neg = (2 * lanes - low - uint(q > 0)) % lanes;
                var rg : float4;
                var blue : float4;
                fft::unpack_rgb(a.v[q], fft::values[j * lanes + frequency_lane(neg)], rg, blue);
                let bin = k == size / 2 ? size / 2 : q * (lanes / 2) + lane / 2;
                work[uint2(y, bin)] = rg;
                // 非 Nyquist 正频率只落在偶数 lane。相邻两个蓝色频点共用一条 float4 列。
                if (k == size / 2)
                    work[uint2(y, bins + size / 4)] = blue;
                else if ((lane & 3) == 0)
                    work[uint2(y, bins + bin / 2)] =
                        float4(blue.xy, fft::values[j * lanes + lane + 2].zw);
            }
        }
    }
}

[shader("compute")]
[numthreads(lanes, 1, 1)]
func rect_kernel_rows(uint3 g: SV_GroupID, uint lane: SV_GroupIndex)->void {
    let p = parameters[0];
    let dy = int(g.x) - int(min(p.radius.y, (p.extent.y - 1) / 2));
    let radius_x = min(p.radius.x, (size - 1) / 2);
    var a : Line;
    [ForceUnroll]
    for (var j = 0u; j < slots; j++) {
        let x = j * lanes + lane;
        let dx = x <= size / 2 ? int(x) : int(x) - int(size);
        var rgb = float3(0);
        if (uint(abs(dx)) <= radius_x && uint(max(abs(dx), abs(dy))) > p.core_radius) {
            let pos = uint2(int2(p.radius) + int2(dx, dy));
            let i = 3 * (pos.y * (2 * p.radius.x + 1) + pos.x);
            rgb = float3(weights[i], weights[i + 1], weights[i + 2]);
        }
        a.v[j] = float4(rgb, 0);
    }
    transform<false>(a, lane);
    store_row(a, g.x, lane);
}

[shader("compute")]
[numthreads(lanes, 1, 1)]
func rect_forward_rows(uint3 g: SV_GroupID, uint lane: SV_GroupIndex)->void {
    let p = parameters[0];
    var width : uint;
    var height : uint;
    scene.GetDimensions(width, height);
    let start = int2(p.origin) - int2(p.halo);
    let row = g.x + p.read_rows.x;
    let y = int(row) + start.y;
    var a : Line;
    [ForceUnroll]
    for (var j = 0u; j < slots; j++) {
        let x = start.x + int(j * lanes + lane);
        var rgb = float3(0);
        if (x >= 0 && uint(x) < width) {
            let v = scene.Load(int3(x, y, 0));
            if (valid(v))
                rgb = v.xyz;
        }
        a.v[j] = float4(rgb, 0);
    }
    transform<false>(a, lane);
    store_row(a, row, lane);
}

func load_blue(y: uint, bin: uint, bins: uint)->float2 {
    let pair = work[uint2(y, bins + bin / 2)];
    return (bin & 1) == 0 ? pair.xy : pair.zw;
}

[shader("compute")]
[numthreads(lanes, 1, 1)]
func rect_kernel_columns(uint3 g: SV_GroupID, uint lane: SV_GroupIndex)->void {
    let p = parameters[0];
    let radius_y = int(min(p.radius.y, (size - 1) / 2));
    var a : Line;
    [ForceUnroll]
    for (var j = 0u; j < slots; j++) {
        let y = j * lanes + lane;
        let dy = y <= size / 2 ? int(y) : int(y) - int(size);
        a.v[j] = 0;
        if (abs(dy) <= radius_y)
            a.v[j] = work[uint2(dy + radius_y, g.x)];
    }
    transform<false>(a, lane);
    [ForceUnroll]
    for (var q = 0u; q < slots; q++)
        transfer[uint2(q * lanes + lane, g.x)] = a.v[q];
}

[shader("compute")]
[numthreads(lanes, 1, 1)]
func rect_filter_columns(uint3 g: SV_GroupID, uint lane: SV_GroupIndex)->void {
    let p = parameters[0];
    var a : Line;
    [ForceUnroll]
    for (var j = 0u; j < slots; j++) {
        let y = j * lanes + lane;
        a.v[j] = 0;
        if (y >= p.read_rows.x && y < p.read_rows.y)
            a.v[j] = work[uint2(y, g.x)];
    }
    transform<false>(a, lane);
    [ForceUnroll]
    for (var q = 0u; q < slots; q++)
        a.v[q] = fft::multiply(a.v[q], transfer[uint2(q * lanes + lane, g.x)]);
    transform<true>(a, lane);
    [ForceUnroll]
    for (var j = 0u; j < slots; j++) {
        let y = j * lanes + lane;
        if (y >= p.halo.y && y < p.halo.y + p.valid_extent.y)
            work[uint2(y, g.x)] = a.v[j] / float(size);
    }
}

[shader("compute")]
[numthreads(lanes, 1, 1)]
func rect_inverse_rows(uint3 g: SV_GroupID, uint lane: SV_GroupIndex)->void {
    let p = parameters[0];
    let row = g.x + p.halo.y;
    let low = lane_frequency(lane);
    const uint bins = size / 2 + 1;
    var a : Line;
    [ForceUnroll]
    for (var q = 0u; q < slots; q++) {
        let k = low * slots + q;
        let bin = half_bin(min(k, size - k));
        var rg = work[uint2(row, bin)];
        var b = load_blue(row, bin, bins);
        if (k >= bins) {
            rg.yw = -rg.yw;
            b.y = -b.y;
        }
        a.v[q] = float4(rg.x - rg.w, rg.y + rg.z, b);
    }
    transform<true>(a, lane);
    [ForceUnroll]
    for (var j = 0u; j < slots; j++) {
        let x = j * lanes + lane;
        if (x >= p.halo.x && x < p.halo.x + p.valid_extent.x) {
            let pos = p.origin + uint2(x - p.halo.x, g.x);
            // 紧随其后的中心核恢复 alpha 与无效像素。远翼只需写光量。
            output[pos] = float4(a.v[j].xyz / float(size), 0);
        }
    }
}
