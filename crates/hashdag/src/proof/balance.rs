//! 构建增量与实际根持有的组合；物理边按槽出现次数记账。
use crate::{
    node::{Child, NodeIdx},
    state::*,
};
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) proof fn count_positive<A>(items: Seq<A>, value: A)
    requires items.contains(value),
    ensures occurrences(items, value) > 0,
{
    let i = choose|i: int| 0 <= i < items.len() && items[i] == value;
    items.lemma_filter_contains(|item: A| item == value, i);
}

pub(crate) proof fn parent_edges_at_least(pool: PoolState, child: Child, count: nat, index: int)
    requires 0 <= index < count <= pool.nodes.len(), pool.nodes[index] is Some,
    ensures parent_edges(pool, child, count) >= occurrences(pool.nodes[index]->Some_0, child),
    decreases count,
{
    if index != count - 1 { parent_edges_at_least(pool, child, (count - 1) as nat, index); }
}

pub(crate) proof fn same_parent_edges(a: PoolState, b: PoolState, child: Child, count: nat)
    requires a.nodes == b.nodes, count <= a.nodes.len(),
    ensures parent_edges(a, child, count) == parent_edges(b, child, count),
    decreases count,
{
    if count > 0 { same_parent_edges(a, b, child, (count - 1) as nat); }
}

pub(crate) proof fn last_credit(pools: Pools, level: int, index: NodeIdx)
    requires valid_pools(pools), 0 <= level < 10, live(pools[level], crate::node::proof::idx(index)),
        pools[level].rcs[crate::node::proof::idx(index)] == 1,
        net_credit(pools, level, crate::node::proof::idx(index)) > 0,
    ensures !pools[level].retained.contains(index),
        level < 9 ==> parent_edges(pools[level + 1], Child::Ptr(index), pools[level + 1].nodes.len()) == 0,
{
    let i = crate::node::proof::idx(index);
    assert(crate::node::proof::at_index(i as u32) == index);
    if pools[level].retained.contains(index) { count_positive(pools[level].retained, index); }
}

pub(crate) proof fn last_has_no_parent(pools: Pools, level: int, index: NodeIdx, parent: int, slot: int)
    requires valid_pools(pools), 0 <= level < 9, live(pools[level], crate::node::proof::idx(index)),
        pools[level].rcs[crate::node::proof::idx(index)] == 1,
        net_credit(pools, level, crate::node::proof::idx(index)) > 0,
        occupied(pools[level + 1], parent), 0 <= slot < 64,
    ensures pools[level + 1].nodes[parent]->Some_0[slot] != Child::Ptr(index),
{
    last_credit(pools, level, index);
    let row = pools[level + 1].nodes[parent]->Some_0;
    assert(valid_pool(pools[level + 1]));
    assert(row.len() == 64);
    if row[slot] == Child::Ptr(index) {
        assert(row.contains(Child::Ptr(index)));
        count_positive(row, Child::Ptr(index));
        parent_edges_at_least(pools[level + 1], Child::Ptr(index), pools[level + 1].nodes.len(), parent);
    }
}

