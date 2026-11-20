// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

module wide_fft;
import fft;
import math.constants;

namespace wide_fft {
public static const let threads : uint = 256;
public static const let size : uint = 4096;
public static const let slots : uint = size / threads;
public static const let batch : uint = 1024 / threads;

/// 样本 i 由 lane=i%256 持有。跨度至少 256 的蝶形在线程内部完成；
/// 更短的蝶形以四个独立的 256 点变换共用原有的 16 KiB 暂存。
public struct Line {
    public var v : float4[slots];

    [mutating]
    func local_levels<let inverse : bool>(lane: uint)->void {
        [ForceUnroll]
        for (var step = 1u; step < slots; step <<= 1) {
            let span = inverse ? step : slots / (2 * step);
            [ForceUnroll]
            for (var pair = 0u; pair < slots / 2; pair++) {
                let offset = pair & (span - 1);
                let first = 2 * pair - offset;
                fft::butterfly<inverse>(v[first], v[first + span],
                                        math::pi * float(offset * threads + lane) /
                                            float(span * threads));
            }
        }
    }

    [mutating]
    public func transform<let inverse : bool>(lane: uint)->void {
        if (!inverse)
            local_levels<false>(lane);
        [ForceUnroll]
        for (var base = 0u; base < slots; base += batch) {
            GroupMemoryBarrierWithGroupSync();
            [ForceUnroll]
            for (var j = 0u; j < batch; j++)
                fft::values[j * threads + lane] = v[base + j];
            fft::transform_blocks<inverse, threads>(lane, threads, 1024);
            [ForceUnroll]
            for (var j = 0u; j < batch; j++)
                v[base + j] = fft::values[j * threads + lane];
        }
        if (inverse)
            local_levels<true>(lane);
    }
};
}
