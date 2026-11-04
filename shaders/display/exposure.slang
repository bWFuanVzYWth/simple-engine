// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 无资源的自动曝光计量；输入为场景线性 RGB，alpha 不参与。
/// 保留无 albedo／材质 guide 的等权计量：明暗材质占比会影响曝光，黑色参与首桶。
module exposure;
import color.primaries;

namespace display
{
internal static const uint EXPOSURE_BIN_COUNT = 256;
internal static const uint EXPOSURE_TAIL_DENOMINATOR = 200;
internal static const float EXPOSURE_MIN_LOG_BRIGHTNESS = -16.0;
internal static const float EXPOSURE_MAX_LOG_BRIGHTNESS = 20.0;
internal static const float EXPOSURE_KEY = 0.16;
internal static const float EXPOSURE_MIN_EV = -16.0;
internal static const float EXPOSURE_MAX_EV = 16.0;
internal static const float EXPOSURE_DARKEN_T90 = 0.5;
internal static const float EXPOSURE_BRIGHTEN_T90 = 2.0;

/// 16B 时域状态；按当前 EV、初始化标记、目标 EV、测量 log2 亮度排列，首次使用前清零。
public struct ExposureState
{
    public float exposure_ev;
    public uint initialized;
    public float target_ev;
    public float measured_log_brightness;
};

/// Rec.2020 的非负线性亮度；调用方负责处理非有限结果。
public float exposure_brightness(float3 rec2020)
{
    return dot(max(rec2020, float3(0.0)), float3(0.2627, 0.6780, 0.0593));
}

/// 非有限 RGB 或亮度返回 -1；有效值映射到 0..255，黑色归首桶。
/// BT.709 先转 Rec.2020，再裁剪负通道；超出计量范围的亮度归端点桶。
public int exposure_bin(float3 radiance, bool linear709)
{
    if (any(isnan(radiance)) || any(isinf(radiance)))
        return -1;
    if (linear709)
        radiance = color::bt709_to_rec2020(radiance);
    float brightness = exposure_brightness(radiance);
    if (isnan(brightness) || isinf(brightness))
        return -1;
    float logarithm = clamp(log2(max(brightness, exp2(EXPOSURE_MIN_LOG_BRIGHTNESS))),
                            EXPOSURE_MIN_LOG_BRIGHTNESS, EXPOSURE_MAX_LOG_BRIGHTNESS);
    float range = EXPOSURE_MAX_LOG_BRIGHTNESS - EXPOSURE_MIN_LOG_BRIGHTNESS;
    return int(min(uint(floor((logarithm - EXPOSURE_MIN_LOG_BRIGHTNESS) *
                              (float(EXPOSURE_BIN_COUNT) / range))),
                   EXPOSURE_BIN_COUNT - 1u));
}

/// 返回桶中心的 log2 亮度，bin 须在 0..255。
public float exposure_bin_log(uint bin)
{
    float range = EXPOSURE_MAX_LOG_BRIGHTNESS - EXPOSURE_MIN_LOG_BRIGHTNESS;
    return EXPOSURE_MIN_LOG_BRIGHTNESS + (float(bin) + 0.5) * (range / float(EXPOSURE_BIN_COUNT));
}

/// 有效测量的目标 EV；compensation 是 0..1 的自动调节强度，不是附加 EV。
/// mean/min/max 分别是剔除两端后按桶中心计量的平均、最小、最大 log2 亮度。
public float exposure_target_ev(float measured, float minimum, float maximum, float compensation)
{
    float range = maximum - minimum;
    float bias = range > 0.0 ? 2.0 * (2.0 * measured - minimum - maximum) / max(range, 2.0) : 0.0;
    float target = clamp(log2(EXPOSURE_KEY) - measured + bias, EXPOSURE_MIN_EV, EXPOSURE_MAX_EV);
    return target * compensation;
}

/// EV 域指数平滑；参数须有限，dt 须非负。暗化 T90 为 0.5 秒，亮化为 2 秒。
public float exposure_adapt(float previous, float target, float dt)
{
    float t90 = target < previous ? EXPOSURE_DARKEN_T90 : EXPOSURE_BRIGHTEN_T90;
    float blend = 1.0 - exp(-max(dt, 0.0) * 2.302585092994046 / t90);
    return lerp(previous, target, clamp(blend, 0.0, 1.0));
}
}
