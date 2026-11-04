//! 稀疏 HKDW：反向 BFS 后先找最短增广路，再用共享访问标记追加不限层搜索。
//! 依据 Duff–Kaya–Uçar §3.3.2：<https://doi.org/10.1145/2049673.2049677>。
//! 终止由匹配数严格增加保证；追加搜索后不假定阶段最短距离单调。

use crate::optimal::fixed::FixedVec;
use crate::optimal::greedy::{
    greedy_augment_len3_sparse_csr_u16, greedy_initialize_sparse_matching_csr_u16,
};
use crate::optimal::matching::{
    ChordBuffer, DfsFrame, HkScratch, IMAGE64_MAX_CONFLICT_EDGES, SparseAdjacencyRef, UNMATCHED_U16,
};

use crate::optimal::{copy, get_mut, u16_index};

#[cfg(verus_only)]
use crate::optimal::matching::proof;
#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "../proof/bfs.rs"]
pub(super) mod levels_proof;

#[cfg(verus_only)]
#[path = "../proof/dfs.rs"]
pub(super) mod path_proof;

#[cfg(verus_only)]
#[path = "../proof/phase.rs"]
pub(super) mod phase_proof;

#[cfg(verus_only)]
#[path = "../proof/cover.rs"]
pub(super) mod cover_proof;

#[cfg(verus_only)]
#[path = "../proof/canonical.rs"]
pub(super) mod choice_proof;

#[cfg(verus_only)]
#[path = "../proof/blocking.rs"]
pub(super) mod block_proof;

