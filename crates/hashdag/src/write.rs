#[path = "write/built.rs"]
pub mod built;
#[path = "write/commit.rs"]
pub mod commit;
#[path = "write/dense_chunks.rs"]
pub mod dense_chunks;
#[path = "write/fill_uniform.rs"]
pub mod fill_uniform;
#[path = "write/frames.rs"]
pub mod frames;
#[path = "write/frontier.rs"]
mod frontier;
#[path = "write/import.rs"]
mod import;
#[path = "write/input.rs"]
mod input;
#[path = "write/ordered.rs"]
mod ordered;
#[path = "write/patch.rs"]
pub(crate) mod patch;
#[path = "write/resolve.rs"]
mod resolve;
#[path = "write/route.rs"]
mod route;
#[path = "write/set_cells.rs"]
pub mod set_cells;
#[path = "write/transaction.rs"]
mod transaction;
#[path = "write/validate.rs"]
mod validate;

use crate::hashdag::{Error, HashDag, RootId};
use crate::node::Child;
use crate::pool::Pool;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "proof/write_batch.rs"]
pub(crate) mod proof;
#[cfg(verus_only)]
use proof::BatchModel;
#[cfg(not(verus_only))]
pub(super) trait BatchModel {}
#[cfg(not(verus_only))]
impl<T> BatchModel for T {}

/// 静态批写输入；校验与构造只借用调用方数据，不保存展开后的批次。
#[cfg_attr(verus_only, verus_verify)]
pub(super) trait Batch: BatchModel {
    #[cfg_attr(verus_only, verus_spec(result => ensures result is Ok <==> self.accepted()))]
    fn validate(&self) -> Result<(), Error>;
    #[cfg_attr(verus_only, verus_spec(result =>
        requires self.accepted(), commit::proof::building(old(pools)),
            old(scratch).routing_empty(),
            crate::state::child_valid(crate::state::pools_view(old(pools)), old_root, 10),
        ensures commit::proof::building(final(pools)),
            crate::state::build_step(crate::state::pools_view(old(pools)), crate::state::pools_view(final(pools))),
            result is Ok ==> crate::state::child_valid(crate::state::pools_view(final(pools)), result->Ok_0, 10)
                && forall|path: int| 0 <= path < crate::state::span(10) ==>
                    #[trigger] crate::state::eval(crate::state::pools_view(final(pools)), result->Ok_0, 10, path)
                        == self.effect(|p: int| crate::state::eval(crate::state::pools_view(old(pools)), old_root, 10, p), path),
    ))]
    fn build(
        self,
        pools: &mut [Pool; crate::hashdag::DAG_DEPTH as usize],
        old_root: Child,
        scratch: &mut frames::WriteScratch,
    ) -> Result<Child, Error>;
}

impl HashDag {
    /// 同步修改单点数据。`cells` 须按 `path` 非降序，同坐标采用最后一项。
    /// 空批次保持原值；所有输入和值在调用结束后均不保留借用。
    ///
    /// # Errors
    ///
    /// root 失效或属于其他实例、输入未排序、节点缺失、容量或引用计数耗尽、
    /// 分配失败时返回错误；失败保持原 root。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only,verus_spec(result =>
        requires old(self).valid(),ensures final(self).set_cells_post(old(self),root,cells,result),
    ))]
    pub fn set_cells(&mut self, root: RootId, cells: &[set_cells::SetCell]) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof! {crate::balance::stable_has_no_orphans(crate::state::pools_view(&self.pools),self.root_children());}
        if cells.is_empty() {
            return self.root_child(root).map(|_| ());
        }
        let batch = set_cells::Cells::checked(cells)?;
        self.write_root(root, batch)
    }

    /// 同步填充均匀区域。`fills` 按对齐后的 `path` 升序且互不重叠。
    /// `path` 按各项 `lod` 向下对齐；空批次保持原值。
    /// 奇数空间 LOD 按序展开八个原生子树，复用实例持有的深度有界候选缓冲。
    /// root 身份与输入约束在资源准备前检查，输入错误不增加保留容量。
    ///
    /// # Errors
    ///
    /// root 失效或属于其他实例、LOD 越界、区域重叠或未排序、节点缺失、
    /// 容量或引用计数耗尽、分配失败时返回错误；失败保持原 root。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only,verus_spec(result =>
        requires old(self).valid(),ensures final(self).fill_uniform_post(old(self),root,fills,result),
    ))]
    pub fn fill_uniform(
        &mut self,
        root: RootId,
        fills: &[fill_uniform::UniformFill],
    ) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof! {crate::balance::stable_has_no_orphans(crate::state::pools_view(&self.pools),self.root_children());}
        if fills.is_empty() {
            return self.root_child(root).map(|_| ());
        }
        self.write_root(root, fill_uniform::Fills(fills))
    }
}

