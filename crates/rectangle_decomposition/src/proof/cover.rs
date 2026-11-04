//! 最终 BFS 闭包与等大顶点覆盖的连接。
use super::*;

verus! {
broadcast use { vstd::seq_lib::group_seq_properties, vstd::set_lib::group_set_properties,
    vstd::set::Set::lemma_map_contains };

pub(in crate::optimal) open spec fn vertices(size: nat) -> Set<int> { Seq::new(size, |i: int| i).to_set() }
pub(in crate::optimal) open spec fn chosen(flags: Seq<bool>, value: bool) -> Set<int> { vertices(flags.len()).filter(|i: int| flags[i] == value) }
pub(in crate::optimal) open spec fn matched_lefts(lefts: Seq<u16>) -> Set<int> { vertices(lefts.len()).filter(|l: int| lefts[l] != u16::MAX) }
pub(in crate::optimal) open spec fn native_pairs(lefts: Seq<u16>) -> Set<(int, int)> { matched_lefts(lefts).map(|l: int| (l, lefts[l] as int)) }

pub(in crate::optimal) proof fn pair_members(lefts: Seq<u16>)
    ensures forall|pair: (int, int)| #[trigger] native_pairs(lefts).contains(pair) <==>
        0 <= pair.0 < lefts.len() && lefts[pair.0] != u16::MAX && pair.1 == lefts[pair.0] as int,
{
    vertices_view(lefts.len());
    assert forall|pair: (int, int)| #[trigger] native_pairs(lefts).contains(pair) <==>
        0 <= pair.0 < lefts.len() && lefts[pair.0] != u16::MAX && pair.1 == lefts[pair.0] as int by {
        if native_pairs(lefts).contains(pair) {
            let l = choose|l: int| matched_lefts(lefts).contains(l) && (l, lefts[l] as int) == pair;
        }
        if 0 <= pair.0 < lefts.len() && lefts[pair.0] != u16::MAX && pair.1 == lefts[pair.0] as int {
            assert(matched_lefts(lefts).contains(pair.0));
        }
    }
}

pub(in crate::optimal) proof fn vertices_view(size: nat)
    ensures vertices(size).len() == size, forall|i: int| #[trigger] vertices(size).contains(i) <==> 0 <= i < size,
{
    let sequence = Seq::new(size, |i: int| i);
    assert(sequence.no_duplicates());
    sequence.unique_seq_to_set();
    assert forall|i: int| #[trigger] vertices(size).contains(i) <==> 0 <= i < size by {
        if vertices(size).contains(i) {
            let j = choose|j: int| 0 <= j < sequence.len() && sequence[j] == i;
        }
        if 0 <= i < size { assert(sequence[i] == i); }
    }
}

pub(in crate::optimal) proof fn collect_certificate(adjacency: &SparseAdjacencyRef, scratch: &HkScratch,
    selected: Seq<bool>, excluded: Seq<bool>)
    requires phase_proof::completed(adjacency, scratch),
        crate::chord_graph::proof::marks_match(scratch.pair_left@, scratch.right_distance@,
            scratch.unmatched_lefts@.len() == 0, selected, excluded),
    ensures proof::marks_certificate(adjacency.offsets@, adjacency.edges@, scratch.pair_left@, scratch.pair_right@, selected, excluded),
{
    hide(proof::rows_valid); hide(proof::edge_in); hide(proof::owner); hide(proof::transpose_prefix);
    reveal(phase_proof::completed);
    let lefts = scratch.pair_left@;
    let rights = scratch.pair_right@;
    let distance = scratch.right_distance@;
    let complete = scratch.unmatched_lefts@.len() == 0;
    assert forall|left: int| 0 <= left < lefts.len() && lefts[left] == u16::MAX implies #[trigger] selected[left] by {
        if complete { assert(scratch.unmatched_lefts@.contains(left as u16)); }
    }
    assert forall|left: int, right: int| #[trigger] proof::edge_in(adjacency.offsets@, adjacency.edges@, left, right)
        implies !(selected[left] && !excluded[right]) by {
        proof::adjacency_edge(adjacency, rights.len(), left, right);
        if !complete && selected[left] && !excluded[right] {
            proof::transpose_edge(adjacency.offsets@, adjacency.edges@, scratch.transpose_offsets@, scratch.transpose_edges@,
                rights.len(), left, right);
            let slot = choose|slot: int| scratch.transpose_offsets@[right] <= slot < scratch.transpose_offsets@[right + 1]
                && (#[trigger] scratch.transpose_edges@[slot]) == left;
            assert(levels_proof::closed_slot(scratch.transpose_edges@, lefts, distance, scratch.shortest_roots@, u16::MAX, right, slot));
        }
    }
}

