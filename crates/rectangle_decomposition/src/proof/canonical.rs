//! 最大独立集的极端选择由图决定，不依赖所用最大匹配或旧缓冲内容。
use super::*;

verus! {
broadcast use { vstd::seq_lib::group_seq_properties, vstd::set_lib::group_set_properties };

proof fn arc_forced(horizontal: Seq<crate::optimal::types::ValuedChord>, vertical: Seq<crate::optimal::types::ValuedChord>,
    offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    other_left: Set<int>, other_right: Set<int>, from: int, target: int)
    requires proof::matching_valid(offsets, edges, lefts, rights),
        proof::transposed(offsets, edges, transpose_offsets, transpose_edges, rights.len()),
        crate::optimal::graph::proof::graph_exact(horizontal, vertical, offsets, edges), vertical.len() == rights.len(),
        proof::index_independent(horizontal, vertical, other_left, other_right),
        cover_proof::saturated(lefts, rights, other_left, other_right), other_right.contains(from),
        levels_proof::reverse_arc(transpose_offsets, transpose_edges, lefts, from, target),
    ensures other_right.contains(target),
{
    hide(proof::rows_valid); hide(proof::edge_in); hide(proof::owner); hide(proof::transpose_prefix);
    let slot = choose|slot: int| transpose_offsets[from] <= slot < transpose_offsets[from + 1]
        && (#[trigger] transpose_edges[slot]) < lefts.len() && lefts[transpose_edges[slot] as int] == target;
    let left = transpose_edges[slot] as int;
    proof::transpose_at(offsets, edges, transpose_offsets, transpose_edges, rights.len(), from, slot);
    assert(crate::optimal::graph::proof::closed_conflict(horizontal[left], vertical[from]));
    assert(!other_left.contains(left));
    assert(other_left.contains(left) != other_right.contains(lefts[left] as int));
}

proof fn walk_forced(horizontal: Seq<crate::optimal::types::ValuedChord>, vertical: Seq<crate::optimal::types::ValuedChord>,
    offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    other_left: Set<int>, other_right: Set<int>, path: Seq<int>, end: int)
    requires proof::matching_valid(offsets, edges, lefts, rights),
        proof::transposed(offsets, edges, transpose_offsets, transpose_edges, rights.len()),
        crate::optimal::graph::proof::graph_exact(horizontal, vertical, offsets, edges), vertical.len() == rights.len(),
        proof::index_independent(horizontal, vertical, other_left, other_right),
        cover_proof::saturated(lefts, rights, other_left, other_right),
        levels_proof::reverse_walk(transpose_offsets, transpose_edges, lefts, rights, path, path.last(), path.len() as int - 1),
        0 <= end < path.len(),
    ensures other_right.contains(path[end]),
    decreases end,
{
    hide(proof::rows_valid); hide(proof::edge_in); hide(proof::owner); hide(proof::transpose_prefix);
    hide(proof::transposed); hide(levels_proof::reverse_arc);
    if end > 0 {
        walk_forced(horizontal, vertical, offsets, edges, transpose_offsets, transpose_edges, lefts, rights, other_left, other_right, path, end - 1);
        arc_forced(horizontal, vertical, offsets, edges, transpose_offsets, transpose_edges, lefts, rights, other_left, other_right, path[end - 1], path[end]);
    }
}

/// 当前实现的全 V 特例与失败 BFS 极端闭包共同给出规范化选择。
pub(in crate::optimal) proof fn from_completed(horizontal: Seq<crate::optimal::types::ValuedChord>, vertical: Seq<crate::optimal::types::ValuedChord>,
    adjacency: &SparseAdjacencyRef, scratch: &HkScratch)
    requires phase_proof::completed(adjacency, scratch),
        crate::optimal::graph::proof::graph_exact(horizontal, vertical, adjacency.offsets@, adjacency.edges@),
        vertical.len() == scratch.pair_right@.len(),
        proof::marks_certificate(adjacency.offsets@, adjacency.edges@, scratch.pair_left@, scratch.pair_right@, scratch.reachable_left@, scratch.reachable_right@),
        crate::chord_graph::proof::marks_match(scratch.pair_left@, scratch.right_distance@, scratch.unmatched_lefts@.len() == 0,
            scratch.reachable_left@, scratch.reachable_right@),
    ensures proof::canonical_selection(horizontal, vertical,
        proof::flag_ids(scratch.reachable_left@, true, scratch.reachable_left@.len() as int),
        proof::flag_ids(scratch.reachable_right@, false, scratch.reachable_right@.len() as int)),
{
    hide(proof::rows_valid); hide(proof::edge_in); hide(proof::owner); hide(proof::transpose_prefix);
    hide(cover_proof::native_pairs);
    reveal(phase_proof::completed);
    let lefts = scratch.pair_left@;
    let rights = scratch.pair_right@;
    let selected = scratch.reachable_left@;
    let excluded = scratch.reachable_right@;
    let distance = scratch.right_distance@;
    let left_ids = proof::flag_ids(selected, true, selected.len() as int);
    let right_ids = proof::flag_ids(excluded, false, excluded.len() as int);
    proof::maximum_from_marks(horizontal, vertical, adjacency.offsets@, adjacency.edges@, lefts, rights, selected, excluded);
    proof::maximum_sets(horizontal, vertical, left_ids, right_ids);
    proof::flags_set(selected, true); proof::flags_set(excluded, false);
    proof::index_set_view(left_ids); proof::index_set_view(right_ids);
    cover_proof::vertices_view(lefts.len()); cover_proof::vertices_view(rights.len());
    let chosen_left = proof::index_set(left_ids);
    let chosen_right = proof::index_set(right_ids);
    if scratch.unmatched_lefts@.len() == 0 {
        assert(chosen_left =~= Set::<int>::empty());
        assert(chosen_right =~= cover_proof::vertices(rights.len()));
        assert(left_ids.len() == 0);
    } else {
        cover_proof::dual_certificate(adjacency.offsets@, adjacency.edges@, lefts, rights, selected, excluded);
        let unmatched = scratch.unmatched_lefts@[0] as int;
        let matched_left = cover_proof::matched_lefts(lefts);
        assert(matched_left.subset_of(cover_proof::vertices(lefts.len())));
        assert(!matched_left.contains(unmatched));
        matched_left.lemma_subset_not_in_lt(cover_proof::vertices(lefts.len()), unmatched);
        assert(chosen_left.len() + chosen_right.len() > vertical.len());
        assert(!proof::index_maximum(horizontal, vertical, Set::empty(), proof::index_domain(vertical.len())));
        assert forall|other_left: Set<int>, other_right: Set<int>| #[trigger] proof::index_maximum(horizontal, vertical, other_left, other_right)
            implies chosen_right.subset_of(other_right) by {
            assert(other_left.len() + other_right.len() == chosen_left.len() + chosen_right.len());
            assert(other_left.subset_of(cover_proof::vertices(lefts.len())));
            assert(other_right.subset_of(cover_proof::vertices(rights.len())));
            assert forall|l: int| 0 <= l < lefts.len() && lefts[l] != u16::MAX
                implies !(other_left.contains(l) && other_right.contains(#[trigger] lefts[l] as int)) by {
                assert(proof::edge_in(adjacency.offsets@, adjacency.edges@, l, lefts[l] as int));
                assert(crate::optimal::graph::proof::closed_conflict(horizontal[l], vertical[lefts[l] as int]));
            }
            cover_proof::equal_selection_saturated(adjacency.offsets@, adjacency.edges@, lefts, rights, selected, excluded, other_left, other_right);
            assert forall|r: int| chosen_right.contains(r) implies #[trigger] other_right.contains(r) by {
                assert(0 <= r < rights.len() && distance[r] != u16::MAX);
                levels_proof::distance_has_walk(scratch.transpose_offsets@, scratch.transpose_edges@, lefts, rights, distance, r);
                let path = choose|path: Seq<int>| levels_proof::reverse_walk(scratch.transpose_offsets@, scratch.transpose_edges@, lefts, rights,
                    path, r, distance[r] as int);
                walk_forced(horizontal, vertical, adjacency.offsets@, adjacency.edges@, scratch.transpose_offsets@, scratch.transpose_edges@,
                    lefts, rights, other_left, other_right, path, path.len() as int - 1);
            }
        }
    }
}
}
