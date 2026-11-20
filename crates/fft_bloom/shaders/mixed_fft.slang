// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 每条轴由构建表给定线程数和寄存器基数。行与列独立特化，不要求 FFT 是正方形或二次幂。
module mixed_fft;
import fft;
import math.constants;

namespace mixed_fft {
public static const let lanes : uint = FFT_LANES;
static const let lane_radix : uint = FFT_LANE_RADIX;
static const let span : uint = lanes / lane_radix;
static const let r0 : uint = FFT_R0;
static const let r1 : uint = FFT_R1;
static const let r2 : uint = FFT_R2;
static const let r3 : uint = FFT_R3;
public static const let slots : uint = r0 * r1 * r2 * r3;
public static const let size : uint = lanes * slots;
public static const let batch : uint = 4;

func rotate<let inverse : bool>(a: float4, turns: float)->float4 {
    var s : float;
    var c : float;
    sincos((inverse ? 2.0 : -2.0) * math::pi * turns, s, c);
    return a * c + fft::times_i(a) * s;
}

// 奇基数利用一对正负角的公共和与差，常量旋转在 Slang 特化时求值。
func butterfly<let radix : uint, let inverse : bool>(inout v: float4[radix])->void {
    if (radix == 2) {
        let a = v[0];
        let b = v[1];
        v[0] = a + b;
        v[1] = a - b;
    } else {
        var result : float4[radix];
        [ForceUnroll]
        for (var k = 0u; k < radix; k++)
            result[k] = v[0];
        [ForceUnroll]
        for (var j = 1u; j <= (radix - 1) / 2; j++) {
            let even = v[j] + v[radix - j];
            let odd = fft::times_i(v[j] - v[radix - j]);
            result[0] += even;
            [ForceUnroll]
            for (var k = 1u; k <= (radix - 1) / 2; k++) {
                let angle = 2.0 * math::pi * float(j * k) / float(radix);
                let a = even * cos(angle);
                let b = odd * sin(inverse ? angle : -angle);
                result[k] += a + b;
                result[radix - k] += a - b;
            }
        }
        v = result;
    }
}

public struct Line {
    public var v : float4[slots];
};

func register_stage<let radix : uint, let width : uint, let inverse : bool>(inout a: Line)->void {
    if (radix == 1)
        return;
    const uint stride = width / radix;
    [ForceUnroll]
    for (var base = 0u; base < slots; base += width) {
        [ForceUnroll]
        for (var offset = 0u; offset < stride; offset++) {
            var v : float4[radix];
            [ForceUnroll]
            for (var j = 0u; j < radix; j++) {
                let sample = a.v[base + j * stride + offset];
                v[j] = inverse && j * offset != 0
                           ? rotate<true>(sample, float(j * offset) / float(width))
                           : sample;
            }
            butterfly<radix, inverse>(v);
            [ForceUnroll]
            for (var j = 0u; j < radix; j++) {
                a.v[base + j * stride + offset] =
                    !inverse && j * offset != 0
                        ? rotate<false>(v[j], float(j * offset) / float(width))
                        : v[j];
            }
        }
    }
}

func register_index(k: uint)->uint {
    return (k % r0) * (slots / r0) + (k / r0 % r1) * (r2 * r3) + (k / (r0 * r1) % r2) * r3 +
           k / (r0 * r1 * r2);
}

// 连续线程读连续输入；寄存器内先做小 FFT，跨线程只通过最多 16 KiB 的共享区交换。
func lane_stage<let inverse : bool>(lane: uint, count: uint)->void {
    if (lane_radix == 1)
        return;
    if (!inverse)
        GroupMemoryBarrierWithGroupSync();
    if (lane < span) {
        [ForceUnroll]
        for (var j = 0u; j < batch; j++) {
            if (j >= count)
                continue;
            var v : float4[lane_radix];
            [ForceUnroll]
            for (var t = 0u; t < lane_radix; t++) {
                let sample = fft::values[j * lanes + lane + t * span];
                v[t] = inverse && t != 0 ? rotate<true>(sample, float(t * lane) / float(lanes))
                                         : sample;
            }
            butterfly<lane_radix, inverse>(v);
            [ForceUnroll]
            for (var t = 0u; t < lane_radix; t++) {
                fft::values[j * lanes + lane + t * span] =
                    !inverse && t != 0 ? rotate<false>(v[t], float(t * lane) / float(lanes)) : v[t];
            }
        }
    }
    if (inverse)
        GroupMemoryBarrierWithGroupSync();
}

/// 输出频率为 lane_frequency(lane) * slots + q。列滤波直接消费此排列。
public func lane_frequency(lane: uint)->uint {
    return lane_radix * fft::frequency(lane % span, span) + lane / span;
}
public func frequency_lane(k: uint)->uint {
    return (k % lane_radix) * span + fft::frequency(k / lane_radix, span);
}
public func half_bin(k: uint)->uint {
    return k == size / 2 ? size / 2 : (k % slots) * (lanes / 2) + frequency_lane(k / slots) / 2;
}

public func transform<let inverse : bool>(inout a: Line, lane: uint)->void {
    var natural : Line;
    if (!inverse) {
        register_stage<r0, slots, false>(a);
        register_stage<r1, slots / r0, false>(a);
        register_stage<r2, r2 * r3, false>(a);
        register_stage<r3, r3, false>(a);
        [ForceUnroll]
        for (var q = 0u; q < slots; q++) {
            let sample = a.v[register_index(q)];
            natural.v[q] = q == 0 ? sample : rotate<false>(sample, float(q * lane) / float(size));
        }
        a = natural;
    }
    [ForceUnroll]
    for (var base = 0u; base < slots; base += batch) {
        let count = min(batch, slots - base);
        GroupMemoryBarrierWithGroupSync();
        [ForceUnroll]
        for (var j = 0u; j < batch; j++)
            if (j < count)
                fft::values[j * lanes + lane] = a.v[(base + j) % slots];
        if (!inverse)
            lane_stage<false>(lane, count);
        fft::transform_blocks<inverse, lanes>(lane, span, count * lanes);
        if (inverse)
            lane_stage<true>(lane, count);
        [ForceUnroll]
        for (var j = 0u; j < batch; j++)
            if (j < count)
                a.v[(base + j) % slots] = fft::values[j * lanes + lane];
    }
    if (inverse) {
        [ForceUnroll]
        for (var q = 0u; q < slots; q++) {
            natural.v[register_index(q)] =
                q == 0 ? a.v[q] : rotate<true>(a.v[q], float(q * lane) / float(size));
        }
        register_stage<r3, r3, true>(natural);
        register_stage<r2, r2 * r3, true>(natural);
        register_stage<r1, slots / r0, true>(natural);
        register_stage<r0, slots, true>(natural);
        a = natural;
    }
}
}