/// 验证目标：对有效 CSR（每侧至多 1953 点、3969 边、右度数至多 63），
/// 要求右顶点域为 `0..right_size`，`right_degrees.len() = right_size` 且逐项等于实际右度数；
/// 初始化构造的转置满足 `r ∈ neighbors(l) ⇔ l ∈ transpose_neighbors(r)`。
/// pair 数组长度等于对应顶点数；`pair_left[l] = r` 当且仅当 `pair_right[r] = l`，
/// 每对都是输入边，未匹配项为 `UNMATCHED_U16`。初始化及增广均保持此不变量。
/// 结束时由最终标记生成的独立集补集是与匹配等大的顶点覆盖，给出最优性证书。
/// 每次成功 BFS 后至少增广一次，匹配数严格增加且至多为两侧较小顶点数；
/// 单轮 DFS 每左点至多入栈一次，queue 每右点至多一次，允许重复的 shortest_roots 至多 |E|。
/// 这些进展与容量结论须在生产循环上证明，不能只验证抽象 HK 算法。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::buffers_valid(old(scratch)), right_size == right_degrees@.len(),
        proof::adjacency_valid(adjacency, right_size as nat), proof::degrees_exact(adjacency.edges@, right_degrees@),
    ensures phase_proof::completed(adjacency, final(scratch)), proof::buffers_valid(final(scratch)), final(scratch).pair_right@.len() == right_size,
        proof::marks_certificate(adjacency.offsets@, adjacency.edges@, final(scratch).pair_left@, final(scratch).pair_right@,
            final(scratch).reachable_left@, final(scratch).reachable_right@),
        crate::chord_graph::proof::marks_match(final(scratch).pair_left@, final(scratch).right_distance@,
            final(scratch).unmatched_lefts@.len() == 0, final(scratch).reachable_left@, final(scratch).reachable_right@),
))]
pub(super) fn hopcroft_karp_dw_sparse_csr_u16(
    adjacency: &SparseAdjacencyRef<'_>,
    right_size: usize,
    right_degrees: &[u8],
    scratch: &mut HkScratch,
) {
    initialize_matching(adjacency, right_size, right_degrees, scratch);
    initialize_search(scratch);
    augment_until_complete(adjacency, scratch);
    collect_independent_set_marks(adjacency, scratch);
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires phase_proof::ready(adjacency, old(scratch)),
    ensures phase_proof::completed(adjacency, final(scratch)),
        final(scratch).pair_left@.len() == old(scratch).pair_left@.len(),
        final(scratch).pair_right@.len() == old(scratch).pair_right@.len(),
))]
fn augment_until_complete(adjacency: &SparseAdjacencyRef<'_>, scratch: &mut HkScratch) {
    proof! { reveal(phase_proof::completed); }
    proof! { phase_proof::count_bound(scratch.pair_left@, scratch.pair_left@.len() as int); }
    #[cfg_attr(verus_only, verus_spec(
        invariant phase_proof::ready(adjacency, scratch),
            scratch.pair_left@.len() == old(scratch).pair_left@.len(), scratch.pair_right@.len() == old(scratch).pair_right@.len(),
            proof::matched_count(scratch.pair_left@, scratch.pair_left@.len() as int) <= scratch.pair_left@.len(),
        decreases scratch.pair_left@.len() - proof::matched_count(scratch.pair_left@, scratch.pair_left@.len() as int),
    ))]
    while !scratch.unmatched_lefts.is_empty() {
        let Some(shortest_depth) = build_reverse_levels(scratch) else {
            proof! { reveal(phase_proof::completed); }
            return;
        };
        proof! { phase_proof::levels_certificate(adjacency, scratch, shortest_depth); }
        augment_phase(adjacency, scratch, shortest_depth);
        proof! {
            proof::reverse_from_transpose(adjacency.offsets@, adjacency.edges@, scratch.pair_left@, scratch.pair_right@,
                scratch.transpose_offsets@, scratch.transpose_edges@);
            phase_proof::count_bound(scratch.pair_left@, scratch.pair_left@.len() as int);
        }
    }
    proof! { reveal(phase_proof::completed); }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::buffers_valid(old(scratch)), right_size == right_degrees@.len(),
        proof::adjacency_valid(adjacency, right_size as nat), proof::degrees_exact(adjacency.edges@, right_degrees@),
    ensures proof::buffers_valid(final(scratch)),
        proof::matching_valid(adjacency.offsets@, adjacency.edges@, final(scratch).pair_left@, final(scratch).pair_right@),
        final(scratch).pair_right@.len() == right_size,
        proof::transposed(adjacency.offsets@, adjacency.edges@,
            final(scratch).transpose_offsets@, final(scratch).transpose_edges@, right_size as nat),
        levels_proof::reverse_arrays(final(scratch).transpose_offsets@, final(scratch).transpose_edges@,
            final(scratch).pair_left@, final(scratch).pair_right@),
        final(scratch).right_distance == old(scratch).right_distance,
        final(scratch).queue == old(scratch).queue,
        final(scratch).unmatched_lefts == old(scratch).unmatched_lefts,
        final(scratch).shortest_roots == old(scratch).shortest_roots,
        final(scratch).reachable_left == old(scratch).reachable_left,
        final(scratch).reachable_right == old(scratch).reachable_right,
        final(scratch).dfs_stack == old(scratch).dfs_stack,
))]
fn initialize_matching(
    adjacency: &SparseAdjacencyRef<'_>,
    right_size: usize,
    right_degrees: &[u8],
    scratch: &mut HkScratch,
) {
    let left_size = adjacency.offsets.len().saturating_sub(1);
    scratch.pair_left.clear();
    scratch.pair_left.resize(left_size, UNMATCHED_U16);
    scratch.pair_right.clear();
    scratch.pair_right.resize(right_size, UNMATCHED_U16);
    // 初始化同时构造转置 CSR，反向 BFS 直接复用它。
    greedy_initialize_sparse_matching_csr_u16(adjacency, right_degrees, scratch);
    {
        let pair_left = scratch.pair_left.as_mut_slice();
        let pair_right = scratch.pair_right.as_mut_slice();
        greedy_augment_len3_sparse_csr_u16(
            adjacency,
            pair_left,
            pair_right,
            &mut scratch.write_offsets,
        );
    }
    proof! { proof::reverse_from_transpose(adjacency.offsets@, adjacency.edges@, scratch.pair_left@, scratch.pair_right@,
    scratch.transpose_offsets@, scratch.transpose_edges@); }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::buffers_valid(old(scratch)), old(scratch).pair_left@.len() <= 1953,
        old(scratch).pair_right@.len() <= 1953,
    ensures proof::buffers_valid(final(scratch)), proof::free_lefts(final(scratch).pair_left@, final(scratch).unmatched_lefts@),
        final(scratch).right_distance@ == Seq::new(old(scratch).pair_right@.len(), |_: int| u16::MAX),
        final(scratch).reachable_left@ == Seq::new(old(scratch).pair_left@.len(), |_: int| false),
        final(scratch).reachable_right@ == Seq::new(old(scratch).pair_right@.len(), |_: int| false),
        final(scratch).pair_left == old(scratch).pair_left, final(scratch).pair_right == old(scratch).pair_right,
        final(scratch).queue == old(scratch).queue, final(scratch).shortest_roots == old(scratch).shortest_roots,
        final(scratch).transpose_offsets == old(scratch).transpose_offsets,
        final(scratch).transpose_edges == old(scratch).transpose_edges,
        final(scratch).write_offsets == old(scratch).write_offsets,
        final(scratch).right_order == old(scratch).right_order,
        final(scratch).right_degree_counts == old(scratch).right_degree_counts,
        final(scratch).dfs_stack == old(scratch).dfs_stack,
))]
fn initialize_search(scratch: &mut HkScratch) {
    crate::optimal::search::initialize(
        scratch.pair_left.as_slice(),
        scratch.pair_right.len(),
        &mut scratch.right_distance,
        &mut scratch.reachable_left,
        &mut scratch.reachable_right,
        &mut scratch.unmatched_lefts,
    );
    proof! {
        assert forall|i: int| 0 <= i < scratch.unmatched_lefts@.len() implies
            (#[trigger] scratch.unmatched_lefts@[i]) < scratch.pair_left@.len()
                && scratch.pair_left@[scratch.unmatched_lefts@[i] as int] == u16::MAX by {
            assert(scratch.unmatched_lefts@.contains(scratch.unmatched_lefts@[i]));
        }
        assert forall|left: int| 0 <= left < scratch.pair_left@.len() implies
            (scratch.pair_left@[left] == u16::MAX <==> #[trigger] scratch.unmatched_lefts@.contains(left as u16)) by {
            assert((left as u16) as int == left);
        }
    }
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    ensures final(values)@ == Seq::new(old(values)@.len(), |_: int| value),
))]
fn fill_search<T: Copy>(values: &mut [T], value: T) {
    let mut next = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant next <= values@.len(), values@.len() == old(values)@.len(),
            forall|i: int| 0 <= i < values@.len() ==> values@[i]
                == if i < next { value } else { old(values)@[i] },
        decreases values@.len() - next,
    ))]
    while next < values.len() {
        *get_mut(values, next) = value;
        next += 1;
    }
}

