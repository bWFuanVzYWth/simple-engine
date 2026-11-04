/// 固定概率的输出估计选择；不改变路径或复用历史。
module fixed_mixture;
import restir_pt.rng;

namespace restir_pt
{
/// probability须为[0,1]有限固定值；端点不抽签，中间值只消费独立输出随机域。
/// F/R各自无偏且概率不依赖估计误差时，混合仍无偏；不要求F/R互相独立，不修复既有偏差。
public bool fixed_mixture_fresh(uint2 pixel, uint sample, float probability)
{
    if (!(probability > 0.0))
        return false;
    if (probability >= 1.0)
        return true;
    TinyUniformSampleGenerator sampler = rng_for_pixel(pixel, sample ^ 0xa511e9b3u);
    return sampler.next1D() < probability;
}
}
