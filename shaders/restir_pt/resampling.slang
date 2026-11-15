/***************************************************************************
 # Copyright (c) 2015-26, NVIDIA CORPORATION. All rights reserved.
 #
 # Redistribution and use in source and binary forms, with or without
 # modification, are permitted provided that the following conditions
 # are met:
 #  * Redistributions of source code must retain the above copyright
 #    notice, this list of conditions and the following disclaimer.
 #  * Redistributions in binary form must reproduce the above copyright
 #    notice, this list of conditions and the following disclaimer in the
 #    documentation and/or other materials provided with the distribution.
 #  * Neither the name of NVIDIA CORPORATION nor the names of its
 #    contributors may be used to endorse or promote products derived
 #    from this software without specific prior written permission.
 #
 # THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS "AS IS" AND ANY
 # EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 # IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
 # PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE COPYRIGHT OWNER OR
 # CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
 # EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
 # PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
 # PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY
 # OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 # (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
 # OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 **************************************************************************/

/// Falcor默认point的双向pairwise MIS；每个forward/reverse shift由scene独立重放与重连。
/// 不把逆向target近似为原target，不把双向MIS换成uniform邻居权重。
module resampling;
import restir_pt.types;
import restir_pt.reservoir;

namespace restir_pt
{
/// NV paired空间邻居的对称几何拒绝：shading normal点积至少0.5，
/// 相机距离差小于较远距离的10%。输入normal已归一，距离为同一相机的非负距离。
public func paired_geometry_valid(canonicalNormal: float3, canonicalDistance: float,
                                  neighborNormal: float3, neighborDistance: float)
    ->bool
{
    return dot(canonicalNormal, neighborNormal) >= 0.5f &&
           abs(canonicalDistance - neighborDistance) <
               0.1f * max(canonicalDistance, neighborDistance);
}

/// NV pairwise有效量：c_sum=canonical M+所有几何有效邻居M，c_c=canonical M。
/// 同一canonical的所有邻居共用此值；c_sum须正且c_sum>=c_c。
public struct PairwiseFactor
{
    public var c_sum : float;
    public var c_c : float;
};

/// forward shift写入的新几何因子随被选source一起保留，支持后续轮次链式shift。
public func shifted_source(source: PathReservoir, shift: PrecomputedShiftData)->PathReservoir
{
    var source = source;
    source.reconnectionJacobian = shift.reconnectionJacobian;
    return source;
}

/// temporal canonical项需要T_c->history的目标值与Jacobian；历史M已做history cap。
public func temporal_canonical_weight(canonical: PathReservoir, history: PathReservoir,
                                      canonicalAtHistory: PrecomputedShiftData)
    ->float
{
    let reversePHat : float =
                          target(canonicalAtHistory.pathThroughput) * canonicalAtHistory.Jacobian;
    return reversePHat > 0.f
               ? 1.f - reversePHat * history.M /
                           (reversePHat * history.M + canonical.M * target(canonical.integrand))
               : 1.f;
}

/// temporal邻居项使用T_history->c；完整源target*源M与目标target*canonical M竞争。
public func temporal_neighbor_weight(canonical: PathReservoir, history: PathReservoir,
                                     historyAtCanonical: PrecomputedShiftData)
    ->float
{
    let sourceTerm : float = target(history.integrand) * history.M;
    return sourceTerm / (sourceTerm + historyAtCanonical.Jacobian *
                                          target(historyAtCanonical.pathThroughput) * canonical.M);
}

/// 单次point temporal轮：先forward历史，再canonical；u.xy分别用于这两次RIS。
/// 两份shift对应同一对canonical/history路径，默认historyLength=20；无有效重投影时调用方保留canonical。
public func temporal_resample(canonical: PathReservoir, history: PathReservoir,
                              canonicalAtHistory: PrecomputedShiftData,
                              historyAtCanonical: PrecomputedShiftData, u: float2,
                              historyLength: float = 20.f)
    ->PathReservoir
{
    var history = history;
    history.M = min(historyLength, history.M);
    let canonicalWeight : float = temporal_canonical_weight(canonical, history, canonicalAtHistory);
    let neighborWeight : float = temporal_neighbor_weight(canonical, history, historyAtCanonical);
    var result : PathReservoir = empty_reservoir();
    merge_reservoir(result, shifted_source(history, historyAtCanonical),
                    historyAtCanonical.pathThroughput, historyAtCanonical.Jacobian, neighborWeight,
                    u.x);
    merge_reservoir(result, canonical, canonical.integrand, 1.f, canonicalWeight, u.y);
    finalize_reuse(result);
    return result;
}

/// 一个spatial轮的canonical起始权重；其余项由每个有效邻居的逆shift补入。
public func spatial_begin(factor: PairwiseFactor)->float
{
    return factor.c_c / factor.c_sum;
}

/// canonical路径在邻居域的逆shift产生的pairwise项；非可逆映射target=0时项为1。
public func spatial_canonical_term(canonical: PathReservoir, neighbor: PathReservoir,
                                   canonicalAtNeighbor: PrecomputedShiftData,
                                   factor: PairwiseFactor)
    ->float
{
    let reversePHat : float =
                          target(canonicalAtNeighbor.pathThroughput) * canonicalAtNeighbor.Jacobian;
    var term = 1.f;
    term -= reversePHat > 0.f ? reversePHat * (factor.c_sum - factor.c_c) /
                                    (reversePHat * (factor.c_sum - factor.c_c) +
                                     target(canonical.integrand) * factor.c_c)
                              : 0.f;
    return neighbor.M / factor.c_sum * term;
}

/// forward邻居MIS；与canonical逆项共同组成NV pairwise MIS，不含最终RIS归一化。
public func spatial_neighbor_weight(neighbor: PathReservoir,
                                    neighborAtCanonical: PrecomputedShiftData,
                                    factor: PairwiseFactor)
    ->float
{
    let sourceTerm : float = target(neighbor.integrand) * (factor.c_sum - factor.c_c);
    return neighbor.M / factor.c_sum * sourceTerm /
           (sourceTerm +
            neighborAtCanonical.Jacobian * target(neighborAtCanonical.pathThroughput) * factor.c_c);
}

/// 合并一个有效paired slot；reverse从neighbor同slot的precomputed缓存取得。
/// 只在mask允许的邻居上调用；几何拒绝、screen边缘和c_sum由workload pass统一形成。
public func spatial_merge_neighbor(inout result: PathReservoir, canonical: PathReservoir,
                                   neighbor: PathReservoir,
                                   neighborAtCanonical: PrecomputedShiftData,
                                   canonicalAtNeighbor: PrecomputedShiftData,
                                   factor: PairwiseFactor, inout canonicalWeight: float, u: float)
    ->bool
{
    canonicalWeight += spatial_canonical_term(canonical, neighbor, canonicalAtNeighbor, factor);
    return merge_reservoir(result, shifted_source(neighbor, neighborAtCanonical),
                           neighborAtCanonical.pathThroughput, neighborAtCanonical.Jacobian,
                           spatial_neighbor_weight(neighbor, neighborAtCanonical, factor), u);
}

/// 全部有效邻居后合并canonical并finalize；返回是否选中了canonical。
public func spatial_finish(inout result: PathReservoir, canonical: PathReservoir,
                           canonicalWeight: float, u: float)
    ->bool
{
    let selected
        : bool = merge_reservoir(result, canonical, canonical.integrand, 1.f, canonicalWeight, u);
    finalize_reuse(result);
    return selected;
}
}