pub(crate) proof fn decrement_effect(before: Pools, after: Pools, level: int, index: NodeIdx)
    requires valid_pools(before), all_occupied_live(before), 0 <= level < 10,
        live(before[level], crate::node::proof::idx(index)),
        net_credit(before, level, crate::node::proof::idx(index)) > 0,
        after.len() == before.len(), valid_pool(after[level]),
        crate::pool::proof::decremented(before[level], after[level], index),
        forall|l: int| 0 <= l < before.len() && l != level ==> #[trigger] after[l] == before[l],
    ensures valid_pools(after), all_occupied_live(after), frozen(after, before),
        forall|l: int| 0 <= l < 10 ==> #[trigger] after[l].nodes.len() == before[l].nodes.len()
            && after[l].generations == before[l].generations && after[l].next_generation == before[l].next_generation
            && after[l].retained == before[l].retained && after[l].constructing == before[l].constructing,
        forall|l: int, i: int| 0 <= l < 10 && 0 <= i < after[l].nodes.len() ==>
            #[trigger] net_credit(after, l, i) + (if l == level && i == crate::node::proof::idx(index) { 1int } else { 0int })
                == net_credit(before, l, i)
                + if before[level].rcs[crate::node::proof::idx(index)] == 1 && l + 1 == level {
                    occurrences(before[level].nodes[crate::node::proof::idx(index)]->Some_0,
                        Child::Ptr(crate::node::proof::at_index(i as u32))) as int
                } else { 0int },
{
    let target = crate::node::proof::idx(index);
    let removed = before[level].rcs[target] == 1;
    if removed { last_credit(before, level, index); }
    assert forall|l: int| #![trigger after[l]] 0 <= l < 10 implies
        valid_pool(after[l]) && after[l].nodes.len() == before[l].nodes.len() by {
        if l != level { assert(after[l] == before[l]); }
    }
    assert forall|l: int, i: int| #![trigger occupied(after[l], i)]
        0 <= l < 10 && occupied(after[l], i) implies
            live(after[l], i) && live(before[l], i)
            && after[l].nodes[i] == before[l].nodes[i] && after[l].generations[i] == before[l].generations[i] by {
        if l == level {
            if i == target { assert(!removed); }
            else { assert(after[l].rcs[i] == before[l].rcs[i]); }
        }
        assert(occupied(before[l], i));
        assert(live(before[l], i));
    }
    assert(all_occupied_live(after));
    assert(frozen(after, before));
    assert forall|l: int, i: int| #![trigger occupied(after[l], i)]
        0 <= l < 10 && occupied(after[l], i) implies
            (!(exists|value: u32| uniform(after[l].nodes[i]->Some_0, value))
                && forall|slot: int| 0 <= slot < 64 ==> child_valid(after, after[l].nodes[i]->Some_0[slot], l as nat)) by {
        assert(after[l].nodes[i] == before[l].nodes[i]);
        assert(occupied(before[l], i));
        let row = before[l].nodes[i]->Some_0;
        assert forall|slot: int| #![trigger child_valid(after, row[slot], l as nat)]
            0 <= slot < 64 implies child_valid(after, row[slot], l as nat) by {
            assert(child_valid(before, row[slot], l as nat));
            if let Child::Ptr(child_index_value) = row[slot] {
                let child_index = crate::node::proof::idx(child_index_value);
                assert(l > 0);
                if l - 1 == level && child_index == target && removed {
                    last_has_no_parent(before, level, index, i, slot);
                    assert(false);
                }
                assert(occupied(after[l - 1], child_index));
                assert(live(after[l - 1], child_index));
            }
        }
    }
    assert(valid_pools(after));
    assert forall|l: int, i: int| #![trigger net_credit(after, l, i)]
        0 <= l < 10 && 0 <= i < after[l].nodes.len() implies
            net_credit(after, l, i) + (if l == level && i == target { 1int } else { 0int })
                == net_credit(before, l, i)
                    + if removed && l + 1 == level { occurrences(before[level].nodes[target]->Some_0,
                        Child::Ptr(crate::node::proof::at_index(i as u32))) as int } else { 0int } by {
        let pointer = Child::Ptr(crate::node::proof::at_index(i as u32));
        if l == level {
            if l < 9 { same_parent_edges(before[l + 1], after[l + 1], pointer, before[l + 1].nodes.len()); }
            if i == target { assert(crate::node::proof::at_index(i as u32) == index); }
        } else if l + 1 == level {
            if removed {
                removed_parent_edges(before[level], after[level], target, before[level].nodes[target]->Some_0, pointer, before[level].nodes.len());
            } else {
                same_parent_edges(before[level], after[level], pointer, before[level].nodes.len());
            }
        } else if l < 9 {
            same_parent_edges(before[l + 1], after[l + 1], pointer, before[l + 1].nodes.len());
        }
    }
}

