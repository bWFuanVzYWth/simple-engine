use crate::hashdag::{DAG_DEPTH, Error, HashDag, NODE_FANOUT, RootId};
#[cfg(not(verus_only))]
use crate::vec_ops;
#[cfg(verus_only)]
use hash_dag_std::vec_ops;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "../proof/write_cleanup.rs"]
pub(crate) mod proof;
#[cfg(test)]
use crate::{node::Child, pool::Pool, write::frames::WriteScratch};

impl HashDag {
    /// 新节点各持有一份构造引用；完整认领子边后入池，旧根持续存活到发布。
    /// 所有退出路径释放构造引用，因此失败不遗留节点，成功不保留额外引用。
    ///
    /// 串行且阻塞；每个新唯一节点记录一个 NodeIdx，结束后保留各层峰值容量。
    /// 回收栈至多 1 + (FANOUT - 1) * DEPTH 项，在写入前准备并复用。
    /// 候选帧和 dense 累积器都按树深度保留堆后备，奇数 LOD 按序展开。
    /// 新唯一节点的构造记录仍可随整批增长；完成或失败后清空 scratch 长度，不缩容。
    /// scratch 不持有节点引用；发布与构造引用释放后即丢弃构建描述。
    /// root 身份检查后执行完整输入校验，通过后才准备资源，不重复扫描输入。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures super::proof::written(old(self), final(self), root, batch, result),
    ))]
    pub(super) fn write_root<B: super::Batch>(
        &mut self,
        root: RootId,
        batch: B,
    ) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost origin = *self; }
        #[cfg(verus_only)]
        proof! { crate::balance::stable_has_no_orphans(crate::state::pools_view(&self.pools), self.root_children()); }
        let old_root = self.root_child(root)?;
        #[cfg(verus_only)]
        proof! { crate::hashdag::view_proof::root_valid(self, root, old_root); }
        batch.validate()?;
        self.scratch_write.clear();
        let release_capacity = 1 + (NODE_FANOUT - 1) * usize::from(DAG_DEPTH);
        vec_ops::clear(&mut self.scratch_stack);
        vec_ops::reserve(&mut self.scratch_stack, release_capacity)?;
        #[cfg(verus_only)]
        proof_decl! { let ghost before_phase = crate::state::pools_view(&self.pools); }
        crate::layers::set_phase::<true>(&mut self.pools);
        #[cfg(verus_only)]
        proof! { crate::layers::proof::phase_balance(before_phase, crate::state::pools_view(&self.pools), self.root_children(), true); }
        #[cfg(verus_only)]
        proof_decl! { let ghost phase = crate::state::pools_view(&self.pools); }
        let candidate = batch.build(&mut self.pools, old_root, &mut self.scratch_write);
        #[cfg(verus_only)]
        proof_decl! { let ghost built = *self; }
        let result = match candidate {
            Ok(new_root) => {
                #[cfg(verus_only)]
                proof! { crate::balance::build_keeps_balance(phase, crate::state::pools_view(&self.pools), self.root_children()); }
                self.replace_root(root, old_root, new_root)
            }
            Err(error) => {
                #[cfg(verus_only)]
                proof! { crate::balance::build_keeps_balance(phase, crate::state::pools_view(&self.pools), self.root_children()); }
                Err(error)
            }
        };
        #[cfg(verus_only)]
        proof_decl! { let ghost published = *self; }
        let cleanup = self.release_construction();
        self.scratch_write.clear();
        #[cfg(verus_only)]
        proof! { super::proof::finish(&origin, phase, &built, &published, self, root, old_root, batch, candidate, result); }
        match result {
            Ok(()) => cleanup,
            Err(error) => Err(error),
        }
    }

    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::eligible(old(self)),
        ensures result is Ok, final(self).valid(),
            old(self).identity_owned() ==> final(self).identity_owned(),
            final(self).roots.same_entries(&old(self).roots),
            crate::state::frozen(crate::state::pools_view(&final(self).pools), crate::state::pools_view(&old(self).pools)),
            crate::balance::no_orphans(crate::state::pools_view(&final(self).pools), final(self).root_children()),
            forall|r: int, path: int| 0 <= r < final(self).root_children().len() && 0 <= path < crate::state::span(10) ==>
                #[trigger] crate::state::eval(crate::state::pools_view(&final(self).pools), final(self).root_children()[r], 10, path)
                    == crate::state::eval(crate::state::pools_view(&old(self).pools), final(self).root_children()[r], 10, path),
            hash_dag_std::vec_model::capacity(final(self).scratch_stack) == hash_dag_std::vec_model::capacity(old(self).scratch_stack),
    ))]
    pub(super) fn release_construction(&mut self) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost origin = crate::state::pools_view(&self.pools); }
        let mut result = Ok(());
        let mut remaining = DAG_DEPTH;
        #[cfg_attr(verus_only, verus_spec(
            invariant remaining <= 10, result is Ok, proof::eligible(self),
                self.roots.same_entries(&old(self).roots),
                crate::state::frozen(crate::state::pools_view(&self.pools), origin),
                hash_dag_std::vec_model::capacity(self.scratch_stack) == hash_dag_std::vec_model::capacity(old(self).scratch_stack),
                forall|l: int| remaining <= l < 10 ==> crate::state::pools_view(&self.pools)[l].retained.len() == 0
                    && !crate::state::pools_view(&self.pools)[l].constructing,
            decreases remaining,
        ))]
        while remaining > 0 {
            let level = remaining - 1;
            #[cfg_attr(verus_only, verus_spec(
                invariant remaining <= 10, remaining > 0, level + 1 == remaining,
                    result is Ok, proof::eligible(self),
                    self.roots.same_entries(&old(self).roots),
                    crate::state::frozen(crate::state::pools_view(&self.pools), origin),
                    hash_dag_std::vec_model::capacity(self.scratch_stack) == hash_dag_std::vec_model::capacity(old(self).scratch_stack),
                    forall|l: int| remaining <= l < 10 ==> crate::state::pools_view(&self.pools)[l].retained.len() == 0
                        && !crate::state::pools_view(&self.pools)[l].constructing,
                ensures crate::state::pools_view(&self.pools)[level as int].retained.len() == 0,
                decreases crate::state::pools_view(&self.pools)[level as int].retained.len(),
            ))]
            loop {
                #[cfg(verus_only)]
                proof_decl! { let ghost before = crate::state::pools_view(&self.pools); }
                let next = self.pools[usize::from(level)].pop_retained();
                #[cfg(verus_only)]
                proof! {
                    assert forall|l: int| 0 <= l < 10 && l != level implies
                        #[trigger] crate::state::pools_view(&self.pools)[l] == before[l] by {}
                }
                let Some(idx) = next else {
                    #[cfg(verus_only)]
                    proof! { assert(crate::state::pools_view(&self.pools) =~= before); }
                    break;
                };
                #[cfg(verus_only)]
                proof! { proof::popped(before, crate::state::pools_view(&self.pools), self.root_children(), level, idx); }
                #[cfg(verus_only)]
                proof_decl! { let ghost middle = crate::state::pools_view(&self.pools); }
                let release = crate::rc::rc_dec_or_remove(
                    &mut self.pools,
                    idx,
                    level,
                    &mut self.scratch_stack,
                );
                #[cfg(verus_only)]
                proof! {
                    proof::released(before, middle, crate::state::pools_view(&self.pools), self.root_children(), level, idx);
                    crate::balance::frozen_transitive(crate::state::pools_view(&self.pools), before, origin);
                }
                result = match result {
                    Ok(()) => release,
                    Err(error) => Err(error),
                };
            }
            #[cfg(verus_only)]
            proof_decl! { let ghost before = crate::state::pools_view(&self.pools); }
            self.pools[usize::from(level)].end_write();
            #[cfg(verus_only)]
            proof! {
                assert forall|l: int| 0 <= l < 10 && l != level implies
                    #[trigger] crate::state::pools_view(&self.pools)[l] == before[l] by {}
                proof::ended(before, crate::state::pools_view(&self.pools), self.root_children(), level);
                crate::balance::frozen_transitive(crate::state::pools_view(&self.pools), before, origin);
            }
            remaining -= 1;
        }
        #[cfg(verus_only)]
        proof! {
            crate::balance::stable_has_no_orphans(crate::state::pools_view(&self.pools), self.root_children());
            proof::preserved_roots(origin, crate::state::pools_view(&self.pools), self.root_children());
        }
        result
    }
}