/// 只读匹配上的反向 BFS；队列和允许重复的根分别由顶点与边预算约束。
/// `Some` 的最短层、完整最短根和 `None` 的可达闭包由逐边闭包与路径见证连接。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires levels_proof::reverse_domain(transpose, pair_left@, pair_right@),
        old(distance)@.len() == pair_right@.len(), old(left_seen)@.len() == pair_left@.len(),
        old(queue).valid(), old(roots).valid(),
    ensures final(queue).valid(), final(roots).valid(), levels_proof::queued(final(distance)@, final(queue)@),
        final(distance)@.len() == pair_right@.len(),
        final(roots)@.len() <= transpose.edges@.len(),
        levels_proof::roots_free(final(roots)@, pair_left@),
        levels_proof::distance_witness(transpose.offsets@, transpose.edges@, pair_left@, pair_right@, final(distance)@),
        levels_proof::roots_sound(transpose.offsets@, transpose.edges@, pair_left@, final(distance)@, final(roots)@,
            match result { Some(depth) => depth, None => u16::MAX }),
        levels_proof::global_closed(transpose.offsets@, transpose.edges@, pair_left@, final(distance)@, final(roots)@,
            match result { Some(depth) => depth, None => u16::MAX }),
        forall|right: int| 0 <= right < final(distance)@.len() && final(distance)@[right] != u16::MAX
            ==> final(distance)@[right] < pair_right@.len(),
        match result { Some(depth) => forall|right: int| 0 <= right < final(distance)@.len() && final(distance)@[right] != u16::MAX
            ==> final(distance)@[right] <= depth as int + 1, None => true },
        final(left_seen)@ == Seq::new(pair_left@.len(), |_: int| false),
        forall|right: int| 0 <= right < pair_right@.len() && pair_right@[right] == u16::MAX ==> final(distance)@[right] == 0,
        match result { Some(depth) => depth < pair_right@.len() && final(roots)@.len() > 0, None => final(roots)@.len() == 0 },
))]
fn reverse_levels(
    transpose: &SparseAdjacencyRef<'_>,
    pair_left: &[u16],
    pair_right: &[u16],
    distance: &mut [u16],
    left_seen: &mut [bool],
    queue: &mut ChordBuffer<u16>,
    roots: &mut FixedVec<u16, IMAGE64_MAX_CONFLICT_EDGES>,
) -> Option<u16> {
    fill_search(distance, UNMATCHED_U16);
    fill_search(left_seen, false);
    queue.clear();
    roots.clear();
    let mut right = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant levels_proof::reverse_domain(transpose, pair_left@, pair_right@),
            queue.valid(), roots.valid(), roots@.len() == 0, queue@.len() <= right,
            levels_proof::queued(distance@, queue@), levels_proof::seed_distance(distance@, pair_right@, right as int),
        decreases pair_right@.len() - right,
    ))]
    while right < pair_right.len() {
        if copy(pair_right, right) == UNMATCHED_U16 {
            proof! { levels_proof::queue_pushed(distance@, queue@, right as u16, 0); }
            *get_mut(distance, right) = 0;
            queue.push(u16_index(right));
        }
        right += 1;
    }
    proof! {
        levels_proof::seed_layered(distance@, queue@, pair_right@);
        levels_proof::seed_witness(transpose.offsets@, transpose.edges@, pair_left@, pair_right@, distance@);
        levels_proof::roots_start(transpose.offsets@, transpose.edges@, pair_left@, distance@, roots@);
    }
    let mut head = 0;
    let mut shortest = UNMATCHED_U16;
    #[cfg_attr(verus_only, verus_spec(
        invariant levels_proof::reverse_domain(transpose, pair_left@, pair_right@),
            distance@.len() == pair_right@.len(), queue.valid(), roots.valid(),
            levels_proof::queued(distance@, queue@), head <= queue@.len(),
            levels_proof::layered(distance@, queue@, head as int),
            levels_proof::distance_witness(transpose.offsets@, transpose.edges@, pair_left@, pair_right@, distance@),
            levels_proof::roots_sound(transpose.offsets@, transpose.edges@, pair_left@, distance@, roots@, shortest),
            levels_proof::scanned_rows(transpose.offsets@, transpose.edges@, pair_left@, distance@, queue@, roots@, shortest, head as int),
            levels_proof::roots_free(roots@, pair_left@),
            roots@.len() <= levels_proof::selected_sum(levels_proof::row_weights(transpose.offsets@), queue@, head as int),
            shortest == u16::MAX <==> roots@.len() == 0,
            shortest == u16::MAX || shortest < pair_right@.len(),
            shortest == u16::MAX || head > 0 && shortest <= distance@[queue@[head as int - 1] as int],
            shortest != u16::MAX ==> levels_proof::queue_ceiling(distance@, queue@, shortest as int + 1),
            forall|r: int| 0 <= r < pair_right@.len() && pair_right@[r] == u16::MAX ==> distance@[r] == 0,
        decreases pair_right@.len() - head,
    ))]
    while head < queue.len() {
        proof! { levels_proof::current_layer(distance@, queue@, head as int); }
        let right = usize::from(copy(queue, head));
        head += 1;
        let depth = copy(distance, right);
        if depth > shortest {
            proof! {
                levels_proof::queue_budget(transpose.offsets@, transpose.edges@.len(), queue@, head as int - 1);
                levels_proof::finish_layer(distance@, queue@, head as int - 1);
                levels_proof::scanned_skipped(transpose, pair_left@, pair_right@, distance@, queue@, roots@, shortest, head as int);
            }
            continue;
        }
        let start = usize::from(copy(transpose.offsets, right));
        let end = usize::from(copy(transpose.offsets, right + 1));
        let mut edge = start;
        #[cfg_attr(verus_only, verus_spec(
            invariant levels_proof::reverse_domain(transpose, pair_left@, pair_right@),
                distance@.len() == pair_right@.len(), queue.valid(), roots.valid(),
                levels_proof::queued(distance@, queue@), 0 < head <= queue@.len(),
                levels_proof::sorted_queue(distance@, queue@), levels_proof::queue_ceiling(distance@, queue@, depth as int + 1),
                levels_proof::distance_witness(transpose.offsets@, transpose.edges@, pair_left@, pair_right@, distance@),
                levels_proof::roots_sound(transpose.offsets@, transpose.edges@, pair_left@, distance@, roots@, shortest),
                levels_proof::scanned_prefix(transpose.offsets@, transpose.edges@, pair_left@, distance@, queue@, roots@, shortest, head as int, edge as int),
                right == queue@[head as int - 1], depth == distance@[right as int], depth <= head - 1,
                start == transpose.offsets@[right as int], end == transpose.offsets@[right as int + 1], start <= edge <= end,
                levels_proof::roots_free(roots@, pair_left@),
                roots@.len() <= levels_proof::selected_sum(levels_proof::row_weights(transpose.offsets@), queue@, head as int - 1) + edge - start,
                shortest == u16::MAX <==> roots@.len() == 0,
                shortest == u16::MAX || shortest < pair_right@.len(),
                shortest == u16::MAX || shortest == depth,
                forall|r: int| 0 <= r < pair_right@.len() && pair_right@[r] == u16::MAX ==> distance@[r] == 0,
            decreases end - edge,
        ))]
        while edge < end {
            let left = copy(transpose.edges, edge);
            let matched = copy(pair_left, usize::from(left));
            proof! {
                if matched != UNMATCHED_U16 && !(depth < shortest && distance@[matched as int] == UNMATCHED_U16) {
                    if depth < shortest { levels_proof::queued_ceiling(distance@, queue@, depth as int + 1, matched as int); }
                    levels_proof::scanned_step(transpose.offsets@, transpose.edges@, pair_left@, distance@, distance@,
                        queue@, queue@, roots@, roots@, shortest, shortest, head as int, edge as int);
                }
            }
            if matched == UNMATCHED_U16 {
                proof! {
                    levels_proof::roots_room(transpose.offsets@, transpose.edges@.len(), queue@, head as int, edge as int, roots@.len() as int);
                    levels_proof::roots_pushed(transpose.offsets@, transpose.edges@, pair_left@, distance@, roots@, shortest, right as int, edge as int);
                    levels_proof::monotone_root(distance@, queue@, roots@, shortest, depth, head as int, left);
                    levels_proof::free_slot(transpose.edges@, pair_left@, distance@, roots@, right as int, edge as int);
                    levels_proof::scanned_step(transpose.offsets@, transpose.edges@, pair_left@, distance@, distance@,
                        queue@, queue@, roots@, roots@.push(left), shortest, depth, head as int, edge as int);
                }
                shortest = depth;
                roots.push(left);
            } else if depth < shortest && copy(distance, usize::from(matched)) == UNMATCHED_U16 {
                let next_depth = depth + 1;
                proof! {
                    levels_proof::queue_pushed(distance@, queue@, matched, next_depth);
                    levels_proof::pushed_layers(distance@, queue@, matched, next_depth);
                    levels_proof::selected_pushed(levels_proof::row_weights(transpose.offsets@), queue@, head as int - 1, matched);
                    levels_proof::witness_pushed(transpose.offsets@, transpose.edges@, pair_left@, pair_right@,
                        distance@, right as int, edge as int, matched, next_depth);
                    levels_proof::roots_distance_changed(transpose.offsets@, transpose.edges@, pair_left@, distance@,
                        roots@, shortest, matched, next_depth);
                    levels_proof::monotone_discovery(distance@, queue@, roots@, shortest, head as int, matched, next_depth);
                    levels_proof::scanned_step(transpose.offsets@, transpose.edges@, pair_left@,
                        distance@, distance@.update(matched as int, next_depth), queue@, queue@.push(matched),
                        roots@, roots@, shortest, shortest, head as int, edge as int);
                }
                *get_mut(distance, usize::from(matched)) = next_depth;
                queue.push(matched);
            }
            edge += 1;
        }
        proof! {
            levels_proof::finish_layer(distance@, queue@, head as int - 1);
            levels_proof::scanned_finished(transpose.offsets@, transpose.edges@, pair_left@, distance@, queue@, roots@, shortest, head as int);
        }
    }
    proof! {
        levels_proof::final_scanned(transpose.offsets@, transpose.edges@, pair_left@, distance@, queue@, roots@, shortest);
        levels_proof::final_cut_bound(distance@, queue@, shortest);
        levels_proof::queue_budget(transpose.offsets@, transpose.edges@.len(), queue@, head as int);
    }
    if shortest == UNMATCHED_U16 {
        None
    } else {
        Some(shortest)
    }
}

