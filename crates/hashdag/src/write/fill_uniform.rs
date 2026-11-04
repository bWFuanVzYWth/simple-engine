use std::convert::Infallible;

use crate::ZOrder20x3;
use crate::hashdag::{DAG_DEPTH, Error};
use crate::node::Child;
use crate::pool::Pool;
use crate::write::frames::WriteScratch;
use crate::write::ordered::{Edit, EditData, Edits, apply_ordered};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "../proof/write_fill.rs"]
pub(super) mod proof;

pub use super::input::UniformFill;

#[cfg_attr(verus_only, verus_verify)]
pub(super) struct Fills<'a>(pub(super) &'a [UniformFill]);

#[cfg_attr(verus_only, verus_verify)]
impl super::Batch for Fills<'_> {
    fn validate(&self) -> Result<(), Error> {
        validate(self.0)
    }
    #[inline(always)]
    fn build(
        self,
        pools: &mut [Pool; DAG_DEPTH as usize],
        root: Child,
        scratch: &mut WriteScratch,
    ) -> Result<Child, Error> {
        fill_uniform(pools, root, self.0, scratch)
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    ensures result is Ok <==> super::validate::proof::fills_prefix(fills@,fills@.len()),no_unwind
))]
pub(super) fn validate(fills: &[UniformFill]) -> Result<(), Error> {
    super::validate::validate_fills(fills)
}

/// 输入已在事务边界完整校验，此处只构建候选 root。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    requires super::commit::proof::building(old(pools)),old(scratch).routing_empty(),super::validate::proof::fills_prefix(fills@,fills@.len()),
        crate::state::child_valid(crate::state::pools_view(old(pools)),root,10),
    ensures super::commit::proof::building(final(pools)),crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        result is Ok ==> crate::state::child_valid(crate::state::pools_view(final(pools)),result->Ok_0,10)
            && forall|path:int|0<=path<crate::state::span(10) ==> #[trigger] crate::state::eval(crate::state::pools_view(final(pools)),result->Ok_0,10,path)
                ==proof::effect(fills@,|p:int|crate::state::eval(crate::state::pools_view(old(pools)),root,10,p),path,fills@.len()),
))]
pub(crate) fn fill_uniform(
    pools: &mut [Pool; DAG_DEPTH as usize],
    root: Child,
    fills: &[UniformFill],
    scratch: &mut WriteScratch,
) -> Result<Child, Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost before=crate::state::pools_view(pools);}
    #[cfg(verus_only)]
    proof! {proof::input_stream(fills@,|p:int|crate::state::eval(before,root,10,p),0);
    assert forall|i:int|0<=i<fills@.len() implies #[trigger] fills@[i].lod<=20 by {assert(super::validate::proof::fill_at(fills@,i as nat));}}
    let result = apply_ordered(
        pools,
        root,
        FillEdits {
            pending: fills,
            part: 0,
            base: 0,
            lod: 0,
            shift: 0,
            parts: 0,
        },
        scratch,
    );
    #[cfg(verus_only)]
    proof! {if result is Ok {assert forall|path:int|0<=path<crate::state::span(10) implies
    #[trigger] crate::state::eval(crate::state::pools_view(pools),result->Ok_0,10,path)
        ==proof::effect(fills@,|p:int|crate::state::eval(before,root,10,p),path,fills@.len()) by {
            proof::input_stream(fills@,|p:int|crate::state::eval(before,root,10,p),path);
        }}}
    result
}

