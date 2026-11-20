// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only
module circular;
import circular_weights;

[[vk::binding(0, 0)]]
var source : Texture2D<float4>;
[[vk::binding(1, 0)]]
[[vk::image_format("rgba32f")]]
var output : RWTexture2D<float4>;
[[vk::binding(2, 0)]]
var detail : Texture2D<float4>;

func extent(image: Texture2D<float4>)->uint2 {
    var w : uint;
    var h : uint;
    image.GetDimensions(w, h);
    return uint2(w, h);
}
func output_extent()->uint2 {
    var w : uint;
    var h : uint;
    output.GetDimensions(w, h);
    return uint2(w, h);
}
func read_black(image: Texture2D<float4>, p: int2, dims: uint2)->float4 {
    if (any(uint2(p) >= dims))
        return 0;
    return image.Load(int3(p, 0));
}
func radiance(v: float4)->float3 {
    return v.w >= 0 && all(isfinite(v)) ? v.xyz : float3(0);
}

groupshared float3 down_tile[20][20];
[shader("compute")]
[numthreads(8, 8, 1)]
func circular_down(uint3 group: SV_GroupID, uint3 local: SV_GroupThreadID, uint lane: SV_GroupIndex)
    ->void {
    let dims = extent(source);
    for (var i = lane; i < 400; i += 64) {
        let at = int2(i % 20, i / 20);
        down_tile[at.y][at.x] = read_black(source, int2(group.xy * 16) + at - 4, dims).xyz;
    }
    GroupMemoryBarrierWithGroupSync();
    let p = group.xy * 8 + local.xy;
    if (any(p >= output_extent()))
        return;
    var sum = float3(0);
    [ForceUnroll]
    for (var y = 0; y < 6; ++y) {
        var row = float3(0);
        [ForceUnroll]
        for (var x = 0; x < 6; ++x)
            row += circular_weights::binomial[x] * down_tile[2 * local.y + y][2 * local.x + x];
        sum += circular_weights::binomial[y] * row;
    }
    // 缩小阶段把层号放进空闲的 alpha，重建时据此选择该尺度的 RGB 权重。
    output[p] = float4(sum, source.Load(int3(0, 0, 0)).w + 1.);
}

// 两次二倍缩小合成的 16 点可分离核。8×4 个输出覆盖 44×28 个输入像素。
// 同一块组内存先供各行读取，所有行结果就绪后复用为列输入。
groupshared float3 raw_tile[28][44];
func quarter_row(at: uint)->float3 {
    let y = at / 8;
    let x = (at % 8) * 4;
    var sum = float3(0);
    [ForceUnroll]
    for (var i = 0; i < 16; ++i)
        sum += circular_weights::quarter_taps[i] * raw_tile[y][x + i];
    return sum;
}
[shader("compute")]
[numthreads(16, 8, 1)]
func circular_quarter(uint3 group: SV_GroupID, uint lane: SV_GroupIndex)->void {
    let dims = extent(source);
    for (var i = lane; i < 28 * 44; i += 128) {
        let at = int2(i % 44, i / 44);
        raw_tile[at.y][at.x] =
            radiance(read_black(source, int2(group.xy * uint2(32, 16)) + at - 14, dims));
    }
    GroupMemoryBarrierWithGroupSync();
    let first = quarter_row(lane);
    var second = float3(0);
    if (lane < 96)
        second = quarter_row(lane + 128);
    GroupMemoryBarrierWithGroupSync();
    raw_tile[lane / 8][lane % 8] = first;
    if (lane < 96)
        raw_tile[(lane + 128) / 8][lane % 8] = second;
    GroupMemoryBarrierWithGroupSync();
    let local = uint2(lane % 8, lane / 8);
    let p = group.xy * uint2(8, 4) + local;
    if (lane < 32 && all(p < output_extent())) {
        var sum = float3(0);
        [ForceUnroll]
        for (var i = 0; i < 16; ++i)
            sum += circular_weights::quarter_taps[i] * raw_tile[4 * local.y + i][local.x];
        output[p] = float4(sum, 2.);
    }
}