/// 从空闲右顶点反向遍历交替路径，记录最短层上的空闲左顶点。
/// `right_distance` 以匹配边数计数，最短增广路径有 2 * depth + 1 条边。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::buffers_valid(old(scratch)),
        levels_proof::reverse_arrays(old(scratch).transpose_offsets@, old(scratch).transpose_edges@, old(scratch).pair_left@, old(scratch).pair_right@),
        old(scratch).right_distance@.len() == old(scratch).pair_right@.len(),
        old(scratch).reachable_left@.len() == old(scratch).pair_left@.len(),
    ensures proof::buffers_valid(final(scratch)), levels_proof::queued(final(scratch).right_distance@, final(scratch).queue@),
        final(scratch).right_distance@.len() == old(scratch).pair_right@.len(),
        final(scratch).shortest_roots@.len() <= old(scratch).transpose_edges@.len(),
        levels_proof::roots_free(final(scratch).shortest_roots@, old(scratch).pair_left@),
        levels_proof::distance_witness(old(scratch).transpose_offsets@, old(scratch).transpose_edges@,
            old(scratch).pair_left@, old(scratch).pair_right@, final(scratch).right_distance@),
        levels_proof::roots_sound(old(scratch).transpose_offsets@, old(scratch).transpose_edges@, old(scratch).pair_left@,
            final(scratch).right_distance@, final(scratch).shortest_roots@, match result { Some(depth) => depth, None => u16::MAX }),
        levels_proof::global_closed(old(scratch).transpose_offsets@, old(scratch).transpose_edges@, old(scratch).pair_left@,
            final(scratch).right_distance@, final(scratch).shortest_roots@, match result { Some(depth) => depth, None => u16::MAX }),
        final(scratch).reachable_left@ == Seq::new(old(scratch).pair_left@.len(), |_: int| false),
        forall|right: int| 0 <= right < old(scratch).pair_right@.len() && old(scratch).pair_right@[right] == u16::MAX
            ==> final(scratch).right_distance@[right] == 0,
        forall|right: int| 0 <= right < final(scratch).right_distance@.len() && final(scratch).right_distance@[right] != u16::MAX
            ==> final(scratch).right_distance@[right] < old(scratch).pair_right@.len(),
        match result { Some(depth) => depth < old(scratch).pair_right@.len() && final(scratch).shortest_roots@.len() > 0
            && (forall|right: int| 0 <= right < final(scratch).right_distance@.len() && final(scratch).right_distance@[right] != u16::MAX
                ==> final(scratch).right_distance@[right] <= depth as int + 1), None => final(scratch).shortest_roots@.len() == 0 },
        final(scratch).pair_left == old(scratch).pair_left, final(scratch).pair_right == old(scratch).pair_right,
        final(scratch).transpose_offsets == old(scratch).transpose_offsets, final(scratch).transpose_edges == old(scratch).transpose_edges,
        final(scratch).unmatched_lefts == old(scratch).unmatched_lefts, final(scratch).reachable_right == old(scratch).reachable_right,
        final(scratch).write_offsets == old(scratch).write_offsets, final(scratch).right_order == old(scratch).right_order,
        final(scratch).right_degree_counts == old(scratch).right_degree_counts, final(scratch).dfs_stack == old(scratch).dfs_stack,
))]
fn build_reverse_levels(scratch: &mut HkScratch) -> Option<u16> {
    {
        let transpose = SparseAdjacencyRef {
            offsets: scratch.transpose_offsets.as_slice(),
            edges: scratch.transpose_edges.as_slice(),
        };
        reverse_levels(
            &transpose,
            scratch.pair_left.as_slice(),
            scratch.pair_right.as_slice(),
            scratch.right_distance.as_mut_slice(),
            scratch.reachable_left.as_mut_slice(),
            &mut scratch.queue,
            &mut scratch.shortest_roots,
        )
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::buffers_valid(old(scratch)),
        phase_proof::certificate(adjacency.offsets@, adjacency.edges@, old(scratch).transpose_offsets@, old(scratch).transpose_edges@,
            old(scratch).pair_left@, old(scratch).pair_right@, old(scratch).right_distance@, old(scratch).shortest_roots@, shortest_depth),
        old(scratch).reachable_left@ == Seq::new(old(scratch).pair_left@.len(), |_: int| false),
        proof::free_lefts(old(scratch).pair_left@, old(scratch).unmatched_lefts@),
    ensures proof::buffers_valid(final(scratch)), 0 < result <= old(scratch).pair_left@.len(),
        proof::matching_valid(adjacency.offsets@, adjacency.edges@, final(scratch).pair_left@, final(scratch).pair_right@),
        proof::matched_preserved(old(scratch).pair_left@, final(scratch).pair_left@),
        proof::matched_preserved(old(scratch).pair_right@, final(scratch).pair_right@),
        proof::matched_count(final(scratch).pair_left@, final(scratch).pair_left@.len() as int)
            == proof::matched_count(old(scratch).pair_left@, old(scratch).pair_left@.len() as int) + result,
        proof::free_lefts(final(scratch).pair_left@, final(scratch).unmatched_lefts@),
        final(scratch).reachable_left@.len() == old(scratch).reachable_left@.len(),
        final(scratch).transpose_offsets == old(scratch).transpose_offsets, final(scratch).transpose_edges == old(scratch).transpose_edges,
        final(scratch).right_distance == old(scratch).right_distance, final(scratch).queue == old(scratch).queue,
        final(scratch).shortest_roots == old(scratch).shortest_roots, final(scratch).reachable_right == old(scratch).reachable_right,
        final(scratch).write_offsets == old(scratch).write_offsets, final(scratch).right_order == old(scratch).right_order,
        final(scratch).right_degree_counts == old(scratch).right_degree_counts,
))]
fn augment_phase(
    adjacency: &SparseAdjacencyRef<'_>,
    scratch: &mut HkScratch,
    shortest_depth: u16,
) -> usize {
    proof_decl! { let ghost initial_left = scratch.pair_left@; let ghost initial_right = scratch.pair_right@; }
    let augmentations = {
        #[cfg(verus_only)]
        let transpose = SparseAdjacencyRef {
            offsets: scratch.transpose_offsets.as_slice(),
            edges: scratch.transpose_edges.as_slice(),
        };
        augment_roots(
            adjacency,
            #[cfg(verus_only)]
            &transpose,
            scratch.pair_left.as_mut_slice(),
            scratch.pair_right.as_mut_slice(),
            scratch.right_distance.as_slice(),
            scratch.shortest_roots.as_slice(),
            shortest_depth,
            scratch.reachable_left.as_mut_slice(),
            &mut scratch.dfs_stack,
        )
    };
    proof_decl! { let ghost roots_left = scratch.pair_left@; let ghost roots_right = scratch.pair_right@; }
    proof! {
        path_proof::bounds(adjacency.offsets@, adjacency.edges@, scratch.pair_left@, scratch.pair_right@, scratch.right_distance@, shortest_depth);
        levels_proof::ids_bound(scratch.unmatched_lefts@, initial_left.len());
    }
    let additional = augment_additional(
        adjacency,
        scratch.pair_left.as_mut_slice(),
        scratch.pair_right.as_mut_slice(),
        scratch.right_distance.as_slice(),
        scratch.unmatched_lefts.as_slice(),
        shortest_depth,
        scratch.reachable_left.as_mut_slice(),
        &mut scratch.dfs_stack,
    );
    proof! {
        phase_proof::preserved_transitive(initial_left, roots_left, scratch.pair_left@);
        phase_proof::preserved_transitive(initial_right, roots_right, scratch.pair_right@);
        phase_proof::count_bound(scratch.pair_left@, scratch.pair_left@.len() as int);
        phase_proof::filtered_free(initial_left, scratch.pair_left@, scratch.unmatched_lefts@);
    }
    crate::optimal::search::remove_matched(&scratch.pair_left, &mut scratch.unmatched_lefts);
    augmentations + additional
}

