// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 两路复数分别占 xy、zw。每个工作组复用 16 KiB 共享内存。
module fft;
import math.constants;

namespace fft {
public static const let lanes : uint = 128;
public groupshared var values : float4[1024];

/// 位反转频序中的槽号，零频始终位于零。
public func frequency(k: uint, size: uint)->uint {
    return reversebits(k) >> (32 - firstbithigh(size));
}

/// 两对复数逐对相乘。
public func multiply(a: float4, b: float4)->float4 {
    return a * b.xxzz + times_i(a) * b.yyww;
}

/// 两路复数均逆时针旋转 90 度。
public func times_i(a: float4)->float4 {
    return float4(-a.y, a.x, -a.w, a.z);
}

public func butterfly<let inverse : bool>(inout a: float4, inout b: float4, angle: float)->void {
    var sine : float;
    var cosine : float;
    sincos(inverse ? angle : -angle, sine, cosine);
    let root = float4(cosine, sine, cosine, sine);
    let left = a;
    if (inverse) {
        let right = multiply(b, root);
        a = left + right;
        b = left - right;
    } else {
        a = left + b;
        b = multiply(left - b, root);
    }
}

func adjacent_pairs<let threads : uint>(lane: uint, count: uint)->void {
    GroupMemoryBarrierWithGroupSync();
    for (var pair = lane; pair < count / 2; pair += threads) {
        let a = values[2 * pair];
        let b = values[2 * pair + 1];
        values[2 * pair] = a + b;
        values[2 * pair + 1] = a - b;
    }
}

/// 两级 radix-2 合成一个 radix-4 蝶形，保留原位反转顺序，每两级只交换一次共享内存。
public func transform_blocks<let inverse : bool, let threads : uint>(lane: uint, size: uint,
                                                                     count: uint)
    ->void {
    let odd = (firstbithigh(size) & 1) != 0;
    if (inverse && odd)
        adjacent_pairs<threads>(lane, count);
    for (var step = 1u; step <= size / 4; step <<= 2) {
        GroupMemoryBarrierWithGroupSync();
        let span = inverse ? step * (odd ? 2u : 1u) : (size / 4) / step;
        for (var group = lane; group < count / 4; group += threads) {
            let offset = group & (span - 1);
            let first = 4 * group - 3 * offset;
            let a = values[first];
            var b = values[first + span];
            var c = values[first + 2 * span];
            var d = values[first + 3 * span];
            var s : float;
            var co : float;
            sincos((inverse ? 0.5 : -0.5) * math::pi * float(offset) / float(span), s, co);
            let w = float4(co, s, co, s);
            let w2 = multiply(w, w);
            let w3 = multiply(w2, w);
            if (inverse) {
                b = multiply(b, w2);
                c = multiply(c, w);
                d = multiply(d, w3);
                let ab = a + b;
                let cd = c + d;
                let dab = a - b;
                let id = times_i(c - d);
                values[first] = ab + cd;
                values[first + span] = dab + id;
                values[first + 2 * span] = ab - cd;
                values[first + 3 * span] = dab - id;
            } else {
                let ac = a + c;
                let bd = b + d;
                let dac = a - c;
                let id = times_i(b - d);
                values[first] = ac + bd;
                values[first + span] = multiply(ac - bd, w2);
                values[first + 2 * span] = multiply(dac - id, w);
                values[first + 3 * span] = multiply(dac + id, w3);
            }
        }
    }
    if (!inverse && odd)
        adjacent_pairs<threads>(lane, count);
    GroupMemoryBarrierWithGroupSync();
}

public func transform<let inverse : bool>(lane: uint, size: uint)->void {
    transform_blocks<inverse, lanes>(lane, size, size);
}

public func unpack_rgb(z: float4, negative: float4, out rg: float4, out blue: float4)->void {
    let opposite = negative.xy * float2(1, -1);
    let sum = (z.xy + opposite) * 0.5;
    let diff = (z.xy - opposite) * 0.5;
    rg = float4(sum, diff.y, -diff.x);
    blue = float4(z.zw, 0, 0);
}

/// 从 FFT(R+iG, B+i0) 和共轭频率恢复三个实信号的正半谱。包括 DC 和 Nyquist。
public func split_rgb(k: uint, size: uint, out rg: float4, out blue: float4)->void {
    unpack_rgb(values[frequency(k, size)], values[frequency((-k) & (size - 1), size)], rg, blue);
}
}