/// 奇数 LOD 的八段复用同一区域的准备量。
#[cfg_attr(verus_only, verus_verify)]
struct FillEdits<'a> {
    pending: &'a [UniformFill],
    base: u64,
    part: u8,
    lod: u8,
    shift: u8,
    parts: u8,
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> Edits<'a, Infallible> for FillEdits<'a> {
    #[inline(always)]
    fn next(&mut self) -> Option<Result<Edit<'a, Infallible>, Error>> {
        self.take_next()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> FillEdits<'a> {
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).valid(), result is None <==> old(self).stream().len() == 0,
            result is None ==> final(self).stream() == old(self).stream(),
            result is Some ==> result->Some_0 is Ok
                && super::ordered::proof::edit_ready(result->Some_0->Ok_0)
                && super::ordered::proof::same_edit(super::ordered::proof::edit_view(result->Some_0->Ok_0), old(self).stream()[0])
                && final(self).stream() == old(self).stream().drop_first(),
        no_unwind
    ))]
    fn take_next(&mut self) -> Option<Result<Edit<'a, Infallible>, Error>> {
        let (fill, rest) = self.pending.split_first()?;
        if self.part == 0 {
            let code = fill.path.value();
            #[cfg(verus_only)]
            proof! { super::validate::proof::span_safe(fill.lod); }
            let bits = fill.lod as u32 * 3;
            self.base = code & !((1_u64 << bits) - 1);
            self.lod = fill.lod / 2;
            self.shift = self.lod * 6;
            self.parts = if fill.lod % 2 == 0 { 1 } else { 8 };
        }
        #[cfg(verus_only)]
        proof! {use_type_invariant(&fill.path);assert(self.prepared(*fill));proof::path_safe(crate::morton::proof::value(fill.path), fill.lod, self.part as u64);}
        let base = self.base;
        let lod = self.lod;
        let shift = self.shift;
        let parts = self.parts;
        let part = self.part;
        self.part += 1;
        if self.part == parts {
            self.part = 0;
            self.pending = rest;
        }
        Some(
            match ZOrder20x3::from_value(base + ((part as u64) << shift)) {
                Some(path) => Ok(Edit {
                    path,
                    lod,
                    data: EditData::Uniform(fill.data),
                }),
                None => Err(Error::InvalidInput),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::ZOrder20x3;
    use proptest::prelude::*;
    use proptest::test_runner::TestCaseError;

    use crate::test_util;
    use crate::{Child, Error, HashDag, UniformFill};

    fn checked<T>(result: Result<T, Error>) -> Result<T, TestCaseError> {
        result.map_err(|error| TestCaseError::fail(format!("HashDag 返回意外错误： {error:?}")))
    }

    #[test]
    fn empty_input_returns_root() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = dag.root();
        let original = checked(dag.root_child(root))?;
        checked(dag.fill_uniform(root, &[]))?;
        prop_assert_eq!(checked(dag.root_child(root))?, original);
        prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        Ok(())
    }

    #[test]
    fn fill_at_lod_one() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = checked(dag.fork_root(dag.root()))?;
        let fills = [UniformFill {
            path: ZOrder20x3::ZERO,
            lod: 1,
            data: 99,
        }];
        checked(dag.fill_uniform(root, &fills))?;
        prop_assert!(matches!(checked(dag.root_child(root))?, Child::Ptr(_)));
        for code in 0..8 {
            let path = ZOrder20x3::from_value(code)
                .ok_or_else(|| TestCaseError::fail("invalid test path"))?;
            prop_assert_eq!(checked(dag.value_at(root, path))?, 99);
        }
        prop_assert_eq!(checked(dag.value_at(dag.root(), ZOrder20x3::ZERO))?, 0);
        checked(dag.release_root(root))?;
        prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        Ok(())
    }

    #[test]
    fn zero_data_fill() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = checked(dag.fork_root(dag.root()))?;
        let fills = [UniformFill {
            path: ZOrder20x3::ZERO,
            lod: 2,
            data: 0,
        }];
        checked(dag.fill_uniform(root, &fills))?;
        prop_assert_eq!(checked(dag.root_child(root))?, Child::Leaf(0));
        checked(dag.release_root(root))?;
        prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        Ok(())
    }

    #[test]
    fn fill_single_at_root_lod() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = checked(dag.fork_root(dag.root()))?;
        let fills = [UniformFill {
            path: ZOrder20x3::ZERO,
            lod: ZOrder20x3::BIT_DEPTH,
            data: 42,
        }];
        checked(dag.fill_uniform(root, &fills))?;
        prop_assert_eq!(checked(dag.root_child(root))?, Child::Leaf(42));
        checked(dag.release_root(root))?;
        prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        Ok(())
    }

    proptest! {
        #[test]
        fn no_leak_after_fill(
            fills in prop::collection::vec(
                (0u32..512u32, 0u32..512u32, 0u32..512u32, 1u8..10u8, 0u32..10u32),
                1..20,
            ),
        ) {
            let mut dag = checked(HashDag::new())?;
            let root = checked(dag.fork_root(dag.root()))?;
            let mut entries: Vec<UniformFill> = (0_u32..).zip(&fills)
                .map(|(index, &(x, y, z, lod, data))| UniformFill {
                    path: ZOrder20x3::encode_unchecked(index * 512 + x, y, z),
                    lod,
                    data,
                })
                .collect();
            entries.sort_by_key(|f| f.path.value());

            checked(dag.fill_uniform(root, &entries))?;
            for fill in &entries {
                prop_assert_eq!(checked(dag.value_at(root, fill.path))?, fill.data);
            }
            checked(dag.release_root(root))?;
            prop_assert_eq!(test_util::total_len(&dag.pools), 0);
        }
    }
}
