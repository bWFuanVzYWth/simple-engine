//! 级联释放的整数余量、重复物理边与显式栈成本。
use super::*;
use crate::{
    node::{Child, proof as node},
    state::{self, Pools},
};
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) open spec fn capacity() -> usize { RELEASE_CAPACITY }

pub(crate) proof fn capacity_layout()
    ensures capacity() == 1 + (crate::layout::NODE_FANOUT as int - 1) * crate::layout::DAG_DEPTH as int,
{}

pub(crate) open spec fn eligible(pools: &[Pool; 10], root: NodeIdx, level: u8) -> bool {
    state::well_formed(pools) && eligible_model(state::pools_view(pools), root, level)
}

pub(crate) open spec fn eligible_model(model: Pools, root: NodeIdx, level: u8) -> bool {
    &&& state::valid_pools(model) && state::all_occupied_live(model)
    &&& level < 10
    &&& (forall|l: int, i: int| 0 <= l < 10 && 0 <= i < model[l].nodes.len() ==>
        state::net_credit(model, l, i) >= 0)
    &&& state::net_credit(model, level as int, node::idx(root)) > 0
}

pub(super) open spec fn work_domain(model: Pools, work: Seq<(NodeIdx, u8)>) -> bool {
    forall|i: int| 0 <= i < work.len() ==> work[i].1 < 10
        && node::idx(work[i].0) < model[work[i].1 as int].nodes.len()
}

pub(crate) open spec fn metadata(before: Pools, after: Pools) -> bool {
    before.len() == after.len() == 10
        && forall|level: int| #![auto] 0 <= level < 10 ==>
            before[level].nodes.len() == after[level].nodes.len()
            && before[level].generations == after[level].generations
            && before[level].next_generation == after[level].next_generation
            && before[level].retained == after[level].retained
            && before[level].constructing == after[level].constructing
}

pub(crate) open spec fn release_step(before: Pools, after: Pools, root: NodeIdx, level: u8) -> bool {
    &&& state::valid_pools(after) && state::all_occupied_live(after)
    &&& state::frozen(after, before) && metadata(before, after)
    &&& forall|l: int, i: int| 0 <= l < 10 && 0 <= i < before[l].nodes.len() ==>
        state::net_credit(after, l, i) == state::net_credit(before, l, i)
            - if l == level && i == node::idx(root) { 1int } else { 0int }
}

pub(super) open spec fn pending(work: Seq<(NodeIdx, u8)>, level: int, index: int) -> nat {
    state::occurrences(work, (node::at_index(index as u32), level as u8))
}

pub(super) open spec fn ledger(before: Pools, current: Pools, work: Seq<(NodeIdx, u8)>, root: NodeIdx, level: u8) -> bool {
    metadata(before, current)
        && forall|l: int, i: int| 0 <= l < 10 && 0 <= i < before[l].nodes.len() ==>
            state::net_credit(current, l, i) == state::net_credit(before, l, i)
                - (if l == level && i == node::idx(root) { 1int } else { 0int })
                + pending(work, l, i)
}

pub(super) open spec fn bonus(removed: Option<PoolEntry>, target_level: u8, level: int, index: int) -> nat {
    if removed is Some && level + 1 == target_level {
        state::occurrences(node::children(removed->Some_0), Child::Ptr(node::at_index(index as u32)))
    } else { 0 }
}

pub(super) open spec fn one_step(before: Pools, after: Pools, index: NodeIdx, level: u8, removed: Option<PoolEntry>) -> bool {
    &&& metadata(before, after) && state::frozen(after, before)
    &&& removed is Some ==> node::children(removed->Some_0) == before[level as int].nodes[node::idx(index)]->Some_0
    &&& forall|l: int, i: int| 0 <= l < 10 && 0 <= i < before[l].nodes.len() ==>
        state::net_credit(after, l, i) + (if l == level && i == node::idx(index) { 1int } else { 0int })
            == state::net_credit(before, l, i) + bonus(removed, level, l, i)
}

pub(super) proof fn positive_live(model: Pools, index: NodeIdx, level: u8)
    requires state::valid_pools(model), state::all_occupied_live(model), level < 10,
        state::net_credit(model, level as int, node::idx(index)) > 0,
    ensures state::live(model[level as int], node::idx(index)),
{
    if !state::occupied(model[level as int], node::idx(index)) {
        assert(state::net_credit(model, level as int, node::idx(index)) <= 0);
    }
}

