//! 真实所有者到差分只读视图的映射；不构造执行期快照。
use super::*;
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(super) open spec fn slot_children(slots: Seq<crate::roots::RootSlot>) -> Seq<Child> {
    slots.filter(|slot: crate::roots::RootSlot| slot.child is Some)
        .map_values(|slot: crate::roots::RootSlot| slot.child->Some_0)
}

pub(super) open spec fn slot_count(slots: Seq<crate::roots::RootSlot>, value: Child) -> nat
    decreases slots.len(),
{
    if slots.len() == 0 { 0 } else { slot_count(slots.drop_last(), value)
        + if slots.last().child == Some(value) { 1nat } else { 0nat } }
}

proof fn children_push(slots: Seq<crate::roots::RootSlot>, slot: crate::roots::RootSlot)
    ensures slot_children(slots.push(slot)) == if slot.child is Some {
        slot_children(slots).push(slot.child->Some_0)
    } else { slot_children(slots) },
{
    let keep = |slot: crate::roots::RootSlot| slot.child is Some;
    slots.lemma_filter_push(slot, keep);
    if slot.child is Some {
        assert(slot_children(slots.push(slot)) =~= slot_children(slots).push(slot.child->Some_0));
    }
}

proof fn children_count(slots: Seq<crate::roots::RootSlot>, value: Child)
    ensures crate::state::occurrences(slot_children(slots), value) == slot_count(slots, value),
    decreases slots.len(),
{
    if slots.len() > 0 {
        let rest = slots.drop_last(); let slot = slots.last();
        assert(slots == rest.push(slot));
        children_count(rest, value); children_push(rest, slot);
        if slot.child is Some {
            crate::rc::proof::push_count(slot_children(rest), slot.child->Some_0, value);
            assert(crate::state::occurrences(slot_children(slots), value)
                == crate::state::occurrences(slot_children(rest), value)
                    + if slot.child->Some_0 == value { 1nat } else { 0nat });
        } else {
            assert(slot_children(slots) == slot_children(rest));
        }
        assert(slot_count(slots, value) == slot_count(rest, value) + if slot.child == Some(value) { 1nat } else { 0nat });
    } else {
        reveal(Seq::filter); assert(slot_children(slots) =~= Seq::empty());
        assert(crate::state::occurrences(slot_children(slots), value) == 0);
    }
    reveal_with_fuel(slot_count, 2);
}

proof fn updated_count(before: Seq<crate::roots::RootSlot>, after: Seq<crate::roots::RootSlot>, index: int, value: Child)
    requires before.len() == after.len(), 0 <= index < before.len(),
        forall|i: int| 0 <= i < before.len() && i != index ==> after[i] == before[i],
    ensures slot_count(after, value) + (if before[index].child == Some(value) { 1nat } else { 0nat })
        == slot_count(before, value) + (if after[index].child == Some(value) { 1nat } else { 0nat }),
    decreases before.len(),
{
    if index < before.len() - 1 {
        updated_count(before.drop_last(), after.drop_last(), index, value);
    } else { assert(before.drop_last() =~= after.drop_last()); }
    reveal_with_fuel(slot_count, 2);
}

pub(super) proof fn root_counts(dag: &HashDag, value: Child)
    ensures crate::state::occurrences(dag.root_children(), value) == slot_count(dag.roots.slots@, value),
{
    children_count(dag.roots.slots@, value);
}

pub(super) proof fn inserted_counts(before: &RootSlots, after: &RootSlots, child: Child, key: RootId, value: Child)
    requires before.valid(), crate::roots::proof::inserted(before, after, child, key),
    ensures crate::state::occurrences(slot_children(after.slots@), value)
        == crate::state::occurrences(slot_children(before.slots@), value) + if child == value { 1nat } else { 0nat },
{
    children_count(before.slots@, value); children_count(after.slots@, value);
    if before.free@.len() > 0 {
        assert(before.slots@[key.slot as int].child is None);
        updated_count(before.slots@, after.slots@, key.slot as int, value);
    } else {
        assert(after.slots@.drop_last() =~= before.slots@);
        reveal_with_fuel(slot_count, 2);
    }
}

