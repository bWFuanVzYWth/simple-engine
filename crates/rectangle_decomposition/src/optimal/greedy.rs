//! 贪心匹配初始化：度 1 匹配、按右部度数排序、3 跳增广。

use crate::optimal::fixed::FixedVec;
use crate::optimal::matching::{
    ChordBuffer, HkScratch, IMAGE64_AXIS_LIMIT, IMAGE64_MAX_CHORDS, IMAGE64_MAX_CONFLICT_EDGES,
    SparseAdjacencyRef, UNMATCHED_U16,
};
use crate::optimal::{copy, get_mut, slice, u16_index};

#[cfg(verus_only)]
use crate::optimal::graph::proof as order_proof;
#[cfg(verus_only)]
use crate::optimal::matching::proof;
#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};

/// 构造精确转置；右度数由输入边的实际出现次数给出，不要求目标内容正确。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::adjacency_valid(adjacency, right_degrees@.len()),
        proof::degrees_exact(adjacency.edges@, right_degrees@),
        old(transpose_offsets).valid(), old(transpose_edges).valid(), old(write_offsets).valid(),
    ensures final(transpose_offsets).valid(), final(transpose_edges).valid(), final(write_offsets).valid(),
        proof::transposed(adjacency.offsets@, adjacency.edges@,
            final(transpose_offsets)@, final(transpose_edges)@, right_degrees@.len()),
        final(write_offsets)@.len() == right_degrees@.len(),
))]
pub(super) fn build_transpose(
    adjacency: &SparseAdjacencyRef<'_>,
    right_degrees: &[u8],
    transpose_offsets: &mut FixedVec<u16, { IMAGE64_MAX_CHORDS + 1 }>,
    transpose_edges: &mut FixedVec<u16, IMAGE64_MAX_CONFLICT_EDGES>,
    write_offsets: &mut ChordBuffer<u16>,
) {
    transpose_offsets.clear();
    transpose_offsets.resize(right_degrees.len() + 1, 0);
    let offsets = transpose_offsets.as_mut_slice();
    proof! { proof::degrees_total(adjacency.edges@, right_degrees@); }
    let mut right = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::adjacency_valid(adjacency, right_degrees@.len()),
            proof::degrees_exact(adjacency.edges@, right_degrees@),
            proof::counted_offsets(right_degrees@, offsets@, right as int),
            crate::optimal::graph::proof::degree_sum(right_degrees@, right_degrees@.len() as int) == adjacency.edges@.len(),
        decreases right_degrees@.len() - right,
    ))]
    while right < right_degrees.len() {
        proof_decl! { let ghost before = offsets@; }
        proof! { proof::degree_sum_monotone(right_degrees@, right as int + 1, right_degrees@.len() as int); }
        let offset = copy(offsets, right) + u16::from(copy(right_degrees, right));
        *get_mut(offsets, right + 1) = offset;
        proof! { proof::transpose_offset_step(right_degrees@, before, offsets@, right as int); }
        right += 1;
    }
    transpose_edges.clear();
    transpose_edges.resize(adjacency.edges.len(), 0);
    write_offsets.clear();
    write_offsets.extend_from_slice(slice(offsets, 0..right_degrees.len()));
    let output = transpose_edges.as_mut_slice();
    let cursors = write_offsets.as_mut_slice();
    proof! { proof::transpose_start(adjacency.offsets@, adjacency.edges@, right_degrees@, offsets@, output@, cursors@); }
    let mut left = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::adjacency_valid(adjacency, right_degrees@.len()),
            proof::degrees_exact(adjacency.edges@, right_degrees@),
            left + 1 <= adjacency.offsets@.len(),
            proof::transpose_progress(adjacency.offsets@, adjacency.edges@, right_degrees@,
                offsets@, output@, cursors@, adjacency.offsets@[left as int] as int),
        decreases adjacency.offsets@.len() - 1 - left,
    ))]
    while left + 1 < adjacency.offsets.len() {
        let mut edge = usize::from(copy(adjacency.offsets, left));
        let end = usize::from(copy(adjacency.offsets, left + 1));
        #[cfg_attr(verus_only, verus_spec(
            invariant proof::adjacency_valid(adjacency, right_degrees@.len()),
                proof::degrees_exact(adjacency.edges@, right_degrees@),
                left + 1 < adjacency.offsets@.len(),
                adjacency.offsets@[left as int] <= edge <= end,
                end == adjacency.offsets@[left as int + 1],
                proof::transpose_progress(adjacency.offsets@, adjacency.edges@, right_degrees@,
                    offsets@, output@, cursors@, edge as int),
            decreases end - edge,
        ))]
        while edge < end {
            proof_decl! { let ghost before = cursors@; let ghost previous = output@; }
            proof! { proof::transpose_ready(adjacency.offsets@, adjacency.edges@, right_degrees@, offsets@, output@, cursors@, edge as int); }
            let right = usize::from(copy(adjacency.edges, edge));
            let slot = copy(cursors, right);
            *get_mut(output, usize::from(slot)) = u16_index(left);
            *get_mut(cursors, right) += 1;
            proof! { proof::transpose_step(adjacency.offsets@, adjacency.edges@, right_degrees@, offsets@, previous, before, output@, cursors@, edge as int, left as int); }
            edge += 1;
        }
        left += 1;
    }
    proof! { proof::transpose_complete(adjacency.offsets@, adjacency.edges@, right_degrees@, offsets@, output@, cursors@); }
}

