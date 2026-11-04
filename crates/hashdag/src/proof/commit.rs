//! 单层实际入池与全局构造账目的连接，不改变执行期布局。
use crate::{
    node::{Child, PoolEntry, proof as node},
    pool::InsertResult,
    state::*,
    write::commit::proof as commit,
};
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

proof fn free_frame(before: Pools, after: Pools, level: u8, entry: PoolEntry, inserted: InsertResult)
    requires level < 10, before.len() == 10, after.len() == 10,
        commit::inserted_delta(before, after, level, entry, 0, inserted),
    ensures forall|height: int| 0 <= height < 10 ==>
        #[trigger] after[height].free.is_prefix_of(before[height].free),
{
    assert forall|height: int| 0 <= height < 10 implies
        #[trigger] after[height].free.is_prefix_of(before[height].free) by {
        if height == level as int {
            if inserted is New && before[height].free.len() > 0 {
                assert(after[height].free == before[height].free.drop_last());
            }
        }
    }
}

#[verifier::spinoff_prover]
pub(crate) proof fn intern_build(before: Pools, after: Pools, level: u8, entry: PoolEntry, inserted: InsertResult)
    requires valid_pools(before), all_occupied_live(before), level < 10, after.len() == 10,
        forall|l: int| 0 <= l < 10 ==> #[trigger] before[l].constructing,
        forall|l: int| 0 <= l < 10 ==> #[trigger] valid_pool(after[l]),
        !node::uniform(node::children(entry)),
        forall|slot: int| 0 <= slot < 64 ==> #[trigger] child_valid(before, node::children(entry)[slot], level as nat),
        commit::inserted_delta(before, after, level, entry, 0, inserted),
    ensures build_step(before, after),
        child_valid(after, Child::Ptr(commit::inserted_index(inserted)), (level + 1) as nat),
        after[level as int].nodes[node::idx(commit::inserted_index(inserted))] == Some(node::children(entry)),
{
    let l = level as int;
    let index = commit::inserted_index(inserted);
    let target = node::idx(index);
    let row = node::children(entry);
    if inserted is Existing {
        assert(after[l].rcs =~= before[l].rcs);
        assert(after[l] == before[l]);
        assert(after =~= before);
        assert(build_step(before, after));
        return;
    }
    assert(!occupied(before[l], target));
    assert(after[l].nodes.len() == before[l].nodes.len() + if before[l].free.len() == 0 { 1int } else { 0int });
    assert forall|i: int| #![trigger after[l].nodes[i]] 0 <= i < after[l].nodes.len() && i != target implies
        i < before[l].nodes.len() && after[l].nodes[i] == before[l].nodes[i]
            && after[l].rcs[i] == before[l].rcs[i] && after[l].generations[i] == before[l].generations[i] by {
        if before[l].free.len() == 0 { assert(target == before[l].nodes.len()); }
    }
    assert forall|height: int| #![trigger after[height]] 0 <= height < 10 implies
        after[height].nodes.len() >= before[height].nodes.len()
            && after[height].constructing == before[height].constructing
            && after[height].next_generation >= before[height].next_generation
            && before[height].retained.is_prefix_of(after[height].retained) by {}
    assert forall|height: int, i: int| #![trigger occupied(after[height], i)]
        0 <= height < 10 && occupied(after[height], i) implies live(after[height], i) by {
        if height == l && i == target { assert(after[l].rcs[i] == 1); }
        else {
            assert(occupied(before[height], i));
            assert(live(before[height], i));
            if height + 1 == l { assert(after[height].rcs[i] >= before[height].rcs[i]); }
        }
    }
    assert(all_occupied_live(after));
    assert forall|height: int, i: int| #![trigger live(before[height], i)]
        0 <= height < 10 && live(before[height], i) implies
            live(after[height], i) && after[height].nodes[i] == before[height].nodes[i]
                && after[height].generations[i] == before[height].generations[i] by {
        if height == l { assert(i != target); }
    }
    assert(frozen(before, after));
    assert forall|height: int, i: int| #![trigger occupied(after[height], i)]
        0 <= height < 10 && occupied(after[height], i) implies
            (!(exists|value: u32| uniform(after[height].nodes[i]->Some_0, value))
            && forall|slot: int| 0 <= slot < 64 ==> child_valid(after, after[height].nodes[i]->Some_0[slot], height as nat)) by {
        if height == l && i == target {
            assert(after[height].nodes[i] == Some(row));
            if exists|value: u32| uniform(row, value) {
                let value = choose|value: u32| uniform(row, value);
                assert(row[0] == Child::Leaf(value));
                assert forall|slot: int| #![trigger row[slot]] 0 <= slot < 64 implies row[slot] == row[0] by {}
                assert(node::uniform(row));
            }
        } else {
            assert(after[height].nodes[i] == before[height].nodes[i]);
            assert(occupied(before[height], i));
        }
        let children = after[height].nodes[i]->Some_0;
        assert forall|slot: int| #![trigger child_valid(after, children[slot], height as nat)]
            0 <= slot < 64 implies child_valid(after, children[slot], height as nat) by {
            if height == l && i == target {
                assert(children == row);
                assert(child_valid(before, row[slot], level as nat));
            } else {
                assert(occupied(before[height], i));
                assert(children == before[height].nodes[i]->Some_0);
                assert(child_valid(before, before[height].nodes[i]->Some_0[slot], height as nat));
            }
            if let Child::Ptr(child_index) = children[slot] {
                let child_slot = node::idx(child_index);
                assert(height > 0);
                assert(live(before[height - 1], child_slot));
                assert(live(after[height - 1], child_slot));
            }
        }
    }
    assert(valid_pools(after));
    assert forall|height: int, i: int| #![trigger net_credit(after, height, i)]
        0 <= height < 10 && 0 <= i < after[height].nodes.len() implies
            net_credit(before, height, i) == net_credit(after, height, i) by {
        let pointer_index = node::at_index(i as u32);
        let pointer = Child::Ptr(pointer_index);
        if height == l {
            crate::pool::proof::occurrences_concat(before[l].retained, seq![index], pointer_index);
            crate::pool::proof::occurrences_singleton(index, pointer_index);
            assert(after[l].retained == before[l].retained + seq![index]);
            if l < 9 { crate::balance::same_parent_edges(before[l + 1], after[l + 1], pointer, before[l + 1].nodes.len()); }
            if i == target {
                assert(pointer_index == index);
                crate::balance::retained_avoid_missing(before[l], index);
                if l < 9 { crate::balance::parents_avoid_missing(before, l, index, before[l + 1].nodes.len()); }
            }
        } else if height + 1 == l {
            crate::pool::proof::pointer_occurrences(entry, pointer_index);
            crate::balance::added_parent_edges(before[l], after[l], target, row, pointer);
        } else if height < 9 {
            crate::balance::same_parent_edges(before[height + 1], after[height + 1], pointer, before[height + 1].nodes.len());
        }
    }
    free_frame(before, after, level, entry, inserted);
}
}