/// 从实际配对与标记构造有限对偶证书，包含精确基数而非最优性前提。
pub(in crate::optimal) proof fn dual_certificate(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, selected: Seq<bool>, excluded: Seq<bool>)
    requires proof::marks_certificate(offsets, edges, lefts, rights, selected, excluded),
    ensures crate::chord_graph::proof::matching(native_pairs(lefts)),
        native_pairs(lefts).len() == matched_lefts(lefts).len(),
        vertices(lefts.len()).difference(chosen(selected, true)).len() + vertices(rights.len()).difference(chosen(excluded, false)).len() == native_pairs(lefts).len(),
        chosen(selected, true).len() + chosen(excluded, false).len() + native_pairs(lefts).len() == lefts.len() + rights.len(),
        chosen(selected, true).subset_of(vertices(lefts.len())), chosen(excluded, false).subset_of(vertices(rights.len())),
        forall|pair: (int, int)| #[trigger] native_pairs(lefts).contains(pair) ==> vertices(lefts.len()).contains(pair.0) && vertices(rights.len()).contains(pair.1),
        crate::chord_graph::proof::independent(native_pairs(lefts), chosen(selected, true), chosen(excluded, false)),
{
    hide(proof::rows_valid); hide(proof::edge_in);
    vertices_view(lefts.len()); vertices_view(rights.len());
    let left_domain = vertices(lefts.len());
    let right_domain = vertices(rights.len());
    let cover_left = left_domain.filter(|l: int| !selected[l]);
    let cover_right = right_domain.filter(|r: int| excluded[r]);
    let matched_left = left_domain.filter(|l: int| lefts[l] != u16::MAX);
    let right_to_left = |r: int| rights[r] as int;
    let right_image = cover_right.map(right_to_left);
    let pair_of = |l: int| (l, lefts[l] as int);
    let pairs = matched_left.map(pair_of);
    assert(cover_right.injective_on(right_to_left)) by {
        assert forall|a: int, b: int| cover_right.contains(a) && cover_right.contains(b)
            && right_to_left(a) == right_to_left(b) implies a == b by {
            assert(rights[a] != u16::MAX && rights[b] != u16::MAX);
            assert(lefts[rights[a] as int] == a && lefts[rights[b] as int] == b);
        }
    }
    vstd::set_lib::lemma_map_size(cover_right, right_image, right_to_left);
    assert(cover_left.disjoint(right_image)) by {
        assert forall|l: int| cover_left.contains(l) implies !right_image.contains(l) by {
            if right_image.contains(l) {
                let r = choose|r: int| cover_right.contains(r) && right_to_left(r) == l;
                assert(lefts[l] == r && rights[r] != u16::MAX);
                assert(selected[l] == excluded[r]);
            }
        }
    }
    assert(cover_left.union(right_image) =~= matched_left) by {
        assert forall|l: int| cover_left.union(right_image).contains(l) <==> matched_left.contains(l) by {
            if right_image.contains(l) {
                let r = choose|r: int| cover_right.contains(r) && right_to_left(r) == l;
                assert(rights[r] != u16::MAX);
                assert(lefts[l] == r);
            }
            if matched_left.contains(l) && selected[l] {
                let r = lefts[l] as int;
                assert(cover_right.contains(r));
                assert(right_to_left(r) == l);
                assert(right_image.contains(l));
            }
        }
    }
    vstd::set_lib::lemma_set_disjoint_lens(cover_left, right_image);
    assert(matched_left.injective_on(pair_of));
    vstd::set_lib::lemma_map_size(matched_left, pairs, pair_of);
    assert(crate::chord_graph::proof::matching(pairs)) by {
        assert forall|a: (int, int), b: (int, int)| #[trigger] pairs.contains(a) && #[trigger] pairs.contains(b)
            && (a.0 == b.0 || a.1 == b.1) implies a == b by {
            let l = choose|l: int| matched_left.contains(l) && pair_of(l) == a;
            let k = choose|k: int| matched_left.contains(k) && pair_of(k) == b;
            if a.1 == b.1 { assert(rights[lefts[l] as int] == l && rights[lefts[k] as int] == k); }
        }
    }
    let selected_left = chosen(selected, true);
    let selected_right = chosen(excluded, false);
    assert(left_domain.difference(selected_left) =~= cover_left);
    assert(right_domain.difference(selected_right) =~= cover_right);
    assert forall|pair: (int, int)| #[trigger] pairs.contains(pair) implies left_domain.contains(pair.0) && right_domain.contains(pair.1) by {
        let l = choose|l: int| matched_left.contains(l) && pair_of(l) == pair;
    }
    assert(crate::chord_graph::proof::independent(pairs, selected_left, selected_right)) by {
        assert forall|pair: (int, int)| #[trigger] pairs.contains(pair) implies !(selected_left.contains(pair.0) && selected_right.contains(pair.1)) by {
            let l = choose|l: int| matched_left.contains(l) && pair_of(l) == pair;
            assert(selected[l] == excluded[lefts[l] as int]);
        }
    }
    vstd::set_lib::lemma_set_difference_len(left_domain, selected_left);
    vstd::set_lib::lemma_set_difference_len(right_domain, selected_right);
    assert(left_domain.intersect(selected_left) =~= selected_left);
    assert(right_domain.intersect(selected_right) =~= selected_right);
}

