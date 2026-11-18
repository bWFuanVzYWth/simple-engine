// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 无资源的自动曝光计量；输入为场景线性 RGB，alpha 不参与。
/// 保留无 albedo／材质 guide 的等权计量：明暗材质占比会影响曝光，黑色参与首桶。
module exposure;
import color.primaries;

namespace display {
internal static const let EXPOSURE_BIN_COUNT : uint = 256;
internal static const let EXPOSURE_MIN_LOG_BRIGHTNESS : float = -16.0;
internal static const let EXPOSURE_MAX_LOG_BRIGHTNESS = 20.0;
internal static const let EXPOSURE_KEY = 0.16;
internal static const let EXPOSURE_MIN_EV : float = -16.0;
internal static const let EXPOSURE_MAX_EV = 16.0;
internal static const let EXPOSURE_DARKEN_T90 = 0.5;
internal static const let EXPOSURE_BRIGHTEN_T90 = 2.0;
internal static const let EXPOSURE_DARKEN_RATE = 2.302585092994046 / EXPOSURE_DARKEN_T90;
internal static const let EXPOSURE_BRIGHTEN_RATE = 2.302585092994046 / EXPOSURE_BRIGHTEN_T90;

/// 16B 时域状态；按当前 EV、初始化标记、目标 EV、测量 log2 亮度排列，首次使用前清零。
public struct ExposureState {
    public var exposure_ev : float;
    public var initialized : uint;
    public var target_ev : float;
    public var measured_log_brightness : float;
};

/// Rec.2020 的非负线性亮度；调用方负责处理非有限结果。
public func exposure_brightness(rec2020: float3)->float {
    return color::rec2020_luminance(max(rec2020, float3(0.0)));
}

/// 非有限 RGB 或亮度返回 -1；有效值映射到 0..255，黑色归首桶。
/// BT.709 先转 Rec.2020，再裁剪负通道；超出计量范围的亮度归端点桶。
public func exposure_bin(radiance: float3, linear709: bool)->int {
    var radiance = radiance;
    if (!all(isfinite(radiance)))
        return -1;
    if (linear709)
        radiance = color::bt709_to_rec2020(radiance);
    let brightness = exposure_brightness(radiance);
    if (!isfinite(brightness))
        return -1;
    let logarithm = clamp(log2(max(brightness, exp2(EXPOSURE_MIN_LOG_BRIGHTNESS))),
                          EXPOSURE_MIN_LOG_BRIGHTNESS, EXPOSURE_MAX_LOG_BRIGHTNESS);
    let range = EXPOSURE_MAX_LOG_BRIGHTNESS - EXPOSURE_MIN_LOG_BRIGHTNESS;
    let bin = uint((logarithm - EXPOSURE_MIN_LOG_BRIGHTNESS) * (float(EXPOSURE_BIN_COUNT) / range));
    return int(min(bin, EXPOSURE_BIN_COUNT - 1u));
}

/// 将0..255内的桶坐标仿射映射到log2亮度；允许传入保留样本的平均桶坐标。
public func exposure_bin_log(bin: float)->float {
    let range = EXPOSURE_MAX_LOG_BRIGHTNESS - EXPOSURE_MIN_LOG_BRIGHTNESS;
    return EXPOSURE_MIN_LOG_BRIGHTNESS + (bin + 0.5) * (range / float(EXPOSURE_BIN_COUNT));
}
/// 返回整数桶中心的log2亮度；桶须在0..255。
public func exposure_bin_log(bin: uint)->float {
    return exposure_bin_log(float(bin));
}

/// 有效测量的目标 EV；compensation 是 0..1 的自动调节强度，不是附加 EV。
/// mean/min/max 分别是剔除两端后按桶中心计量的平均、最小、最大 log2 亮度。
public func exposure_target_ev(measured: float, minimum: float, maximum: float, compensation: float)
    ->float {
    let range = maximum - minimum;
    let bias = range > 0.0 ? 2.0 * (2.0 * measured - minimum - maximum) / max(range, 2.0) : 0.0;
    let target = clamp(log2(EXPOSURE_KEY) - measured + bias, EXPOSURE_MIN_EV, EXPOSURE_MAX_EV);
    return target * compensation;
}

/// EV 域指数平滑；参数须有限，dt 须非负。暗化 T90 为 0.5 秒，亮化为 2 秒。
public func exposure_adapt(previous: float, target: float, dt: float)->float {
    let rate : float = target < previous ? EXPOSURE_DARKEN_RATE : EXPOSURE_BRIGHTEN_RATE;
    let blend = 1.0 - exp(-max(dt, 0.0) * rate);
    return lerp(previous, target, clamp(blend, 0.0, 1.0));
}
}
