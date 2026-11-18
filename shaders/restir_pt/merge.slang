/// 复用合并只保留热字段与赢家索引；冷路径身份在RIS和颜色差计算结束后恢复一次。
module merge;
import restir_pt.storage;
import restir_pt.resampling;
import restir_pt.types;
import restir_pt.reservoir;

namespace restir_pt {
public static const let NO_REUSE_WINNER = 0xffffffffu;

/// 热结果与冷来源互相独立；source只在有效赢家时读取，canonical标识未shift的中心来源。
public struct MergeSelection {
    public var hot : PathReservoir;
    public var source : uint;
    public var canonical : bool;
    public var reconnection_jacobian : float;
};

/// canonical/history仅要求M、UCW与integrand；两个方向的shift已由独立追踪完成。
/// 源历史M只在本轮竞争中封顶，不改变源UCW、随机身份或颜色所有权。
public func temporal_selection(canonical: PathReservoir, history: PathReservoir,
                               canonical_at_history: PrecomputedShiftData,
                               history_at_canonical: PrecomputedShiftData, offset: uint,
                               history_offset: uint, u: float2, history_length: float)
    ->MergeSelection {
    var history = history;
    history.sample_mass = min(history_length, history.sample_mass);
    var result : MergeSelection = { empty_reservoir(), NO_REUSE_WINNER, false, 0.0 };
    let canonical_weight = temporal_canonical_weight(canonical, history, canonical_at_history);
    let neighbor_weight = temporal_neighbor_weight(canonical, history, history_at_canonical);
    if (result.hot.merge_without_writing(target(history_at_canonical.path_throughput) *
                                             history_at_canonical.jacobian,
                                         history, u.x, neighbor_weight)) {
        result.hot.integrand = history_at_canonical.path_throughput;
        result.source = history_offset;
        result.reconnection_jacobian = history_at_canonical.reconnection_jacobian;
    }
    if (result.hot.merge_without_writing(target(canonical.integrand), canonical, u.y,
                                         canonical_weight)) {
        result.hot.integrand = canonical.integrand;
        result.source = offset;
        result.canonical = true;
    }
    finalize_reuse(result.hot);
    return result;
}

/// 按原slot顺序竞争；调用方提供有效邻居集合、原随机流及独立CV颜色累加。
public struct SpatialSelection {
    public var canonical : PathReservoir;
    public var factor : PairwiseFactor;
    public var canonical_weight : float;
    public var selected : MergeSelection;

    [mutating]
    public func init(center: PathReservoir, mass_sum: float)->void {
        canonical = center;
        factor = { mass_sum, center.sample_mass };
        canonical_weight = spatial_begin(factor);
        selected = { empty_reservoir(), NO_REUSE_WINNER, false, 0.0 };
    }

    [mutating]
    public func add(neighbor: PathReservoir, source: uint, forward: PrecomputedShiftData,
                    reverse: PrecomputedShiftData, u: float)
        ->void {
        canonical_weight += spatial_canonical_term(canonical, neighbor, reverse, factor);
        if (selected.hot.merge_without_writing(
                target(forward.path_throughput) * forward.jacobian, neighbor, u,
                spatial_neighbor_weight(neighbor, forward, factor))) {
            selected.hot.integrand = forward.path_throughput;
            selected.source = source;
            selected.reconnection_jacobian = forward.reconnection_jacobian;
        }
    }

    [mutating]
    public func finish(offset: uint, u: float)->void {
        if (selected.hot.merge_without_writing(target(canonical.integrand), canonical, u,
                                               canonical_weight)) {
            selected.hot.integrand = canonical.integrand;
            selected.source = offset;
            selected.canonical = true;
        }
        finalize_reuse(selected.hot);
    }
};

/// 冷字段只来自获胜源；shift后的几何因子随赢家推进，不把邻居颜色复制进路径。
public func restore_selection(cold: PathReservoir, selected: MergeSelection)->PathReservoir {
    var cold = cold;
    if (selected.source == NO_REUSE_WINNER)
        return selected.hot;
    cold.sample_mass = selected.hot.sample_mass;
    cold.weight = selected.hot.weight;
    cold.integrand = selected.hot.integrand;
    if (!selected.canonical)
        cold.reconnection_jacobian = selected.reconnection_jacobian;
    return cold;
}

/// spatial所有来源都在本轮只读A；temporal由调用方显式选择A或历史B后再恢复。
public func load_selection(buffer: RWByteAddressBuffer, selected: MergeSelection)->PathReservoir {
    if (selected.source == NO_REUSE_WINNER)
        return selected.hot;
    return restore_selection(load_reservoir(buffer, selected.source), selected);
}
}