/// 复用最短阶段的访问集，追加搜索尚未访问的空闲左顶点。
#[allow(clippy::too_many_arguments, reason = "图、匹配与搜索缓冲分别借用")]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires path_proof::domain(adjacency.offsets@, adjacency.edges@, old(pair_left)@, old(pair_right)@, distance@, shortest_depth),
        old(seen)@.len() == old(pair_left)@.len(), old(stack).valid(),
        roots@.len() <= 1953, forall|i: int| 0 <= i < roots@.len() ==> #[trigger] roots@[i] < old(pair_left)@.len(),
    ensures final(stack).valid(), result <= old(pair_left)@.len(), final(seen)@.len() == old(seen)@.len(),
        path_proof::domain(adjacency.offsets@, adjacency.edges@, final(pair_left)@, final(pair_right)@, distance@, shortest_depth),
        proof::matching_valid(adjacency.offsets@, adjacency.edges@, final(pair_left)@, final(pair_right)@),
        proof::matched_preserved(old(pair_left)@, final(pair_left)@), proof::matched_preserved(old(pair_right)@, final(pair_right)@),
        proof::matched_count(final(pair_left)@, final(pair_left)@.len() as int)
            == proof::matched_count(old(pair_left)@, old(pair_left)@.len() as int) + result,
))]
fn augment_additional(
    adjacency: &SparseAdjacencyRef<'_>,
    pair_left: &mut [u16],
    pair_right: &mut [u16],
    distance: &[u16],
    roots: &[u16],
    shortest_depth: u16,
    seen: &mut [bool],
    stack: &mut ChordBuffer<DfsFrame>,
) -> usize {
    proof_decl! { let ghost initial_left = pair_left@; let ghost initial_right = pair_right@; }
    proof! { path_proof::bounds(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@, distance@, shortest_depth); }
    let mut augmentations = 0usize;
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= roots@.len(), roots@.len() <= 1953, stack.valid(),
            initial_left == old(pair_left)@, initial_right == old(pair_right)@, initial_left.len() <= 1953,
            seen@.len() == initial_left.len(),
            forall|i: int| 0 <= i < roots@.len() ==> #[trigger] roots@[i] < initial_left.len(),
            path_proof::domain(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@, distance@, shortest_depth),
            proof::matched_preserved(initial_left, pair_left@), proof::matched_preserved(initial_right, pair_right@),
            proof::matched_count(pair_left@, pair_left@.len() as int) == proof::matched_count(initial_left, initial_left.len() as int) + augmentations,
            augmentations <= index,
        decreases roots@.len() - index,
    ))]
    while index < roots.len() {
        let left = usize::from(copy(roots, index));
        if !copy(seen, left)
            && copy(pair_left, left) == UNMATCHED_U16
            && dfs(
                left,
                adjacency,
                pair_left,
                pair_right,
                distance,
                shortest_depth,
                seen,
                stack,
                false,
            )
        {
            augmentations += 1;
        }
        index += 1;
    }
    proof! { path_proof::unmodified(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@, distance@, shortest_depth); }
    proof! { phase_proof::count_bound(pair_left@, pair_left@.len() as int); }
    augmentations
}

