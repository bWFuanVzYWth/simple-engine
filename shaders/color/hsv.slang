// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// RGB 与 HSV 的分量转换。
module hsv;

namespace color {
/// 有限 RGB 转 HSV；最大分量不超过 1e-7 时 H 和 S 取零。
/// 允许负分量，作 gamut 修正时 S 可超过 1。
public func rgb_to_hsv(color: float3)->float3 {
    let maximum = max(color.x, max(color.y, color.z));
    let chroma = maximum - min(color.x, min(color.y, color.z));
    if (chroma <= 0.0 || maximum <= 1.0e-7)
        return float3(0.0, 0.0, maximum);

    let red = maximum == color.x;
    let green = maximum == color.y;
    let numerator =
        select(red, color.y - color.z, select(green, color.z - color.x, color.x - color.y));
    let sector = select(red, 0.0, select(green, 2.0, 4.0));
    let hue = frac((numerator / chroma + sector) * (1.0 / 6.0));
    return float3(hue, chroma / maximum, maximum);
}

/// H 以一圈为单位并周期折返；S 和 V 由调用方约束，不执行 gamut 裁剪。
public func hsv_to_rgb(hsv: float3)->float3 {
    let primary = saturate(abs(frac(hsv.x + float3(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0);
    return hsv.z * lerp(1.0, primary, hsv.y);
}
}