/// 每个匹配对恰选一端，未匹配点全选，故任意独立选择的基数都不更大。
pub(in crate::optimal) proof fn marks_bound(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    selected: Seq<bool>, excluded: Seq<bool>, other_left: Set<int>, other_right: Set<int>)
    requires proof::marks_certificate(offsets, edges, lefts, rights, selected, excluded),
        other_left.subset_of(vertices(lefts.len())), other_right.subset_of(vertices(rights.len())),
        forall|l: int| 0 <= l < lefts.len() && lefts[l] != u16::MAX
            ==> !(other_left.contains(l) && other_right.contains(#[trigger] lefts[l] as int)),
    ensures other_left.len() + other_right.len() <= chosen(selected, true).len() + chosen(excluded, false).len(),
{
    hide(proof::rows_valid); hide(proof::edge_in);
    dual_certificate(offsets, edges, lefts, rights, selected, excluded);
    pair_members(lefts);
    let pairs = native_pairs(lefts);
    let selected_left = chosen(selected, true);
    let selected_right = chosen(excluded, false);
    assert(crate::chord_graph::proof::independent(pairs, other_left, other_right)) by {
        assert forall|pair: (int, int)| #[trigger] pairs.contains(pair) implies !(other_left.contains(pair.0) && other_right.contains(pair.1)) by {
            assert(lefts[pair.0] == pair.1);
        }
    }
    crate::chord_graph::proof::independent_maximum_from_certificate(pairs, pairs, vertices(lefts.len()), vertices(rights.len()),
        selected_left, selected_right, other_left, other_right);
}

pub(in crate::optimal) open spec fn saturated(lefts: Seq<u16>, rights: Seq<u16>, left: Set<int>, right: Set<int>) -> bool {
    (forall|l: int| 0 <= l < lefts.len() && lefts[l] == u16::MAX ==> #[trigger] left.contains(l))
        && (forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> #[trigger] right.contains(r))
        && forall|l: int| 0 <= l < lefts.len() && lefts[l] != u16::MAX
            ==> left.contains(l) != right.contains(#[trigger] lefts[l] as int)
}

/// 任意等大独立集都占满配对预算：每对恰选一端，未匹配点全选。
pub(in crate::optimal) proof fn equal_selection_saturated(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    selected: Seq<bool>, excluded: Seq<bool>, other_left: Set<int>, other_right: Set<int>)
    requires proof::marks_certificate(offsets, edges, lefts, rights, selected, excluded),
        other_left.subset_of(vertices(lefts.len())), other_right.subset_of(vertices(rights.len())),
        other_left.len() + other_right.len() == chosen(selected, true).len() + chosen(excluded, false).len(),
        forall|l: int| 0 <= l < lefts.len() && lefts[l] != u16::MAX
            ==> !(other_left.contains(l) && other_right.contains(#[trigger] lefts[l] as int)),
    ensures saturated(lefts, rights, other_left, other_right),
{
    hide(proof::rows_valid); hide(proof::edge_in);
    dual_certificate(offsets, edges, lefts, rights, selected, excluded);
    pair_members(lefts);
    vertices_view(lefts.len()); vertices_view(rights.len());
    let pairs = native_pairs(lefts);
    let cover_left = vertices(lefts.len()).difference(other_left);
    let cover_right = vertices(rights.len()).difference(other_right);
    vstd::set_lib::lemma_set_difference_len(vertices(lefts.len()), other_left);
    vstd::set_lib::lemma_set_difference_len(vertices(rights.len()), other_right);
    assert(vertices(lefts.len()).intersect(other_left) =~= other_left);
    assert(vertices(rights.len()).intersect(other_right) =~= other_right);
    assert(crate::chord_graph::proof::covers(pairs, cover_left, cover_right));
    crate::chord_graph::proof::matching_cover_tight(pairs, cover_left, cover_right);
    assert forall|l: int| 0 <= l < lefts.len() && lefts[l] == u16::MAX implies #[trigger] other_left.contains(l) by {
        if cover_left.contains(l) {
            let pair = choose|pair: (int, int)| pairs.contains(pair) && pair.0 == l;
            assert(lefts[pair.0] != u16::MAX);
        }
    }
    assert forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX implies #[trigger] other_right.contains(r) by {
        if cover_right.contains(r) {
            let pair = choose|pair: (int, int)| pairs.contains(pair) && pair.1 == r;
            assert(lefts[pair.0] == r);
            assert(rights[r] == pair.0);
        }
    }
    assert forall|l: int| 0 <= l < lefts.len() && lefts[l] != u16::MAX implies
        other_left.contains(l) != other_right.contains(#[trigger] lefts[l] as int) by {
        assert(pairs.contains((l, lefts[l] as int)));
    }
}
}