/// 根列表只负责调度；路径搜索和配对提交各自独占所修改的缓冲。
#[allow(clippy::too_many_arguments, reason = "匹配、分层和搜索缓冲分别借用")]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires phase_proof::certificate(adjacency.offsets@, adjacency.edges@, _transpose.offsets@, _transpose.edges@,
            old(pair_left)@, old(pair_right)@, distance@, roots@, shortest_depth),
        old(seen)@ == Seq::new(old(pair_left)@.len(), |_: int| false), old(stack).valid(),
    ensures final(stack).valid(), 0 < result <= old(pair_left)@.len(),
        final(seen)@.len() == old(seen)@.len(),
        path_proof::domain(adjacency.offsets@, adjacency.edges@, final(pair_left)@, final(pair_right)@, distance@, shortest_depth),
        proof::matching_valid(adjacency.offsets@, adjacency.edges@, final(pair_left)@, final(pair_right)@),
        proof::matched_preserved(old(pair_left)@, final(pair_left)@), proof::matched_preserved(old(pair_right)@, final(pair_right)@),
        proof::matched_count(final(pair_left)@, final(pair_left)@.len() as int)
            == proof::matched_count(old(pair_left)@, old(pair_left)@.len() as int) + result,
))]
fn augment_roots(
    adjacency: &SparseAdjacencyRef<'_>,
    #[cfg(verus_only)] _transpose: &SparseAdjacencyRef<'_>,
    pair_left: &mut [u16],
    pair_right: &mut [u16],
    distance: &[u16],
    roots: &[u16],
    shortest_depth: u16,
    seen: &mut [bool],
    stack: &mut ChordBuffer<DfsFrame>,
) -> usize {
    proof_decl! { let ghost initial_left = pair_left@; let ghost initial_right = pair_right@; }
    proof! { phase_proof::certificate_fields(adjacency.offsets@, adjacency.edges@, _transpose.offsets@, _transpose.edges@,
    pair_left@, pair_right@, distance@, roots@, shortest_depth); }
    let mut augmentations = 0usize;
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= roots@.len(), stack.valid(),
            phase_proof::certificate(adjacency.offsets@, adjacency.edges@, _transpose.offsets@, _transpose.edges@,
                initial_left, initial_right, distance@, roots@, shortest_depth),
            levels_proof::roots_free(roots@, initial_left), roots@.len() > 0,
            initial_left == old(pair_left)@, initial_right == old(pair_right)@,
            seen@.len() == initial_left.len(), initial_left.len() <= 1953,
            path_proof::domain(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@, distance@, shortest_depth),
            proof::matching_valid(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@),
            proof::matched_preserved(initial_left, pair_left@), proof::matched_preserved(initial_right, pair_right@),
            proof::matched_count(pair_left@, pair_left@.len() as int) == proof::matched_count(initial_left, initial_left.len() as int) + augmentations,
            augmentations <= index, index > 0 ==> augmentations > 0,
            index == 0 ==> pair_left@ == initial_left && pair_right@ == initial_right && seen@ == Seq::new(initial_left.len(), |_: int| false),
        decreases roots@.len() - index,
    ))]
    while index < roots.len() {
        let left = usize::from(copy(roots, index));
        if !copy(seen, left) && copy(pair_left, left) == UNMATCHED_U16 {
            let found = dfs(
                left,
                adjacency,
                pair_left,
                pair_right,
                distance,
                shortest_depth,
                seen,
                stack,
                true,
            );
            if found {
                proof! { phase_proof::count_bound(pair_left@, pair_left@.len() as int); }
                augmentations += 1;
            }
            proof! {
                if !found && index == 0 {
                    assert(stack@ =~= Seq::<DfsFrame>::empty());
                    phase_proof::first_failure(adjacency.offsets@, adjacency.edges@, _transpose.offsets@, _transpose.edges@,
                        initial_left, initial_right, distance@, roots@, shortest_depth, seen@);
                }
            }
        }
        index += 1;
    }
    proof! { phase_proof::count_bound(pair_left@, pair_left@.len() as int); }
    augmentations
}