pub(super) proof fn one_effect(before: Pools, after: Pools, index: NodeIdx, level: u8, removed: Option<PoolEntry>)
    requires state::valid_pools(before), state::all_occupied_live(before), level < 10,
        state::live(before[level as int], node::idx(index)),
        state::net_credit(before, level as int, node::idx(index)) > 0,
        after.len() == before.len(), state::valid_pool(after[level as int]),
        crate::pool::proof::decremented(before[level as int], after[level as int], index),
        forall|l: int| 0 <= l < 10 && l != level ==> after[l] == before[l],
        removed is Some <==> before[level as int].rcs[node::idx(index)] == 1,
        removed is Some ==> node::children(removed->Some_0) == before[level as int].nodes[node::idx(index)]->Some_0,
    ensures state::valid_pools(after), state::all_occupied_live(after), one_step(before, after, index, level, removed),
{
    crate::balance::decrement_effect(before, after, level as int, index);
}

pub(super) open spec fn count_level(work: Seq<(NodeIdx, u8)>, level: int) -> nat
    decreases work.len(),
{
    if work.len() == 0 { 0 } else { count_level(work.drop_last(), level)
        + if work.last().1 == level { 1nat } else { 0nat } }
}

pub(super) open spec fn below(work: Seq<(NodeIdx, u8)>, ceiling: nat) -> nat
    decreases work.len(),
{
    if work.len() == 0 { 0 } else { below(work.drop_last(), ceiling)
        + if work.last().1 < ceiling { 1nat } else { 0nat } }
}

pub(super) open spec fn shape(work: Seq<(NodeIdx, u8)>, top: u8) -> bool {
    &&& top < 10
    &&& (forall|i: int| 0 <= i < work.len() ==> work[i].1 <= top)
    &&& (forall|i: int, j: int| 0 <= i < j < work.len() ==> work[i].1 >= work[j].1)
    &&& count_level(work, top as int) <= 1
    &&& (forall|level: int| 0 <= level < top ==> count_level(work, level) <= 64)
}

pub(super) open spec fn power(level: nat) -> nat
    decreases level,
{ if level == 0 { 1 } else { 65 * power((level - 1) as nat) } }

pub(super) open spec fn weight(work: Seq<(NodeIdx, u8)>) -> nat
    decreases work.len(),
{ if work.len() == 0 { 0 } else { weight(work.drop_last()) + power(work.last().1 as nat) } }

pub(super) proof fn power_positive(level: nat)
    ensures power(level) > 0,
    decreases level,
{ if level > 0 { power_positive((level - 1) as nat); } }

pub(super) proof fn weight_push(work: Seq<(NodeIdx, u8)>, item: (NodeIdx, u8))
    ensures weight(work.push(item)) == weight(work) + power(item.1 as nat),
{
    assert(work.push(item).drop_last() =~= work);
    assert(work.push(item).last() == item);
    reveal_with_fuel(weight, 2);
}

proof fn count_push(work: Seq<(NodeIdx, u8)>, item: (NodeIdx, u8), level: int)
    ensures count_level(work.push(item), level) == count_level(work, level)
        + if item.1 == level { 1nat } else { 0nat },
{
    assert(work.push(item).drop_last() =~= work);
    assert(work.push(item).last() == item);
    reveal_with_fuel(count_level, 2);
}

pub(crate) proof fn push_count<A>(items: Seq<A>, item: A, value: A)
    ensures state::occurrences(items.push(item), value) == state::occurrences(items, value)
        + if item == value { 1nat } else { 0nat },
{ items.lemma_filter_len_push(|candidate: A| candidate == value, item); }

proof fn below_split(work: Seq<(NodeIdx, u8)>, ceiling: nat)
    requires ceiling > 0,
    ensures below(work, ceiling) == below(work, (ceiling - 1) as nat) + count_level(work, ceiling - 1),
    decreases work.len(),
{
    if work.len() > 0 { below_split(work.drop_last(), ceiling); }
    reveal_with_fuel(below, 2); reveal_with_fuel(count_level, 2);
}

proof fn below_all(work: Seq<(NodeIdx, u8)>, ceiling: nat)
    requires forall|i: int| 0 <= i < work.len() ==> work[i].1 < ceiling,
    ensures below(work, ceiling) == work.len(),
    decreases work.len(),
{
    if work.len() > 0 { below_all(work.drop_last(), ceiling); }
    reveal_with_fuel(below, 2);
}

proof fn band_bound(work: Seq<(NodeIdx, u8)>, ceiling: nat)
    requires forall|level: int| 0 <= level < ceiling ==> count_level(work, level) <= 64,
    ensures below(work, ceiling) <= 64 * ceiling,
    decreases ceiling,
{
    if ceiling == 0 { below_zero(work); }
    else { below_split(work, ceiling); band_bound(work, (ceiling - 1) as nat); }
}