#[cfg(test)]
impl HashDag {
    /// 故障测试借助真实事务入口注入中途失败，不替代生产批写实现。
    pub(super) fn write_root_test<V, B>(
        &mut self,
        root: RootId,
        validate: V,
        build: B,
    ) -> Result<(), Error>
    where
        V: Fn() -> Result<(), Error>,
        B: FnOnce(
            &mut [Pool; DAG_DEPTH as usize],
            Child,
            &mut WriteScratch,
        ) -> Result<Child, Error>,
    {
        struct Inject<V, B> {
            validate: V,
            build: B,
        }
        impl<V, B> super::Batch for Inject<V, B>
        where
            V: Fn() -> Result<(), Error>,
            B: FnOnce(
                &mut [Pool; DAG_DEPTH as usize],
                Child,
                &mut WriteScratch,
            ) -> Result<Child, Error>,
        {
            fn validate(&self) -> Result<(), Error> {
                (self.validate)()
            }
            fn build(
                self,
                pools: &mut [Pool; DAG_DEPTH as usize],
                root: Child,
                scratch: &mut WriteScratch,
            ) -> Result<Child, Error> {
                (self.build)(pools, root, scratch)
            }
        }
        self.write_root(root, Inject { validate, build })
    }
}

#[cfg(test)]
mod tests {
    use proptest::{prop_assert, prop_assert_eq, prop_assert_ne, test_runner::TestCaseError};

