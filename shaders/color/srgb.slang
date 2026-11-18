// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// sRGB 传递函数和有符号线性外推。
module srgb;

namespace color {
static const let INVERSE_SRGB_SCALE = 1.0 / 1.055;
static const let INVERSE_SRGB_SLOPE = 1.0 / 12.92;
static const let INVERSE_SRGB_GAMMA = 1.0 / 2.4;

/// 通道独立的 sRGB 编码；负输入沿线性分支外推，输出保留符号。
public func encode_srgb(linear_rgb: float3)->float3 {
    let cutoff = linear_rgb < 0.0031308;
    let higher : float3 = 1.055 * pow(max(linear_rgb, 0.0), INVERSE_SRGB_GAMMA) - 0.055;
    let lower = linear_rgb * 12.92;
    return float3(cutoff.x ? lower.x : higher.x, cutoff.y ? lower.y : higher.y,
                  cutoff.z ? lower.z : higher.z);
}

/// 通道独立的 sRGB 解码；有限输入可超出 [0,1]。
public func decode_srgb(encoded: float3)->float3 {
    let cutoff = encoded <= 0.04045;
    let lower : float3 = encoded * INVERSE_SRGB_SLOPE;
    let higher : float3 = pow((encoded + 0.055) * INVERSE_SRGB_SCALE, 2.4);
    return float3(cutoff.x ? lower.x : higher.x, cutoff.y ? lower.y : higher.y,
                  cutoff.z ? lower.z : higher.z);
}
}
