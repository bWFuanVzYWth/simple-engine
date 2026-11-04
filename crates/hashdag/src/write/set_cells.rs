use crate::hashdag::{DAG_DEPTH, Error};
use crate::node::Child;
use crate::pool::Pool;
use crate::write::frames::WriteScratch;
use crate::write::ordered::{Edit, EditData, Edits, apply_ordered};

pub use super::input::SetCell;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "../proof/write_set_cells.rs"]
pub(super) mod proof;

/// 非降序约束在构造时检查，保留单点入口既有的错误优先级。
#[cfg_attr(verus_only, verus_verify)]
pub(super) struct Cells<'a> {
    cells: &'a [SetCell],
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> Cells<'a> {
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is Ok <==> super::patch::proof::sorted(cells@),
            result is Ok ==> result->Ok_0.inputs()==cells@ && proof::input_domain(cells@),
        no_unwind
    ))]
    pub(super) fn checked(cells: &'a [SetCell]) -> Result<Self, Error> {
        if !super::patch::sorted(cells) {
            return Err(Error::InvalidInput);
        }
        Ok(Self { cells })
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl super::Batch for Cells<'_> {
    fn validate(&self) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof! {use_type_invariant(self);}
        Ok(())
    }
    #[inline(always)]
    fn build(
        self,
        pools: &mut [Pool; DAG_DEPTH as usize],
        root: Child,
        scratch: &mut WriteScratch,
    ) -> Result<Child, Error> {
        set_cells(pools, root, self.cells, scratch)
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    requires super::commit::proof::building(old(pools)),old(scratch).routing_empty(),super::patch::proof::sorted(cells@),proof::input_domain(cells@),
        crate::state::child_valid(crate::state::pools_view(old(pools)),root,10),
    ensures super::commit::proof::building(final(pools)),crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        result is Ok ==> crate::state::child_valid(crate::state::pools_view(final(pools)),result->Ok_0,10)
            && forall|path:int|0<=path<crate::state::span(10) ==> #[trigger] crate::state::eval(crate::state::pools_view(final(pools)),result->Ok_0,10,path)
                ==proof::effect(cells@,|p:int|crate::state::eval(crate::state::pools_view(old(pools)),root,10,p),path,cells@.len()),
))]
pub(crate) fn set_cells(
    pools: &mut [Pool; DAG_DEPTH as usize],
    root: Child,
    cells: &[SetCell],
    scratch: &mut WriteScratch,
) -> Result<Child, Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost before=crate::state::pools_view(pools);}
    #[cfg(verus_only)]
    proof! {proof::stream_valid(cells@);}
    // 共享祖先只路由一次原生叶，叶内重复点在 patch 时取最后一项。
    let result = apply_ordered(pools, root, CellEdits { pending: cells }, scratch);
    #[cfg(verus_only)]
    proof! {if result is Ok {assert forall|path:int|0<=path<crate::state::span(10) implies
    #[trigger] crate::state::eval(crate::state::pools_view(pools),result->Ok_0,10,path)
        ==proof::effect(cells@,|p:int|crate::state::eval(before,root,10,p),path,cells@.len()) by {
            proof::stream_contract(cells@,|p:int|crate::state::eval(before,root,10,p),path);
        }}}
    result
}