    use crate::hashdag::ROOT_LOD;
    use crate::node::{NodeIdx, PoolEntry};
    use crate::write::commit::commit_node_layered;
    use crate::{DenseRegion, SetCell, UniformFill, ZOrder20x3};

    use super::*;

    fn checked<T>(result: Result<T, Error>) -> Result<T, TestCaseError> {
        result.map_err(|error| TestCaseError::fail(format!("HashDag 返回意外错误： {error:?}")))
    }

    fn intern_nonuniform(
        pools: &mut [Pool; DAG_DEPTH as usize],
        level: u8,
        entry: &PoolEntry,
    ) -> Result<NodeIdx, Error> {
        let (index, folded) = commit_node_layered(pools, level, entry, 0)?;
        if folded.is_some() {
            return Err(Error::BuildFrameMissing);
        }
        Ok(index)
    }

    fn scratch_is_empty(scratch: &WriteScratch) -> bool {
        scratch.frames.is_empty() && scratch.dense.is_empty()
    }

    fn scratch_capacities(scratch: &WriteScratch) -> [usize; 2] {
        [scratch.frames.capacity(), scratch.dense.capacity()]
    }

    fn path(code: u64) -> Result<ZOrder20x3, TestCaseError> {
        ZOrder20x3::from_value(code).ok_or_else(|| TestCaseError::fail("invalid sample path"))
    }

