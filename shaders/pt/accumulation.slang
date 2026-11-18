/// 按绝对样本累计场景线性 HDR；合成及显示由输出适配器负责。
module accumulation;

namespace pt {
/// 一个绝对样本；负 error 为粘滞失败，失败及非有限样本不计数。
public func accumulate_sample(inout sum: float4, inout count: uint, radiance: float3, error: float)
    ->void {
    if (sum.w < 0)
        return;
    if (error < 0)
        sum.w = error;
    else if (!all(isfinite(radiance)))
        sum.w = -1;
    else {
        sum.xyz += radiance;
        count += 1u;
        if (!all(isfinite(sum.xyz)))
            sum.w = -1;
    }
}

/// 有效计数须大于零；线性HDR保留BT.709工作空间，不引用显示变换参数。
public func resolve_linear_hdr(sum: float4, count: uint)->float4 {
    if (sum.w < 0)
        return float4(0, 0, 0, sum.w);
    return float4(sum.xyz / float(count), 1);
}
}