proof fn below_zero(work: Seq<(NodeIdx, u8)>)
    ensures below(work, 0) == 0,
    decreases work.len(),
{
    if work.len() > 0 { below_zero(work.drop_last()); }
    reveal_with_fuel(below, 2);
}

pub(super) proof fn shape_bound(work: Seq<(NodeIdx, u8)>, top: u8)
    requires shape(work, top),
    ensures work.len() <= 1 + 64 * (top as nat), work.len() < capacity(),
{
    below_all(work, top as nat + 1);
    below_split(work, top as nat + 1);
    band_bound(work, top as nat);
}

pub(super) proof fn shape_popped(work: Seq<(NodeIdx, u8)>, top: u8)
    requires shape(work, top), work.len() > 0,
    ensures shape(work.drop_last(), top),
        forall|i: int| 0 <= i < work.drop_last().len() ==> work.drop_last()[i].1 >= work.last().1,
{
    let rest = work.drop_last();
    assert(work == rest.push(work.last()));
    assert forall|level: int| 0 <= level <= top implies count_level(rest, level) <= count_level(work, level) by {
        reveal_with_fuel(count_level, 2);
    }
}

pub(super) open spec fn pairs(edges: Seq<NodeIdx>, level: u8) -> Seq<(NodeIdx, u8)> {
    Seq::new(edges.len(), |i: int| (edges[i], level))
}

pub(super) proof fn pairs_push(edges: Seq<NodeIdx>, item: NodeIdx, level: u8)
    ensures pairs(edges.push(item), level) == pairs(edges, level).push((item, level)),
{ assert(pairs(edges.push(item), level) =~= pairs(edges, level).push((item, level))); }

proof fn count_absent(work: Seq<(NodeIdx, u8)>, level: int)
    requires forall|i: int| 0 <= i < work.len() ==> work[i].1 != level,
    ensures count_level(work, level) == 0,
    decreases work.len(),
{
    if work.len() > 0 { count_absent(work.drop_last(), level); }
    reveal_with_fuel(count_level, 2);
}

proof fn append_level(base: Seq<(NodeIdx, u8)>, edges: Seq<NodeIdx>, child: u8, level: int)
    ensures count_level(base + pairs(edges, child), level) == count_level(base, level)
        + if level == child { edges.len() } else { 0nat },
    decreases edges.len(),
{
    if edges.len() > 0 {
        let rest = edges.drop_last();
        append_level(base, rest, child, level);
        assert(edges == rest.push(edges.last()));
        pairs_push(rest, edges.last(), child);
        assert(base + pairs(edges, child) =~= (base + pairs(rest, child)).push((edges.last(), child)));
        count_push(base + pairs(rest, child), (edges.last(), child), level);
    } else { assert(pairs(edges, child) =~= Seq::empty()); assert(base + pairs(edges, child) =~= base); }
}

pub(super) proof fn append_shape(base: Seq<(NodeIdx, u8)>, edges: Seq<NodeIdx>, child: u8, top: u8)
    requires shape(base, top), child < top, edges.len() <= 64,
        forall|i: int| 0 <= i < base.len() ==> base[i].1 > child,
    ensures shape(base + pairs(edges, child), top),
{
    let work = base + pairs(edges, child);
    count_absent(base, child as int);
    assert forall|i: int| 0 <= i < work.len() implies work[i].1 <= top by {
        if i < base.len() { assert(work[i] == base[i]); }
        else { assert(work[i].1 == child); }
    }
    assert forall|i: int, j: int| 0 <= i < j < work.len() implies work[i].1 >= work[j].1 by {
        if j < base.len() { assert(base[i].1 >= base[j].1); }
        else if i >= base.len() { assert(work[i].1 == child && work[j].1 == child); }
        else { assert(work[i].1 > child && work[j].1 == child); }
    }
    append_level(base, edges, child, top as int);
    assert forall|level: int| 0 <= level < top implies count_level(work, level) <= 64 by {
        append_level(base, edges, child, level);
    }
}