pub(crate) proof fn increment_effect(before: Pools, after: Pools, level: int, index: NodeIdx, amount: nat)
    requires valid_pools(before), all_occupied_live(before), 0 <= level < 10,
        live(before[level], crate::node::proof::idx(index)), after.len() == before.len(),
        valid_pool(after[level]), crate::pool::proof::added(before[level], after[level], index, amount),
        forall|l: int| 0 <= l < before.len() && l != level ==> #[trigger] after[l] == before[l],
    ensures valid_pools(after), all_occupied_live(after), frozen(before, after),
        forall|l: int| 0 <= l < 10 ==> #[trigger] after[l].nodes == before[l].nodes
            && after[l].generations == before[l].generations && after[l].next_generation == before[l].next_generation
            && after[l].retained == before[l].retained && after[l].constructing == before[l].constructing,
        forall|l: int, i: int| 0 <= l < 10 && 0 <= i < after[l].nodes.len() ==>
            #[trigger] net_credit(after, l, i) == net_credit(before, l, i)
                + if l == level && i == crate::node::proof::idx(index) { amount as int } else { 0int },
{
    let target = crate::node::proof::idx(index);
    assert forall|l: int| #![trigger after[l]] 0 <= l < 10 implies after[l].nodes == before[l].nodes by {}
    assert forall|l: int, i: int| #![trigger occupied(after[l], i)]
        0 <= l < 10 && occupied(after[l], i) implies live(after[l], i) by {
        assert(live(before[l], i));
        if l == level && i == target { assert(after[l].rcs[i] >= before[l].rcs[i]); }
    }
    assert(all_occupied_live(after));
    assert forall|l: int| #![trigger valid_pool(after[l])] 0 <= l < 10 implies valid_pool(after[l]) by {}
    assert forall|l: int, i: int| #![trigger occupied(after[l], i)]
        0 <= l < 10 && occupied(after[l], i) implies
            (!(exists|value: u32| uniform(after[l].nodes[i]->Some_0, value))
            && forall|slot: int| 0 <= slot < 64 ==> child_valid(after, after[l].nodes[i]->Some_0[slot], l as nat)) by {
        assert(occupied(before[l], i));
        let row = after[l].nodes[i]->Some_0;
        assert(row == before[l].nodes[i]->Some_0);
        assert forall|slot: int| #![trigger child_valid(after, row[slot], l as nat)]
            0 <= slot < 64 implies child_valid(after, row[slot], l as nat) by {
            assert(child_valid(before, row[slot], l as nat));
            if let Child::Ptr(child_index) = row[slot] {
                assert(l > 0);
                assert(occupied(after[l - 1], crate::node::proof::idx(child_index)));
                assert(live(after[l - 1], crate::node::proof::idx(child_index)));
            }
        }
    }
    assert(valid_pools(after));
    assert forall|l: int, i: int| #![trigger net_credit(after, l, i)]
        0 <= l < 10 && 0 <= i < after[l].nodes.len() implies net_credit(after, l, i) == net_credit(before, l, i)
            + if l == level && i == target { amount as int } else { 0int } by {
        if l < 9 {
            same_parent_edges(before[l + 1], after[l + 1], Child::Ptr(crate::node::proof::at_index(i as u32)), before[l + 1].nodes.len());
        }
    }
}

