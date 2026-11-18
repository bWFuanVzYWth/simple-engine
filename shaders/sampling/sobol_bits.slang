// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-FileCopyrightText: Copyright (c) 1998-2020 Matt Pharr, Wenzel Jakob, and Greg Humphreys
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
// FastOwen来自PBRT-v4；第三方许可见 ../../LICENSES/pbrt-Apache-2.0.txt。

/// 两种Sobol构造共用的无状态整数原语；不分配采样域、维度或随机身份。
module sobol_bits;

namespace sampling::sobol_bits {
public func hash(value: uint)->uint {
    var value = value;
    value ^= value >> 16;
    value *= 0x7feb352du;
    value ^= value >> 15;
    value *= 0x846ca68bu;
    return value ^ (value >> 16);
}

public func spread(value: uint)->uint {
    var value = value;
    value &= 0xffffu;
    value = (value | (value << 8)) & 0x00ff00ffu;
    value = (value | (value << 4)) & 0x0f0f0f0fu;
    value = (value | (value << 2)) & 0x33333333u;
    return (value | (value << 1)) & 0x55555555u;
}

public func reversed_y(value: uint)->uint {
    var value = value;
    value ^= (value >> 1) & 0x55555555u;
    value ^= (value >> 2) & 0x33333333u;
    value ^= (value >> 4) & 0x0f0f0f0fu;
    value ^= (value >> 8) & 0x00ff00ffu;
    value ^= (value >> 16) & 0x0000ffffu;
    return value;
}

public func owen_reversed(value: uint, seed: uint)->uint {
    var value = value;
    value ^= value * 0x3d20adeau;
    value += seed;
    value *= (seed >> 16) | 1u;
    value ^= value * 0x05526c56u;
    value ^= value * 0x53a22864u;
    return reversebits(value);
}
}