pub(super) proof fn append_weight(base: Seq<(NodeIdx, u8)>, edges: Seq<NodeIdx>, level: u8)
    ensures weight(base + pairs(edges, level)) == weight(base) + edges.len() * power(level as nat),
    decreases edges.len(),
{
    if edges.len() > 0 {
        let rest = edges.drop_last();
        append_weight(base, rest, level);
        assert(edges == rest.push(edges.last())); pairs_push(rest, edges.last(), level);
        assert(base + pairs(edges, level) =~= (base + pairs(rest, level)).push((edges.last(), level)));
        weight_push(base + pairs(rest, level), (edges.last(), level));
        let n = edges.len(); let p = power(level as nat);
        assert(rest.len() * p + p == n * p) by(nonlinear_arith) requires rest.len() + 1 == n;
    } else { assert(pairs(edges, level) =~= Seq::empty()); assert(base + pairs(edges, level) =~= base); }
}

pub(super) proof fn seed_shape(root: NodeIdx, level: u8)
    requires level < 10,
    ensures shape(seq![(root, level)], 9),
{
    reveal_with_fuel(count_level, 2);
    assert forall|l: int| 0 <= l < 9 implies count_level(seq![(root, level)], l) <= 64 by {
        reveal_with_fuel(count_level, 2);
    }
}

pub(super) proof fn prefix_step<A>(items: Seq<A>, end: int)
    requires 0 <= end < items.len(),
    ensures items.subrange(0, end + 1) == items.subrange(0, end).push(items[end]),
        items.subrange(end, items.len() as int)[0] == items[end],
        items.subrange(end, items.len() as int).drop_first() == items.subrange(end + 1, items.len() as int),
{
    assert(items.subrange(0, end + 1) =~= items.subrange(0, end).push(items[end]));
    assert(items.subrange(end, items.len() as int).drop_first() =~= items.subrange(end + 1, items.len() as int));
}

pub(super) proof fn weight_decreases(work: Seq<(NodeIdx, u8)>, edges: Seq<NodeIdx>)
    requires work.len() > 0, edges.len() <= 64,
        work.last().1 == 0 ==> edges.len() == 0,
    ensures weight(work.drop_last() + pairs(edges, (work.last().1 - 1) as u8)) < weight(work),
{
    let level = work.last().1;
    let base = work.drop_last();
    assert(work == base.push(work.last())); weight_push(base, work.last());
    power_positive(level as nat);
    if level > 0 {
        append_weight(base, edges, (level - 1) as u8);
        power_positive((level - 1) as nat);
        reveal_with_fuel(power, 2);
        assert(edges.len() * power((level - 1) as nat) < power(level as nat)) by(nonlinear_arith)
            requires edges.len() <= 64, power((level - 1) as nat) > 0,
                power(level as nat) == 65 * power((level - 1) as nat);
    } else { assert(pairs(edges, (level - 1) as u8) =~= Seq::empty()); }
}

pub(super) proof fn initial_ledger(model: Pools, root: NodeIdx, level: u8)
    requires eligible_model(model, root, level),
    ensures work_domain(model, seq![(root, level)]), ledger(model, model, seq![(root, level)], root, level),
{
    positive_live(model, root, level);
    assert forall|l: int, i: int| 0 <= l < 10 && 0 <= i < model[l].nodes.len() implies
        state::net_credit(model, l, i) == state::net_credit(model, l, i)
            - (if l == level && i == node::idx(root) { 1int } else { 0int })
            + pending(seq![(root, level)], l, i) by {
        reveal_with_fuel(Seq::filter, 2);
        assert(0 <= i <= u32::MAX);
        assert(node::idx(node::at_index(i as u32)) == i);
    }
}

pub(super) proof fn pending_credit(before: Pools, current: Pools, work: Seq<(NodeIdx, u8)>, root: NodeIdx, level: u8)
    requires eligible_model(before, root, level), ledger(before, current, work, root, level),
        work_domain(before, work), work.len() > 0,
    ensures state::net_credit(current, work.last().1 as int, node::idx(work.last().0)) > 0,
{
    let (index, lod) = work.last();
    let pos = node::idx(index);
    assert(work.contains((index, lod)));
    crate::balance::count_positive(work, (index, lod));
    assert(node::at_index(pos as u32) == index);
    assert(state::net_credit(before, lod as int, pos)
        - (if lod == level && pos == node::idx(root) { 1int } else { 0int }) >= 0);
}

pub(super) open spec fn emitted(removed: Option<PoolEntry>, level: u8) -> Seq<NodeIdx> {
    if removed is Some && level > 0 { crate::pool::proof::entry_edges(removed->Some_0) } else { Seq::empty() }
}

pub(super) open spec fn child_level(level: u8) -> u8 { if level > 0 { (level - 1) as u8 } else { 0 } }

pub(super) proof fn emitted_bound(removed: Option<PoolEntry>, level: u8)
    ensures emitted(removed, level).len() <= 64, level == 0 ==> emitted(removed, level).len() == 0,
{
    if removed is Some && level > 0 {
        let entry = removed->Some_0;
        node::pointer_sequence_bound(node::values(entry.values), node::mask(entry.mask));
    }
}

