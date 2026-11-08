/// 首表面重建启发式；只改变提案概率，不改变物理响应。单一有效通道退化为概率1。
module primary_policy;

namespace openpbr_primary
{
public float reflection_probability(float reflection, float interior)
{
    return reflection > 0 ? (interior > 0 ? 0.5 : 1.0) : 0.0;
}

/// 2 spp 对选择维度分层，其他随机维度和后续反弹仍独立。调用方只传1或2。
public float selection(float random, uint samples, uint lane)
{
    return samples == 2 ? (float(lane) + random) * 0.5 : random;
}
}
