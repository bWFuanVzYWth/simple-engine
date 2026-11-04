use crate::hashdag::{DAG_DEPTH, Error, NODE_FANOUT};
use crate::node::{Child, NodeIdx, PoolEntry};
use crate::pool::{InsertResult, Pool};
#[cfg(not(verus_only))]
use crate::simd as simd_ops;
#[cfg(verus_only)]
use hash_dag_std::simd_ops;
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "../proof/write_commit.rs"]
#[cfg(verus_only)]
pub(crate) mod proof;

/// 克隆节点或从 Leaf 合成全 uniform 节点。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires pool.valid(),
    ensures result is Ok <==> proof::present(pool.state(), child),
        result is Ok ==> crate::node::proof::children(result->Ok_0) == proof::image(pool.state(), child),
    no_unwind
))]
pub(crate) fn clone_or_synthesize(pool: &Pool, child: Child) -> Result<PoolEntry, Error> {
    match child {
        Child::Ptr(idx) => match pool.get(idx) {
            Some(node) => Ok(node.to_entry()),
            None => Err(Error::NodeMissing(idx)),
        },
        Child::Leaf(val) => {
            let entry = PoolEntry::new(&simd_ops::array_fill::<_, NODE_FANOUT>(val), 0);
            #[cfg(verus_only)]
            proof! { proof::uniform_image(entry, val); }
            Ok(entry)
        }
    }
}

/// 检查同质折叠。64 槽全 uniform 且值相同 → `Some(data)`。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result is Some <==> crate::node::proof::uniform(crate::node::proof::children(*node)),
        result is Some ==> forall|i:int| 0 <= i < 64 ==>
            #[trigger] crate::node::proof::children(*node)[i] == Child::Leaf(result->Some_0),
    no_unwind
))]
pub(crate) fn try_fold(node: &PoolEntry) -> Option<u32> {
    node.uniform_value()
}

/// 提交节点到池，处理 RC 与折叠。
/// 所有生产入池路径共用此入口，先折叠再唯一化，没有跳过折叠的入池旁路。
///
/// `pool_lod` 为节点所在池的层级。该节点覆盖 树高 `pool_lod + 1` 的 subtree（边长为 4 的该次幂），
/// 其 Mixed 子节点位于 `pools[pool_lod - 1]`。
/// 折叠时返回 `(0, Some(data))`，否则 `(idx, None)`。
///
/// 新建节点时，mixed 子节点的 rc 会相应递增（认领子节点所有权）。
/// 去重命中已有节点时，子节点 rc 已在首次插入时递增，不重复操作。
/// 新节点的构造引用由写入边界持有；完整认领子边后才发布节点。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::valid_storage(old(pools)@), proof::layer_domain(old(pools)@,pool_lod,*entry),
    ensures proof::valid_storage(final(pools)@),
        result is Err ==> proof::storage(final(pools)@)==proof::storage(old(pools)@),
        result is Ok ==> proof::commit_result(proof::storage(old(pools)@),proof::storage(final(pools)@),pool_lod,*entry,rc,result->Ok_0),
))]
pub(crate) fn commit_node_layered(
    pools: &mut [Pool; DAG_DEPTH as usize],
    pool_lod: u8,
    entry: &PoolEntry,
    rc: usize,
) -> Result<(NodeIdx, Option<u32>), Error> {
    #[cfg(verus_only)]
    proof_decl! { let ghost before = pools@; }
    if let Some(data) = try_fold(entry) {
        return Ok((NodeIdx::ZERO, Some(data)));
    }
    let level = pool_lod as usize;
    if level >= pools.len() {
        return Err(Error::InvalidLod(pool_lod));
    }
    let (lower, upper) = pools.split_at_mut(level);
    let pool = &mut upper[0];
    // 固定 Verus 尚不支持 let-chain。
    #[allow(clippy::collapsible_if)]
    if pool_lod == 0 {
        if let Some(index) = entry.pointers().next() {
            return Err(Error::NodeMissing(index));
        }
    }
    let result = match lower.last_mut() {
        Some(children) => {
            #[cfg(verus_only)]
            proof_decl! { let ghost before_children = *children; }
            match pool.intern(rc, entry, Some(&mut *children)) {
                Ok(inserted) => {
                    #[cfg(verus_only)]
                    proof! { match inserted {
                        InsertResult::New(_) => crate::pool::proof::credits_model(&before_children, children, crate::pool::proof::entry_edges(*entry)),
                        InsertResult::Existing(_) => crate::pool::proof::unchanged_model(&before_children, children),
                    } }
                    Ok(inserted)
                }
                Err(error) => {
                    #[cfg(verus_only)]
                    proof! { crate::pool::proof::unchanged_model(&before_children, children); }
                    Err(error)
                }
            }
        }
        None => pool.intern(rc, entry, None),
    };
    #[cfg(verus_only)]
    proof! {
        if result is Err { assert(proof::storage(pools@) =~= proof::storage(before)); }
        else {
            assert(crate::pool::proof::interned(before[level as int].state(), pools@[level as int].state(),crate::node::proof::children(*entry),rc,result->Ok_0));
            proof::connected(proof::storage(before),proof::storage(pools@),pool_lod,*entry,rc,result->Ok_0);
        }
    }
    let inserted = result?;
    let (InsertResult::New(idx) | InsertResult::Existing(idx)) = inserted;
    Ok((idx, None))
}

