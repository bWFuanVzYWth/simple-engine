//! 阶段入口配对与当前配对的差异就是已占用路径；不保存路径族或执行快照。
use super::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(in crate::optimal) open spec fn dead_rows(offsets: Seq<u16>, edges: Seq<u16>, initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16, seen: Seq<bool>) -> bool {
    forall|left: int, right: int| #[trigger] proof::edge_in(offsets, edges, left, right)
        && seen[left] && lefts[left] == initial_left[left] && path_proof::allowed(initial_left, distance, depth, left, right)
        ==> rights[right] != initial_right[right] || initial_right[right] != u16::MAX && seen[initial_right[right] as int]
}

#[verifier::opaque]
pub(in crate::optimal) open spec fn relation(offsets: Seq<u16>, edges: Seq<u16>, initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16, seen: Seq<bool>) -> bool {
    path_proof::domain(offsets, edges, initial_left, initial_right, distance, depth)
        && path_proof::domain(offsets, edges, lefts, rights, distance, depth)
        && proof::matched_preserved(initial_left, lefts) && proof::matched_preserved(initial_right, rights)
        && seen.len() == lefts.len()
        && (forall|l: int| 0 <= l < lefts.len() && lefts[l] != initial_left[l] ==> #[trigger] seen[l]
            && lefts[l] != u16::MAX && distance[lefts[l] as int] as int + 1 == path_proof::rank(initial_left, distance, depth, l))
        && (forall|r: int| 0 <= r < rights.len() && rights[r] != initial_right[r] ==> rights[r] != u16::MAX
            && seen[#[trigger] rights[r] as int] && (initial_right[r] != u16::MAX ==> seen[initial_right[r] as int]))
        && dead_rows(offsets, edges, initial_left, initial_right, lefts, rights, distance, depth, seen)
}

pub(in crate::optimal) proof fn initial(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16)
    requires path_proof::domain(offsets, edges, lefts, rights, distance, depth),
    ensures relation(offsets, edges, lefts, rights, lefts, rights, distance, depth, Seq::new(lefts.len(), |_: int| false)),
{ reveal(relation); reveal(path_proof::domain); }

pub(in crate::optimal) proof fn path_fresh(offsets: Seq<u16>, edges: Seq<u16>, initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16, before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, start: int, terminal: u16)
    requires relation(offsets, edges, initial_left, initial_right, lefts, rights, distance, depth, before),
        path_proof::state(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, true),
        proof::path_links(offsets, edges, lefts, rights, stack, terminal), stack.len() == depth as int + 1,
    ensures forall|i: int| 0 <= i < stack.len() ==> initial_left[(#[trigger] stack[i]).left as int] == lefts[stack[i].left as int]
        && initial_right[proof::path_right(edges, stack, terminal, i) as int] == rights[proof::path_right(edges, stack, terminal, i) as int]
        && distance[proof::path_right(edges, stack, terminal, i) as int] == depth as int - i
        && distance[proof::path_right(edges, stack, terminal, i) as int] as int + 1
            == path_proof::rank(initial_left, distance, depth, stack[i].left as int),
{
    hide(proof::edge_in); hide(proof::rows_valid);
    reveal(relation); reveal(path_proof::state); reveal(path_proof::domain);
    proof::complete_path_links(offsets, edges, lefts, rights, stack, terminal);
    assert forall|i: int| 0 <= i < stack.len() implies initial_left[(#[trigger] stack[i]).left as int] == lefts[stack[i].left as int]
        && initial_right[proof::path_right(edges, stack, terminal, i) as int] == rights[proof::path_right(edges, stack, terminal, i) as int]
        && distance[proof::path_right(edges, stack, terminal, i) as int] == depth as int - i
        && distance[proof::path_right(edges, stack, terminal, i) as int] as int + 1
            == path_proof::rank(initial_left, distance, depth, stack[i].left as int) by {
        proof::path_entry(offsets, edges, lefts, rights, stack, terminal, i);
        let l = stack[i].left as int;
        let r = proof::path_right(edges, stack, terminal, i) as int;
        assert(!before[l]);
        assert(lefts[l] == initial_left[l]);
        if i + 1 < stack.len() {
            let next = stack[i + 1].left as int;
            assert(!before[next]);
            assert(rights[r] == next);
            assert(initial_right[r] == rights[r]);
            assert(lefts[next] == r);
        } else {
            assert(initial_right[r] == u16::MAX);
        }
    }
}

pub(in crate::optimal) proof fn failed(offsets: Seq<u16>, edges: Seq<u16>, initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16, before: Seq<bool>, seen: Seq<bool>, start: int)
    requires relation(offsets, edges, initial_left, initial_right, lefts, rights, distance, depth, before),
        path_proof::state(offsets, edges, lefts, rights, distance, depth, before, seen, Seq::empty(), start, true),
    ensures relation(offsets, edges, initial_left, initial_right, lefts, rights, distance, depth, seen),
{
    hide(proof::edge_in); hide(proof::rows_valid);
    reveal(relation); reveal(path_proof::state); reveal(path_proof::domain);
    assert forall|left: int, right: int| #[trigger] proof::edge_in(offsets, edges, left, right)
        && seen[left] && lefts[left] == initial_left[left] && path_proof::allowed(initial_left, distance, depth, left, right)
        implies rights[right] != initial_right[right] || initial_right[right] != u16::MAX && seen[initial_right[right] as int] by {
        path_proof::neighbor(offsets, edges, lefts, rights, distance, depth, left, right);
        if before[left] {
            assert(rights[right] != initial_right[right] || initial_right[right] != u16::MAX && before[initial_right[right] as int]);
        } else {
            assert(!path_proof::contains(Seq::empty(), left));
            assert(path_proof::closed(offsets, edges, lefts, rights, distance, depth, seen, left, offsets[left + 1] as int, true));
            path_proof::closed_neighbor(offsets, edges, lefts, rights, distance, depth, seen, left, right);
        }
    }
}

#[verifier::opaque]
pub(in crate::optimal) open spec fn changes(initial_left: Seq<u16>, initial_right: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    after_left: Seq<u16>, after_right: Seq<u16>, edges: Seq<u16>, stack: Seq<DfsFrame>, terminal: u16) -> bool {
    (forall|l: int| 0 <= l < lefts.len() ==> (#[trigger] after_left[l] != initial_left[l]
        <==> lefts[l] != initial_left[l] || proof::path_has_left(stack, 0, l)))
        && forall|r: int| 0 <= r < rights.len() ==> (#[trigger] after_right[r] != initial_right[r]
            <==> rights[r] != initial_right[r] || proof::path_has_right(edges, stack, terminal, 0, r))
}

proof fn changed_slots(offsets: Seq<u16>, edges: Seq<u16>, initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, after_left: Seq<u16>, after_right: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, start: int, terminal: u16)
    requires relation(offsets, edges, initial_left, initial_right, lefts, rights, distance, depth, before),
        path_proof::state(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, true),
        proof::path_links(offsets, edges, lefts, rights, stack, terminal), stack.len() == depth as int + 1,
        proof::path_state(lefts, rights, after_left, after_right, edges, stack, terminal, 0),
    ensures changes(initial_left, initial_right, lefts, rights, after_left, after_right, edges, stack, terminal),
{
    hide(proof::edge_in); hide(proof::rows_valid); hide(proof::path_right);
    reveal(changes); reveal(relation); reveal(path_proof::domain);
    path_fresh(offsets, edges, initial_left, initial_right, lefts, rights, distance, depth, before, seen, stack, start, terminal);
    proof::complete_path_links(offsets, edges, lefts, rights, stack, terminal);
    assert forall|l: int| 0 <= l < lefts.len() implies (#[trigger] after_left[l] != initial_left[l]
        <==> lefts[l] != initial_left[l] || proof::path_has_left(stack, 0, l)) by {
        if proof::path_has_left(stack, 0, l) {
            let i = choose|i: int| 0 <= i < stack.len() && (#[trigger] stack[i]).left == l;
            proof::path_entry(offsets, edges, lefts, rights, stack, terminal, i);
            assert(after_left[l] == proof::path_right(edges, stack, terminal, i));
            if i > 0 {
                proof::path_entry(offsets, edges, lefts, rights, stack, terminal, i - 1);
                assert(lefts[l] == proof::path_right(edges, stack, terminal, i - 1));
                assert(proof::path_right(edges, stack, terminal, i - 1) != proof::path_right(edges, stack, terminal, i));
            }
        }
    }
    assert forall|r: int| 0 <= r < rights.len() implies (#[trigger] after_right[r] != initial_right[r]
        <==> rights[r] != initial_right[r] || proof::path_has_right(edges, stack, terminal, 0, r)) by {
        if proof::path_has_right(edges, stack, terminal, 0, r) {
            let i = choose|i: int| 0 <= i < stack.len() && #[trigger] proof::path_right(edges, stack, terminal, i) == r;
            proof::path_entry(offsets, edges, lefts, rights, stack, terminal, i);
            assert(after_right[r] == stack[i].left);
            if i + 1 < stack.len() { assert(stack[i].left != stack[i + 1].left); }
        }
    }
}

pub(in crate::optimal) proof fn committed(offsets: Seq<u16>, edges: Seq<u16>, initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, after_left: Seq<u16>, after_right: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, start: int, terminal: u16)
    requires relation(offsets, edges, initial_left, initial_right, lefts, rights, distance, depth, before),
        path_proof::state(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, true),
        proof::path_links(offsets, edges, lefts, rights, stack, terminal), stack.len() == depth as int + 1,
        proof::path_state(lefts, rights, after_left, after_right, edges, stack, terminal, 0),
        proof::matching_valid(offsets, edges, after_left, after_right),
        proof::matched_preserved(lefts, after_left), proof::matched_preserved(rights, after_right),
    ensures relation(offsets, edges, initial_left, initial_right, after_left, after_right, distance, depth, seen),
{
    hide(proof::edge_in); hide(proof::rows_valid); hide(proof::path_right);
    reveal(relation); reveal(changes); reveal(path_proof::state); reveal(path_proof::domain);
    path_fresh(offsets, edges, initial_left, initial_right, lefts, rights, distance, depth, before, seen, stack, start, terminal);
    changed_slots(offsets, edges, initial_left, initial_right, lefts, rights, after_left, after_right, distance, depth, before, seen, stack, start, terminal);
    phase_proof::preserved_transitive(initial_left, lefts, after_left);
    phase_proof::preserved_transitive(initial_right, rights, after_right);
    path_proof::after_commit(offsets, edges, lefts, rights, distance, depth, after_left, after_right);
    assert forall|l: int| 0 <= l < after_left.len() && after_left[l] != initial_left[l] implies #[trigger] seen[l]
        && after_left[l] != u16::MAX && distance[after_left[l] as int] as int + 1 == path_proof::rank(initial_left, distance, depth, l) by {
        if proof::path_has_left(stack, 0, l) {
            let i = choose|i: int| 0 <= i < stack.len() && (#[trigger] stack[i]).left == l;
            assert(after_left[l] == proof::path_right(edges, stack, terminal, i));
        } else {
            assert(after_left[l] == lefts[l]);
        }
    }
    assert forall|r: int| 0 <= r < after_right.len() && after_right[r] != initial_right[r] implies after_right[r] != u16::MAX
        && seen[#[trigger] after_right[r] as int] && (initial_right[r] != u16::MAX ==> seen[initial_right[r] as int]) by {
        if proof::path_has_right(edges, stack, terminal, 0, r) {
            let i = choose|i: int| 0 <= i < stack.len() && #[trigger] proof::path_right(edges, stack, terminal, i) == r;
            proof::path_entry(offsets, edges, lefts, rights, stack, terminal, i);
            assert(after_right[r] == stack[i].left);
            if i + 1 < stack.len() { assert(initial_right[r] == stack[i + 1].left); }
            else { assert(initial_right[r] == u16::MAX); }
        } else {
            assert(after_right[r] == rights[r]);
        }
    }
    dead_committed(offsets, edges, initial_left, initial_right, lefts, rights, after_left, after_right, distance, depth, before, seen, stack, start, terminal);
}
pub(in crate::optimal) proof fn dead_committed(offsets: Seq<u16>, edges: Seq<u16>, initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, after_left: Seq<u16>, after_right: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, start: int, terminal: u16)
    requires relation(offsets, edges, initial_left, initial_right, lefts, rights, distance, depth, before),
        path_proof::state(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, true),
        proof::path_links(offsets, edges, lefts, rights, stack, terminal), stack.len() == depth as int + 1,
        proof::path_state(lefts, rights, after_left, after_right, edges, stack, terminal, 0),
        proof::matching_valid(offsets, edges, after_left, after_right),
        proof::matched_preserved(lefts, after_left), proof::matched_preserved(rights, after_right),
    ensures dead_rows(offsets, edges, initial_left, initial_right, after_left, after_right, distance, depth, seen),
{
    hide(proof::edge_in); hide(proof::rows_valid); hide(proof::path_right);
    reveal(relation); reveal(changes); reveal(path_proof::state); reveal(path_proof::domain);
    changed_slots(offsets, edges, initial_left, initial_right, lefts, rights, after_left, after_right, distance, depth, before, seen, stack, start, terminal);
    assert forall|left: int, right: int| #[trigger] proof::edge_in(offsets, edges, left, right)
        && seen[left] && after_left[left] == initial_left[left] && path_proof::allowed(initial_left, distance, depth, left, right)
        implies after_right[right] != initial_right[right] || initial_right[right] != u16::MAX && seen[initial_right[right] as int] by {
        path_proof::neighbor(offsets, edges, lefts, rights, distance, depth, left, right);
        assert(!proof::path_has_left(stack, 0, left));
        assert(lefts[left] == initial_left[left]);
        if after_right[right] == initial_right[right] {
            assert(rights[right] == initial_right[right]);
            if before[left] {
                assert(initial_right[right] != u16::MAX && before[initial_right[right] as int]);
            } else {
                assert(!path_proof::contains(stack, left)) by {
                    if path_proof::contains(stack, left) {
                        let i = choose|i: int| 0 <= i < stack.len() && path_proof::nodes(stack)[i] == left as u16;
                        assert(stack[i].left == left);
                        assert(proof::path_has_left(stack, 0, left));
                    }
                }
                assert(path_proof::closed(offsets, edges, lefts, rights, distance, depth, seen, left, offsets[left + 1] as int, true));
                path_proof::closed_neighbor(offsets, edges, lefts, rights, distance, depth, seen, left, right);
            }
        }
    }
}
}