pub(super) proof fn removed_counts(before: &RootSlots, after: &RootSlots, key: RootId, child: Child, value: Child)
    requires before.valid(), before.lookup(key) == Some(child), crate::roots::proof::removed(before, after, key),
    ensures crate::state::occurrences(slot_children(after.slots@), value) + (if child == value { 1nat } else { 0nat })
        == crate::state::occurrences(slot_children(before.slots@), value),
{
    children_count(before.slots@, value); children_count(after.slots@, value);
    updated_count(before.slots@, after.slots@, key.slot as int, value);
}

pub(super) proof fn replaced_counts(before: &RootSlots, after: &RootSlots, key: RootId, old_child: Child, child: Child, value: Child)
    requires before.valid(), before.lookup(key) == Some(old_child), crate::roots::proof::replaced(before, after, key, child),
    ensures crate::state::occurrences(slot_children(after.slots@), value) + (if old_child == value { 1nat } else { 0nat })
        == crate::state::occurrences(slot_children(before.slots@), value) + (if child == value { 1nat } else { 0nat }),
{
    children_count(before.slots@, value); children_count(after.slots@, value);
    updated_count(before.slots@, after.slots@, key.slot as int, value);
}

pub(super) proof fn active_contains(roots: &RootSlots, key: RootId, child: Child)
    requires roots.lookup(key) == Some(child),
    ensures slot_children(roots.slots@).contains(child),
{
    let keep = |slot: crate::roots::RootSlot| slot.child is Some;
    roots.slots@.lemma_filter_contains(keep, key.slot as int);
    let position = choose|i: int| 0 <= i < roots.slots@.filter(keep).len()
        && roots.slots@.filter(keep)[i] == roots.slots@[key.slot as int];
    assert(slot_children(roots.slots@)[position] == child);
}

pub(super) proof fn children_domain(slots: Seq<crate::roots::RootSlot>, position: int)
    requires 0 <= position < slot_children(slots).len(),
    ensures exists|i: int| 0 <= i < slots.len() && slots[i].child == Some(slot_children(slots)[position]),
{
    let keep = |slot: crate::roots::RootSlot| slot.child is Some;
    let filtered = slots.filter(keep);
    let slot = filtered[position];
    slots.lemma_filter_pred(keep, position);
    slots.lemma_filter_contains_rev(keep, slot);
    let i = choose|i: int| 0 <= i < slots.len() && slots[i] == slot;
    assert(slots[i].child == Some(slot_children(slots)[position]));
}

pub(super) proof fn roots_domain(before: &RootSlots, after: &RootSlots)
    requires before.valid(), after.valid(), after.owner == before.owner,
        forall|root: RootId| after.lookup(root) is Some ==> before.lookup(root) == after.lookup(root),
    ensures forall|i: int| 0 <= i < slot_children(after.slots@).len() ==>
        slot_children(before.slots@).contains(slot_children(after.slots@)[i]),
{
    assert forall|i: int| 0 <= i < slot_children(after.slots@).len() implies
        slot_children(before.slots@).contains(slot_children(after.slots@)[i]) by {
        children_domain(after.slots@, i);
        let pos = choose|pos: int| 0 <= pos < after.slots@.len()
            && after.slots@[pos].child == Some(slot_children(after.slots@)[i]);
        let key = RootId { owner: after.owner, slot: pos as usize, generation: after.slots@[pos].generation };
        assert(after.lookup(key) is Some);
        assert(before.lookup(key) == after.lookup(key));
        active_contains(before, key, before.lookup(key)->Some_0);
    }
}

pub open(crate) spec fn logical_same(before: &HashDag, after: &HashDag) -> bool {
    before.roots.same_entries(&after.roots)
        && crate::state::pools_view(&before.pools) == crate::state::pools_view(&after.pools)
}

pub open(crate) spec fn forked(before: &HashDag, after: &HashDag, source: RootId, result: Result<RootId, Error>) -> bool {
    &&& after.valid()
    &&& (result is Err ==> logical_same(before, after))
    &&& (result is Ok ==> {
        let key = result->Ok_0;
        &&& before.roots.lookup(source) is Some
        &&& after.roots.lookup(key) == before.roots.lookup(source)
        &&& before.roots.lookup(key) is None
        &&& (forall|old_root: RootId| before.roots.lookup(old_root) is Some ==>
            after.roots.lookup(old_root) == before.roots.lookup(old_root))
        &&& crate::roots::proof::preserves_revoked(&before.roots, &after.roots)
        &&& crate::state::frozen(crate::state::pools_view(&before.pools), crate::state::pools_view(&after.pools))
    })
}

