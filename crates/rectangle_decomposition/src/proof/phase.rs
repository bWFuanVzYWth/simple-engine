//! 最短根调度、配对进展与未匹配列表的组合边界。
use super::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(in crate::optimal) open spec fn ready(adjacency: &SparseAdjacencyRef, scratch: &HkScratch) -> bool {
    proof::buffers_valid(scratch) && proof::adjacency_valid(adjacency, scratch.pair_right@.len())
        && proof::matching_valid(adjacency.offsets@, adjacency.edges@, scratch.pair_left@, scratch.pair_right@)
        && proof::transposed(adjacency.offsets@, adjacency.edges@, scratch.transpose_offsets@, scratch.transpose_edges@, scratch.pair_right@.len())
        && levels_proof::reverse_arrays(scratch.transpose_offsets@, scratch.transpose_edges@, scratch.pair_left@, scratch.pair_right@)
        && proof::free_lefts(scratch.pair_left@, scratch.unmatched_lefts@)
        && scratch.right_distance@.len() == scratch.pair_right@.len()
        && scratch.reachable_left@.len() == scratch.pair_left@.len()
        && scratch.reachable_right@.len() == scratch.pair_right@.len()
}

#[verifier::opaque]
pub(in crate::optimal) open spec fn completed(adjacency: &SparseAdjacencyRef, scratch: &HkScratch) -> bool {
    ready(adjacency, scratch) && (scratch.unmatched_lefts@.len() == 0
        || levels_proof::global_closed(scratch.transpose_offsets@, scratch.transpose_edges@, scratch.pair_left@,
            scratch.right_distance@, scratch.shortest_roots@, u16::MAX)
            && levels_proof::distance_witness(scratch.transpose_offsets@, scratch.transpose_edges@, scratch.pair_left@, scratch.pair_right@, scratch.right_distance@)
            && (forall|r: int| 0 <= r < scratch.pair_right@.len() && scratch.pair_right@[r] == u16::MAX ==> scratch.right_distance@[r] == 0)
            && (forall|r: int| 0 <= r < scratch.pair_right@.len() && scratch.right_distance@[r] != u16::MAX ==> scratch.right_distance@[r] < scratch.pair_right@.len()))
}

#[verifier::opaque]
pub(in crate::optimal) open spec fn certificate(offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, depth: u16) -> bool {
    path_proof::domain(offsets, edges, lefts, rights, distance, depth)
        && proof::transposed(offsets, edges, transpose_offsets, transpose_edges, rights.len())
        && levels_proof::distance_witness(transpose_offsets, transpose_edges, lefts, rights, distance)
        && levels_proof::global_closed(transpose_offsets, transpose_edges, lefts, distance, roots, depth)
        && levels_proof::roots_sound(transpose_offsets, transpose_edges, lefts, distance, roots, depth)
        && levels_proof::roots_free(roots, lefts) && roots.len() > 0
}

pub(in crate::optimal) proof fn levels_certificate(adjacency: &SparseAdjacencyRef, scratch: &HkScratch, depth: u16)
    requires ready(adjacency, scratch), depth < scratch.pair_right@.len(), scratch.shortest_roots@.len() > 0,
        levels_proof::distance_witness(scratch.transpose_offsets@, scratch.transpose_edges@, scratch.pair_left@, scratch.pair_right@, scratch.right_distance@),
        levels_proof::roots_sound(scratch.transpose_offsets@, scratch.transpose_edges@, scratch.pair_left@, scratch.right_distance@, scratch.shortest_roots@, depth),
        levels_proof::global_closed(scratch.transpose_offsets@, scratch.transpose_edges@, scratch.pair_left@, scratch.right_distance@, scratch.shortest_roots@, depth),
        levels_proof::roots_free(scratch.shortest_roots@, scratch.pair_left@),
        forall|r: int| 0 <= r < scratch.pair_right@.len() && scratch.pair_right@[r] == u16::MAX ==> scratch.right_distance@[r] == 0,
        forall|r: int| 0 <= r < scratch.right_distance@.len() && scratch.right_distance@[r] != u16::MAX ==> scratch.right_distance@[r] < scratch.pair_right@.len(),
    ensures certificate(adjacency.offsets@, adjacency.edges@, scratch.transpose_offsets@, scratch.transpose_edges@,
        scratch.pair_left@, scratch.pair_right@, scratch.right_distance@, scratch.shortest_roots@, depth),
{
    reveal(certificate); reveal(path_proof::domain);
}

