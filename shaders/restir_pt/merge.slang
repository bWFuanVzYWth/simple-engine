/// 复用合并只保留热字段与赢家索引；冷路径身份在RIS和颜色差计算结束后恢复一次。
module merge;
import restir_pt.storage;
import restir_pt.resampling;
import restir_pt.types;
import restir_pt.reservoir;

namespace restir_pt
{
public static const uint NO_REUSE_WINNER = 0xffffffffu;

/// 热结果与冷来源互相独立；source只在有效赢家时读取，canonical标识未shift的中心来源。
public struct MergeSelection
{
    public PathReservoir hot;
    public uint source;
    public bool canonical;
    public float reconnectionJacobian;
};

/// canonical/history仅要求M、UCW与integrand；两个方向的shift已由独立追踪完成。
/// 源历史M只在本轮竞争中封顶，不改变源UCW、随机身份或颜色所有权。
public MergeSelection temporal_selection(PathReservoir canonical, PathReservoir history,
                                         PrecomputedShiftData canonicalAtHistory,
                                         PrecomputedShiftData historyAtCanonical, uint offset,
                                         uint historyOffset, float2 u, float historyLength)
{
    history.M = min(historyLength, history.M);
    MergeSelection result = { empty_reservoir(), NO_REUSE_WINNER, false, 0.0 };
    float canonicalWeight = temporal_canonical_weight(canonical, history, canonicalAtHistory);
    float neighborWeight = temporal_neighbor_weight(canonical, history, historyAtCanonical);
    if (result.hot.mergeWithoutWriting(target(historyAtCanonical.pathThroughput) *
                                           historyAtCanonical.Jacobian,
                                       history, u.x, neighborWeight))
    {
        result.hot.integrand = historyAtCanonical.pathThroughput;
        result.source = historyOffset;
        result.reconnectionJacobian = historyAtCanonical.reconnectionJacobian;
    }
    if (result.hot.mergeWithoutWriting(target(canonical.integrand), canonical, u.y,
                                       canonicalWeight))
    {
        result.hot.integrand = canonical.integrand;
        result.source = offset;
        result.canonical = true;
    }
    finalize_reuse(result.hot);
    return result;
}

/// 按原slot顺序竞争；调用方提供有效邻居集合、原随机流及独立CV颜色累加。
public struct SpatialSelection
{
    public PathReservoir canonical;
    public PairwiseFactor factor;
    public float canonicalWeight;
    public MergeSelection selected;

    [mutating]
    public void init(PathReservoir center, float massSum)
    {
        canonical = center;
        factor = { massSum, center.M };
        canonicalWeight = spatial_begin(factor);
        selected = { empty_reservoir(), NO_REUSE_WINNER, false, 0.0 };
    }

    [mutating]
    public void add(PathReservoir neighbor, uint source, PrecomputedShiftData forward,
                    PrecomputedShiftData reverse, float u)
    {
        canonicalWeight += spatial_canonical_term(canonical, neighbor, reverse, factor);
        if (selected.hot.mergeWithoutWriting(target(forward.pathThroughput) * forward.Jacobian,
                                             neighbor, u,
                                             spatial_neighbor_weight(neighbor, forward, factor)))
        {
            selected.hot.integrand = forward.pathThroughput;
            selected.source = source;
            selected.reconnectionJacobian = forward.reconnectionJacobian;
        }
    }

    [mutating]
    public void finish(uint offset, float u)
    {
        if (selected.hot.mergeWithoutWriting(target(canonical.integrand), canonical, u,
                                             canonicalWeight))
        {
            selected.hot.integrand = canonical.integrand;
            selected.source = offset;
            selected.canonical = true;
        }
        finalize_reuse(selected.hot);
    }
};

/// 冷字段只来自获胜源；shift后的几何因子随赢家推进，不把邻居颜色复制进路径。
public PathReservoir restore_selection(PathReservoir cold, MergeSelection selected)
{
    if (selected.source == NO_REUSE_WINNER)
        return selected.hot;
    cold.M = selected.hot.M;
    cold.weight = selected.hot.weight;
    cold.integrand = selected.hot.integrand;
    if (!selected.canonical)
        cold.reconnectionJacobian = selected.reconnectionJacobian;
    return cold;
}

/// spatial所有来源都在本轮只读A；temporal由调用方显式选择A或历史B后再恢复。
public PathReservoir load_selection(RWByteAddressBuffer buffer, MergeSelection selected)
{
    if (selected.source == NO_REUSE_WINNER)
        return selected.hot;
    return restore_selection(load_reservoir(buffer, selected.source), selected);
}
}