pub open(crate) spec fn released(before: &HashDag, after: &HashDag, root: RootId, result: Result<(), Error>) -> bool {
    &&& after.valid()
    &&& hash_dag_std::vec_model::capacity(after.roots.free) == hash_dag_std::vec_model::capacity(before.roots.free)
    &&& hash_dag_std::vec_model::capacity(after.scratch_stack) == hash_dag_std::vec_model::capacity(before.scratch_stack)
    &&& (result is Err ==> logical_same(before, after))
    &&& (result is Ok ==> {
        &&& before.roots.lookup(root) is Some
        &&& after.roots.revoked(root)
        &&& crate::roots::proof::preserves_others(&before.roots, &after.roots, root)
        &&& crate::roots::proof::preserves_revoked(&before.roots, &after.roots)
        &&& crate::state::frozen(crate::state::pools_view(&after.pools), crate::state::pools_view(&before.pools))
    })
}

pub(super) open spec fn credited(before: crate::state::Pools, after: crate::state::Pools, child: Child) -> bool {
    &&& crate::state::frozen(before, after) && crate::rc::proof::metadata(before, after)
    &&& (forall|l: int| 0 <= l < 10 ==> after[l].nodes == before[l].nodes)
    &&& (forall|l: int, i: int| 0 <= l < 10 && 0 <= i < before[l].nodes.len() ==>
        crate::state::net_credit(after, l, i) == crate::state::net_credit(before, l, i)
            + if l == 9 && child == Child::Ptr(crate::node::proof::at_index(i as u32)) { 1int } else { 0int })
}

pub(super) proof fn root_credit(before: crate::state::Pools, after: crate::state::Pools, child: Child)
    requires crate::state::valid_pools(before), crate::state::all_occupied_live(before),
        crate::state::child_valid(before, child, 10),
        child is Ptr ==> after.len() == 10
            && crate::state::valid_pool(after[9])
            && crate::pool::proof::added(before[9], after[9], crate::node::proof::at_index(crate::node::proof::payload(child)), 1nat)
            && (forall|l: int| 0 <= l < 9 ==> after[l] == before[l]),
        child is Leaf ==> after == before,
    ensures crate::state::valid_pools(after), crate::state::all_occupied_live(after), credited(before, after, child),
{
    if let Child::Ptr(index) = child {
        crate::balance::increment_effect(before, after, 9, index, 1nat);
    }
}

pub(super) proof fn root_fact(dag: &HashDag, root: RootId, child: Child)
    requires dag.valid(), dag.roots.lookup(root) == Some(child),
    ensures crate::state::child_valid(crate::state::pools_view(&dag.pools), child, 10),
        dag.root_children().contains(child),
        child is Ptr ==> hash_dag_std::vec_model::capacity(dag.scratch_stack) >= crate::rc::proof::capacity(),
{
    active_contains(&dag.roots, root, child);
    let i = choose|i: int| 0 <= i < dag.root_children().len() && dag.root_children()[i] == child;
    assert(crate::state::child_valid(crate::state::pools_view(&dag.pools), dag.root_children()[i], 10));
}

pub(super) proof fn logical_preserved(before: &HashDag, after: &HashDag)
    requires before.valid(), logical_same(before, after), after.roots.valid(),
        crate::state::well_formed(&after.pools),
        hash_dag_std::vec_model::capacity(after.scratch_stack) == hash_dag_std::vec_model::capacity(before.scratch_stack),
    ensures after.valid(),
{}