pub(crate) proof fn removed_parent_edges(before: PoolState, after: PoolState, parent: int, row: NodeModel, child: Child, count: nat)
    requires 0 <= parent < before.nodes.len(), before.nodes[parent] == Some(row),
        after.nodes == before.nodes.update(parent, None), count <= before.nodes.len(),
    ensures parent_edges(after, child, count)
        + (if parent < count { occurrences(row, child) } else { 0 }) == parent_edges(before, child, count),
    decreases count,
{
    if count > 0 {
        removed_parent_edges(before, after, parent, row, child, (count - 1) as nat);
        let i = (count - 1) as int;
        assert(after.nodes.len() == before.nodes.len());
        if i == parent {
            assert(after.nodes[i] is None);
        } else {
            assert(after.nodes[i] == before.nodes[i]);
        }
    }
}

pub(crate) proof fn padded_parent_edges(before: PoolState, padded: PoolState, child: Child, count: nat)
    requires before.nodes.len() <= padded.nodes.len(), count <= padded.nodes.len(),
        forall|i: int| 0 <= i < padded.nodes.len() ==> #[trigger] padded.nodes[i]
            == if i < before.nodes.len() { before.nodes[i] } else { None },
    ensures parent_edges(padded, child, count)
        == parent_edges(before, child, if count < before.nodes.len() { count } else { before.nodes.len() }),
    decreases count,
{
    if count > 0 {
        padded_parent_edges(before, padded, child, (count - 1) as nat);
        let i = (count - 1) as int;
        if i < before.nodes.len() { assert(padded.nodes[i] == before.nodes[i]); }
        else { assert(padded.nodes[i] is None); }
    }
}

pub(crate) proof fn added_parent_edges(before: PoolState, after: PoolState, parent: int, row: NodeModel, child: Child)
    requires 0 <= parent < after.nodes.len(), !occupied(before, parent), before.nodes.len() <= after.nodes.len(),
        after.nodes[parent] == Some(row),
        forall|i: int| 0 <= i < after.nodes.len() && i != parent ==> #[trigger] after.nodes[i]
            == if i < before.nodes.len() { before.nodes[i] } else { None },
    ensures parent_edges(after, child, after.nodes.len()) == parent_edges(before, child, before.nodes.len()) + occurrences(row, child),
{
    let padded = PoolState { nodes: Seq::new(after.nodes.len(), |i: int|
        if i < before.nodes.len() { before.nodes[i] } else { None }), ..before };
    assert(padded.nodes[parent] is None);
    assert(padded.nodes =~= after.nodes.update(parent, None));
    padded_parent_edges(before, padded, child, after.nodes.len());
    removed_parent_edges(after, padded, parent, row, child, after.nodes.len());
}

pub(crate) proof fn count_absent<A>(items: Seq<A>, value: A)
    requires !items.contains(value),
    ensures occurrences(items, value) == 0,
{
    items.lemma_all_neg_filter_empty(|item: A| item == value);
}

pub(crate) proof fn parents_avoid_missing(pools: Pools, level: int, index: NodeIdx, count: nat)
    requires valid_pools(pools), 0 <= level < 9,
        !live(pools[level], crate::node::proof::idx(index)), count <= pools[level + 1].nodes.len(),
    ensures parent_edges(pools[level + 1], Child::Ptr(index), count) == 0,
    decreases count,
{
    if count > 0 {
        parents_avoid_missing(pools, level, index, (count - 1) as nat);
        let i = (count - 1) as int;
        if pools[level + 1].nodes[i] is Some {
            let row = pools[level + 1].nodes[i]->Some_0;
            assert(valid_pool(pools[level + 1]));
            assert(occupied(pools[level + 1], i));
            assert(row.len() == 64);
            assert forall|slot: int| #![trigger row[slot]] 0 <= slot < row.len()
                implies row[slot] != Child::Ptr(index) by {
                assert(child_valid(pools, row[slot], (level + 1) as nat));
            }
            count_absent(row, Child::Ptr(index));
        }
    }
}

pub(crate) proof fn retained_avoid_missing(pool: PoolState, index: NodeIdx)
    requires valid_pool(pool), !live(pool, crate::node::proof::idx(index)),
    ensures occurrences(pool.retained, index) == 0,
{
    if pool.retained.contains(index) {
        let i = choose|i: int| 0 <= i < pool.retained.len() && pool.retained[i] == index;
        assert(live(pool, crate::node::proof::idx(pool.retained[i])));
    }
    count_absent(pool.retained, index);
}

