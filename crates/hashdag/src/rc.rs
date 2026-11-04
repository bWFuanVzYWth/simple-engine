use crate::node::{NodeIdx, PoolEntry};
use crate::pool::Pool;
#[cfg(not(verus_only))]
use crate::vec_ops;
#[cfg(verus_only)]
use hash_dag_std::vec_ops;

use crate::hashdag::{DAG_DEPTH, Error};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "proof/rc.rs"]
pub(crate) mod proof;

#[cfg_attr(verus_only, verus_verify)]
// 同源固定公式；proof::capacity_layout 强制绑定公共布局常量。
pub(crate) const RELEASE_CAPACITY: usize = 1 + (64 - 1) * 10;

/// 用显式栈递减引用计数，移除 RC=0 的节点；不分配内存。
/// `start_lod` 为 `root` 所在的 pool 层级。
/// `scratch` 由所有者预先准备至少 `RELEASE_CAPACITY` 项，调用后内容未定义。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::eligible(old(pools), root, start_lod),
    ensures crate::state::well_formed(final(pools)),
        hash_dag_std::vec_model::capacity(*final(scratch)) == hash_dag_std::vec_model::capacity(*old(scratch)),
        result is Ok ==> proof::release_step(crate::state::pools_view(old(pools)), crate::state::pools_view(final(pools)), root, start_lod)
            && final(scratch)@.len() == 0,
        result is Err ==> crate::state::pools_view(final(pools)) == crate::state::pools_view(old(pools)),
        result is Ok <==> hash_dag_std::vec_model::capacity(*old(scratch)) >= proof::capacity(),
))]
// pop 前需记录擦除账目；保留分支结构连接回收后的重数证明。
#[allow(
    clippy::while_let_loop,
    clippy::single_match,
    clippy::collapsible_match
)]
pub(crate) fn rc_dec_or_remove(
    pools: &mut [Pool; DAG_DEPTH as usize],
    root: NodeIdx,
    start_lod: u8,
    scratch: &mut Vec<(NodeIdx, u8)>,
) -> Result<(), Error> {
    if start_lod >= DAG_DEPTH {
        return Err(Error::InvalidLod(start_lod));
    }
    if vec_ops::capacity(scratch) < RELEASE_CAPACITY {
        return Err(Error::CapacityOverflow);
    }
    #[cfg(verus_only)]
    proof_decl! { let ghost origin = crate::state::pools_view(pools); }
    vec_ops::clear(scratch);
    vec_ops::push(scratch, (root, start_lod));
    #[cfg(verus_only)]
    proof! {
        assert(scratch@ =~= seq![(root, start_lod)]);
        assert(crate::state::pools_view(pools) == origin);
        proof::initial_ledger(origin, root, start_lod); proof::seed_shape(root, start_lod);
    }
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::eligible_model(origin, root, start_lod), crate::state::well_formed(pools),
            crate::state::all_occupied_live(crate::state::pools_view(pools)),
            crate::state::frozen(crate::state::pools_view(pools), origin),
            proof::ledger(origin, crate::state::pools_view(pools), scratch@, root, start_lod),
            proof::work_domain(origin, scratch@), proof::shape(scratch@, 9),
            hash_dag_std::vec_model::capacity(*scratch) == hash_dag_std::vec_model::capacity(*old(scratch)),
            hash_dag_std::vec_model::capacity(*scratch) >= proof::capacity(),
        ensures scratch@.len() == 0,
        decreases proof::weight(scratch@),
    ))]
    loop {
        #[cfg(verus_only)]
        proof_decl! { let ghost old_work = scratch@; let ghost before = crate::state::pools_view(pools); }
        let Some((idx, lod)) = vec_ops::pop(scratch) else {
            break;
        };
        #[cfg(verus_only)]
        proof! {
            proof::pending_credit(origin, before, old_work, root, start_lod);
            proof::positive_live(before, idx, lod); proof::shape_popped(old_work, 9);
        }
        let result = release_one(pools, idx, lod);
        #[cfg(verus_only)]
        proof_decl! { let ghost deleted = result->Ok_0; }
        match result {
            Ok(Some(node)) => {
                if lod > 0 {
                    push_nonuniform_children(scratch, &node, lod - 1);
                }
            }
            Ok(None) => {}
            Err(error) => return Err(error),
        }
        #[cfg(verus_only)]
        proof! {
            let after = crate::state::pools_view(pools);
            proof::emitted_bound(deleted, lod);
            proof::weight_decreases(old_work, proof::emitted(deleted, lod));
            proof::advance_ledger(origin, before, after, old_work, scratch@, root, start_lod, deleted);
            proof::appended_domain(origin, before, after, old_work.drop_last(), idx, lod, deleted);
            crate::balance::frozen_transitive(after, before, origin);
        }
    }
    #[cfg(verus_only)]
    proof! { proof::finish(origin, crate::state::pools_view(pools), scratch@, root, start_lod); }
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires crate::state::well_formed(old(pools)),
        crate::state::all_occupied_live(crate::state::pools_view(old(pools))), level < 10,
        crate::state::net_credit(crate::state::pools_view(old(pools)), level as int, crate::node::proof::idx(index)) > 0,
    ensures result is Ok, crate::state::well_formed(final(pools)),
        crate::state::all_occupied_live(crate::state::pools_view(final(pools))),
        result is Ok ==> proof::one_step(crate::state::pools_view(old(pools)), crate::state::pools_view(final(pools)), index, level, result->Ok_0),
))]
#[inline(always)]
fn release_one(
    pools: &mut [Pool; DAG_DEPTH as usize],
    index: NodeIdx,
    level: u8,
) -> Result<Option<PoolEntry>, Error> {
    #[cfg(verus_only)]
    proof_decl! { let ghost before = crate::state::pools_view(pools); }
    #[cfg(verus_only)]
    proof! {
        proof::positive_live(before, index, level);
        if before[level as int].rcs[crate::node::proof::idx(index)] == 1 {
            crate::balance::last_credit(before, level as int, index);
        }
    }
    let Some(pool) = pools.get_mut(usize::from(level)) else {
        return Err(Error::InvalidLod(level));
    };
    #[cfg(verus_only)]
    proof! { pool.valid_model(); assert(pool.state() == before[level as int]); }
    let result = pool.rc_dec_or_remove(index);
    #[cfg(verus_only)]
    proof! { pool.valid_model(); }
    #[cfg(verus_only)]
    proof! { assert(result is Ok); proof::one_effect(before, crate::state::pools_view(pools), index, level, result->Ok_0); }
    result
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::shape(old(stack)@, 9), child_lod < 9,
        hash_dag_std::vec_model::capacity(*old(stack)) >= proof::capacity(),
        forall|i: int| 0 <= i < old(stack)@.len() ==> old(stack)@[i].1 > child_lod,
    ensures final(stack)@ == old(stack)@ + proof::pairs(crate::pool::proof::entry_edges(*entry), child_lod),
        proof::shape(final(stack)@, 9),
        hash_dag_std::vec_model::capacity(*final(stack)) == hash_dag_std::vec_model::capacity(*old(stack)),
    no_unwind
))]
#[inline(always)]
// 耗尽分支需先证明所有重复边均已转移到待释放栈。
#[allow(clippy::while_let_loop)]
fn push_nonuniform_children(stack: &mut Vec<(NodeIdx, u8)>, entry: &PoolEntry, child_lod: u8) {
    #[cfg(verus_only)]
    proof_decl! { let ghost base = stack@; let ghost edges = crate::pool::proof::entry_edges(*entry); let ghost mut processed = 0int; }
    let mut pointers = entry.pointers();
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::shape(base, 9), child_lod < 9,
            hash_dag_std::vec_model::capacity(*stack) == hash_dag_std::vec_model::capacity(*old(stack)),
            hash_dag_std::vec_model::capacity(*stack) >= proof::capacity(),
            forall|i: int| 0 <= i < base.len() ==> base[i].1 > child_lod,
            0 <= processed <= edges.len() <= 64,
            edges == crate::pool::proof::entry_edges(*entry),
            crate::node::proof::remaining(pointers) == edges.subrange(processed, edges.len() as int),
            stack@ == base + proof::pairs(edges.subrange(0, processed), child_lod),
        ensures stack@ == base + proof::pairs(edges, child_lod),
        decreases crate::node::proof::remaining(pointers).len(),
    ))]
    loop {
        let Some(idx) = pointers.next() else {
            #[cfg(verus_only)]
            proof! { assert(processed == edges.len()); assert(edges.subrange(0, processed) == edges); }
            break;
        };
        #[cfg(verus_only)]
        proof! {
            proof::append_shape(base, edges.subrange(0, processed), child_lod, 9);
            proof::shape_bound(stack@, 9);
            proof::prefix_step(edges, processed);
            proof::pairs_push(edges.subrange(0, processed), idx, child_lod);
        }
        vec_ops::push(stack, (idx, child_lod));
        #[cfg(verus_only)]
        proof! { processed = processed + 1; }
    }
    #[cfg(verus_only)]
    proof! { proof::append_shape(base, edges, child_lod, 9); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hashdag::NODE_FANOUT;
    use crate::node::{Child, PoolEntry};
    use crate::test_util;
    use crate::write::commit::commit_node_layered;

    #[test]
    fn rc_dec_or_remove_cascades() {
        let mut pools: [Pool; DAG_DEPTH as usize] = std::array::from_fn(|_| Pool::new());

        let mut leaf_arr = [42u32; NODE_FANOUT];
        leaf_arr[0] = 99;
        let leaf = PoolEntry::new(&leaf_arr, 0);
        let Some((leaf_idx, _)) = test_util::ok(commit_node_layered(&mut pools, 0, &leaf, 0))
        else {
            return;
        };

        let mut mid_children = [Child::Leaf(0); NODE_FANOUT];
        mid_children[0] = Child::Ptr(leaf_idx);
        let mid = PoolEntry::from_children(&mid_children);
        let Some((mid_idx, _)) = test_util::ok(commit_node_layered(&mut pools, 1, &mid, 0)) else {
            return;
        };

        let mut root_children = [Child::Leaf(0); NODE_FANOUT];
        root_children[0] = Child::Ptr(mid_idx);
        let root = PoolEntry::from_children(&root_children);
        let Some((root_idx, _)) = test_util::ok(commit_node_layered(&mut pools, 2, &root, 1))
        else {
            return;
        };

        let count_before: usize = pools.iter().map(Pool::len).sum();
        let mut scratch = Vec::with_capacity(RELEASE_CAPACITY);
        let _ = test_util::ok(rc_dec_or_remove(&mut pools, root_idx, 2, &mut scratch));
        assert_eq!(pools.iter().map(Pool::len).sum::<usize>(), 0);
        assert!(count_before >= 3);
    }

    #[test]
    fn shared_child_survives_one_parent_release() {
        let mut pools: [Pool; DAG_DEPTH as usize] = std::array::from_fn(|_| Pool::new());

        // 插入子节点
        let mut leaf_slots = [0u32; NODE_FANOUT];
        leaf_slots[0] = 1;
        let leaf = PoolEntry::new(&leaf_slots, 0);
        let Some((leaf_idx, _)) = test_util::ok(commit_node_layered(&mut pools, 0, &leaf, 0))
        else {
            return;
        };

        // 父节点 A 引用子节点（slot 0），RC=1
        let mut parent_a = [Child::Leaf(0); NODE_FANOUT];
        parent_a[0] = Child::Ptr(leaf_idx);
        let node_a = PoolEntry::from_children(&parent_a);
        let Some((idx_a, _)) = test_util::ok(commit_node_layered(&mut pools, 1, &node_a, 1)) else {
            return;
        };

        // 父节点 B 也引用同一个子节点（slot 1），RC=1，由于去重子节点 RC=2
        let mut parent_b = [Child::Leaf(0); NODE_FANOUT];
        parent_b[1] = Child::Ptr(leaf_idx);
        let node_b = PoolEntry::from_children(&parent_b);
        let Some((idx_b, _)) = test_util::ok(commit_node_layered(&mut pools, 1, &node_b, 1)) else {
            return;
        };

        // 释放 A — 子节点 RC 从 2 → 1
        let mut scratch = Vec::with_capacity(RELEASE_CAPACITY);
        let _ = test_util::ok(rc_dec_or_remove(&mut pools, idx_a, 1, &mut scratch));
        assert!(pools[0].get(leaf_idx).is_some());

        // 释放 B — 子节点 RC 从 1 → 0，级联删除
        let _ = test_util::ok(rc_dec_or_remove(&mut pools, idx_b, 1, &mut scratch));
        assert_eq!(
            pools
                .iter()
                .map(super::super::pool::Pool::len)
                .sum::<usize>(),
            0
        );
    }
}
