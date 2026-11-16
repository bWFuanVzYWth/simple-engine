// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 稳定贡献计算，不决定 NEE、预算、介质拓扑或随机域。
/// 因子与 PDF 须有限非负，分母须为正；调用方保证所需最终结果可表示。
/// 普通数值范围直接乘除，极端范围以 frexp/ldexp 拆分；不修复或裁剪输入。
module transport;

namespace pt
{
// 非零因子位于 2^-40..2^40 时，三个因子的乘除仍在 float32 正规数域。
// 87=127-40；指数区间以无符号位模式检查，零因子也安全。其余输入保留指数拆分。
func common_factor(value: float)->bool
{
    let bits = asuint(value);
    return (bits == 0) | (bits - (87u << 23u) <= (80u << 23u));
}
func common_factor(value: float3)->bool
{
    let bits = asuint(value);
    return all((bits == 0) | (bits - (87u << 23u) <= (80u << 23u)));
}

/// 计算 first*second/denominator；极端范围拆分指数，避免可表示结果的中间乘积溢出。
public func product_over(first: float, second: float, denominator: float)->float
{
    if (common_factor(first) & common_factor(second) & common_factor(denominator))
        return (first * second) / denominator;
    var a : int;
    var b : int;
    var c : int;
    let significand : float = frexp(first, a) * frexp(second, b) / frexp(denominator, c);
    return ldexp(significand, a + b - c);
}

/// 按 RGB 分量计算相同的乘积／标量分母，语义同标量重载。
public func product_over(first: float3, second: float3, denominator: float)->float3
{
    if (common_factor(first) & common_factor(second) & common_factor(denominator))
        return (first * second) / denominator;
    var a : int3;
    var b : int3;
    var c : int;
    let significand : float3 = frexp(first, a) * frexp(second, b) / frexp(denominator, c);
    return ldexp(significand, a + b - int3(c));
}

/// 计算两个 RGB 因子与标量权重的乘积，保留最终的可表示结果。
public func triple_product(first: float3, second: float3, third: float)->float3
{
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
public func power_heuristic(first: float, second: float)->float
{
    if (first <= 0.0)
        return 0.0;
    if (second <= 0.0)
        return 1.0;
    if (first >= second)
    {
        let ratio = second / first;
        return 1.0 / (1.0 + ratio * ratio);
    }
    let ratio = first / second;
    return ratio * ratio / (1.0 + ratio * ratio);
}

/// 合并 sampled 策略的权重与 1/pdf，避免单独权重下溢抹掉可表示贡献。
/// 两种 PDF 必须同量度；delta 概率质量不可与立体角密度混用。
public func power_heuristic_over_pdf(sampled: float, other: float)->float
{
    if (sampled <= 0.0)
        return 0.0;
    if (other <= 0.0)
        return 1.0 / sampled;
    if (common_factor(sampled) & common_factor(other))
        return sampled / (sampled * sampled + other * other);
    if (sampled >= other)
    {
        let ratio = other / sampled;
        return (1.0 / sampled) / (1.0 + ratio * ratio);
    }
    let ratio = sampled / other;
    return (ratio / other) / (1.0 + ratio * ratio);
}

/// 面积密度乘 distance²/lightCosine 转成立体角密度；无支持域时返回0。
/// 输入使用同一长度单位，light_cosine 是发光侧单位法线与连接方向的正余弦。
public func area_solid_angle_pdf(area_pdf: float, distance_squared: float, light_cosine: float)
    ->float
{
    if (distance_squared <= 0.0 || light_cosine <= 0.0 || area_pdf <= 0.0)
        return 0.0;
    return product_over(area_pdf, distance_squared, light_cosine);
}

/// 均匀介质 Beer 透射；extinction 非负、distance 非负且单位互逆，不包含散射源。
public func segment_transmittance(extinction: float3, distance: float)->float3
{
    return exp(-extinction * distance);
}
}