    #[test]
    fn mixed_writes_reuse_peak_capacity_and_clear_scratch() -> Result<(), TestCaseError> {
        let cells = (0..256_u64)
            .map(|index| {
                Ok(SetCell {
                    path: path(index << 50)?,
                    data: 7,
                })
            })
            .collect::<Result<Vec<_>, TestCaseError>>()?;
        let fills = (0..64_u64)
            .map(|index| {
                Ok(UniformFill {
                    path: path((index << 9) + (1 << 30))?,
                    lod: 1,
                    data: 9,
                })
            })
            .collect::<Result<Vec<_>, TestCaseError>>()?;
        let dense_values = (0..1_u32 << 18)
            .map(|code| code.wrapping_mul(0x9e37_79b9).rotate_left(13))
            .collect::<Vec<_>>();
        let chunks = [DenseRegion {
            path: ZOrder20x3::ZERO,
            lod: 6,
            values: &dense_values,
        }];
        let mut dag = checked(HashDag::new())?;
        let root = dag.root();
        for _ in 0..2 {
            checked(dag.set_cells(root, &cells))?;
            checked(dag.fill_uniform(root, &fills))?;
            checked(dag.set_dense_regions(root, &chunks))?;
        }
        let capacities = scratch_capacities(&dag.scratch_write);
        prop_assert!(capacities.iter().all(|capacity| *capacity > 0));
        eprintln!(
            "write scratch retained_buffer_bytes={}",
            dag.scratch_write.retained_bytes()
        );
        for _ in 0..4 {
            checked(dag.set_cells(root, &cells))?;
            prop_assert!(scratch_is_empty(&dag.scratch_write));
            checked(dag.fill_uniform(root, &fills))?;
            prop_assert!(scratch_is_empty(&dag.scratch_write));
            checked(dag.set_dense_regions(root, &chunks))?;
            prop_assert!(scratch_is_empty(&dag.scratch_write));
            checked(dag.set_cells(root, &[]))?;
            checked(dag.fill_uniform(root, &[]))?;
            checked(dag.set_dense_regions(root, &[]))?;
            prop_assert!(scratch_is_empty(&dag.scratch_write));
            prop_assert_eq!(scratch_capacities(&dag.scratch_write), capacities);
            for index in [0_usize, 63, 64, 4095, 4096, dense_values.len() - 1] {
                let code = u64::try_from(index).map_err(|error| {
                    TestCaseError::fail(format!("invalid sample index: {error}"))
                })?;
                let expected = dense_values
                    .get(index)
                    .copied()
                    .ok_or_else(|| TestCaseError::fail("missing dense sample"))?;
                prop_assert_eq!(checked(dag.value_at(root, path(code)?))?, expected);
            }
        }
        // 容量复用只是资源契约检查；没有计量 allocator 调用或所有节点池分配。
        Ok(())
    }

    #[test]
    fn failed_build_clears_frames_without_releasing_borrowed_children() -> Result<(), TestCaseError>
    {
        let mut dag = checked(HashDag::new())?;
        let root = dag.root();
        let path = ZOrder20x3::ZERO;
        checked(dag.set_cells(root, &[SetCell { path, data: 7 }]))?;
        let original = checked(dag.root_child(root))?;
        let Child::Ptr(index) = original else {
            return Err(TestCaseError::fail("root is not a node"));
        };
        let previous_lengths = dag.pools.each_ref().map(Pool::len);
        let result = dag.write_root_test(
            root,
            || Ok(()),
            |pools, old_root, scratch| {
                let node = pools
                    .get(usize::from(ROOT_LOD))
                    .ok_or(Error::InvalidLod(ROOT_LOD))?
                    .get(index)
                    .ok_or(Error::NodeMissing(index))?
                    .to_entry();
                scratch.frames.push(crate::write::frames::BuildFrame {
                    entry: node,
                    original: old_root,
                    path,
                    any_modified: true,
                    pool_lod: ROOT_LOD,
                });
                scratch
                    .dense
                    .push(crate::write::dense_chunks::DenseFrame::default());
                Err(Error::InvalidInput)
            },
        );
        prop_assert!(matches!(result, Err(Error::InvalidInput)));
        prop_assert!(scratch_is_empty(&dag.scratch_write));
        prop_assert_eq!(checked(dag.root_child(root))?, original);
        prop_assert_eq!(dag.pools.each_ref().map(Pool::len), previous_lengths);
        prop_assert_eq!(
            checked(
                dag.pools
                    .get(usize::from(ROOT_LOD))
                    .ok_or(Error::InvalidLod(ROOT_LOD))
            )?
            .ref_count(index),
            Some(1)
        );
        checked(dag.set_cells(root, &[SetCell { path, data: 9 }]))?;
        prop_assert_eq!(checked(dag.value_at(root, path))?, 9);
        prop_assert!(scratch_is_empty(&dag.scratch_write));
        Ok(())
    }