proof fn inserted_domain(before: &RootSlots, after: &RootSlots, child: Child, key: RootId)
    requires before.valid(), crate::roots::proof::inserted(before, after, child, key),
        slot_children(before.slots@).contains(child),
    ensures forall|i: int| 0 <= i < slot_children(after.slots@).len() ==>
        slot_children(before.slots@).contains(slot_children(after.slots@)[i]),
{
    assert forall|i: int| 0 <= i < slot_children(after.slots@).len() implies
        slot_children(before.slots@).contains(slot_children(after.slots@)[i]) by {
        children_domain(after.slots@, i);
        let pos = choose|pos: int| 0 <= pos < after.slots@.len()
            && after.slots@[pos].child == Some(slot_children(after.slots@)[i]);
        if pos == key.slot { assert(slot_children(after.slots@)[i] == child); }
        else {
            assert(pos < before.slots@.len());
            assert(before.slots@[pos].child == after.slots@[pos].child);
            let old = RootId { owner: before.owner, slot: pos as usize, generation: before.slots@[pos].generation };
            active_contains(before, old, slot_children(after.slots@)[i]);
        }
    }
}

pub(super) proof fn fork_committed(before: &HashDag, after: &HashDag, source: RootId, child: Child, key: RootId)
    requires before.valid(), before.roots.lookup(source) == Some(child),
        crate::roots::proof::inserted(&before.roots, &after.roots, child, key),
        crate::state::well_formed(&after.pools), crate::state::all_occupied_live(crate::state::pools_view(&after.pools)),
        credited(crate::state::pools_view(&before.pools), crate::state::pools_view(&after.pools), child),
        hash_dag_std::vec_model::capacity(after.scratch_stack) == hash_dag_std::vec_model::capacity(before.scratch_stack),
    ensures forked(before, after, source, Ok(key)),
{
    let b = crate::state::pools_view(&before.pools); let a = crate::state::pools_view(&after.pools);
    root_fact(before, source, child); inserted_domain(&before.roots, &after.roots, child, key);
    assert(before.root_children() == slot_children(before.roots.slots@));
    assert(after.root_children() == slot_children(after.roots.slots@));
    assert forall|i: int| 0 <= i < after.root_children().len() implies before.root_children().contains(after.root_children()[i]) by {}
    assert forall|i: int| #![trigger after.root_children()[i]] 0 <= i < after.root_children().len() implies
        (after.root_children()[i] matches Child::Ptr(index) ==> 0 <= crate::node::proof::idx(index) < a[9].nodes.len()) by {
        let value = after.root_children()[i];
        let old = choose|old: int| 0 <= old < before.root_children().len() && before.root_children()[old] == value;
        assert(crate::state::child_valid(b, value, 10));
    }
    assert forall|l: int, i: int| 0 <= l < 10 && 0 <= i < a[l].nodes.len() implies
        crate::state::net_credit(a, l, i) == if l == 9 {
            crate::state::occurrences(after.root_children(), Child::Ptr(crate::node::proof::at_index(i as u32))) as int
        } else { 0int } by {
        assert(0 <= i <= u32::MAX);
        let value = Child::Ptr(crate::node::proof::at_index(i as u32));
        crate::balance::net_matches_roots(b, before.root_children(), l, i);
        inserted_counts(&before.roots, &after.roots, child, key, value);
    }
    crate::balance::from_net(a, after.root_children());
    assert(crate::state::stable(a, after.root_children()));
    if exists|i: int| 0 <= i < after.root_children().len() && after.root_children()[i] is Ptr {
        let i = choose|i: int| 0 <= i < after.root_children().len() && after.root_children()[i] is Ptr;
        let old = choose|old: int| 0 <= old < before.root_children().len() && before.root_children()[old] == after.root_children()[i];
        assert(before.root_children()[old] is Ptr);
    }
    assert(after.valid());
    assert forall|root: RootId| before.roots.lookup(root) is Some implies after.roots.lookup(root) == before.roots.lookup(root) by {
        assert(root != key);
    }
}