pub(in crate::optimal) proof fn certificate_fields(offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, depth: u16)
    requires certificate(offsets, edges, transpose_offsets, transpose_edges, lefts, rights, distance, roots, depth),
    ensures path_proof::domain(offsets, edges, lefts, rights, distance, depth), levels_proof::roots_free(roots, lefts),
        roots.len() > 0, proof::matching_valid(offsets, edges, lefts, rights), lefts.len() <= 1953, rights.len() <= 1953,
{
    reveal(certificate);
    path_proof::unmodified(offsets, edges, lefts, rights, distance, depth);
}

pub(in crate::optimal) proof fn first_failure(offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, depth: u16, seen: Seq<bool>)
    requires certificate(offsets, edges, transpose_offsets, transpose_edges, lefts, rights, distance, roots, depth),
        path_proof::state(offsets, edges, lefts, rights, distance, depth, Seq::new(lefts.len(), |_: int| false), seen, Seq::empty(), roots[0] as int, true),
    ensures false,
{
    reveal(certificate);
    assert(roots.contains(roots[0]));
    path_proof::first_root_succeeds(offsets, edges, transpose_offsets, transpose_edges, lefts, rights, distance, depth, roots, seen, roots[0] as int);
}

pub(in crate::optimal) proof fn preserved_transitive(first: Seq<u16>, middle: Seq<u16>, last: Seq<u16>)
    requires proof::matched_preserved(first, middle), proof::matched_preserved(middle, last),
    ensures proof::matched_preserved(first, last),
{}

pub(in crate::optimal) proof fn count_bound(pairs: Seq<u16>, end: int)
    requires 0 <= end <= pairs.len(),
    ensures proof::matched_count(pairs, end) <= end,
    decreases end,
{
    if end > 0 { count_bound(pairs, end - 1); }
}

pub(in crate::optimal) open spec fn ordered(ids: Seq<u16>) -> bool {
    forall|i: int, j: int| 0 <= i < j < ids.len() ==> #[trigger] ids[i] < #[trigger] ids[j]
}

proof fn filter_ordered(ids: Seq<u16>, pred: spec_fn(u16) -> bool)
    requires ordered(ids), ids.no_duplicates(),
    ensures ordered(ids.filter(pred)), ids.filter(pred).no_duplicates(),
        forall|id: u16| #[trigger] ids.filter(pred).contains(id) <==> ids.contains(id) && pred(id),
    decreases ids.len(),
{
    if ids.len() > 0 {
        let prefix = ids.drop_last();
        let last = ids.last();
        assert(ordered(prefix));
        filter_ordered(prefix, pred);
        assert(ids =~= prefix.push(last));
        prefix.lemma_filter_push(last, pred);
        let out = ids.filter(pred);
        assert forall|id: u16| #[trigger] out.contains(id) <==> ids.contains(id) && pred(id) by {}
        if pred(last) {
            assert(!prefix.contains(last));
            assert forall|i: int, j: int| 0 <= i < j < out.len() implies #[trigger] out[i] < #[trigger] out[j] by {
                if j == out.len() - 1 {
                    assert(prefix.filter(pred).contains(out[i]));
                    assert(prefix.contains(out[i]));
                    let k = choose|k: int| 0 <= k < prefix.len() && prefix[k] == out[i];
                    assert(ids[k] < ids[ids.len() as int - 1]);
                }
            }
        }
    } else {
        reveal_with_fuel(Seq::filter, 1);
    }
}

pub(in crate::optimal) proof fn filtered_free(before: Seq<u16>, after: Seq<u16>, ids: Seq<u16>)
    requires proof::free_lefts(before, ids), proof::matched_preserved(before, after), before.len() <= 1953,
    ensures forall|i: int| 0 <= i < ids.len() ==> #[trigger] ids[i] < after.len(),
        proof::free_lefts(after, ids.filter(|id: u16| after[id as int] == u16::MAX)),
{
    let pred = |id: u16| after[id as int] == u16::MAX;
    let out = ids.filter(pred);
    filter_ordered(ids, pred);
    assert forall|i: int| 0 <= i < out.len() implies (#[trigger] out[i]) < after.len() && after[out[i] as int] == u16::MAX by {
        assert(out.contains(out[i]));
        assert(ids.contains(out[i]));
        let k = choose|k: int| 0 <= k < ids.len() && ids[k] == out[i];
    }
    assert forall|left: int| 0 <= left < after.len() implies (after[left] == u16::MAX <==> #[trigger] out.contains(left as u16)) by {
        if after[left] == u16::MAX { assert(before[left] == u16::MAX); }
    }
}
}
