/// ReSTCV 的像素颜色估计；不改变 Enhanced 的路径选样、Hybrid 或 paired 邻居调度。
/// 参考 Shi 等 2026，Eq.6–11：https://hercier.github.io/restcv/
/// 颜色属于像素而非赢家路径，使用独立工作集；调用方维持同一帧场景和 shift 的对应关系。
module control_variate;

import restir_pt.types;
import restir_pt.reservoir;

namespace restir_pt
{
/// 基于确定性反射率代理的逐通道系数，合法范围为[0,2]。
/// 零分母或非有限比例回退1；该系数不读取随机路径辐射，代理无需等于物理反射率。
public float3 cv_alpha(float3 canonicalReflectance, float3 neighborReflectance)
{
    float3 ratio = canonicalReflectance / neighborReflectance;
    return float3(isfinite(ratio.x) ? clamp(ratio.x, 0.f, 2.f) : 1.f,
                  isfinite(ratio.y) ? clamp(ratio.y, 0.f, 2.f) : 1.f,
                  isfinite(ratio.z) ? clamp(ratio.z, 0.f, 2.f) : 1.f);
}

bool cv_mass_valid(float mass)
{
    return isfinite(mass) && mass > 0.f;
}

float3 cv_mapped(PrecomputedShiftData shifted)
{
    return isfinite(shifted.Jacobian) && shifted.Jacobian > 0.f
               ? shifted.pathThroughput * shifted.Jacobian
               : float3(0.f);
}

// 两个来源的MIS仅服务于像素差，按源measure形成partition of unity；不是空间选样的c_sum权重。
// 先缩放PDF和M，避免HDR*历史量及其相加溢出；正常支持域不引入log/exp。
float cv_pair_weight(PathReservoir source, float otherMass, float3 mapped)
{
    float p = target(source.integrand);
    if (!cv_mass_valid(source.M) || !isfinite(source.weight) || source.weight <= 0.f ||
        !isfinite(p) || p <= 0.f)
        return 0.f;
    float q = target(mapped);
    if (!cv_mass_valid(otherMass) || !(q > 0.f) || !isfinite(q))
        return 1.f;
    float pdfScale = max(p, q);
    float massScale = max(source.M, otherMass);
    float first = (p / pdfScale) * (source.M / massScale);
    float second = (q / pdfScale) * (otherMass / massScale);
    return first / (first + second);
}

/// 估计 I_c-alpha*I_n；两个RGB均为各自源measure，shift乘自身Jacobian后才相减。
/// 两个差值MIS使用Mc/Mn与目标密度；与逆映射处互补，故可独立于Enhanced空间选样MIS。
/// 不可逆shift用Jacobian<=0表示，该方向贡献为0，源独占域采用权重1；不借用反向Jacobian。
/// canonical/neighbor须各有正有限M；允许合法零贡献路径，缺失来源不是零贡献样本。
/// 输入RGB、UCW及有效shift乘积须有限；alpha须为确定性有限系数，不取自随机路径辐射。
/// 固定场景、正确UCW及可逆映射支持下保持积分；负RGB是合法校正，不能在此clamp。
public float3 cv_difference(PathReservoir canonical, PathReservoir neighbor,
                            PrecomputedShiftData neighborAtCanonical,
                            PrecomputedShiftData canonicalAtNeighbor, float3 alpha)
{
    if (!cv_mass_valid(canonical.M) || !cv_mass_valid(neighbor.M))
        return 0.f;
    float3 forward = cv_mapped(neighborAtCanonical);
    float3 reverse = cv_mapped(canonicalAtNeighbor);
    float a = cv_pair_weight(canonical, neighbor.M, reverse);
    float b = cv_pair_weight(neighbor, canonical.M, forward);
    return a * canonical.weight * (canonical.integrand - alpha * reverse) +
           b * neighbor.weight * (forward - alpha * neighbor.integrand);
}

/// 用邻居累计颜色和双向差值形成当前像素估计；不以重采样赢家选择该颜色。
/// 邻居颜色估计须对应邻居像素积分，两个source保持自身UCW；颜色不绑定选中的路径。
/// 不满足正M时返回0，调用方不应把该项加入组合。
public float3 cv_from_neighbor(float3 neighborColor, PathReservoir canonical,
                               PathReservoir neighbor, PrecomputedShiftData neighborAtCanonical,
                               PrecomputedShiftData canonicalAtNeighbor, float3 alpha)
{
    if (!cv_mass_valid(canonical.M) || !cv_mass_valid(neighbor.M))
        return 0.f;
    return alpha * neighborColor +
           cv_difference(canonical, neighbor, neighborAtCanonical, canonicalAtNeighbor, alpha);
}

/// 单轮颜色组合；整个轮次共享同一canonical，不逐邻居改变其路径或颜色输入。
/// mass为实际参与的置信度和，不代表独立样本数；不与reservoir RIS权重和混用。
/// 参与集合、M及alpha须独立于随机辐射和shift成败；几何拒绝可由固定primary决定。
public struct CVAccumulator
{
    /// 累计RGB分子；缺失canonical时直接保存回退颜色。
    public float3 weighted;
    /// 正canonical与已添加项的M之和；零表示禁用邻居的回退状态。
    public float mass;

    /// 初始化中心颜色和正有限M；缺失中心保持原颜色，并禁用后续邻居项。
    [mutating]
    public void init(float3 canonicalColor, float canonicalMass)
    {
        mass = cv_mass_valid(canonicalMass) ? canonicalMass : 0.f;
        weighted = mass > 0.f ? canonicalColor * mass : canonicalColor;
    }

    /// 添加一个完整的from-neighbor估计；无效/零M项不参与分母，合法负RGB保留。
    [mutating]
    public void add(float3 estimate, float neighborMass)
    {
        if (mass > 0.f && cv_mass_valid(neighborMass))
        {
            weighted += neighborMass * estimate;
            mass += neighborMass;
        }
    }

    /// 输出按置信度组合的颜色；缺失中心直接返回初始化颜色，不制造邻域支持。
    public float3 resolve()
    {
        return mass > 0.f ? weighted / mass : weighted;
    }
}

/// temporal颜色更新；historyLength与路径temporal轮相同，cap仅作用于M，不修改源UCW。
/// 调用方须先确认颜色历史有效；路径历史有效本身不意味着颜色历史可读。
/// 无有效历史或canonical时保留初采颜色；所有颜色输入包含合法零值和有限负值。
public float3 cv_temporal(float3 canonicalColor, float3 historyColor, PathReservoir canonical,
                          PathReservoir history, PrecomputedShiftData canonicalAtHistory,
                          PrecomputedShiftData historyAtCanonical, float3 alpha,
                          float historyLength = 20.f)
{
    if (!cv_mass_valid(canonical.M) || !cv_mass_valid(history.M) || !cv_mass_valid(historyLength))
        return canonicalColor;
    history.M = min(history.M, historyLength);
    CVAccumulator result;
    result.init(canonicalColor, canonical.M);
    result.add(cv_from_neighbor(historyColor, canonical, history, historyAtCanonical,
                                canonicalAtHistory, alpha),
               history.M);
    return result.resolve();
}
}