    #[test]
    fn alternating_writes_preserve_snapshot_after_invalid_and_empty_batches()
    -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = dag.root();
        let original =
            std::array::from_fn::<_, 512, _>(|index| u32::from(index.is_multiple_of(3)) + 1);
        let replacement = original.map(|value| value + 4);
        let dense = |values| DenseRegion {
            path: ZOrder20x3::ZERO,
            lod: 3,
            values,
        };
        checked(dag.set_dense_regions(root, &[dense(&original)]))?;
        let snapshot = checked(dag.fork_root(root))?;
        let snapshot_child = checked(dag.root_child(snapshot))?;

        for data in 7..11 {
            checked(dag.set_cells(root, &[]))?;
            checked(dag.fill_uniform(root, &[]))?;
            checked(dag.set_dense_regions(root, &[]))?;
            checked(dag.set_cells(
                root,
                &[SetCell {
                    path: ZOrder20x3::ZERO,
                    data,
                }],
            ))?;
            let before_invalid = checked(dag.root_child(root))?;
            let invalid = UniformFill {
                path: ZOrder20x3::ZERO,
                lod: 1,
                data,
            };
            prop_assert!(matches!(
                dag.fill_uniform(root, &[invalid, invalid]),
                Err(Error::InvalidInput)
            ));
            prop_assert!(matches!(
                dag.set_dense_regions(root, &[dense(&original[..511])]),
                Err(Error::InvalidDenseValues)
            ));
            prop_assert_eq!(checked(dag.root_child(root))?, before_invalid);
            checked(dag.fill_uniform(root, &[invalid]))?;
            checked(dag.set_dense_regions(root, &[dense(&replacement)]))?;
            prop_assert_eq!(checked(dag.root_child(snapshot))?, snapshot_child);
            for (index, (&expected, &snapshot_value)) in
                replacement.iter().zip(&original).enumerate()
            {
                let path = ZOrder20x3::from_value(u64::try_from(index).map_err(|error| {
                    TestCaseError::fail(format!("invalid sample index: {error}"))
                })?)
                .ok_or_else(|| TestCaseError::fail("invalid sample path"))?;
                prop_assert_eq!(checked(dag.value_at(root, path))?, expected);
                prop_assert_eq!(checked(dag.value_at(snapshot, path))?, snapshot_value);
            }
        }
        checked(dag.release_root(snapshot))?;
        Ok(())
    }

    #[test]
    fn failed_build_releases_chain_and_preserves_snapshots() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = dag.root();
        let path = ZOrder20x3::ZERO;
        checked(dag.set_cells(root, &[SetCell { path, data: 7 }]))?;
        let snapshot = checked(dag.fork_root(root))?;
        let original = checked(dag.root_child(root))?;
        let previous_lengths = dag.pools.each_ref().map(Pool::len);
        let mut new_indices = [None; DAG_DEPTH as usize];
        let mut built_all_levels = false;

        let result = dag.write_root_test(
            root,
            || Ok(()),
            |pools, _, _| {
                let mut child = Child::Leaf(9);
                for (level, recorded) in (0..DAG_DEPTH).zip(&mut new_indices) {
                    let mut children = [Child::Leaf(0); NODE_FANOUT];
                    children[0] = child;
                    children[1] = child;
                    let index =
                        intern_nonuniform(pools, level, &PoolEntry::from_children(&children))?;
                    *recorded = Some(index);
                    child = Child::Ptr(index);
                }
                built_all_levels = true;
                Err(Error::InvalidInput)
            },
        );

        prop_assert!(built_all_levels);
        prop_assert!(matches!(result, Err(Error::InvalidInput)));
        prop_assert_eq!(checked(dag.root_child(root))?, original);
        prop_assert_eq!(checked(dag.root_child(snapshot))?, original);
        prop_assert_eq!(dag.pools.each_ref().map(Pool::len), previous_lengths);
        for (pool, recorded) in dag.pools.iter().zip(new_indices) {
            let index = recorded.ok_or_else(|| TestCaseError::fail("missing constructed node"))?;
            prop_assert!(pool.get(index).is_none());
            prop_assert_eq!(pool.ref_count(index), Some(0));
        }
        for current in [root, snapshot] {
            prop_assert_eq!(checked(dag.value_at(current, path))?, 7);
            let neighbor = ZOrder20x3::from_value(1)
                .ok_or_else(|| TestCaseError::fail("invalid neighbor path"))?;
            prop_assert_eq!(checked(dag.value_at(current, neighbor))?, 0);
        }

        checked(dag.set_cells(root, &[SetCell { path, data: 11 }]))?;
        prop_assert_eq!(checked(dag.value_at(root, path))?, 11);
        prop_assert_eq!(checked(dag.value_at(snapshot, path))?, 7);
        checked(dag.fill_uniform(
            root,
            &[UniformFill {
                path,
                lod: HashDag::AXIS_BITS,
                data: 0,
            }],
        ))?;
        checked(dag.release_root(snapshot))?;
        prop_assert!(dag.pools.iter().all(|pool| pool.len() == 0));
        Ok(())
    }

    #[test]
    fn failed_build_releases_nonadjacent_shared_patterns() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = dag.root();
        let mut recorded_indices = None;
        let mut recorded_counts = None;
        let mut recorded_lengths = None;

        let result = dag.write_root_test(
            root,
            || Ok(()),
            |pools, _, _| {
                let mut leaf_values = [0; NODE_FANOUT];
                leaf_values[0] = 3;
                let leaf = intern_nonuniform(pools, 0, &PoolEntry::new(&leaf_values, 0))?;
                let children_a = [Child::Ptr(leaf); NODE_FANOUT];
                let entry_a = PoolEntry::from_children(&children_a);
                let first_a = intern_nonuniform(pools, 1, &entry_a)?;
                let mut children_b = children_a;
                children_b[0] = Child::Leaf(2);
                let b = intern_nonuniform(pools, 1, &PoolEntry::from_children(&children_b))?;
                let second_a = intern_nonuniform(pools, 1, &entry_a)?;
                recorded_indices = Some([leaf, first_a, b, second_a]);
                recorded_counts = Some([
                    pools[0].ref_count(leaf),
                    pools[1].ref_count(first_a),
                    pools[1].ref_count(b),
                ]);
                recorded_lengths = Some([pools[0].len(), pools[1].len()]);
                Err(Error::InvalidInput)
            },
        );

        prop_assert!(matches!(result, Err(Error::InvalidInput)));
        let [leaf, first_a, b, second_a] = recorded_indices
            .ok_or_else(|| TestCaseError::fail("failed before repeated pattern was constructed"))?;
        prop_assert_eq!(first_a, second_a);
        prop_assert_ne!(first_a, b);
        prop_assert_eq!(recorded_lengths, Some([1, 2]));
        // 新叶的一份构造引用，以及两个不同父节点分别认领的 64、63 条边。
        prop_assert_eq!(recorded_counts, Some([Some(128), Some(1), Some(1)]));
        prop_assert_eq!(checked(dag.root_child(root))?, Child::Leaf(0));
        prop_assert!(dag.pools.iter().all(|pool| pool.len() == 0));
        prop_assert!(dag.pools[0].get(leaf).is_none());
        prop_assert_eq!(dag.pools[0].ref_count(leaf), Some(0));
        for index in [first_a, b] {
            prop_assert!(dag.pools[1].get(index).is_none());
            prop_assert_eq!(dag.pools[1].ref_count(index), Some(0));
        }
        Ok(())
    }
}