#[cfg(test)]
mod tests {
    use crate::ZOrder20x3;
    use proptest::{prop_assert, prop_assert_eq, prop_assert_ne, test_runner::TestCaseError};

    use crate::hashdag::{DAG_DEPTH, Error, HashDag};
    use crate::node::Child;
    use crate::test_util;
    use crate::write::fill_uniform::UniformFill;
    use crate::write::set_cells::SetCell;

    fn checked<T>(result: Result<T, Error>) -> Result<T, TestCaseError> {
        result.map_err(|error| TestCaseError::fail(format!("HashDag 返回意外错误： {error:?}")))
    }

    #[test]
    fn empty_writes_still_validate_owner() -> Result<(), Error> {
        let source = HashDag::new()?;
        let mut target = HashDag::new()?;
        let root = source.root();
        assert!(matches!(
            target.set_cells(root, &[]),
            Err(Error::WrongOwner)
        ));
        assert!(matches!(
            target.fill_uniform(root, &[]),
            Err(Error::WrongOwner)
        ));
        assert!(matches!(
            target.set_dense_regions(root, &[]),
            Err(Error::WrongOwner)
        ));
        Ok(())
    }

    #[test]
    fn invalid_dense_tail_does_not_prepare_resources() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let before = dag.retained_bytes();
        let regions = [
            crate::DenseRegion {
                path: ZOrder20x3::ZERO,
                lod: 0,
                values: &[7],
            },
            crate::DenseRegion {
                path: ZOrder20x3::from_value(8).unwrap(),
                lod: 0,
                values: &[],
            },
        ];
        assert!(matches!(
            dag.set_dense_regions(dag.root(), &regions),
            Err(Error::InvalidDenseValues)
        ));
        assert_eq!(dag.retained_bytes(), before);
        Ok(())
    }

    #[test]
    fn invalid_fill_tail_does_not_prepare_resources() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let before = dag.retained_bytes();
        let fills = [
            UniformFill {
                path: ZOrder20x3::ZERO,
                lod: 0,
                data: 7,
            },
            UniformFill {
                path: ZOrder20x3::from_value(8).unwrap(),
                lod: HashDag::AXIS_BITS + 1,
                data: 9,
            },
        ];
        assert!(matches!(
            dag.fill_uniform(dag.root(), &fills),
            Err(Error::InvalidLod(_))
        ));
        assert_eq!(dag.retained_bytes(), before);
        Ok(())
    }

    #[test]
    fn invalid_region_data_does_not_override_root_identity_error() -> Result<(), Error> {
        let source = HashDag::new()?;
        let mut target = HashDag::new()?;
        let released = target.fork_root(target.root())?;
        target.release_root(released)?;
        let regions = [crate::DenseRegion {
            path: ZOrder20x3::ZERO,
            lod: HashDag::AXIS_BITS + 1,
            values: &[],
        }];
        let fills = [UniformFill {
            path: ZOrder20x3::ZERO,
            lod: HashDag::AXIS_BITS + 1,
            data: 7,
        }];
        for (root, expected) in [
            (source.root(), Error::WrongOwner),
            (released, Error::RootMissing),
        ] {
            for result in [
                target.set_dense_regions(root, &regions),
                target.fill_uniform(root, &fills),
            ] {
                let error = result.unwrap_err();
                assert_eq!(
                    std::mem::discriminant(&error),
                    std::mem::discriminant(&expected),
                    "{error:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn unchanged_batches_do_not_retain_replaced_roots() -> Result<(), TestCaseError> {
        let path = ZOrder20x3::ZERO;
        let same_value = [SetCell { path, data: 1 }];
        let reverted = [SetCell { path, data: 2 }, SetCell { path, data: 1 }];
        for batch in [same_value.as_slice(), reverted.as_slice()] {
            let mut dag = checked(HashDag::new())?;
            let root = dag.root();
            checked(dag.set_cells(root, &same_value))?;
            let original = checked(dag.root_child(root))?;
            prop_assert!(matches!(original, Child::Ptr(_)));

            checked(dag.set_cells(root, batch))?;
            prop_assert_eq!(checked(dag.root_child(root))?, original);
            prop_assert_eq!(checked(dag.value_at(root, path))?, 1);
            checked(dag.fill_uniform(
                root,
                &[UniformFill {
                    path,
                    lod: HashDag::AXIS_BITS,
                    data: 0,
                }],
            ))?;
            prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        }
        Ok(())
    }

    #[test]
    fn snapshot_releases_after_reverted_batch() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = dag.root();
        let path = ZOrder20x3::ZERO;
        checked(dag.set_cells(root, &[SetCell { path, data: 1 }]))?;
        let snapshot = checked(dag.fork_root(root))?;

        checked(dag.set_cells(
            root,
            &[SetCell { path, data: 2 }, SetCell { path, data: 1 }],
        ))?;
        prop_assert_eq!(
            checked(dag.root_child(root))?,
            checked(dag.root_child(snapshot))?
        );
        checked(dag.fill_uniform(
            root,
            &[UniformFill {
                path,
                lod: HashDag::AXIS_BITS,
                data: 0,
            }],
        ))?;
        prop_assert_eq!(checked(dag.value_at(snapshot, path))?, 1);
        prop_assert_eq!(test_util::total_len(&dag.pools), usize::from(DAG_DEPTH));
        checked(dag.release_root(snapshot))?;
        prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        Ok(())
    }

    #[test]
    fn reverted_batches_reuse_slots_across_versions() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = dag.root();
        let path = ZOrder20x3::ZERO;
        for data in 1..=32 {
            checked(dag.set_cells(root, &[SetCell { path, data }]))?;
            checked(dag.set_cells(
                root,
                &[
                    SetCell {
                        path,
                        data: data + 1,
                    },
                    SetCell { path, data },
                ],
            ))?;
            prop_assert_eq!(checked(dag.value_at(root, path))?, data);
            checked(dag.fill_uniform(
                root,
                &[UniformFill {
                    path,
                    lod: HashDag::AXIS_BITS,
                    data: 0,
                }],
            ))?;
            prop_assert_eq!(test_util::total_len(&dag.pools), 0);
            for lod in 0..DAG_DEPTH {
                prop_assert_eq!(checked(dag.pool_slot_count(lod))?, 1);
            }
        }
        Ok(())
    }

    #[test]
    fn changed_batch_can_reuse_snapshot_root() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = dag.root();
        let path = ZOrder20x3::ZERO;
        checked(dag.set_cells(root, &[SetCell { path, data: 1 }]))?;
        let snapshot = checked(dag.fork_root(root))?;
        checked(dag.set_cells(root, &[SetCell { path, data: 2 }]))?;
        prop_assert_ne!(
            checked(dag.root_child(root))?,
            checked(dag.root_child(snapshot))?
        );

        checked(dag.set_cells(root, &[SetCell { path, data: 1 }]))?;
        prop_assert_eq!(
            checked(dag.root_child(root))?,
            checked(dag.root_child(snapshot))?
        );
        checked(dag.release_root(snapshot))?;
        prop_assert_eq!(checked(dag.value_at(root, path))?, 1);
        checked(dag.fill_uniform(
            root,
            &[UniformFill {
                path,
                lod: HashDag::AXIS_BITS,
                data: 0,
            }],
        ))?;
        prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        Ok(())
    }

    #[test]
    fn mixed_region_depths_preserve_gaps() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let root = dag.root();
        dag.fill_uniform(
            root,
            &[UniformFill {
                path: ZOrder20x3::ZERO,
                lod: HashDag::AXIS_BITS,
                data: 17,
            }],
        )?;
        let mut fills = Vec::new();
        for (base, lod) in [(0, 1), (64, 2), (512, 3), (4096, 4), (32768, 5)] {
            fills.push(UniformFill {
                path: ZOrder20x3::from_value(base + 1).expect("区域内坐标"),
                lod,
                data: u32::from(lod),
            });
        }
        dag.fill_uniform(root, &fills)?;
        for fill in fills {
            let span = 1_u64 << (u32::from(fill.lod) * 3);
            let base = fill.path.value() & !(span - 1);
            for code in [base, base + span - 1] {
                assert_eq!(
                    dag.value_at(root, ZOrder20x3::from_value(code).expect("区域坐标"))?,
                    fill.data
                );
            }
            let after = ZOrder20x3::from_value(base + span).expect("相邻坐标");
            assert_eq!(dag.value_at(root, after)?, 17);
        }
        Ok(())
    }
}
