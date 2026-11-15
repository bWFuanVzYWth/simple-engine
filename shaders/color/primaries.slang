/// D65 线性 RGB 原色转换；不应用传递函数、曝光或 gamut clipping。
module primaries;
namespace color
{
/// 场景线性 BT.709/sRGB 转 Rec.2020，允许超出目标 gamut 的有符号值。
public func bt709_to_rec2020(value: float3)->float3
{
    return float3(fma(0.627403896, value.x, fma(0.329283038, value.y, 0.043313066 * value.z)),
                  fma(0.069097289, value.x, fma(0.919540395, value.y, 0.011362316 * value.z)),
                  fma(0.016391439, value.x, fma(0.088013308, value.y, 0.895595253 * value.z)));
}
/// 场景线性 Rec.2020 转 BT.709/sRGB；裁剪策略由消费者选择。
public func rec2020_to_bt709(value: float3)->float3
{
    return float3(fma(1.660491002, value.x, fma(-0.587641139, value.y, -0.072849863 * value.z)),
                  fma(-0.124550475, value.x, fma(1.132899897, value.y, -0.008349422 * value.z)),
                  fma(-0.018150763, value.x, fma(-0.100578898, value.y, 1.118729661 * value.z)));
}
}
