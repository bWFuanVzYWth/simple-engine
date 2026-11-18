// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 贡献乘除与 MIS 权重；可表示的最终结果使用指数拆分避免中间溢出。
/// 因子与 PDF 须有限非负，除法分母须为正。
module weights;

namespace transport {
// 非零因子位于 2^-40..2^40 时，三个因子的乘除仍在 float32 正规数域。
// 87=127-40；指数区间以无符号位模式检查，零因子也安全。其余输入保留指数拆分。
func common_factor(value: float)->bool {
    let bits = asuint(value);
    return (bits == 0) | (bits - (87u << 23u) <= (80u << 23u));
}
func common_factor(value: float3)->bool {
    let bits = asuint(value);
    return all((bits == 0) | (bits - (87u << 23u) <= (80u << 23u)));
}

/// 计算 first*second/denominator；极端范围拆分指数，避免可表示结果的中间乘积溢出。
public func product_over(first: float, second: float, denominator: float)->float {
    if (common_factor(first) & common_factor(second) & common_factor(denominator))
        return (first * second) / denominator;
    var a : int;
    var b : int;
    var c : int;
    let significand : float = frexp(first, a) * frexp(second, b) / frexp(denominator, c);
    return ldexp(significand, a + b - c);
}

/// 按 RGB 分量计算相同的乘积／标量分母，语义同标量重载。
public func product_over(first: float3, second: float3, denominator: float)->float3 {
    if (common_factor(first) & common_factor(second) & common_factor(denominator))
        return (first * second) / denominator;
    var a : int3;
    var b : int3;
    var c : int;
    let significand : float3 = frexp(first, a) * frexp(second, b) / frexp(denominator, c);
    return ldexp(significand, a + b - int3(c));
}

/// 计算两个 RGB 因子与标量权重的乘积，保留最终的可表示结果。
public func triple_product(first: float3, second: float3, third: float)->float3 {
    if (common_factor(first) & common_factor(second) & common_factor(third))
        return (first * second) * third;
    var a : int3;
    var b : int3;
    var c : int;
    let significand : float3 = frexp(first, a) * frexp(second, b) * frexp(third, c);
    return ldexp(significand, a + b + int3(c));
}

/// 同量度的两策略 power heuristic；first<=0 为0，second<=0 为1。
/// 非零路径按较大 PDF 归一化，避免大比值平方溢出。
public func power_heuristic(first: float, second: float)->float {
    if (!all(float2(first, second) > 0.0))
        return float(first > 0.0);
    let ratio = min(first, second) / max(first, second);
    let square = ratio * ratio;
    return select(first >= second, 1.0, square) / (1.0 + square);
}

/// 合并 sampled 策略的权重与 1/pdf，避免单独权重下溢抹掉可表示贡献。
/// 两种 PDF 必须同量度；delta 概率质量不可与立体角密度混用。
public func power_heuristic_over_pdf(sampled: float, other: float)->float {
    if (sampled <= 0.0)
        return 0.0;
    if (other <= 0.0)
        return 1.0 / sampled;
    if (common_factor(sampled) & common_factor(other))
        return sampled / (sampled * sampled + other * other);
    if (sampled >= other) {
        let ratio = other / sampled;
        return (1.0 / sampled) / (1.0 + ratio * ratio);
    }
    let ratio = sampled / other;
    return (ratio / other) / (1.0 + ratio * ratio);
}
}