pub(super) proof fn root_eligible(dag: &HashDag, root: RootId, index: NodeIdx)
    requires dag.valid(), dag.roots.lookup(root) == Some(Child::Ptr(index)),
    ensures crate::rc::proof::eligible(&dag.pools, index, 9),
        hash_dag_std::vec_model::capacity(dag.scratch_stack) >= crate::rc::proof::capacity(),
{
    root_fact(dag, root, Child::Ptr(index));
    let pools = crate::state::pools_view(&dag.pools);
    crate::balance::count_positive(dag.root_children(), Child::Ptr(index));
    crate::balance::net_matches_roots(pools, dag.root_children(), 9, crate::node::proof::idx(index));
    assert forall|l: int, i: int| 0 <= l < 10 && 0 <= i < pools[l].nodes.len() implies
        crate::state::net_credit(pools, l, i) >= 0 by {
        assert(i <= u32::MAX);
        crate::balance::net_matches_roots(pools, dag.root_children(), l, i);
    }
}

pub(super) open spec fn uncredited(before: crate::state::Pools, after: crate::state::Pools, child: Child) -> bool {
    match child {
        Child::Leaf(_) => after == before,
        Child::Ptr(index) => crate::rc::proof::release_step(before, after, index, 9),
    }
}

pub(super) proof fn released_committed(before: &HashDag, after: &HashDag, root: RootId, child: Child)
    requires before.valid(), before.roots.lookup(root) == Some(child),
        crate::roots::proof::removed(&before.roots, &after.roots, root),
        crate::state::well_formed(&after.pools),
        uncredited(crate::state::pools_view(&before.pools), crate::state::pools_view(&after.pools), child),
        hash_dag_std::vec_model::capacity(after.scratch_stack) == hash_dag_std::vec_model::capacity(before.scratch_stack),
    ensures released(before, after, root, Ok(())),
{
    let b = crate::state::pools_view(&before.pools); let a = crate::state::pools_view(&after.pools);
    assert forall|key: RootId| after.roots.lookup(key) is Some implies before.roots.lookup(key) == after.roots.lookup(key) by {
        assert(key != root);
    }
    roots_domain(&before.roots, &after.roots);
    assert(after.root_children() == slot_children(after.roots.slots@));
    assert(before.root_children() == slot_children(before.roots.slots@));
    assert forall|i: int| 0 <= i < after.root_children().len() implies before.root_children().contains(after.root_children()[i]) by {}
    assert forall|i: int| #![trigger after.root_children()[i]] 0 <= i < after.root_children().len() implies
        (after.root_children()[i] matches Child::Ptr(index) ==> 0 <= crate::node::proof::idx(index) < a[9].nodes.len()) by {
        let value = after.root_children()[i];
        let old = choose|old: int| 0 <= old < before.root_children().len() && before.root_children()[old] == value;
        assert(crate::state::child_valid(b, value, 10));
    }
    assert forall|l: int, i: int| 0 <= l < 10 && 0 <= i < a[l].nodes.len() implies
        crate::state::net_credit(a, l, i) == if l == 9 {
            crate::state::occurrences(after.root_children(), Child::Ptr(crate::node::proof::at_index(i as u32))) as int
        } else { 0int } by {
        assert(0 <= i <= u32::MAX);
        let value = Child::Ptr(crate::node::proof::at_index(i as u32));
        crate::balance::net_matches_roots(b, before.root_children(), l, i);
        removed_counts(&before.roots, &after.roots, root, child, value);
        if let Child::Ptr(index) = child {
            assert((value == child) <==> i == crate::node::proof::idx(index));
        }
    }
    crate::balance::from_net(a, after.root_children());
    assert(crate::state::stable(a, after.root_children()));
    if exists|i: int| 0 <= i < after.root_children().len() && after.root_children()[i] is Ptr {
        let i = choose|i: int| 0 <= i < after.root_children().len() && after.root_children()[i] is Ptr;
        let old = choose|old: int| 0 <= old < before.root_children().len() && before.root_children()[old] == after.root_children()[i];
        assert(before.root_children()[old] is Ptr);
    }
}

pub open(crate) spec fn published(before: &HashDag, after: &HashDag, root: RootId, old_child: Child, child: Child, result: Result<(), Error>) -> bool {
    &&& after.building()
    &&& (result is Err ==> logical_same(before, after))
    &&& (result is Ok ==> crate::roots::proof::replaced(&before.roots, &after.roots, root, child)
        && crate::rc::proof::metadata(crate::state::pools_view(&before.pools), crate::state::pools_view(&after.pools))
        && crate::state::frozen(crate::state::pools_view(&after.pools), crate::state::pools_view(&before.pools)))
}

