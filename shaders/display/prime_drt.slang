// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 无资源的 primeDRT；参数由 Rust display::PrimeDrtSettings 在输出校准变化时推导。
/// 输入采用 D65 场景线性 RGB；曝光乘积、原色转换及后续中间量须可由有限 f32 表示。
/// 保留 reference 的非有限输入归零语义，不修复不受支持的中间溢出。
module prime_drt;
import color.primaries;

namespace display
{
/// 两个 float4 的 32B 参数，内容与 Rust PrimeDrtParameters::to_array 一致。
/// 参数须来自有效主机设置；不得将编码输出峰值误作线性 headroom。
public struct PrimeDrtParameters
{
    /// 仅应用一次的有限正手动曝光倍率。
    public float exposure_multiplier;
    /// 压缩曲线的渐近峰值，由起点与 headroom 推导。
    public float curve_peak;
    /// headroom 经 sRGB 传递函数编码后的输出上限。
    public float encoded_output_peak;
    /// 编码域内向原色相恢复的比例，范围 0..1。
    public float hue_compensation;
    /// 编码域内附加饱和倍率的增量，范围 0..0.5。
    public float saturation_compensation;
    /// 与恒等映射相切的场景线性压缩起点，范围 0..0.99。
    public float compression_start;
    /// 保留为零，使独立参数块与两个 float4 一致。
    public float2 padding;
};

static const float3 NEUTRAL_WEIGHTS =
    float3(0.2120053547549465, 0.3921825078090138, 0.3958121374360396);
static const float GAMUT_COMPRESSION = 0.04;

internal float3 to_virtual_rgb(float3 color)
{
    float neutral = dot(color, NEUTRAL_WEIGHTS);
    return lerp(color, neutral.xxx, GAMUT_COMPRESSION);
}

internal float3 from_virtual_rgb(float3 color)
{
    // neutral 投影不变，因此这是上述线性映射的逆。
    float neutral = dot(color, NEUTRAL_WEIGHTS);
    return (color - GAMUT_COMPRESSION * neutral.xxx) / (1.0 - GAMUT_COMPRESSION);
}

internal float curve_component(float value, PrimeDrtParameters parameters)
{
    if (value <= parameters.compression_start)
        return value;
    float distance = value - parameters.compression_start;
    float extent = parameters.curve_peak - parameters.compression_start;
    // 倒数形式避免 distance/extent 溢出及 GPU 大分母倒数冲零。
    return parameters.compression_start + extent / (1.0 + extent / distance);
}

internal float3 curve(float3 value, PrimeDrtParameters parameters)
{
    return float3(curve_component(value.x, parameters), curve_component(value.y, parameters),
                  curve_component(value.z, parameters));
}

internal float3 encode_srgb(float3 linear_rgb)
{
    // 负 outset 值保留到 HSV 补偿，不能在此饱和裁剪。
    bool3 cutoff = linear_rgb < 0.0031308;
    float3 higher = 1.055 * pow(max(linear_rgb, 0.0), 1.0 / 2.4) - 0.055;
    float3 lower = linear_rgb * 12.92;
    return float3(cutoff.x ? lower.x : higher.x, cutoff.y ? lower.y : higher.y,
                  cutoff.z ? lower.z : higher.z);
}

internal float3 decode_srgb(float3 encoded)
{
    bool3 cutoff = encoded <= 0.04045;
    float3 lower = encoded / 12.92;
    float3 higher = pow((encoded + 0.055) / 1.055, 2.4);
    return float3(cutoff.x ? lower.x : higher.x, cutoff.y ? lower.y : higher.y,
                  cutoff.z ? lower.z : higher.z);
}

internal float3 rgb_to_hsv(float3 color)
{
    float maximum = max(color.x, max(color.y, color.z));
    float minimum = min(color.x, min(color.y, color.z));
    float chroma = maximum - minimum;
    float hue = 0.0;
    if (chroma > 0.0 && maximum > 1.0e-7)
    {
        if (maximum == color.x)
            hue = (color.y - color.z) / chroma;
        else if (maximum == color.y)
            hue = (color.z - color.x) / chroma + 2.0;
        else
            hue = (color.x - color.y) / chroma + 4.0;
        hue = frac(hue / 6.0);
    }
    float saturation = maximum > 1.0e-7 ? chroma / max(maximum, 1.0e-7) : 0.0;
    return float3(hue, saturation, maximum);
}

internal float3 hsv_to_rgb(float3 hsv)
{
    float3 primary =
        clamp(abs(frac(hsv.x + float3(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, 0.0, 1.0);
    return hsv.z * lerp(1.0.xxx, primary, hsv.y);
}

internal float3 compensate_hsv(float3 original_linear, float3 mapped_display,
                               float hue_compensation, float saturation_compensation)
{
    if (hue_compensation == 0.0 && saturation_compensation == 0.0 && all(mapped_display >= 0.0))
        return mapped_display;
    float3 mapped_hsv = rgb_to_hsv(mapped_display);
    if (mapped_hsv.y == 0.0)
        return mapped_display;
    if (hue_compensation > 0.0 && mapped_hsv.y > 1.0e-7)
    {
        float3 original_hsv = rgb_to_hsv(encode_srgb(original_linear));
        if (original_hsv.y > 1.0e-7)
        {
            float hue_offset = original_hsv.x - mapped_hsv.x;
            hue_offset -= floor(hue_offset + 0.5);
            mapped_hsv.x = frac(mapped_hsv.x + hue_compensation * hue_offset);
        }
    }
    // 负 outset 造成的饱和越界即使零补偿也截断；H/V 与中性端点不变。
    mapped_hsv.y = min(mapped_hsv.y * (1.0 + saturation_compensation), 1.0);
    return hsv_to_rgb(mapped_hsv);
}

internal float3 tonemap(float3 linear_bt709, PrimeDrtParameters parameters)
{
    float3 nonnegative = max(linear_bt709, float3(0.0));
    float3 working = to_virtual_rgb(nonnegative);
    float3 mapped_linear = from_virtual_rgb(curve(working, parameters));
    float3 mapped_display = encode_srgb(mapped_linear);
    return compensate_hsv(nonnegative, mapped_display, parameters.hue_compensation,
                          parameters.saturation_compensation);
}

/// 场景线性 BT.709／sRGB 直接转显示 sRGB 编码 RGB，不经过 Rec.2020 往返矩阵。
/// 非有限输入归零；其余输入先截负再乘曝光，随后执行完整 primeDRT。
/// 返回范围为 [0,encoded_output_peak]；alpha、附件编码和最终量化由调用方处理。
public float3 prime_drt_bt709(float3 hdr_bt709, PrimeDrtParameters parameters)
{
    if (any(isnan(hdr_bt709)) || any(isinf(hdr_bt709)))
        return float3(0.0);
    float3 exposed = max(hdr_bt709, float3(0.0)) * parameters.exposure_multiplier;
    return clamp(tonemap(exposed, parameters), 0.0, parameters.encoded_output_peak);
}

/// 场景线性 Rec.2020 转显示 sRGB 编码 RGB，按显示原色变换后裁剪。
/// 非有限输入归零；先截负并乘曝光，再转 BT.709 和截负，负 outset 在后续 HSV 前保留。
/// 返回范围为 [0,encoded_output_peak]；不负责 HDR 输出协议或附件传递函数。
public float3 prime_drt_rec2020(float3 hdr_rec2020, PrimeDrtParameters parameters)
{
    if (any(isnan(hdr_rec2020)) || any(isinf(hdr_rec2020)))
        return float3(0.0);
    float3 exposed = max(hdr_rec2020, float3(0.0)) * parameters.exposure_multiplier;
    float3 linear709 = max(color::rec2020_to_bt709(exposed), float3(0.0));
    return clamp(tonemap(linear709, parameters), 0.0, parameters.encoded_output_peak);
}

/// 将完整 primeDRT 的编码输出解码为显示线性 BT.709，供 sRGB attachment 或线性图像接口。
/// 输入与数值前提同 prime_drt_bt709；范围为 [0,headroom]，不重复执行原色变换。
public float3 prime_drt_linear_bt709(float3 hdr_bt709, PrimeDrtParameters parameters)
{
    return decode_srgb(prime_drt_bt709(hdr_bt709, parameters));
}
}