#[cfg_attr(verus_only, verus_verify)]
struct CellEdits<'a> {
    pending: &'a [SetCell],
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> Edits<'a, &'a [SetCell]> for CellEdits<'a> {
    #[inline(always)]
    fn next(&mut self) -> Option<Result<Edit<'a, &'a [SetCell]>, Error>> {
        self.take_next()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> CellEdits<'a> {
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is None <==> old(self).stream().len() == 0,
            result is None ==> final(self).stream() == old(self).stream(),
            result is Some ==> result->Some_0 is Ok
                && super::ordered::proof::edit_ready(result->Some_0->Ok_0)
                && super::ordered::proof::same_edit(super::ordered::proof::edit_view(result->Some_0->Ok_0), old(self).stream()[0])
                && final(self).stream() == old(self).stream().drop_first(),
        no_unwind
    ))]
    fn take_next(&mut self) -> Option<Result<Edit<'a, &'a [SetCell]>, Error>> {
        #[cfg(verus_only)]
        proof_decl! { let ghost source = self.pending@; }
        let (first, _) = self.pending.split_first()?;
        let prefix = first.path.value() >> 6;
        let mut end = 1;
        #[cfg_attr(verus_only, verus_spec(
            invariant source == self.pending@, 0 < end <= self.pending@.len(), source.len() > 0,
                prefix == crate::morton::proof::value(source[0].path) >> 6,
                forall|i:int| 0 <= i < end ==> crate::morton::proof::value(#[trigger] source[i].path) >> 6 == prefix,
            decreases self.pending@.len() - end,
        ))]
        while end < self.pending.len() && self.pending[end].path.value() >> 6 == prefix {
            end += 1;
        }
        #[cfg(verus_only)]
        proof! { proof::group_found(source, end as nat); }
        let (group, rest) = self.pending.split_at(end);
        self.pending = rest;
        Some(Ok(Edit {
            path: first.path,
            lod: 1,
            data: EditData::Patch(group),
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::ZOrder20x3;
    use proptest::prelude::*;
    use proptest::test_runner::TestCaseError;

    use crate::test_util;
    use crate::{Child, Error, HashDag, SetCell};

    fn checked<T>(result: Result<T, Error>) -> Result<T, TestCaseError> {
        result.map_err(|error| TestCaseError::fail(format!("HashDag 返回意外错误： {error:?}")))
    }

    #[test]
    fn empty_input_returns_root() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = dag.root();
        let original = checked(dag.root_child(root))?;
        checked(dag.set_cells(root, &[]))?;
        prop_assert_eq!(checked(dag.root_child(root))?, original);
        prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        Ok(())
    }

    #[test]
    fn zero_data_overwrites_default() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = checked(dag.fork_root(dag.root()))?;
        let cells = [
            SetCell {
                path: ZOrder20x3::ZERO,
                data: 1,
            },
            SetCell {
                path: ZOrder20x3::ZERO,
                data: 0,
            },
        ];
        checked(dag.set_cells(root, &cells))?;
        prop_assert_eq!(checked(dag.root_child(root))?, Child::Leaf(0));
        prop_assert_eq!(checked(dag.value_at(root, ZOrder20x3::ZERO))?, 0);
        checked(dag.release_root(root))?;
        prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        Ok(())
    }

    #[test]
    fn single_cell_modify() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = checked(dag.fork_root(dag.root()))?;
        let cells = [SetCell {
            path: ZOrder20x3::ZERO,
            data: 42,
        }];
        checked(dag.set_cells(root, &cells))?;
        prop_assert!(matches!(checked(dag.root_child(root))?, Child::Ptr(_)));
        prop_assert_eq!(checked(dag.value_at(root, ZOrder20x3::ZERO))?, 42);
        prop_assert_eq!(checked(dag.value_at(dag.root(), ZOrder20x3::ZERO))?, 0);
        checked(dag.release_root(root))?;
        prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        Ok(())
    }

    #[test]
    fn repeated_cell_uses_last_value() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = checked(dag.fork_root(dag.root()))?;
        let cells = [
            SetCell {
                path: ZOrder20x3::ZERO,
                data: 1,
            },
            SetCell {
                path: ZOrder20x3::ZERO,
                data: 2,
            },
        ];
        checked(dag.set_cells(root, &cells))?;
        prop_assert_eq!(checked(dag.value_at(root, ZOrder20x3::ZERO))?, 2);
        checked(dag.release_root(root))?;
        prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        Ok(())
    }

    #[test]
    fn adjacent_leaf_patches_preserve_other_values() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let root = dag.root();
        let initial: Vec<_> = (0..128_u64)
            .map(|code| SetCell {
                path: ZOrder20x3::from_value(code).expect("有效 Morton 路径"),
                data: code as u32 + 1,
            })
            .collect();
        dag.set_cells(root, &initial)?;
        dag.set_cells(
            root,
            &[
                SetCell {
                    data: 901,
                    ..initial[63]
                },
                SetCell {
                    data: 902,
                    ..initial[64]
                },
            ],
        )?;
        let paths: Vec<_> = initial.iter().map(|cell| cell.path).collect();
        let mut actual = vec![0; paths.len()];
        dag.values_at(root, &paths, &mut actual)?;
        let mut expected: Vec<_> = initial.iter().map(|cell| cell.data).collect();
        expected[63] = 901;
        expected[64] = 902;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn duplicate_leaf_patches_preserve_final_values() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let root = dag.root();
        let left = ZOrder20x3::from_value(63).expect("左叶末尾");
        let right = ZOrder20x3::from_value(64).expect("右叶起点");
        let initial = [
            SetCell {
                path: left,
                data: 7,
            },
            SetCell {
                path: right,
                data: 11,
            },
        ];
        dag.set_cells(root, &initial)?;
        dag.set_cells(
            root,
            &[
                SetCell {
                    data: 99,
                    ..initial[0]
                },
                initial[0],
                SetCell {
                    data: 99,
                    ..initial[1]
                },
                initial[1],
            ],
        )?;
        let paths = [left, right];
        let mut actual = [0; 2];
        dag.values_at(root, &paths, &mut actual)?;
        assert_eq!(actual, [7, 11]);
        Ok(())
    }

    proptest! {
        #[test]
        fn no_leak_after_set_cells(
            cells in prop::collection::vec(
                (0u32..1024u32, 0u32..1024u32, 0u32..1024u32, 0u32..10u32),
                1..30,
            ),
        ) {
            let mut dag = checked(HashDag::new())?;
            let root = checked(dag.fork_root(dag.root()))?;
            let mut entries: Vec<SetCell> = cells
                .iter()
                .map(|&(x, y, z, data)| SetCell {
                    path: ZOrder20x3::encode_unchecked(x, y, z),
                    data,
                })
                .collect();
            entries.sort_by_key(|c| c.path.value());

            checked(dag.set_cells(root, &entries))?;
            for group in entries.chunk_by(|a, b| a.path == b.path) {
                let last = group.last().ok_or_else(|| TestCaseError::fail("empty cell group"))?;
                prop_assert_eq!(checked(dag.value_at(root, last.path))?, last.data);
            }
            checked(dag.release_root(root))?;
            prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        }
    }
}
