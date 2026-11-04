/// D65 线性 RGB 原色转换；不应用传递函数、曝光或 gamut clipping。
module primaries;
namespace color
{
/// 场景线性 BT.709/sRGB 转 Rec.2020，允许超出目标 gamut 的有符号值。
public float3 bt709_to_rec2020(float3 value)
{
    return float3(dot(float3(0.627403896, 0.329283038, 0.043313066), value),
                  dot(float3(0.069097289, 0.919540395, 0.011362316), value),
                  dot(float3(0.016391439, 0.088013308, 0.895595253), value));
}
/// 场景线性 Rec.2020 转 BT.709/sRGB；裁剪策略由消费者选择。
public float3 rec2020_to_bt709(float3 value)
{
    return float3(dot(float3(1.660491002, -0.587641139, -0.072849863), value),
                  dot(float3(-0.124550475, 1.132899897, -0.008349422), value),
                  dot(float3(-0.018150763, -0.100578898, 1.118729661), value));
}
}