/// 扫描一个左点尚未检查的邻接后缀，只前进其复用游标。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::adjacency_valid(adjacency, pair_right@.len()),
        proof::cursors_valid(adjacency.offsets@, old(cursors)@), left < old(cursors)@.len(),
    ensures proof::cursors_valid(adjacency.offsets@, final(cursors)@),
        final(cursors)@[left as int] >= old(cursors)@[left as int],
        forall|l: int| 0 <= l < old(cursors)@.len() && l != left ==> final(cursors)@[l] == old(cursors)@[l],
        match result { Some(right) => right < pair_right@.len() && pair_right@[right as int] == u16::MAX
            && proof::edge_in(adjacency.offsets@, adjacency.edges@, left as int, right as int), None => true },
))]
fn find_free_neighbor(
    adjacency: &SparseAdjacencyRef<'_>,
    left: usize,
    pair_right: &[u16],
    cursors: &mut [u16],
) -> Option<u16> {
    proof_decl! { let ghost initial = cursors@; }
    let end = copy(adjacency.offsets, left + 1);
    let mut result: Option<u16> = None;
    {
        let next = get_mut(cursors, left);
        #[cfg_attr(verus_only, verus_spec(
            invariant proof::adjacency_valid(adjacency, pair_right@.len()), left + 1 < adjacency.offsets@.len(),
                proof::cursors_valid(adjacency.offsets@, initial), left < initial.len(),
                adjacency.offsets@[left as int] <= *next <= end,
                end == adjacency.offsets@[left as int + 1], *next >= initial[left as int],
                match result { Some(right) => right < pair_right@.len() && pair_right@[right as int] == u16::MAX
                    && proof::edge_in(adjacency.offsets@, adjacency.edges@, left as int, right as int), None => true },
            decreases end - *next,
        ))]
        while *next < end {
            let edge = usize::from(*next);
            let right = copy(adjacency.edges, edge);
            *next += 1;
            if copy(pair_right, usize::from(right)) == UNMATCHED_U16 {
                proof! { proof::edge_at(adjacency.offsets@, adjacency.edges@, left as int, edge as int); }
                result = Some(right);
                break;
            }
        }
    }
    result
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::adjacency_valid(adjacency, old(pair_right)@.len()),
        proof::matching_valid(adjacency.offsets@, adjacency.edges@, old(pair_left)@, old(pair_right)@),
        old(next_edges).valid(),
    ensures proof::matching_valid(adjacency.offsets@, adjacency.edges@, final(pair_left)@, final(pair_right)@),
        proof::matched_preserved(old(pair_left)@, final(pair_left)@),
        proof::matched_preserved(old(pair_right)@, final(pair_right)@),
        proof::matched_count(final(pair_left)@, final(pair_left)@.len() as int)
            >= proof::matched_count(old(pair_left)@, old(pair_left)@.len() as int),
        final(next_edges).valid(), proof::cursors_valid(adjacency.offsets@, final(next_edges)@),
))]
pub(super) fn greedy_augment_len3_sparse_csr_u16(
    adjacency: &SparseAdjacencyRef<'_>,
    pair_left: &mut [u16],
    pair_right: &mut [u16],
    next_edges: &mut ChordBuffer<u16>,
) {
    proof_decl! { let ghost initial_left = pair_left@; let ghost initial_right = pair_right@; }
    next_edges.clear();
    next_edges.extend_from_slice(slice(adjacency.offsets, 0..pair_left.len()));
    let cursors = next_edges.as_mut_slice();
    let mut next_left = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::adjacency_valid(adjacency, pair_right@.len()),
            proof::matching_valid(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@),
            proof::matched_preserved(initial_left, pair_left@), proof::matched_preserved(initial_right, pair_right@),
            proof::matched_count(pair_left@, pair_left@.len() as int) >= proof::matched_count(initial_left, initial_left.len() as int),
            proof::cursors_valid(adjacency.offsets@, cursors@), next_left <= pair_left@.len(),
        decreases pair_left@.len() - next_left,
    ))]
    while next_left < pair_left.len() {
        let left = next_left;
        next_left += 1;
        if copy(pair_left, left) != UNMATCHED_U16 {
            continue;
        }
        let mut edge = usize::from(copy(adjacency.offsets, left));
        let end = usize::from(copy(adjacency.offsets, left + 1));
        #[cfg_attr(verus_only, verus_spec(
            invariant proof::adjacency_valid(adjacency, pair_right@.len()),
                proof::matching_valid(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@),
                proof::matched_preserved(initial_left, pair_left@), proof::matched_preserved(initial_right, pair_right@),
                proof::matched_count(pair_left@, pair_left@.len() as int) >= proof::matched_count(initial_left, initial_left.len() as int),
                proof::cursors_valid(adjacency.offsets@, cursors@), next_left <= pair_left@.len(),
                left < pair_left@.len(), adjacency.offsets@[left as int] <= edge <= end,
                end == adjacency.offsets@[left as int + 1],
            invariant_except_break pair_left@[left as int] == u16::MAX,
            decreases end - edge,
        ))]
        while edge < end {
            proof! { proof::edge_at(adjacency.offsets@, adjacency.edges@, left as int, edge as int); }
            let right = usize::from(copy(adjacency.edges, edge));
            edge += 1;
            let matched_left = copy(pair_right, right);
            if matched_left == UNMATCHED_U16 {
                crate::optimal::matching::match_free(adjacency, left, right, pair_left, pair_right);
                break;
            }

            let matched_left = usize::from(matched_left);
            // 长度 1/3 的增广只会减少空闲右顶点，已检查的邻居不会再次可用。
            // 成功选中的右顶点也立即占用，所以每条内层边至多检查一次。
            if let Some(alternate_right) =
                find_free_neighbor(adjacency, matched_left, pair_right, cursors)
            {
                let alternate_right = usize::from(alternate_right);
                crate::optimal::matching::match_three(
                    adjacency,
                    left,
                    right,
                    matched_left,
                    alternate_right,
                    pair_left,
                    pair_right,
                );
                break;
            }
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::buffers_valid(old(scratch)), proof::adjacency_valid(adjacency, right_degrees@.len()),
        proof::degrees_exact(adjacency.edges@, right_degrees@),
        proof::matching_valid(adjacency.offsets@, adjacency.edges@, old(scratch).pair_left@, old(scratch).pair_right@),
        old(scratch).pair_right@.len() == right_degrees@.len(),
        forall|left: int| 0 <= left < old(scratch).pair_left@.len() ==> old(scratch).pair_left@[left] == u16::MAX,
    ensures proof::buffers_valid(final(scratch)),
        proof::matching_valid(adjacency.offsets@, adjacency.edges@, final(scratch).pair_left@, final(scratch).pair_right@),
        final(scratch).pair_right@.len() == right_degrees@.len(),
        proof::transposed(adjacency.offsets@, adjacency.edges@,
            final(scratch).transpose_offsets@, final(scratch).transpose_edges@, right_degrees@.len()),
        proof::sorted_rights(right_degrees@, final(scratch).right_order@),
        final(scratch).right_distance == old(scratch).right_distance,
        final(scratch).queue == old(scratch).queue,
        final(scratch).unmatched_lefts == old(scratch).unmatched_lefts,
        final(scratch).shortest_roots == old(scratch).shortest_roots,
        final(scratch).reachable_left == old(scratch).reachable_left,
        final(scratch).reachable_right == old(scratch).reachable_right,
        final(scratch).dfs_stack == old(scratch).dfs_stack,
))]
pub(super) fn greedy_initialize_sparse_matching_csr_u16(
    adjacency: &SparseAdjacencyRef<'_>,
    right_degrees: &[u8],
    scratch: &mut HkScratch,
) {
    let pair_left = scratch.pair_left.as_mut_slice();
    let pair_right = scratch.pair_right.as_mut_slice();
    let mut left = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::adjacency_valid(adjacency, right_degrees@.len()),
            proof::matching_valid(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@),
            left <= pair_left@.len(), pair_right@.len() == right_degrees@.len(),
            forall|l: int| left <= l < pair_left@.len() ==> pair_left@[l] == u16::MAX,
        decreases pair_left@.len() - left,
    ))]
    while left < pair_left.len() {
        let start = copy(adjacency.offsets, left);
        if copy(adjacency.offsets, left + 1) - start == 1 {
            let right = usize::from(copy(adjacency.edges, usize::from(start)));
            if copy(pair_right, right) == UNMATCHED_U16 {
                proof! { proof::edge_at(adjacency.offsets@, adjacency.edges@, left as int, start as int); }
                crate::optimal::matching::match_free(adjacency, left, right, pair_left, pair_right);
            }
        }
        left += 1;
    }

    build_transpose(
        adjacency,
        right_degrees,
        &mut scratch.transpose_offsets,
        &mut scratch.transpose_edges,
        &mut scratch.write_offsets,
    );

    counting_sort_right_order_u16(
        right_degrees,
        &mut scratch.right_order,
        &mut scratch.right_degree_counts,
    );
    let order = scratch.right_order.as_slice();
    let transpose_offsets = scratch.transpose_offsets.as_slice();
    let transpose_edges = scratch.transpose_edges.as_slice();
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::adjacency_valid(adjacency, right_degrees@.len()),
            proof::matching_valid(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@),
            pair_right@.len() == right_degrees@.len(),
            proof::transposed(adjacency.offsets@, adjacency.edges@,
                transpose_offsets@, transpose_edges@, right_degrees@.len()),
            proof::sorted_rights(right_degrees@, order@), index <= order@.len(),
        decreases order@.len() - index,
    ))]
    while index < order.len() {
        let right = usize::from(copy(order, index));
        if copy(pair_right, right) == UNMATCHED_U16 {
            let mut edge = usize::from(copy(transpose_offsets, right));
            let end = usize::from(copy(transpose_offsets, right + 1));
            #[cfg_attr(verus_only, verus_spec(
                invariant proof::matching_valid(adjacency.offsets@, adjacency.edges@, pair_left@, pair_right@),
                    proof::transposed(adjacency.offsets@, adjacency.edges@,
                        transpose_offsets@, transpose_edges@, right_degrees@.len()),
                    right < pair_right@.len(), pair_right@.len() == right_degrees@.len(),
                    transpose_offsets@[right as int] <= edge <= end,
                    end == transpose_offsets@[right as int + 1],
                invariant_except_break pair_right@[right as int] == u16::MAX,
                decreases end - edge,
            ))]
            while edge < end {
                proof! { proof::transpose_at(adjacency.offsets@, adjacency.edges@,
                transpose_offsets@, transpose_edges@, right_degrees@.len(), right as int, edge as int); }
                let left = usize::from(copy(transpose_edges, edge));
                if copy(pair_left, left) == UNMATCHED_U16 {
                    crate::optimal::matching::match_free(
                        adjacency, left, right, pair_left, pair_right,
                    );
                    break;
                }
                edge += 1;
            }
        }
        index += 1;
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires order_proof::degree_domain(right_degrees@), old(right_order).valid(), old(degree_counts).valid(),
    ensures final(right_order).valid(), final(degree_counts).valid(),
        proof::sorted_rights(right_degrees@, final(right_order)@),
))]
pub(super) fn counting_sort_right_order_u16(
    right_degrees: &[u8],
    right_order: &mut ChordBuffer<u16>,
    degree_counts: &mut FixedVec<usize, IMAGE64_AXIS_LIMIT>,
) {
    let mut right = 0;
    let mut max_degree = 0usize;
    #[cfg_attr(verus_only, verus_spec(
        invariant order_proof::degree_domain(right_degrees@), right <= right_degrees@.len(), max_degree < 64,
            forall|i: int| 0 <= i < right ==> right_degrees@[i] <= max_degree,
        decreases right_degrees@.len() - right,
    ))]
    while right < right_degrees.len() {
        let degree = usize::from(copy(right_degrees, right));
        if max_degree < degree {
            max_degree = degree;
        }
        right += 1;
    }
    degree_counts.clear();
    degree_counts.resize(max_degree + 1, 0);
    let counts = degree_counts.as_mut_slice();
    let mut right = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant order_proof::degree_domain(right_degrees@), right <= right_degrees@.len(),
            counts@.len() == max_degree + 1, max_degree < 64,
            forall|i: int| 0 <= i < right_degrees@.len() ==> right_degrees@[i] <= max_degree,
            order_proof::histogram(right_degrees@, counts@, right as int),
        decreases right_degrees@.len() - right,
    ))]
    while right < right_degrees.len() {
        proof_decl! { let ghost before = counts@; }
        let degree = usize::from(copy(right_degrees, right));
        proof! { order_proof::count_bounds(right_degrees@, right as int, degree as int, degree as int); }
        *get_mut(counts, degree) += 1;
        proof! { order_proof::histogram_step(right_degrees@, before, counts@, right as int); }
        right += 1;
    }
    let mut degree = 0;
    let mut offset = 0usize;
    proof! { order_proof::count_bounds(right_degrees@, right_degrees@.len() as int, 0, 0); }
    #[cfg_attr(verus_only, verus_spec(
        invariant order_proof::degree_domain(right_degrees@), degree <= counts@.len(),
            counts@.len() == max_degree + 1, max_degree < 64,
            forall|i: int| 0 <= i < right_degrees@.len() ==> right_degrees@[i] <= max_degree,
            order_proof::prefix_histogram(right_degrees@, counts@, degree as int),
            offset == order_proof::less(right_degrees@, right_degrees@.len() as int, degree as int),
        decreases counts@.len() - degree,
    ))]
    while degree < counts.len() {
        proof_decl! { let ghost before = counts@; }
        proof! { order_proof::count_bounds(right_degrees@, right_degrees@.len() as int, degree as int, degree as int + 1); }
        let current = copy(counts, degree);
        *get_mut(counts, degree) = offset;
        proof! { order_proof::prefix_step(right_degrees@, before, counts@, degree as int); }
        offset += current;
        degree += 1;
    }
    right_order.clear();
    right_order.resize(right_degrees.len(), 0);
    let order = right_order.as_mut_slice();
    let mut right = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant order_proof::degree_domain(right_degrees@), right <= right_degrees@.len(),
            counts@.len() == max_degree + 1, max_degree < 64,
            forall|i: int| 0 <= i < right_degrees@.len() ==> right_degrees@[i] <= max_degree,
            order_proof::scatter_counters(right_degrees@, counts@, right as int),
            order_proof::order_prefix(right_degrees@, proof::order_u32(order@), right as int),
        decreases right_degrees@.len() - right,
    ))]
    while right < right_degrees.len() {
        proof_decl! { let ghost before = counts@; let ghost order_before = order@; }
        let degree = usize::from(copy(right_degrees, right));
        let slot = get_mut(counts, degree);
        proof! { order_proof::rank_bound(right_degrees@, right as int); }
        *get_mut(order, *slot) = u16_index(right);
        *slot += 1;
        proof! { proof::order_u16_step(right_degrees@, before, counts@, order_before, order@, right as int); }
        right += 1;
    }
    proof! { proof::complete_u16_order(right_degrees@, order@); }
}
