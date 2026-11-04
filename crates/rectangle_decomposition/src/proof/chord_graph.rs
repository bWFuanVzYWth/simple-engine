//! 闭段位域及 HK 最终标记的精确语义；不假设图构建或增广正确。

use super::SparseEdge;
use vstd::prelude::*;

verus! {

pub(crate) open spec fn count_below(edges: Seq<SparseEdge>, end: int, left: int) -> nat
    recommends 0 <= end <= edges.len(),
    decreases end,
{
    if end <= 0 { 0 } else {
        count_below(edges, end - 1, left) + if edges[end - 1].left < left { 1nat } else { 0nat }
    }
}

pub(crate) open spec fn grouped_right(edges: Seq<SparseEdge>) -> bool {
    forall|i: int, j: int, k: int| #![auto]
        0 <= i < j < k < edges.len() && edges[i].right == edges[k].right
        ==> edges[j].right == edges[i].right
}

pub(crate) open spec fn starts_match(edges: Seq<SparseEdge>, starts: Seq<u32>, end: int) -> bool {
    0 <= end <= edges.len()
        && (forall|i: int| 0 <= i < end ==> (#[trigger] edges[i]).right < starts.len()
            && 0 <= starts[edges[i].right as int] <= i
            && edges[starts[edges[i].right as int] as int].right == edges[i].right)
        && (forall|right: int| 0 <= right < starts.len()
            && (forall|i: int| 0 <= i < end ==> (#[trigger] edges[i]).right != right)
            ==> #[trigger] starts[right] == 0)
}

pub(crate) proof fn new_group_unseen(edges: Seq<SparseEdge>, end: int)
    requires grouped_right(edges), 0 <= end < edges.len(),
        end == 0 || edges[end].right != edges[end - 1].right,
    ensures forall|i: int| 0 <= i < end ==> (#[trigger] edges[i]).right != edges[end].right,
{
    assert forall|i: int| 0 <= i < end implies (#[trigger] edges[i]).right != edges[end].right by {
        if edges[i].right == edges[end].right {
            assert(end > 0);
            assert(i < end - 1);
            assert(edges[end - 1].right == edges[i].right);
        }
    }
}

pub(crate) proof fn starts_step(edges: Seq<SparseEdge>, before: Seq<u32>, after: Seq<u32>, end: int)
    requires 0 <= end < edges.len(), end <= 3969, before.len() <= 1953,
        edges[end].right < before.len(), starts_match(edges, before, end), grouped_right(edges),
        end == 0 || edges[end].right != edges[end - 1].right ==>
            after == before.update(edges[end].right as int, end as u32),
        end > 0 && edges[end].right == edges[end - 1].right ==> after == before,
    ensures starts_match(edges, after, end + 1),
{
    hide(grouped_right);
    if end == 0 || edges[end].right != edges[end - 1].right { new_group_unseen(edges, end); }
    assert forall|i: int| 0 <= i < end + 1 implies (#[trigger] edges[i]).right < after.len()
        && 0 <= after[edges[i].right as int] <= i
        && edges[after[edges[i].right as int] as int].right == edges[i].right by {
        if end == 0 || edges[end].right != edges[end - 1].right {
            if i < end {
                assert(edges[i].right != edges[end].right);
            }
        }
    }
    assert forall|right: int| 0 <= right < after.len()
        && (forall|i: int| 0 <= i < end + 1 ==> (#[trigger] edges[i]).right != right)
        implies #[trigger] after[right] == 0 by {
        assert(right != edges[end].right);
    }
}

pub(crate) open spec fn counted_prefix(edges: Seq<SparseEdge>, offsets: Seq<u16>, end: int) -> bool {
    offsets.len() > 0 && 0 <= end <= edges.len() && offsets[0] == 0
        && (forall|i: int| 0 <= i < offsets.len() ==> #[trigger] offsets[i] <= end)
        && (forall|i: int| 1 <= i < offsets.len() ==> #[trigger] offsets[i] as int
            == count_below(edges, end, i) - count_below(edges, end, i - 1))
}

pub(crate) open spec fn prefix_loop(edges: Seq<SparseEdge>, offsets: Seq<u16>, next: int) -> bool {
    offsets.len() > 0 && 1 <= next <= offsets.len()
        && (forall|i: int| 0 <= i < next ==> #[trigger] offsets[i] == count_below(edges, edges.len() as int, i))
        && (forall|i: int| next <= i < offsets.len() ==> #[trigger] offsets[i] as int
            == count_below(edges, edges.len() as int, i) - count_below(edges, edges.len() as int, i - 1))
}

pub(crate) open spec fn offsets_match(edges: Seq<SparseEdge>, offsets: Seq<u16>) -> bool {
    offsets.len() > 0 && forall|i: int| 0 <= i < offsets.len()
        ==> #[trigger] offsets[i] == count_below(edges, edges.len() as int, i)
}

pub(crate) open spec fn offsets_shape(edges: Seq<SparseEdge>, offsets: Seq<u16>) -> bool {
    offsets.len() > 0 && offsets[0] == 0 && offsets[offsets.len() as int - 1] == edges.len()
        && forall|i: int| 0 <= i < offsets.len() - 1 ==> #[trigger] offsets[i] <= offsets[i + 1]
}

pub(crate) proof fn count_monotone(edges: Seq<SparseEdge>, end: int, first: int, last: int)
    requires 0 <= end <= edges.len(), first <= last,
    ensures count_below(edges, end, first) <= count_below(edges, end, last),
    decreases end,
{
    if end > 0 { count_monotone(edges, end - 1, first, last); }
}

pub(crate) proof fn count_all(edges: Seq<SparseEdge>, end: int, limit: int)
    requires 0 <= end <= edges.len(),
        forall|i: int| 0 <= i < end ==> (#[trigger] edges[i]).left < limit,
    ensures count_below(edges, end, limit) == end,
    decreases end,
{
    if end > 0 { count_all(edges, end - 1, limit); }
}

pub(crate) proof fn offsets_complete(edges: Seq<SparseEdge>, offsets: Seq<u16>)
    requires offsets_match(edges, offsets),
        forall|i: int| 0 <= i < edges.len() ==> (#[trigger] edges[i]).left as int + 1 < offsets.len(),
    ensures offsets_shape(edges, offsets),
{
    count_bound(edges, edges.len() as int, 0);
    count_all(edges, edges.len() as int, offsets.len() as int - 1);
    assert forall|i: int| 0 <= i < offsets.len() - 1 implies #[trigger] offsets[i] <= offsets[i + 1] by {
        count_monotone(edges, edges.len() as int, i, i + 1);
    }
}

pub(crate) proof fn count_bound(edges: Seq<SparseEdge>, end: int, left: int)
    requires 0 <= end <= edges.len(),
    ensures count_below(edges, end, left) <= end,
        left <= 0 ==> count_below(edges, end, left) == 0,
    decreases end,
{
    if end > 0 { count_bound(edges, end - 1, left); }
}

pub(crate) proof fn count_step(edges: Seq<SparseEdge>, before: Seq<u16>, after: Seq<u16>, end: int, slot: int)
    requires counted_prefix(edges, before, end), end < edges.len(),
        slot == edges[end].left as int + 1, 0 < slot < before.len(),
        after == before.update(slot, (before[slot] + 1) as u16), before[slot] < 65535,
    ensures counted_prefix(edges, after, end + 1),
{
    assert forall|i: int| 1 <= i < after.len() implies #[trigger] after[i] as int
        == count_below(edges, end + 1, i) - count_below(edges, end + 1, i - 1) by {
        reveal_with_fuel(count_below, 1);
    }
}

pub(crate) proof fn prefix_start(edges: Seq<SparseEdge>, offsets: Seq<u16>)
    requires counted_prefix(edges, offsets, edges.len() as int),
    ensures prefix_loop(edges, offsets, 1),
{
    count_bound(edges, edges.len() as int, 0);
}

pub(crate) proof fn prefix_step(edges: Seq<SparseEdge>, before: Seq<u16>, after: Seq<u16>, left: int)
    requires prefix_loop(edges, before, left), left < before.len(),
        after == before.update(left, (before[left] + before[left - 1]) as u16),
        before[left] + before[left - 1] <= 65535,
    ensures prefix_loop(edges, after, left + 1),
{
}

pub(crate) open spec fn equal_count(edges: Seq<SparseEdge>, end: int, left: int) -> int {
    count_below(edges, end, left + 1) - count_below(edges, end, left)
}

pub(crate) proof fn equal_count_monotone(edges: Seq<SparseEdge>, start: int, end: int, left: int)
    requires 0 <= start <= end <= edges.len(),
    ensures equal_count(edges, start, left) <= equal_count(edges, end, left),
    decreases end - start,
{
    if end > start { equal_count_monotone(edges, start, end - 1, left); }
}

pub(crate) open spec fn scatter_inputs(
    edges: Seq<SparseEdge>, right: u16, offsets: Seq<u16>, cursors: Seq<u16>, length: nat,
) -> bool {
    offsets.len() == cursors.len() + 1 && offsets.len() > 0 && length <= 3969
        && offsets[0] == 0 && offsets[offsets.len() as int - 1] == length
        && (forall|i: int, j: int| 0 <= i <= j < offsets.len() ==> #[trigger] offsets[i] <= #[trigger] offsets[j])
        && (forall|i: int| 0 <= i < edges.len() ==> (#[trigger] edges[i]).left < cursors.len() && edges[i].right == right)
        && (forall|left: int| 0 <= left < cursors.len() ==> offsets[left] <= #[trigger] cursors[left]
            && cursors[left] as int + equal_count(edges, edges.len() as int, left) <= offsets[left + 1])
}

pub(crate) open spec fn scatter_state(
    edges: Seq<SparseEdge>, right: u16, offsets: Seq<u16>, initial: Seq<u16>, original: Seq<u16>,
    cursors: Seq<u16>, output: Seq<u16>, end: int,
) -> bool {
    0 <= end <= edges.len() && cursors.len() == initial.len() && output.len() == original.len()
        && (forall|left: int| 0 <= left < cursors.len() ==> #[trigger] cursors[left] as int
            == initial[left] as int + equal_count(edges, end, left))
        && (forall|left: int, slot: int| 0 <= left < cursors.len()
            && offsets[left] <= slot < offsets[left + 1] ==> #[trigger] output[slot]
                == if initial[left] <= slot < #[trigger] cursors[left] { right } else { original[slot] })
}

pub(crate) proof fn scatter_available(
    edges: Seq<SparseEdge>, right: u16, offsets: Seq<u16>, initial: Seq<u16>, original: Seq<u16>,
    cursors: Seq<u16>, output: Seq<u16>, end: int,
)
    requires scatter_inputs(edges, right, offsets, initial, original.len()),
        scatter_state(edges, right, offsets, initial, original, cursors, output, end), end < edges.len(),
    ensures offsets[edges[end].left as int] <= cursors[edges[end].left as int]
        < offsets[edges[end].left as int + 1] <= output.len(), cursors[edges[end].left as int] < 65535,
{
    let left = edges[end].left as int;
    equal_count_monotone(edges, 0, end, left);
    equal_count_monotone(edges, end + 1, edges.len() as int, left);
    assert(equal_count(edges, end + 1, left) == equal_count(edges, end, left) + 1);
}

pub(crate) proof fn scatter_step(
    edges: Seq<SparseEdge>, right: u16, offsets: Seq<u16>, initial: Seq<u16>, original: Seq<u16>,
    before: Seq<u16>, previous: Seq<u16>, after: Seq<u16>, output: Seq<u16>, end: int,
)
    requires scatter_inputs(edges, right, offsets, initial, original.len()),
        scatter_state(edges, right, offsets, initial, original, before, previous, end), end < edges.len(),
        after == before.update(edges[end].left as int, (before[edges[end].left as int] + 1) as u16),
        output == previous.update(before[edges[end].left as int] as int, edges[end].right),
        before[edges[end].left as int] < 65535,
    ensures scatter_state(edges, right, offsets, initial, original, after, output, end + 1),
{
    let changed = edges[end].left as int;
    let written = before[changed] as int;
    scatter_available(edges, right, offsets, initial, original, before, previous, end);
    equal_count_monotone(edges, 0, end, changed);
    assert(initial[changed] <= written < previous.len());
    assert(after[changed] as int == written + 1);
    assert forall|left: int| 0 <= left < after.len() implies #[trigger] after[left] as int
        == initial[left] as int + equal_count(edges, end + 1, left) by {
    }
    assert forall|left: int, slot: int| 0 <= left < after.len()
        && offsets[left] <= slot < offsets[left + 1] implies #[trigger] output[slot]
            == if initial[left] <= slot < #[trigger] after[left] { right } else { original[slot] } by {
        if left != changed {
            if left < changed { assert(slot < offsets[changed]); }
            else { assert(offsets[changed + 1] <= slot); }
            assert(slot != written);
            assert(after[left] == before[left]);
        } else {
            if slot == written {
                assert(initial[left] <= slot < after[left]);
            } else {
                assert((initial[left] <= slot < after[left]) <==> (initial[left] <= slot < before[left]));
            }
        }
    }
}

pub(crate) open spec fn mask_matches(mask: u64, start: u8, end: u8) -> bool {
    forall|bit: u64| bit < 64 ==> #[trigger] ((mask >> bit) & 1u64)
        == if 1 <= bit && bit <= 63 && start as u64 <= bit && bit <= end as u64 { 1u64 } else { 0u64 }
}

pub(crate) proof fn empty_mask(start: u8, end: u8)
    requires start > end || end == 0 || start > 63,
    ensures mask_matches(0, start, end),
{
    assert forall|bit: u64| bit < 64 implies #[trigger] ((0u64 >> bit) & 1u64)
        == if 1 <= bit && bit <= 63 && start as u64 <= bit && bit <= end as u64 { 1u64 } else { 0u64 } by {
        assert(((0u64 >> bit) & 1u64) == 0) by (bit_vector);
    }
}

pub(crate) proof fn shift_positive(bit: u8)
    requires bit < 64,
    ensures (1u64 << bit) > 0,
{
    assert((1u64 << bit) > 0) by (bit_vector) requires bit < 64;
}

pub(crate) proof fn mask_complete(mask: u64, start: u8, end: u8, first: u8, last: u8)
    requires
        first == if start < 1 { 1u8 } else { start },
        last == if end > 63 { 63u8 } else { end },
        first <= last,
        mask == (u64::MAX << first)
            & if last >= 63 { u64::MAX } else { ((1u64 << ((last + 1) as u8)) - 1) as u64 },
    ensures mask_matches(mask, start, end),
{
    assert forall|bit: u64| bit < 64 implies #[trigger] ((mask >> bit) & 1u64)
        == if 1 <= bit && bit <= 63 && start as u64 <= bit && bit <= end as u64 { 1u64 } else { 0u64 } by {
        assert(1 <= first <= last <= 63);
        assert(((mask >> bit) & 1u64) ==
            if first as u64 <= bit && bit <= last as u64 { 1u64 } else { 0u64 }) by (bit_vector)
            requires bit < 64, 1 <= first <= last <= 63,
                mask == (u64::MAX << first)
                    & if last >= 63 { u64::MAX } else { ((1u64 << ((last + 1) as u8)) - 1) as u64 };
    }
}

pub(crate) open spec fn pair_indices(pair_left: Seq<u16>, right_len: nat) -> bool {
    forall|i: int| 0 <= i < pair_left.len() ==> #[trigger] pair_left[i] == u16::MAX
        || pair_left[i] < right_len
}

pub(crate) open spec fn marks_match(
    pair_left: Seq<u16>, distances: Seq<u16>, complete: bool,
    selected: Seq<bool>, excluded: Seq<bool>,
) -> bool {
    selected.len() == pair_left.len() && excluded.len() == distances.len()
        && (forall|i: int| 0 <= i < selected.len() ==> #[trigger] selected[i]
            == (!complete && (pair_left[i] == u16::MAX || distances[pair_left[i] as int] == u16::MAX)))
        && (forall|i: int| 0 <= i < excluded.len() ==> #[trigger] excluded[i]
            == (!complete && distances[i] == u16::MAX))
}

/// 数学匹配：任意两条不同边均不共享左端或右端。
pub(crate) open spec fn matching(edges: Set<(int, int)>) -> bool {
    forall|a: (int, int), b: (int, int)|
        #[trigger] edges.contains(a) && #[trigger] edges.contains(b)
        && (a.0 == b.0 || a.1 == b.1) ==> a == b
}

/// 左右覆盖集合至少包含每条边的一个端点。
pub(crate) open spec fn covers(edges: Set<(int, int)>, left: Set<int>, right: Set<int>) -> bool {
    forall|edge: (int, int)| #[trigger] edges.contains(edge)
        ==> left.contains(edge.0) || right.contains(edge.1)
}

/// 独立集与两侧节点域的补集构成顶点覆盖。
pub(crate) open spec fn independent(
    edges: Set<(int, int)>, left: Set<int>, right: Set<int>,
) -> bool {
    forall|edge: (int, int)| #[trigger] edges.contains(edge)
        ==> !(left.contains(edge.0) && right.contains(edge.1))
}

/// 弱对偶；递归仅存在于擦除的 proof，终止量为剩余匹配边数。
pub(crate) proof fn matching_cover_bound(
    edges: Set<(int, int)>, left: Set<int>, right: Set<int>,
)
    requires matching(edges), covers(edges, left, right),
    ensures edges.len() <= left.len() + right.len(),
    decreases edges.len(),
{
    if !edges.is_empty() {
        let edge = edges.choose();
        let rest = edges.remove(edge);
        assert(matching(rest));
        if left.contains(edge.0) {
            let smaller = left.remove(edge.0);
            assert(covers(rest, smaller, right)) by {
                assert forall|other: (int, int)| #[trigger] rest.contains(other)
                    implies smaller.contains(other.0) || right.contains(other.1) by {
                    assert(other != edge);
                    assert(other.0 != edge.0);
                }
            }
            matching_cover_bound(rest, smaller, right);
        } else {
            assert(right.contains(edge.1));
            let smaller = right.remove(edge.1);
            assert(covers(rest, left, smaller)) by {
                assert forall|other: (int, int)| #[trigger] rest.contains(other)
                    implies left.contains(other.0) || smaller.contains(other.1) by {
                    assert(other != edge);
                    assert(other.1 != edge.1);
                }
            }
            matching_cover_bound(rest, left, smaller);
        }
    }
}

/// 等大覆盖没有冗余端点：每个匹配对恰覆盖一端，未匹配顶点不在覆盖中。
pub(crate) proof fn matching_cover_tight(
    edges: Set<(int, int)>, left: Set<int>, right: Set<int>,
)
    requires matching(edges), covers(edges, left, right), edges.len() == left.len() + right.len(),
    ensures
        forall|edge: (int, int)| #[trigger] edges.contains(edge)
            ==> left.contains(edge.0) != right.contains(edge.1),
        forall|l: int| #[trigger] left.contains(l) ==> exists|edge: (int, int)| edges.contains(edge) && edge.0 == l,
        forall|r: int| #[trigger] right.contains(r) ==> exists|edge: (int, int)| edges.contains(edge) && edge.1 == r,
{
    assert forall|edge: (int, int)| #[trigger] edges.contains(edge)
        implies left.contains(edge.0) != right.contains(edge.1) by {
        if left.contains(edge.0) && right.contains(edge.1) {
            let smaller = left.remove(edge.0);
            assert(covers(edges, smaller, right)) by {
                assert forall|other: (int, int)| #[trigger] edges.contains(other)
                    implies smaller.contains(other.0) || right.contains(other.1) by {
                    if other.0 == edge.0 { assert(other == edge); }
                }
            }
            matching_cover_bound(edges, smaller, right);
        }
    }
    assert forall|l: int| #[trigger] left.contains(l)
        implies exists|edge: (int, int)| edges.contains(edge) && edge.0 == l by {
        if !(exists|edge: (int, int)| edges.contains(edge) && edge.0 == l) {
            let smaller = left.remove(l);
            assert(covers(edges, smaller, right)) by {
                assert forall|edge: (int, int)| #[trigger] edges.contains(edge)
                    implies smaller.contains(edge.0) || right.contains(edge.1) by {
                    assert(edge.0 != l);
                }
            }
            matching_cover_bound(edges, smaller, right);
        }
    }
    assert forall|r: int| #[trigger] right.contains(r)
        implies exists|edge: (int, int)| edges.contains(edge) && edge.1 == r by {
        if !(exists|edge: (int, int)| edges.contains(edge) && edge.1 == r) {
            let smaller = right.remove(r);
            assert(covers(edges, left, smaller)) by {
                assert forall|edge: (int, int)| #[trigger] edges.contains(edge)
                    implies left.contains(edge.0) || smaller.contains(edge.1) by {
                    assert(edge.1 != r);
                }
            }
            matching_cover_bound(edges, left, smaller);
        }
    }
}

/// 与匹配等大的覆盖证书同时证明匹配最大；不假设 HKDW 的算法正确性。
pub(crate) proof fn matching_maximum_from_certificate(
    graph: Set<(int, int)>, pairs: Set<(int, int)>,
    cover_left: Set<int>, cover_right: Set<int>, other: Set<(int, int)>,
)
    requires pairs.subset_of(graph), matching(pairs),
        covers(graph, cover_left, cover_right),
        cover_left.len() + cover_right.len() == pairs.len(),
        other.subset_of(graph), matching(other),
    ensures other.len() <= pairs.len(),
{
    assert(covers(other, cover_left, cover_right));
    matching_cover_bound(other, cover_left, cover_right);
}

/// 对偶证书推出独立集的全局基数最优；调用者仍须从真实 CSR 与最终数组建立前提。
pub(crate) proof fn independent_maximum_from_certificate(
    graph: Set<(int, int)>, pairs: Set<(int, int)>,
    left_domain: Set<int>, right_domain: Set<int>,
    chosen_left: Set<int>, chosen_right: Set<int>,
    other_left: Set<int>, other_right: Set<int>,
)
    requires
        forall|edge: (int, int)| #[trigger] graph.contains(edge)
            ==> left_domain.contains(edge.0) && right_domain.contains(edge.1),
        pairs.subset_of(graph), matching(pairs),
        chosen_left.subset_of(left_domain), chosen_right.subset_of(right_domain),
        other_left.subset_of(left_domain), other_right.subset_of(right_domain),
        independent(graph, chosen_left, chosen_right),
        independent(graph, other_left, other_right),
        left_domain.difference(chosen_left).len() + right_domain.difference(chosen_right).len()
            == pairs.len(),
    ensures chosen_left.len() + chosen_right.len() >= other_left.len() + other_right.len(),
{
    let other_cover_left = left_domain.difference(other_left);
    let other_cover_right = right_domain.difference(other_right);
    assert(covers(pairs, other_cover_left, other_cover_right));
    matching_cover_bound(pairs, other_cover_left, other_cover_right);
    vstd::set_lib::lemma_set_difference_len(left_domain, chosen_left);
    vstd::set_lib::lemma_set_difference_len(right_domain, chosen_right);
    vstd::set_lib::lemma_set_difference_len(left_domain, other_left);
    vstd::set_lib::lemma_set_difference_len(right_domain, other_right);
    assert(left_domain.intersect(chosen_left) =~= chosen_left);
    assert(right_domain.intersect(chosen_right) =~= chosen_right);
    assert(left_domain.intersect(other_left) =~= other_left);
    assert(right_domain.intersect(other_right) =~= other_right);
}

}
