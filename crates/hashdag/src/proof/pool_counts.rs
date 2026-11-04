//! 子边前缀的引用计数与回滚计数。
use super::*;
use crate::{node::proof as node, state};
use vstd::prelude::*;

verus! {

pub(super) open spec fn bounded(edges: Seq<crate::NodeIdx>, start: int, end: int, len: nat) -> bool {
    forall|i: int| start <= i < end ==> 0 <= node::idx(#[trigger] edges[i]) < len
}

pub(super) open spec fn outstanding(base: &Pool, current: &Pool, edges: Seq<crate::NodeIdx>, start: int, end: int) -> bool {
    0 <= start <= end <= edges.len()
        && current.credits(base, edges.subrange(start, end))
}

pub(super) proof fn empty(base: &Pool)
    ensures base.credits(base, Seq::empty()),
{
    reveal(Seq::filter);
}

pub(super) proof fn slice_step<A>(edges: Seq<A>, start: int, end: int)
    requires 0 <= start <= end < edges.len(),
    ensures edges.subrange(start, end + 1) == edges.subrange(start, end).push(edges[end]),
        edges.subrange(end, edges.len() as int).drop_first() == edges.subrange(end + 1, edges.len() as int),
        edges.subrange(end, edges.len() as int)[0] == edges[end],
{
    assert(edges.subrange(start, end + 1) =~= edges.subrange(start, end).push(edges[end]));
    assert(edges.subrange(end, edges.len() as int).drop_first() =~= edges.subrange(end + 1, edges.len() as int));
}

pub(super) proof fn head_step<A>(edges: Seq<A>, start: int, end: int, value: A)
    requires 0 <= start < end <= edges.len(),
    ensures state::occurrences(edges.subrange(start, end), value)
        == state::occurrences(edges.subrange(start + 1, end), value)
            + if edges[start] == value { 1nat } else { 0nat },
{
    let rest = edges.subrange(start + 1, end);
    assert(edges.subrange(start, end) =~= seq![edges[start]] + rest);
    rest.lemma_filter_prepend(edges[start], |item| item == value);
}

pub(super) proof fn push_count<A>(items: Seq<A>, item: A, value: A)
    ensures state::occurrences(items.push(item), value)
        == state::occurrences(items, value) + if item == value { 1nat } else { 0nat },
{
    items.lemma_filter_len_push(|candidate: A| candidate == value, item);
}

proof fn zero_count<A>(items: Seq<A>, value: A)
    requires forall|i:int| 0 <= i < items.len() ==> #[trigger] items[i] != value,
    ensures state::occurrences(items, value) == 0,
{
    assert(items.all(|item:A| item != value));
    items.lemma_all_neg_filter_empty(|item:A| item == value);
}

pub(super) proof fn same_storage_transitive(base: &Pool, before: &Pool, after: &Pool)
    requires before.same_storage(base), after.same_storage(before),
    ensures after.same_storage(base),
{}

pub(super) proof fn increment(base: &Pool, before: &Pool, after: &Pool,
    edges: Seq<crate::NodeIdx>, end: int)
    requires 0 <= end < edges.len(), before.credits(base, edges.subrange(0, end)),
        before.valid(), after.valid(),
        0 <= node::idx(edges[end]) < before.counts().len(),
        before.counts()[node::idx(edges[end])] + 1 <= usize::MAX,
        after.count_change(before, node::idx(edges[end]), 1),
    ensures after.credits(base, edges.subrange(0, end + 1)),
{
    same_storage_transitive(base, before, after);
    slice_step(edges, 0, end);
    assert forall|i: int| 0 <= i < after.counts().len() implies #[trigger] after.counts()[i]
        == base.counts()[i] + state::occurrences(edges.subrange(0, end + 1), node::at_index(i as u32)) by {
        push_count(edges.subrange(0, end), edges[end], node::at_index(i as u32));
        assert(0 <= i <= u32::MAX);
        assert(before.counts()[i] == base.counts()[i] + state::occurrences(edges.subrange(0, end), node::at_index(i as u32)));
        assert(state::occurrences(edges.subrange(0, end + 1), node::at_index(i as u32))
            == state::occurrences(edges.subrange(0, end), node::at_index(i as u32))
                + if edges[end] == node::at_index(i as u32) { 1nat } else { 0nat });
        if i == node::idx(edges[end]) {
            assert(edges[end] == node::at_index(i as u32));
            assert(after.counts()[i] == before.counts()[i] + 1);
        } else {
            assert(edges[end] != node::at_index(i as u32));
            assert(after.counts()[i] == before.counts()[i]);
        }
    }
}

pub(super) proof fn domain(base: &Pool, current: &Pool, edges: Seq<crate::NodeIdx>, position: int)
    requires base.edge_domain(edges), current.same_storage(base), 0 <= position < edges.len(),
    ensures current.count_domain(node::idx(edges[position])),
{
    assert(base.count_domain(node::idx(edges[position])));
    assert(current.state().nodes == base.state().nodes);
}

pub(super) proof fn monotone_valid(base: &Pool, current: &Pool)
    requires base.valid(), current.same_storage(base), current.counts().len() == base.counts().len(),
        forall|i: int| 0 <= i < current.counts().len() ==> #[trigger] current.counts()[i] >= base.counts()[i]
            && (base.state().nodes[i] is None ==> current.counts()[i] == 0),
    ensures current.valid(),
{
    assert(current.state().nodes == base.state().nodes);
    assert(current.state().generations == base.state().generations);
    assert(current.state().free == base.state().free);
    assert(current.state().retained == base.state().retained);
    assert(current.state().next_generation == base.state().next_generation);
    assert(current.state().free.no_duplicates());
    assert(current.state().retained.no_duplicates());
    assert(current.state().nodes.len() == current.state().rcs.len() == current.state().generations.len());
    assert(current.state().nodes.len() <= u32::MAX as int + 1);
    assert forall|i: int| #![trigger current.state().nodes[i]] 0 <= i < current.state().nodes.len() implies
        (current.state().nodes[i] is None <==> current.state().free.contains(node::at_index(i as u32)))
        && (current.state().nodes[i] is None ==> current.state().rcs[i] == 0)
        && current.state().generations[i] <= current.state().next_generation
        && (current.state().nodes[i] is Some ==> current.state().nodes[i]->Some_0.len() == 64
            && 0 < current.state().generations[i]) by {
        assert(base.state().nodes[i] is None ==> base.state().rcs[i] == 0);
        base.read_model(i);
    }
    assert forall|i: int| #![trigger current.state().free[i]] 0 <= i < current.state().free.len() implies
        0 <= node::idx(current.state().free[i]) < current.state().nodes.len() by {
        assert(current.state().free[i] == base.state().free[i]);
    }
    assert forall|i: int, j: int| state::occupied(current.state(), i) && state::occupied(current.state(), j)
        && current.state().nodes[i] == current.state().nodes[j] implies i == j by {
        assert(state::occupied(base.state(), i)); assert(state::occupied(base.state(), j));
    }
    assert forall|i: int, j: int| state::occupied(current.state(), i) && state::occupied(current.state(), j)
        && current.state().generations[i] == current.state().generations[j] implies i == j by {
        assert(state::occupied(base.state(), i)); assert(state::occupied(base.state(), j));
    }
    assert forall|i: int| 0 <= i < current.state().retained.len() implies
        state::live(current.state(), node::idx(current.state().retained[i])) by {
        let index = node::idx(current.state().retained[i]);
        assert(state::live(base.state(), index));
        assert(current.counts()[index] >= base.counts()[index]);
    }
    assert(state::valid_pool(current.state()));
    assert(current.valid());
}

pub(super) proof fn rollback(base: &Pool, before: &Pool, after: &Pool,
    edges: Seq<crate::NodeIdx>, start: int, end: int)
    requires base.valid(), base.edge_domain(edges), outstanding(base, before, edges, start, end),
        start < end, before.valid(), after.same_storage(before),
        bounded(edges, start, end, before.counts().len()),
        after.counts() == before.counts().update(node::idx(edges[start]),
            (before.counts()[node::idx(edges[start])] - 1) as usize),
    ensures outstanding(base, after, edges, start + 1, end), after.valid(),
{
    same_storage_transitive(base, before, after);
    rollback_target(base, before, edges, start, end);
    let k = node::idx(edges[start]);
    assert forall|i: int| 0 <= i < after.counts().len() implies #[trigger] after.counts()[i]
        == base.counts()[i] + state::occurrences(edges.subrange(start + 1, end), node::at_index(i as u32)) by {
        head_step(edges, start, end, node::at_index(i as u32));
        assert(0 <= i <= u32::MAX);
        if i == k { assert(edges[start] == node::at_index(i as u32)); }
        else { assert(edges[start] != node::at_index(i as u32)); }
    }
    assert forall|i: int| 0 <= i < after.counts().len() implies #[trigger] after.counts()[i] >= base.counts()[i]
        && (base.state().nodes[i] is None ==> after.counts()[i] == 0) by {
        assert(after.counts()[i] == base.counts()[i] + state::occurrences(edges.subrange(start + 1, end), node::at_index(i as u32)));
        assert(after.counts()[i] >= base.counts()[i]);
        if base.state().nodes[i] is None {
            assert(base.counts()[i] == 0);
            let rest = edges.subrange(start + 1, end);
            assert forall|j: int| 0 <= j < rest.len() implies rest[j] != node::at_index(i as u32) by {
                assert(base.count_domain(node::idx(edges[start + 1 + j])));
                if rest[j] == node::at_index(i as u32) { assert(state::occupied(base.state(), i)); }
            }
            zero_count(rest, node::at_index(i as u32));
            assert(state::occurrences(rest, node::at_index(i as u32)) == 0);
            assert(after.counts()[i] == 0);
        }
    }
    monotone_valid(base, after);
}

pub(super) proof fn rollback_target(base: &Pool, current: &Pool, edges: Seq<crate::NodeIdx>, start: int, end: int)
    requires base.valid(), current.valid(), outstanding(base, current, edges, start, end), start < end,
        bounded(edges, start, end, current.counts().len()),
    ensures 0 <= node::idx(edges[start]) < current.counts().len(), current.counts()[node::idx(edges[start])] > 0,
{
    head_step(edges, start, end, edges[start]);
    assert(edges[start] == node::at_index(node::idx(edges[start]) as u32));
}
}