pub(crate) proof fn net_matches_roots(pools: Pools, roots: Seq<Child>, level: int, index: int)
    requires balanced(pools, roots), 0 <= level < 10, 0 <= index <= u32::MAX,
    ensures net_credit(pools, level, index) == if level == 9 {
        occurrences(roots, Child::Ptr(crate::node::proof::at_index(index as u32))) as int
    } else { 0 },
{
    let pointer = crate::node::proof::at_index(index as u32);
    assert(crate::node::proof::idx(pointer) == index);
    if !occupied(pools[level], index) {
        retained_avoid_missing(pools[level], pointer);
        if level < 9 {
            parents_avoid_missing(pools, level, pointer, pools[level + 1].nodes.len());
        } else {
            assert forall|i: int| #![trigger roots[i]] 0 <= i < roots.len()
                implies roots[i] != Child::Ptr(pointer) by {
                assert(child_valid(pools, roots[i], 10));
            }
            count_absent(roots, Child::Ptr(pointer));
        }
    }
}

pub(crate) proof fn from_net(pools: Pools, roots: Seq<Child>)
    requires valid_pools(pools), all_occupied_live(pools),
        forall|r: int| 0 <= r < roots.len() ==> match #[trigger] roots[r] {
            Child::Leaf(_) => true,
            Child::Ptr(index) => crate::node::proof::idx(index) < pools[9].nodes.len(),
        },
        forall|level: int, i: int| 0 <= level < 10 && 0 <= i < pools[level].nodes.len() ==>
            #[trigger] net_credit(pools, level, i) == if level == 9 {
                occurrences(roots, Child::Ptr(crate::node::proof::at_index(i as u32))) as int
            } else { 0int },
    ensures balanced(pools, roots),
{
    assert forall|r: int| #![trigger roots[r]] 0 <= r < roots.len() implies child_valid(pools, roots[r], 10) by {
        if let Child::Ptr(index) = roots[r] {
            let i = crate::node::proof::idx(index);
            assert(roots.contains(Child::Ptr(index)));
            count_positive(roots, Child::Ptr(index));
            assert(crate::node::proof::at_index(i as u32) == index);
            assert(net_credit(pools, 9, i) > 0);
            if !occupied(pools[9], i) {
                retained_avoid_missing(pools[9], index);
                assert(net_credit(pools, 9, i) == 0);
            }
            assert(live(pools[9], i));
        }
    }
    assert forall|level: int, i: int| #![trigger occupied(pools[level], i)]
        0 <= level < 10 && occupied(pools[level], i) implies pools[level].rcs[i] ==
            (if level == 9 { occurrences(roots, Child::Ptr(crate::node::proof::at_index(i as u32))) } else { 0 })
            + (if level < 9 { parent_edges(pools[level + 1], Child::Ptr(crate::node::proof::at_index(i as u32)), pools[level + 1].nodes.len()) } else { 0 })
            + occurrences(pools[level].retained, crate::node::proof::at_index(i as u32)) by {
        assert(net_credit(pools, level, i) == if level == 9 {
            occurrences(roots, Child::Ptr(crate::node::proof::at_index(i as u32))) as int
        } else { 0int });
    }
}

