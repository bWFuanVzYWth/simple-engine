/// 无资源绑定的多重重要性采样权重，不决定积分器的策略或样本数。
module mis;
import pt.transport;

namespace pt
{
/// 两种策略各一个样本的 power heuristic，beta=2，返回 first 策略的权重。
/// 两个 PDF 须使用相同量度且有限，first > 0、second >= 0；不做边界校验。
/// 来源：https://pbr-book.org/4ed/Monte_Carlo_Integration/Improving_Efficiency
public float power_weight(float first, float second)
{
    return power_heuristic(first, second);
}
}