pub(super) proof fn claim_eligible(before: &HashDag, pools: &[Pool; 10], old_child: Child, child: Child, index: NodeIdx)
    requires before.building(), before.root_children().contains(old_child), old_child == Child::Ptr(index),
        crate::state::well_formed(pools), crate::state::all_occupied_live(crate::state::pools_view(pools)),
        credited(crate::state::pools_view(&before.pools), crate::state::pools_view(pools), child),
    ensures crate::rc::proof::eligible(pools, index, 9),
{
    let b = crate::state::pools_view(&before.pools); let a = crate::state::pools_view(pools);
    crate::balance::count_positive(before.root_children(), old_child);
    crate::balance::net_matches_roots(b, before.root_children(), 9, crate::node::proof::idx(index));
    assert forall|l: int, i: int| 0 <= l < 10 && 0 <= i < a[l].nodes.len() implies
        crate::state::net_credit(a, l, i) >= 0 by {
        assert(i <= u32::MAX);
        crate::balance::net_matches_roots(b, before.root_children(), l, i);
    }
}

pub(super) proof fn publish_committed(before: &HashDag, after: &HashDag, root: RootId, old_child: Child, child: Child, claimed: crate::state::Pools)
    requires before.building(), before.roots.lookup(root) == Some(old_child),
        crate::roots::proof::replaced(&before.roots, &after.roots, root, child),
        crate::state::child_valid(crate::state::pools_view(&before.pools), child, 10),
        crate::state::well_formed(&after.pools),
        crate::state::all_occupied_live(claimed),
        credited(crate::state::pools_view(&before.pools), claimed, child),
        uncredited(claimed, crate::state::pools_view(&after.pools), old_child),
        hash_dag_std::vec_model::capacity(after.scratch_stack) == hash_dag_std::vec_model::capacity(before.scratch_stack),
    ensures published(before, after, root, old_child, child, Ok(())),
{
    let b = crate::state::pools_view(&before.pools); let a = crate::state::pools_view(&after.pools);
    assert(before.root_children() == slot_children(before.roots.slots@));
    assert(after.root_children() == slot_children(after.roots.slots@));
    assert forall|i: int| #![trigger after.root_children()[i]] 0 <= i < after.root_children().len() implies
        (after.root_children()[i] matches Child::Ptr(index) ==> 0 <= crate::node::proof::idx(index) < a[9].nodes.len()) by {
        children_domain(after.roots.slots@, i);
        let slot = choose|slot: int| 0 <= slot < after.roots.slots@.len()
            && after.roots.slots@[slot].child == Some(after.root_children()[i]);
        if slot != root.slot {
            let key = RootId { owner: before.roots.owner, slot: slot as usize, generation: before.roots.slots@[slot].generation };
            active_contains(&before.roots, key, after.root_children()[i]);
            let old = choose|old: int| 0 <= old < before.root_children().len() && before.root_children()[old] == after.root_children()[i];
            assert(crate::state::child_valid(b, before.root_children()[old], 10));
        }
    }
    assert forall|l: int, i: int| 0 <= l < 10 && 0 <= i < a[l].nodes.len() implies
        crate::state::net_credit(a, l, i) == if l == 9 {
            crate::state::occurrences(after.root_children(), Child::Ptr(crate::node::proof::at_index(i as u32))) as int
        } else { 0int } by {
        assert(0 <= i <= u32::MAX);
        let value = Child::Ptr(crate::node::proof::at_index(i as u32));
        crate::balance::net_matches_roots(b, before.root_children(), l, i);
        replaced_counts(&before.roots, &after.roots, root, old_child, child, value);
        if let Child::Ptr(index) = old_child {
            assert((value == old_child) <==> i == crate::node::proof::idx(index));
        }
    }
    crate::balance::from_net(a, after.root_children());
    assert(crate::state::all_occupied_live(a));
    assert(crate::rc::proof::metadata(b, a));
    assert forall|l: int, i: int| 0 <= l < claimed.len() && crate::state::live(claimed[l], i) implies
        crate::state::live(b[l], i) && b[l].nodes[i] == claimed[l].nodes[i]
            && b[l].generations[i] == claimed[l].generations[i] by {
        assert(crate::state::occupied(b[l], i));
    }
    assert(crate::state::frozen(claimed, b));
    crate::balance::frozen_transitive(a, claimed, b);
}