#[allow(
    clippy::too_many_arguments,
    reason = "路径搜索和配对提交独占各自输出缓冲"
)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires path_proof::domain(adjacency.offsets@, adjacency.edges@, old(pair_left)@, old(pair_right)@, distance@, shortest_depth),
        old(seen)@.len() == old(pair_left)@.len(), start_left < old(pair_left)@.len(), old(pair_left)@[start_left as int] == u16::MAX,
        !old(seen)@[start_left as int], old(stack).valid(),
    ensures final(stack).valid(), path_proof::state(adjacency.offsets@, adjacency.edges@, old(pair_left)@, old(pair_right)@,
            distance@, shortest_depth, old(seen)@, final(seen)@, final(stack)@, start_left as int, layered),
        path_proof::domain(adjacency.offsets@, adjacency.edges@, final(pair_left)@, final(pair_right)@, distance@, shortest_depth),
        path_proof::increased(old(seen)@, final(seen)@), final(seen)@[start_left as int],
        proof::matching_valid(adjacency.offsets@, adjacency.edges@, final(pair_left)@, final(pair_right)@),
        proof::matched_preserved(old(pair_left)@, final(pair_left)@), proof::matched_preserved(old(pair_right)@, final(pair_right)@),
        proof::matched_count(final(pair_left)@, final(pair_left)@.len() as int)
            == proof::matched_count(old(pair_left)@, old(pair_left)@.len() as int) + if result { 1nat } else { 0nat },
        !result ==> final(pair_left)@ == old(pair_left)@ && final(pair_right)@ == old(pair_right)@ && final(stack)@.len() == 0,
        result ==> proof::path_state(old(pair_left)@, old(pair_right)@, final(pair_left)@, final(pair_right)@,
            adjacency.edges@, final(stack)@, final(pair_left)@[final(stack)@.last().left as int], 0),
        result ==> proof::path_links(adjacency.offsets@, adjacency.edges@, old(pair_left)@, old(pair_right)@,
            final(stack)@, final(pair_left)@[final(stack)@.last().left as int]) && (layered ==> final(stack)@.len() == shortest_depth as int + 1),
))]
#[inline(always)]
fn dfs(
    start_left: usize,
    adjacency: &SparseAdjacencyRef<'_>,
    pair_left: &mut [u16],
    pair_right: &mut [u16],
    distance: &[u16],
    shortest_depth: u16,
    seen: &mut [bool],
    stack: &mut ChordBuffer<DfsFrame>,
    layered: bool,
) -> bool {
    proof_decl! { let ghost initial_left = pair_left@; let ghost initial_right = pair_right@; }
    let terminal = find_path(
        start_left,
        adjacency,
        #[cfg(verus_only)]
        pair_left,
        pair_right,
        distance,
        shortest_depth,
        seen,
        stack,
        layered,
    );
    proof! { path_proof::observed(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@, distance@, shortest_depth,
    old(seen)@, seen@, stack@, start_left as int, layered); }
    if let Some(right) = terminal {
        crate::optimal::matching::commit_path(
            adjacency,
            stack.as_slice(),
            usize::from(right),
            pair_left,
            pair_right,
        );
        proof! { path_proof::after_commit(adjacency.offsets@, adjacency.edges@, initial_left, initial_right,
        distance@, shortest_depth, pair_left@, pair_right@);
        path_proof::last_assigned(initial_left, initial_right, pair_left@, pair_right@, adjacency.edges@, stack@, right); }
        true
    } else {
        proof! { path_proof::unmodified(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@, distance@, shortest_depth); }
        false
    }
}

