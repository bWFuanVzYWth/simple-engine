/// 固定概率的输出估计选择；不改变路径或复用历史。
module fixed_mixture;
import restir_pt.rng;

namespace restir_pt
{
/// probability须为[0,1]有限固定值；端点不抽签，中间值只消费独立输出随机域。
/// F/R各自无偏且概率不依赖估计误差时，混合仍无偏；不要求F/R互相独立，不修复既有偏差。
public func fixed_mixture_fresh(pixel: uint2, sample_value: uint, probability: float)->bool
{
    if (!(probability > 0.0))
        return false;
    if (probability >= 1.0)
        return true;
    var sampler : TinyUniformSampleGenerator = rng_for_pixel(pixel, sample_value ^ 0xa511e9b3u);
    return sampler.next_1d() < probability;
}
}
