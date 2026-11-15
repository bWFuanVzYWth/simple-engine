/// 首表面重建启发式；只改变提案概率，不改变物理响应。单一有效通道退化为概率1。
module primary_policy;

namespace openpbr_primary
{
public func reflection_probability(reflection: float, interior: float)->float
{
    return reflection > 0 ? (interior > 0 ? 0.5 : 1.0) : 0.0;
}

/// 对选择维度分层，其他随机维度和后续反弹仍独立。
/// 调用方保证 samples 为 1 或 2、lane<samples，random 是 [0,1) 的非负随机数。
public func selection(random: float, samples: uint, lane: uint)->float
{
    return (float(lane) + random) / float(samples);
}
}