/// 在只读匹配上找交替路径；`layered` 限定本轮最短层，成功的栈直接交给配对提交核。
#[allow(
    clippy::too_many_arguments,
    reason = "只读图、匹配及分层参数与搜索缓冲分别借用"
)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires path_proof::domain(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@, shortest_depth),
        old(seen)@.len() == _pair_left@.len(), start_left < _pair_left@.len(), _pair_left@[start_left as int] == u16::MAX,
        !old(seen)@[start_left as int], old(stack).valid(),
    ensures final(stack).valid(), path_proof::state(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@,
            distance@, shortest_depth, old(seen)@, final(seen)@, final(stack)@, start_left as int, layered),
        match result {
            Some(right) => proof::path_links(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, final(stack)@, right)
                && (layered ==> final(stack)@.len() == shortest_depth as int + 1),
            None => final(stack)@.len() == 0,
        },
))]
#[inline(always)]
fn find_path(
    start_left: usize,
    adjacency: &SparseAdjacencyRef<'_>,
    #[cfg(verus_only)] _pair_left: &[u16],
    pair_right: &[u16],
    distance: &[u16],
    shortest_depth: u16,
    seen: &mut [bool],
    stack: &mut ChordBuffer<DfsFrame>,
    layered: bool,
) -> Option<u16> {
    proof_decl! { let ghost initial_seen = seen@; }
    proof! {
        path_proof::bounds(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@, shortest_depth);
        path_proof::start(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@, shortest_depth, seen@, start_left as int, layered);
    }
    stack.clear();
    stack.push(DfsFrame {
        left: u16_index(start_left),
        next_edge: copy(adjacency.offsets, start_left),
    });
    *get_mut(seen, start_left) = true;
    #[cfg_attr(verus_only, verus_spec(
        invariant initial_seen == old(seen)@, stack.valid(), path_proof::state(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@,
            distance@, shortest_depth, initial_seen, seen@, stack@, start_left as int, layered),
        decreases path_proof::work(seen@, stack@),
    ))]
    while !stack.is_empty() {
        proof! { path_proof::at(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@, shortest_depth,
        initial_seen, seen@, stack@, start_left as int, layered); }
        proof_decl! { let ghost prior_work = path_proof::work(seen@, stack@); }
        let top = stack.len() - 1;
        let DfsFrame {
            left,
            next_edge: mut edge,
        } = copy(stack.as_slice(), top);
        let left = usize::from(left);
        let mut descended = false;
        let end = copy(adjacency.offsets, left + 1);
        #[cfg_attr(verus_only, verus_spec(
            invariant initial_seen == old(seen)@, stack.valid(), path_proof::state(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@,
                    distance@, shortest_depth, initial_seen, seen@, stack@, start_left as int, layered),
                path_proof::domain(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@, shortest_depth),
                seen@.len() == _pair_left@.len(), _pair_left@.len() <= 1953, pair_right@.len() <= 1953,
                0 < stack@.len(), left < _pair_left@.len(), adjacency.offsets@.len() == _pair_left@.len() + 1,
                end == adjacency.offsets@[left as int + 1], adjacency.offsets@[left as int] <= edge <= end,
                end <= adjacency.edges@.len(), adjacency.edges@.len() <= 3969,
                !descended ==> path_proof::work(seen@, stack@) == prior_work,
                descended ==> path_proof::work(seen@, stack@) < prior_work,
                !descended ==> top + 1 == stack@.len() && left == stack@[top as int].left
                    && path_proof::closed(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@, shortest_depth, seen@, left as int, edge as int, layered)
                    && (layered ==> path_proof::rank(_pair_left@, distance@, shortest_depth, left as int) == shortest_depth as int + 2 - stack@.len()),
            decreases end - edge,
        ))]
        while edge < end && !descended {
            proof! { path_proof::slot(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@, shortest_depth, left as int, edge as int); }
            let current = edge;
            let right = usize::from(copy(adjacency.edges, usize::from(current)));
            edge += 1;
            if layered
                && usize::from(copy(distance, right)) + stack.len()
                    != usize::from(shortest_depth) + 1
            {
                proof! { path_proof::scan(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@,
                shortest_depth, seen@, left as int, current as int, layered); }
                continue;
            }
            let matched = copy(pair_right, right);
            if matched == UNMATCHED_U16 {
                proof! { path_proof::found(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@, shortest_depth,
                initial_seen, seen@, stack@, start_left as int, current as int, layered); }
                return Some(u16_index(right));
            }
            let next_left = usize::from(matched);
            proof! {
                if seen@[next_left as int] {
                    path_proof::scan(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@,
                        shortest_depth, seen@, left as int, current as int, layered);
                }
            }
            if !copy(seen, next_left) {
                proof! { path_proof::descend(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@, shortest_depth,
                initial_seen, seen@, stack@, start_left as int, current as int, layered); }
                {
                    get_mut(stack.as_mut_slice(), top).next_edge = edge;
                }
                *get_mut(seen, next_left) = true;
                stack.push(DfsFrame {
                    left: matched,
                    next_edge: copy(adjacency.offsets, next_left),
                });
                descended = true;
            }
        }
        if !descended {
            proof! { path_proof::ascend(adjacency.offsets@, adjacency.edges@, _pair_left@, pair_right@, distance@, shortest_depth,
            initial_seen, seen@, stack@, start_left as int, layered); }
            stack.pop();
        }
    }
    None
}

/// 将最终反向 BFS 的对偶覆盖编码为左侧选中、右侧排除标记。
///
/// 若左侧已全部匹配，右侧全体就是最大独立集，不能读取成功 BFS 的旧层次。
/// 否则最后一次 BFS 必须失败：没有最短深度截断，`right_distance` 恰好记录
/// 从空闲右顶点出发的完整交替可达集 `Z_R`。此时 `Z_L` 中的每个左顶点均已匹配，
/// 且其匹配右顶点也在 `Z_R`；反过来也成立。因此最大独立集为 `(L \ Z_L) ∪ Z_R`。
/// 保留调用方现有的布尔编码，避免再从空闲左顶点扫描一次冲突边。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires phase_proof::completed(_adjacency, old(scratch)),
    ensures phase_proof::completed(_adjacency, final(scratch)),
        proof::buffers_valid(final(scratch)),
        final(scratch).pair_left == old(scratch).pair_left, final(scratch).pair_right == old(scratch).pair_right,
        proof::marks_certificate(_adjacency.offsets@, _adjacency.edges@, final(scratch).pair_left@, final(scratch).pair_right@,
            final(scratch).reachable_left@, final(scratch).reachable_right@),
        crate::chord_graph::proof::marks_match(final(scratch).pair_left@, final(scratch).right_distance@,
            final(scratch).unmatched_lefts@.len() == 0, final(scratch).reachable_left@, final(scratch).reachable_right@),
))]
fn collect_independent_set_marks(_adjacency: &SparseAdjacencyRef<'_>, scratch: &mut HkScratch) {
    proof! { reveal(phase_proof::completed); }
    if scratch.unmatched_lefts.is_empty() {
        fill_search(scratch.reachable_left.as_mut_slice(), false);
        fill_search(scratch.reachable_right.as_mut_slice(), false);
        proof! { cover_proof::collect_certificate(_adjacency, scratch, scratch.reachable_left@, scratch.reachable_right@); }
        return;
    }
    crate::chord_graph::mark_independent_set(
        scratch.pair_left.as_slice(),
        scratch.right_distance.as_slice(),
        false,
        scratch.reachable_left.as_mut_slice(),
        scratch.reachable_right.as_mut_slice(),
    );
    proof! { cover_proof::collect_certificate(_adjacency, scratch, scratch.reachable_left@, scratch.reachable_right@); }
}