pub(crate) proof fn build_keeps_balance(before: Pools, after: Pools, roots: Seq<Child>)
    requires balanced(before, roots), build_step(before, after),
    ensures balanced(after, roots),
{
    assert forall|i: int| #![trigger roots[i]] 0 <= i < roots.len()
        implies child_valid(after, roots[i], 10) by {
        crate::canonical::frozen_value(before, after, roots[i], 10, 0);
    }
    assert forall|level: int, index: int| #![trigger occupied(after[level], index)]
        0 <= level < 10 && occupied(after[level], index) implies
            after[level].rcs[index] ==
                (if level == 9 { occurrences(roots, Child::Ptr(crate::node::proof::at_index(index as u32))) } else { 0 })
                + (if level < 9 { parent_edges(after[level + 1], Child::Ptr(crate::node::proof::at_index(index as u32)), after[level + 1].nodes.len()) } else { 0 })
                + occurrences(after[level].retained, crate::node::proof::at_index(index as u32)) by {
        assert(valid_pool(after[level]));
        assert(0 <= index <= u32::MAX);
        net_matches_roots(before, roots, level, index);
        assert(net_credit(after, level, index) == net_credit(before, level, index));
    }
}

pub(crate) proof fn frozen_transitive(a: Pools, b: Pools, c: Pools)
    requires frozen(a, b), frozen(b, c),
    ensures frozen(a, c),
{}

pub(crate) proof fn build_transitive(a: Pools, b: Pools, c: Pools)
    requires build_step(a, b), build_step(b, c),
    ensures build_step(a, c),
{
    frozen_transitive(a, b, c);
    assert forall|level: int| #![trigger c[level]] 0 <= level < 10 implies
        a[level].retained.is_prefix_of(c[level].retained)
            && c[level].free.is_prefix_of(a[level].free) by {
        assert forall|i: int| #![trigger a[level].retained[i]] 0 <= i < a[level].retained.len()
            implies a[level].retained[i] == c[level].retained[i] by {}
        assert forall|i: int| #![trigger c[level].free[i]] 0 <= i < c[level].free.len()
            implies c[level].free[i] == a[level].free[i] by {}
    }
    assert forall|level: int, index: int| #![trigger net_credit(c, level, index)]
        0 <= level < 10 && 0 <= index < c[level].nodes.len()
        implies net_credit(a, level, index) == net_credit(c, level, index) by {
        if index < b[level].nodes.len() {
            assert(net_credit(a, level, index) == net_credit(b, level, index));
        } else {
            let pointer = crate::node::proof::at_index(index as u32);
            retained_avoid_missing(a[level], pointer);
            retained_avoid_missing(b[level], pointer);
            if level < 9 {
                parents_avoid_missing(a, level, pointer, a[level + 1].nodes.len());
                parents_avoid_missing(b, level, pointer, b[level + 1].nodes.len());
            }
        }
    }
}

proof fn parent_witness(pool: PoolState, child: Child, count: nat)
    requires count <= pool.nodes.len(), parent_edges(pool, child, count) > 0,
    ensures exists|i: int| 0 <= i < count && pool.nodes[i] is Some
        && pool.nodes[i]->Some_0.contains(child),
    decreases count,
{
    assert(count > 0);
    let i = (count - 1) as int;
    if pool.nodes[i] is Some && occurrences(pool.nodes[i]->Some_0, child) > 0 {
        if !pool.nodes[i]->Some_0.contains(child) { count_absent(pool.nodes[i]->Some_0, child); }
    } else { parent_witness(pool, child, (count - 1) as nat); }
}

proof fn reach_parent(pools: Pools, index: NodeIdx, height: nat, address: Address, slot: int)
    requires 0 < height <= pools.len(), occupied(pools[height - 1], crate::node::proof::idx(index)),
        0 <= slot < pools[height - 1].nodes[crate::node::proof::idx(index)]->Some_0.len(),
        reachable(pools, pools[height - 1].nodes[crate::node::proof::idx(index)]->Some_0[slot], (height - 1) as nat, address),
    ensures reachable(pools, Child::Ptr(index), height, address),
{}