// 二维重建为缩小算子的四倍转置。两个奇偶相位各自保持单位权重和。
func phase_weights(p: uint)->float3 {
    let even = 2. * float3(circular_weights::binomial[4], circular_weights::binomial[2],
                           circular_weights::binomial[0]);
    return (p & 1) == 0 ? even : even.zyx;
}
groupshared float3 coarse_tile[10][10];
func load_coarse(group: uint2, lane: uint)->void {
    let dims = extent(source);
    if (lane < 100) {
        let at = int2(lane % 10, lane / 10);
        coarse_tile[at.y][at.x] = read_black(source, int2(group * 8) + at, dims).xyz;
    }
}
func reconstruct(local: uint2)->float3 {
    let a = local / 2;
    let x = phase_weights(local.x);
    let y = phase_weights(local.y);
    var sum = float3(0);
    [ForceUnroll]
    for (var i = 0; i < 3; ++i)
        sum += y[i] * (x.x * coarse_tile[a.y + i][a.x] + x.y * coarse_tile[a.y + i][a.x + 1] +
                       x.z * coarse_tile[a.y + i][a.x + 2]);
    return sum;
}

[shader("compute")]
[numthreads(16, 16, 1)]
func circular_up(uint3 group: SV_GroupID, uint3 local: SV_GroupThreadID, uint lane: SV_GroupIndex)
    ->void {
    load_coarse(group.xy, lane);
    GroupMemoryBarrierWithGroupSync();
    let p = group.xy * 16 + local.xy;
    if (all(p < output_extent())) {
        let fine = output[p];
        let level = uint(fine.w);
        let coarse = reconstruct(local.xy);
        let tail = level + 1 == circular_weights::levels ? coarse * circular_weights::wings[level]
                                                         : coarse;
        output[p] = float4(fine.xyz * circular_weights::wings[level - 1] + tail, 0);
    }
}

// 每个 16×16 输出块只需重建 10×10 个半分辨率值。先在组内取它们共享的 8×8 粗层。
groupshared float3 quarter_tile[8][8];
groupshared float3 first_rows[24][10];
func quarter_reconstruct(p: uint2)->float3 {
    let a = p >> 1;
    let x = phase_weights(p.x);
    let y = phase_weights(p.y);
    var sum = float3(0);
    [ForceUnroll]
    for (var i = 0; i < 3; ++i)
        sum += y[i] * (x.x * quarter_tile[a.y + i][a.x] + x.y * quarter_tile[a.y + i][a.x + 1] +
                       x.z * quarter_tile[a.y + i][a.x + 2]);
    return sum;
}

func reconstruct_first_level(lane: uint)->void {
    if (lane < 100) {
        let at = uint2(lane % 10, lane / 10);
        let sum = quarter_reconstruct(at + 1);
        var half_value = float3(0);
        [ForceUnroll]
        for (var i = 0; i < 6; ++i)
            half_value += circular_weights::binomial[i] * first_rows[2 * at.y + i][at.x];
        coarse_tile[at.y][at.x] = circular_weights::wings[0] * half_value + sum;
    }
}

// 在一个工作组内重建中间一级，再直接写下一级，跳过中间图的整次写回。
[shader("compute")]
[numthreads(16, 16, 1)]
func circular_up_pair(uint3 group: SV_GroupID, uint3 local: SV_GroupThreadID,
                      uint lane: SV_GroupIndex)
    ->void {
    if (lane < 64) {
        let q = uint2(lane % 8, lane / 8);
        quarter_tile[q.y][q.x] = read_black(source, int2(group.xy * 4 + q), extent(source)).xyz;
    }
    GroupMemoryBarrierWithGroupSync();
    if (lane < 100) {
        let at = uint2(lane % 10, lane / 10);
        let p = group.xy * 8 + at;
        var value = float3(0);
        if (all(p < extent(detail))) {
            let raw = detail.Load(int3(p, 0));
            value = raw.xyz * circular_weights::wings[uint(raw.w) - 1] + quarter_reconstruct(at);
        }
        coarse_tile[at.y][at.x] = value;
    }
    GroupMemoryBarrierWithGroupSync();
    let p = group.xy * 16 + local.xy;
    if (all(p < output_extent())) {
        let raw = output[p];
        output[p] =
            float4(raw.xyz * circular_weights::wings[uint(raw.w) - 1] + reconstruct(local.xy), 0);
    }
}

