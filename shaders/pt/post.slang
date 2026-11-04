/// K3 的线性 HDR 累积与显示变换；大气合成、输出布局由宿主适配。
module post;

import display.prime_drt;

namespace pt
{
/// 一个绝对样本；负 error 为粘滞失败，失败及非有限样本不计数。
public void accumulate_sample(inout float4 sum, inout uint count, float3 radiance, float error)
{
    if (sum.w < 0)
        return;
    if (error < 0)
        sum.w = error;
    else if (!all(isfinite(radiance)))
        sum.w = -1;
    else
    {
        sum.xyz += radiance;
        count += 1u;
        if (!all(isfinite(sum.xyz)))
            sum.w = -1;
    }
}

/// 有效计数须大于零；线性 HDR 保留 BT.709 工作空间，显示模式使用 Prime DRT。
public float4 resolve_image(float4 sum, uint count, bool linear_hdr,
                            display::PrimeDrtParameters drt)
{
    if (sum.w < 0)
        return float4(0, 0, 0, sum.w);
    float3 average = sum.xyz / float(count);
    return float4(linear_hdr ? average : display::prime_drt_linear_bt709(average, drt), 1);
}
}