proof fn follow_edge(pools: Pools, root: Child, height: nat, parent: Address, child: Address)
    requires valid_pools(pools), child_valid(pools, root, height), reachable(pools, root, height, parent),
        0 < parent.level < 10, occupied(pools[parent.level as int], parent.index),
        child.level + 1 == parent.level,
        0 <= child.index <= u32::MAX,
        pools[parent.level as int].nodes[parent.index]->Some_0.contains(Child::Ptr(crate::node::proof::at_index(child.index as u32))),
    ensures reachable(pools, root, height, child),
    decreases height,
{
    if let Child::Ptr(index) = root {
        let i = crate::node::proof::idx(index);
        assert(0 < height <= pools.len());
        assert(occupied(pools[height - 1], i));
        let row = pools[height - 1].nodes[i]->Some_0;
        let slot: int;
        if parent.level == height - 1 && parent.index == i {
            let pointer = Child::Ptr(crate::node::proof::at_index(child.index as u32));
            slot = choose|slot: int| 0 <= slot < row.len() && row[slot] == pointer;
            assert(child_valid(pools, row[slot], (height - 1) as nat));
            assert(reachable(pools, row[slot], (height - 1) as nat, child));
        } else {
            slot = choose|slot: int| 0 <= slot < row.len() && reachable(pools, row[slot], (height - 1) as nat, parent);
            follow_edge(pools, row[slot], (height - 1) as nat, parent, child);
        }
        reach_parent(pools, index, height, child, slot);
    }
}

pub(crate) open spec fn no_orphans(pools: Pools, roots: Seq<Child>) -> bool {
    forall|level: int, i: int| 0 <= level < 10 && occupied(pools[level], i) ==>
        exists|r: int| 0 <= r < roots.len() && #[trigger] reachable(pools, roots[r], 10, Address { level: level as nat, index: i })
}

proof fn live_rooted(pools: Pools, roots: Seq<Child>, level: int, index: int)
    requires stable(pools, roots), 0 <= level < 10, occupied(pools[level], index),
    ensures exists|r: int| 0 <= r < roots.len() &&
        #[trigger] reachable(pools, roots[r], 10, Address { level: level as nat, index }),
    decreases 10 - level,
{
    let pointer = Child::Ptr(crate::node::proof::at_index(index as u32));
    assert(live(pools[level], index));
    assert(occurrences(pools[level].retained, crate::node::proof::at_index(index as u32)) == 0);
    if level == 9 {
        assert(occurrences(roots, pointer) > 0);
        if !roots.contains(pointer) { count_absent(roots, pointer); }
        let r = choose|r: int| 0 <= r < roots.len() && roots[r] == pointer;
        assert(reachable(pools, roots[r], 10, Address { level: level as nat, index }));
    } else {
        assert(parent_edges(pools[level + 1], pointer, pools[level + 1].nodes.len()) > 0);
        parent_witness(pools[level + 1], pointer, pools[level + 1].nodes.len());
        let parent = choose|i: int| 0 <= i < pools[level + 1].nodes.len()
            && pools[level + 1].nodes[i] is Some && #[trigger] pools[level + 1].nodes[i]->Some_0.contains(pointer);
        live_rooted(pools, roots, level + 1, parent);
        let r = choose|r: int| 0 <= r < roots.len() &&
            reachable(pools, roots[r], 10, Address { level: (level + 1) as nat, index: parent });
        follow_edge(pools, roots[r], 10, Address { level: (level + 1) as nat, index: parent },
            Address { level: level as nat, index });
    }
}

/// 稳定边界的正引用守恒与严格分层排除不可达活节点。
pub(crate) proof fn stable_has_no_orphans(pools: Pools, roots: Seq<Child>)
    requires stable(pools, roots),
    ensures no_orphans(pools, roots),
{
    assert forall|level: int, i: int| 0 <= level < 10 && occupied(pools[level], i) implies
        exists|r: int| 0 <= r < roots.len() && reachable(pools, roots[r], 10, Address { level: level as nat, index: i }) by {
        live_rooted(pools, roots, level, i);
    }
}

}
