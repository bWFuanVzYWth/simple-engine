// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// sRGB 传递函数和有符号线性外推。
module srgb;

namespace color {
/// 通道独立的 sRGB 编码；负输入沿线性分支外推，输出保留符号。
public func encode_srgb(linear_rgb: float3)->float3 {
    let curve = 1.055 * pow(max(linear_rgb, 0.0), 1.0 / 2.4) - 0.055;
    return select(linear_rgb < 0.0031308, linear_rgb * 12.92, curve);
}

/// 通道独立的 sRGB 解码；有限输入可超出 [0,1]。
public func decode_srgb(encoded: float3)->float3 {
    let curve = pow((encoded + 0.055) * (1.0 / 1.055), 2.4);
    return select(encoded <= 0.04045, encoded * (1.0 / 12.92), curve);
}
}