groupshared float3 near_tile[24][24];
// 按离中心行的距离，选取 5×5 径向积分核的三个互异系数。
static const uint3 core_index[3] = { uint3(0, 1, 3), uint3(1, 2, 4), uint3(3, 4, 5) };
func core_row(d: uint, center: float3, side: float3, outer: float3)->float3 {
    let index = core_index[d];
    return circular_weights::core[index.x] * center + circular_weights::core[index.y] * side +
           circular_weights::core[index.z] * outer;
}

[shader("compute")]
[numthreads(8, 8, 1)]
func circular_composite(uint3 group: SV_GroupID, uint3 local: SV_GroupThreadID,
                        uint lane: SV_GroupIndex)
    ->void {
    let dims = extent(detail);
    let origin = group.xy * 16;
    let p = origin + 2 * local.xy;
    var base : float4[4];
    [ForceUnroll]
    for (var i = 0; i < 4; ++i) {
        let at = uint2(i % 2, i / 2);
        base[i] = read_black(detail, int2(p + at), dims);
        near_tile[2 * local.y + 4 + at.y][2 * local.x + 4 + at.x] = radiance(base[i]);
    }
    let q = uint2(lane % 8, lane / 8);
    quarter_tile[q.y][q.x] = read_black(source, int2(group.xy * 4 + q), extent(source)).xyz;
    // 六行、六列的核心邻域由四个相邻输出共用，边缘再向外读到二项式滤波的支撑域。
    for (var i = lane; i < 320; i += 64) {
        let side = i - 192;
        let at = i < 192 ? uint2(i % 24, i / 24 + (i >= 96 ? 16 : 0))
                         : uint2(side % 8 + (side % 8 >= 4 ? 16 : 0), 4 + side / 8);
        near_tile[at.y][at.x] = radiance(read_black(detail, int2(origin + at) - 4, dims));
    }
    GroupMemoryBarrierWithGroupSync();
    for (var k = lane; k < 240; k += 64) {
        let x = k % 10;
        let y = k / 10;
        var row = float3(0);
        [ForceUnroll]
        for (var i = 0; i < 6; ++i)
            row += circular_weights::binomial[i] * near_tile[y][2 * x + i];
        first_rows[y][x] = row;
    }
    GroupMemoryBarrierWithGroupSync();
    reconstruct_first_level(lane);
    reconstruct_first_level(lane + 64);
    GroupMemoryBarrierWithGroupSync();
    var rgb : float3[4];
    [ForceUnroll]
    for (var i = 0; i < 4; ++i)
        rgb[i] = reconstruct(local.xy * 2 + uint2(i % 2, i / 2));
    [ForceUnroll]
    for (var row = 0; row < 6; ++row) {
        let y = 2 * local.y + 2 + row;
        let x = 2 * local.x + 2;
        let a = near_tile[y][x];
        let b = near_tile[y][x + 1];
        let c = near_tile[y][x + 2];
        let d = near_tile[y][x + 3];
        let e = near_tile[y][x + 4];
        let f = near_tile[y][x + 5];
        if (row < 5) {
            rgb[0] += core_row(abs(int(row) - 2), c, b + d, a + e);
            rgb[1] += core_row(abs(int(row) - 2), d, c + e, b + f);
        }
        if (row > 0) {
            rgb[2] += core_row(abs(int(row) - 3), c, b + d, a + e);
            rgb[3] += core_row(abs(int(row) - 3), d, c + e, b + f);
        }
    }
    [ForceUnroll]
    for (var i = 0; i < 4; ++i) {
        let at = p + uint2(i % 2, i / 2);
        if (all(at < dims))
            output[at] =
                base[i].w >= 0 && all(isfinite(base[i])) ? float4(rgb[i], base[i].w) : base[i];
    }
}