pub(super) proof fn initialized(dag: &HashDag)
    requires dag.roots.valid(), dag.roots.slots@.len() == 1,
        dag.roots.lookup(dag.default_id()) == Some(Child::Leaf(0)),
        crate::state::well_formed(&dag.pools),
        forall|level: int| 0 <= level < 10 ==> #[trigger] dag.pools@[level].state().nodes.len() == 0
            && dag.pools@[level].state().retained.len() == 0 && !dag.pools@[level].state().constructing,
    ensures dag.valid(), dag.root_result(dag.default_id(), Ok(Child::Leaf(0))),
{
    assert(dag.roots.slots@[0].child == Some(Child::Leaf(0)));
    assert(dag.roots.slots@ =~= seq![dag.roots.slots@[0]]);
    reveal_with_fuel(Seq::filter, 2);
    assert(dag.root_children() =~= seq![Child::Leaf(0)]);
    assert forall|i: int| #![trigger dag.root_children()[i]] 0 <= i < dag.root_children().len()
        implies crate::state::child_valid(crate::state::pools_view(&dag.pools), dag.root_children()[i], 10) by {}
    assert(crate::state::balanced(crate::state::pools_view(&dag.pools), dag.root_children()));
    assert(crate::state::stable(crate::state::pools_view(&dag.pools), dag.root_children()));
}

impl HashDag {
    pub open(crate) spec fn identity_owned(&self)->bool {
        identity::ticket_valid(self.owner_ticket@,self.roots.owner)
    }

    pub(crate) open(crate) spec fn publish_result(&self, before: &HashDag, root: RootId, old_child: Child, child: Child, result: Result<(), Error>) -> bool {
        published(before, self, root, old_child, child, result)
    }

    /// 根发布与构建保护的事务态；允许 constructing 与 retained 非空。
    pub(crate) open(crate) spec fn building(&self) -> bool {
        self.roots.valid() && crate::state::well_formed(&self.pools)
            && crate::state::balanced(crate::state::pools_view(&self.pools), self.root_children())
            && crate::state::all_occupied_live(crate::state::pools_view(&self.pools))
            && hash_dag_std::vec_model::capacity(self.scratch_stack) >= crate::rc::proof::capacity()
    }

    pub open(crate) spec fn default_id(&self) -> RootId {
        RootId { owner: self.roots.owner, slot: 0, generation: 0 }
    }

    pub open(crate) spec fn root_result(&self, root: RootId, result: Result<Child, Error>) -> bool {
        &&& (result is Ok <==> self.roots.lookup(root) is Some)
        &&& (result is Ok ==> result->Ok_0 == self.roots.lookup(root)->Some_0)
        &&& (root.owner != self.roots.owner ==> result is Err && result->Err_0 is WrongOwner)
        &&& (root.owner == self.roots.owner && self.roots.lookup(root) is None ==>
            result is Err && result->Err_0 is RootMissing)
    }

    pub(crate) open(crate) spec fn root_children(&self) -> Seq<Child> {
        self.roots.slots@.filter(|slot: crate::roots::RootSlot| slot.child is Some)
            .map_values(|slot: crate::roots::RootSlot| slot.child->Some_0)
    }

    pub open(crate) spec fn valid(&self) -> bool {
        self.roots.valid() && crate::state::well_formed(&self.pools)
            && crate::state::stable(crate::state::pools_view(&self.pools), self.root_children())
            && ((exists|i: int| 0 <= i < self.root_children().len() && self.root_children()[i] is Ptr)
                ==> hash_dag_std::vec_model::capacity(self.scratch_stack) >= crate::rc::proof::capacity())
    }
}

impl View for HashDag {
    type V = crate::diff::GraphModel;
    open(crate) spec fn view(&self) -> crate::diff::GraphModel {
        crate::diff::GraphModel {
            roots: self.roots,
            nodes: Seq::new(10, |level: int| self.pools@[level].read_nodes()),
        }
    }
}
}