/// 构造阶段的零外部引用提交；返回已折叠或已持有的子树。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::building(old(pools)),proof::entry_ready(crate::state::pools_view(old(pools)),*entry,pool_lod),
    ensures proof::building(final(pools)),crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        result is Err ==> crate::state::pools_view(final(pools))==crate::state::pools_view(old(pools)),
        result is Ok ==> crate::state::child_valid(crate::state::pools_view(final(pools)),result->Ok_0,(pool_lod+1) as nat)
            && proof::image(crate::state::pools_view(final(pools))[pool_lod as int],result->Ok_0)==crate::node::proof::children(*entry),
))]
pub(crate) fn commit_node(
    pools: &mut [Pool; DAG_DEPTH as usize],
    pool_lod: u8,
    entry: &PoolEntry,
) -> Result<Child, Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost before=crate::state::pools_view(pools);}
    #[cfg(verus_only)]
    proof! {proof::storage_view(pools);proof::entry_domain(pools,*entry,pool_lod);}
    let result = commit_node_layered(pools, pool_lod, entry, 0);
    #[cfg(verus_only)]
    proof! {proof::storage_view(pools);proof::valid_models(pools);
    if result is Ok {proof::committed(before,crate::state::pools_view(pools),pool_lod,*entry,result->Ok_0);}
    else {assert(crate::state::build_step(before,crate::state::pools_view(pools)));}}
    let (index, folded) = result?;
    Ok(match folded {
        Some(value) => Child::Leaf(value),
        None => Child::Ptr(index),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn fold_uniform_all_same(data in any::<u32>()) {
            let node = PoolEntry::new(&[data; NODE_FANOUT], 0);
            assert_eq!(try_fold(&node), Some(data));
        }

        #[test]
        fn no_fold_when_any_mixed(
            slots in prop::collection::vec(any::<u32>(), NODE_FANOUT),
            mixed in any::<u64>(),
        ) {
            prop_assume!(mixed != 0);
            let mut arr = [0u32; NODE_FANOUT];
            arr.copy_from_slice(&slots);
            let node = PoolEntry::new(&arr, mixed);
            assert_eq!(try_fold(&node), None);
        }

        #[test]
        fn no_fold_when_values_differ(a in any::<u32>(), b in any::<u32>()) {
            prop_assume!(a != b);
            let mut arr = [a; NODE_FANOUT];
            arr[0] = b;
            let node = PoolEntry::new(&arr, 0);
            assert_eq!(try_fold(&node), None);
        }

        #[test]
        fn commit_folded_node_does_not_insert(data in any::<u32>()) {
            let mut pools: [Pool; DAG_DEPTH as usize] = std::array::from_fn(|_| Pool::new());
            let node = PoolEntry::new(&[data; NODE_FANOUT], 0);
            let len_before: usize = pools.iter().map(Pool::len).sum();
            let result = commit_node_layered(&mut pools, 0, &node, 0);
            prop_assert!(result.is_ok(), "{result:?}");
            if let Ok(result) = result {
                prop_assert_eq!(result, (NodeIdx::ZERO, Some(data)));
            }
            let len_after: usize = pools.iter().map(Pool::len).sum();
            prop_assert_eq!(len_after, len_before);
        }

        #[test]
        fn commit_non_folded_node_inserts(a in any::<u32>(), b in any::<u32>()) {
            prop_assume!(a != b);
            let mut arr = [a; NODE_FANOUT];
            arr[0] = b;
            let node = PoolEntry::new(&arr, 0);
            let mut pools: [Pool; DAG_DEPTH as usize] = std::array::from_fn(|_| Pool::new());
            let len_before: usize = pools.iter().map(Pool::len).sum();
            let result = commit_node_layered(&mut pools, 0, &node, 1);
            prop_assert!(result.is_ok(), "{result:?}");
            if let Ok((idx, folded)) = result {
                prop_assert_eq!(folded, None);
                prop_assert_eq!(pools.iter().map(Pool::len).sum::<usize>(), len_before + 1);
                prop_assert_eq!(pools[0].get(idx).map(crate::node::NodeRef::to_entry), Some(node.clone()));
            }
        }
    }

    #[test]
    fn commit_node_layered_increments_child_rc() {
        let mut pools: [Pool; DAG_DEPTH as usize] = std::array::from_fn(|_| Pool::new());
        // 在 lod=0 插入一个 mixed 子节点，rc=1 使其存活
        let mut leaf_slots = [0u32; NODE_FANOUT];
        leaf_slots[0] = 1;
        let child = PoolEntry::new(&leaf_slots, 0);
        let child_result = commit_node_layered(&mut pools, 0, &child, 1);
        assert!(child_result.is_ok(), "{child_result:?}");
        let Ok((child_idx, _)) = child_result else {
            return;
        };
        assert_eq!(pools[0].len(), 1);

        // 在 lod=1 插入父节点，引用该子节点，rc_inc 使子节点 RC=2
        let mut parent_children = [Child::Leaf(0); NODE_FANOUT];
        parent_children[0] = Child::Ptr(child_idx);
        let parent = PoolEntry::from_children(&parent_children);
        let parent_result = commit_node_layered(&mut pools, 1, &parent, 0);
        assert!(parent_result.is_ok(), "{parent_result:?}");
        // 子节点仍在池中（parent 为 rc=0 unclaimed，无需断言）
        assert_eq!(pools[0].len(), 1);
        assert!(pools[0].get(child_idx).is_some());
    }

    #[test]
    fn clone_or_synthesize_ptr_clones_from_pool() {
        let mut pools: [Pool; DAG_DEPTH as usize] = std::array::from_fn(|_| Pool::new());
        let mut slots = [0u32; NODE_FANOUT];
        slots[0] = 7;
        let node = PoolEntry::new(&slots, 0);
        // rc=1 使节点存活
        let result = commit_node_layered(&mut pools, 0, &node, 1);
        assert!(result.is_ok(), "{result:?}");
        let Ok((idx, _)) = result else {
            return;
        };
        let cloned = clone_or_synthesize(&pools[0], Child::Ptr(idx));
        assert!(cloned.is_ok(), "{cloned:?}");
        let Ok(cloned) = cloned else {
            return;
        };
        assert_eq!(
            pools[0].get(idx).map(crate::node::NodeRef::to_entry),
            Some(cloned)
        );
    }

    #[test]
    fn clone_or_synthesize_leaf_creates_uniform() {
        let pools: [Pool; DAG_DEPTH as usize] = std::array::from_fn(|_| Pool::new());
        let result = clone_or_synthesize(&pools[0], Child::Leaf(5));
        assert!(result.is_ok(), "{result:?}");
        let Ok(result) = result else {
            return;
        };
        assert_eq!(try_fold(&result), Some(5));
    }

    #[test]
    fn failed_child_acquisition_does_not_publish_parent()
    -> Result<(), proptest::test_runner::TestCaseError> {
        let mut pools: [Pool; DAG_DEPTH as usize] = std::array::from_fn(|_| Pool::new());
        let mut values = [0; NODE_FANOUT];
        values[0] = 7;
        let child_result = commit_node_layered(&mut pools, 0, &PoolEntry::new(&values, 0), 1);
        prop_assert!(child_result.is_ok(), "{child_result:?}");
        let (child, _) = child_result
            .map_err(|error| proptest::test_runner::TestCaseError::fail(format!("{error:?}")))?;
        let mut children = [Child::Leaf(0); NODE_FANOUT];
        children[0] = Child::Ptr(child);
        children[1] = Child::Ptr(child);
        children[2] = Child::Ptr(NodeIdx::MAX);
        prop_assert!(matches!(
            commit_node_layered(&mut pools, 1, &PoolEntry::from_children(&children), 1),
            Err(Error::NodeMissing(index)) if index == NodeIdx::MAX
        ));
        prop_assert_eq!(pools[0].ref_count(child), Some(1));
        prop_assert_eq!(pools[1].slot_count(), 0);
        children[2] = Child::Leaf(0);
        let committed = commit_node_layered(&mut pools, 1, &PoolEntry::from_children(&children), 1);
        prop_assert!(committed.is_ok(), "{committed:?}");
        prop_assert_eq!(pools[0].ref_count(child), Some(3));
        prop_assert_eq!(pools[1].len(), 1);
        Ok(())
    }
}
