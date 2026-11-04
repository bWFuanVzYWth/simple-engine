// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 稳定贡献计算，不决定 NEE、预算、介质拓扑或随机域。
/// 因子与 PDF 须有限非负，分母须为正；调用方保证所需最终结果可表示。
/// 使用 frexp/ldexp 运算，不添加任意输入修复、阈值或饱和裁剪。
module transport;

namespace pt
{
/// 通过指数拆分计算 first*second/denominator，避免可表示结果的中间乘积溢出。
public float product_over(float first, float second, float denominator)
{
    int a, b, c;
    float significand = frexp(first, a) * frexp(second, b) / frexp(denominator, c);
    return ldexp(significand, a + b - c);
}

/// 按 RGB 分量计算相同的乘积／标量分母，语义同标量重载。
public float3 product_over(float3 first, float3 second, float denominator)
{
    int3 a, b;
    int c;
    float3 significand = frexp(first, a) * frexp(second, b) / frexp(denominator, c);
    return ldexp(significand, a + b - int3(c));
}

/// 通过指数拆分计算两个 RGB 因子与标量权重的乘积，保留最终的可表示结果。
public float3 triple_product(float3 first, float3 second, float third)
{
    int3 a, b;
    int c;
    float3 significand = frexp(first, a) * frexp(second, b) * frexp(third, c);
    return ldexp(significand, a + b + int3(c));
}

/// 同量度的两策略 power heuristic；first<=0 为0，second<=0 为1。
/// 非零路径按较大 PDF 归一化，避免大比值平方溢出。
public float power_heuristic(float first, float second)
{
    if (first <= 0.0)
        return 0.0;
    if (second <= 0.0)
        return 1.0;
    if (first >= second)
    {
        float ratio = second / first;
        return 1.0 / (1.0 + ratio * ratio);
    }
    float ratio = first / second;
    return ratio * ratio / (1.0 + ratio * ratio);
}

/// 合并 sampled 策略的权重与 1/pdf，避免单独权重下溢抹掉可表示贡献。
/// 两种 PDF 必须同量度；delta 概率质量不可与立体角密度混用。
public float power_heuristic_over_pdf(float sampled, float other)
{
    if (sampled <= 0.0)
        return 0.0;
    if (other <= 0.0)
        return 1.0 / sampled;
    if (sampled >= other)
    {
        float ratio = other / sampled;
        return (1.0 / sampled) / (1.0 + ratio * ratio);
    }
    float ratio = sampled / other;
    return (ratio / other) / (1.0 + ratio * ratio);
}

/// 面积密度乘 distance²/lightCosine 转成立体角密度；无支持域时返回0。
/// 输入使用同一长度单位，light_cosine 是发光侧单位法线与连接方向的正余弦。
public float area_solid_angle_pdf(float area_pdf, float distance_squared, float light_cosine)
{
    if (distance_squared <= 0.0 || light_cosine <= 0.0 || area_pdf <= 0.0)
        return 0.0;
    return product_over(area_pdf, distance_squared, light_cosine);
}

/// 均匀介质 Beer 透射；extinction 非负、distance 非负且单位互逆，不包含散射源。
public float3 segment_transmittance(float3 extinction, float distance)
{
    return exp(-extinction * distance);
}
}
