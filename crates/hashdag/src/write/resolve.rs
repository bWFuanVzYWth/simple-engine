//! 解析单个输入子树；候选帧路由不参与 dense 构建或局部编号翻译。
use super::dense_chunks::build_dense_subtree;
use super::frames::WriteScratch;
use super::ordered::{Edit, EditData, LeafPatch, Replacement};
use crate::hashdag::{DAG_DEPTH, Error};
use crate::node::Child;
use crate::pool::Pool;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "../proof/write_resolve.rs"]
pub(super) mod proof;

#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires super::commit::proof::building(old(pools)), proof::ready(*edit, crate::state::pools_view(old(pools)), old(scratch)),
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)), crate::state::pools_view(final(pools))),
        final(scratch).frames@ == old(scratch).frames@,
        result is Ok ==> proof::selected(*edit, result->Ok_0, crate::state::pools_view(final(pools)))
            && proof::context(*edit, crate::state::pools_view(old(pools)), crate::state::pools_view(final(pools)), old(scratch), final(scratch)),
))]
pub(super) fn resolve_edit<'a, P: LeafPatch>(
    pools: &mut [Pool; DAG_DEPTH as usize],
    edit: &Edit<'a, P>,
    scratch: &mut WriteScratch,
) -> Result<Replacement<P>, Error> {
    #[cfg(verus_only)]
    proof_decl! { let ghost before = crate::state::pools_view(pools); let ghost previous = *scratch; }
    Ok(match &edit.data {
        EditData::Uniform(value) => {
            #[cfg(verus_only)]
            proof! { proof::uniform_selected(*edit, before, *value); }
            Replacement::Value(Child::Leaf(*value))
        }
        EditData::Dense(values) => {
            let child = build_dense_subtree(pools, edit.lod, values, &mut scratch.dense)?;
            #[cfg(verus_only)]
            proof! { proof::cache_frozen(before, crate::state::pools_view(pools), &previous, scratch); proof::dense_selected(*edit, crate::state::pools_view(pools), child, values@); }
            Replacement::Value(child)
        }
        EditData::Built { tree, part } => {
            if *part == 0 {
                scratch.imported.import(pools, tree)?;
            }
            #[cfg(verus_only)]
            proof! { proof::built_selected(*edit, crate::state::pools_view(pools), scratch, tree, *part); }
            Replacement::Value(scratch.imported.roots[*part])
        }
        EditData::Patch(patch) => Replacement::Patch(*patch),
    })
}