pub(super) proof fn finish(origin: Pools, current: Pools, work: Seq<(NodeIdx, u8)>, root: NodeIdx, level: u8)
    requires state::valid_pools(current), state::all_occupied_live(current), state::frozen(current, origin),
        ledger(origin, current, work, root, level), work.len() == 0,
    ensures release_step(origin, current, root, level),
{
    reveal(Seq::filter);
}

proof fn append_pending(work: Seq<(NodeIdx, u8)>, edges: Seq<NodeIdx>, child: u8, level: int, index: int)
    requires 0 <= level < 10,
    ensures pending(work + pairs(edges, child), level, index) == pending(work, level, index)
        + if level == child { state::occurrences(edges, node::at_index(index as u32)) } else { 0nat },
    decreases edges.len(),
{
    let target = node::at_index(index as u32);
    if edges.len() > 0 {
        let rest = edges.drop_last(); let item = edges.last();
        assert(edges == rest.push(item)); pairs_push(rest, item, child);
        append_pending(work, rest, child, level, index);
        assert(work + pairs(edges, child) =~= (work + pairs(rest, child)).push((item, child)));
        push_count(work + pairs(rest, child), (item, child), (target, level as u8));
        push_count(rest, item, target);
    } else {
        assert(pairs(edges, child) =~= Seq::empty()); assert(work + pairs(edges, child) =~= work);
        reveal(Seq::filter);
    }
}

pub(super) proof fn advance_ledger(origin: Pools, before: Pools, after: Pools, old_work: Seq<(NodeIdx, u8)>, work: Seq<(NodeIdx, u8)>,
    root: NodeIdx, start: u8, removed: Option<PoolEntry>)
    requires state::valid_pools(origin), ledger(origin, before, old_work, root, start), old_work.len() > 0,
        one_step(before, after, old_work.last().0, old_work.last().1, removed),
        work == old_work.drop_last() + pairs(emitted(removed, old_work.last().1), child_level(old_work.last().1)),
    ensures ledger(origin, after, work, root, start),
{
    let (index, level) = old_work.last(); let base = old_work.drop_last();
    assert(old_work == base.push((index, level)));
    assert forall|l: int, i: int| 0 <= l < 10 && 0 <= i < origin[l].nodes.len() implies
        state::net_credit(after, l, i) == state::net_credit(origin, l, i)
            - (if l == start && i == node::idx(root) { 1int } else { 0int }) + pending(work, l, i) by {
        assert(0 <= i <= u32::MAX);
        let target = node::at_index(i as u32);
        push_count(base, (index, level), (target, l as u8));
        append_pending(base, emitted(removed, level), child_level(level), l, i);
        if removed is Some && level > 0 {
            crate::pool::proof::pointer_occurrences(removed->Some_0, target);
        } else { reveal(Seq::filter); }
    }
}

pub(super) proof fn appended_domain(origin: Pools, before: Pools, after: Pools, work: Seq<(NodeIdx, u8)>, index: NodeIdx, level: u8, removed: Option<PoolEntry>)
    requires state::valid_pools(before), state::live(before[level as int], node::idx(index)), level < 10,
        metadata(origin, before), metadata(before, after), work_domain(origin, work),
        one_step(before, after, index, level, removed),
    ensures work_domain(origin, work + pairs(emitted(removed, level), child_level(level))),
{
    let edges = emitted(removed, level); let output = work + pairs(edges, child_level(level));
    assert forall|i: int| 0 <= i < output.len() implies output[i].1 < 10
        && node::idx(output[i].0) < origin[output[i].1 as int].nodes.len() by {
        if i < work.len() { assert(output[i] == work[i]); }
        else {
            assert(removed is Some && level > 0);
            let edge = edges[i - work.len() as int];
            crate::pool::proof::pointer_occurrences(removed->Some_0, edge);
            assert(edges.contains(edge)); crate::balance::count_positive(edges, edge);
            let row = node::children(removed->Some_0);
            assert(row.contains(Child::Ptr(edge))) by {
                if !row.contains(Child::Ptr(edge)) { crate::balance::count_absent(row, Child::Ptr(edge)); }
            }
            let slot = choose|slot: int| 0 <= slot < row.len() && row[slot] == Child::Ptr(edge);
            assert(state::child_valid(before, row[slot], level as nat));
            assert(state::live(before[level as int - 1], node::idx(edge)));
        }
    }
}
}
