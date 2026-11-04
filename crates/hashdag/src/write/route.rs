//! 单个原生子树的活动祖先路由；只增长深度有界的实例 scratch。
use super::commit::clone_or_synthesize;
use super::frames::{BuildFrame, WriteScratch};
use super::frontier::{contains, replace_child};
use super::ordered::{Edit, LeafPatch, Replacement, finish_frame};
use crate::hashdag::{DAG_DEPTH, Error, ROOT_LOD};
use crate::pool::Pool;
use crate::{Child, ZOrder20x3};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "../proof/write_route.rs"]
pub(super) mod proof;

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    requires super::commit::proof::building(pools),crate::state::child_valid(crate::state::pools_view(pools),root,10),old(scratch).routing_empty(),
    ensures super::resolve::proof::cache_same(old(scratch),final(scratch)),
        result is Ok ==> proof::preserved(crate::state::pools_view(pools),crate::state::pools_view(pools),root,old(scratch).frames@,final(scratch).frames@)
            && final(scratch).frames@.len()==1,
))]
fn open_root(
    pools: &[Pool; DAG_DEPTH as usize],
    root: Child,
    scratch: &mut WriteScratch,
) -> Result<(), Error> {
    scratch.frames.try_reserve_exact(usize::from(DAG_DEPTH))?;
    let entry = clone_or_synthesize(&pools[usize::from(ROOT_LOD)], root)?;
    scratch.frames.push(BuildFrame {
        entry,
        original: root,
        path: ZOrder20x3::ZERO,
        any_modified: false,
        pool_lod: ROOT_LOD,
    });
    #[cfg(test)]
    scratch.record_frame();
    #[cfg(verus_only)]
    proof! {assert(scratch.frames@==seq![scratch.frames@[0]]);proof::root_opened(crate::state::pools_view(pools),root,scratch.frames@[0]);}
    Ok(())
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    requires super::commit::proof::building(old(pools)),crate::state::child_valid(crate::state::pools_view(old(pools)),_root,10),
        super::frontier::proof::chain_ready(crate::state::pools_view(old(pools)),old(scratch).frames@),old(scratch).frames@.len()>0,
        edit.lod<10,super::ordered::proof::edit_shape(super::ordered::proof::edit_view(*edit)),
    ensures super::commit::proof::building(final(pools)),crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        super::resolve::proof::cache_same(old(scratch),final(scratch)),
        result is Ok ==> proof::preserved(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools)),_root,old(scratch).frames@,final(scratch).frames@)
            && proof::at_edit(final(scratch).frames@,*edit),
))]
fn close_until<P: LeafPatch>(
    pools: &mut [Pool; DAG_DEPTH as usize],
    _root: Child,
    edit: &Edit<'_, P>,
    scratch: &mut WriteScratch,
) -> Result<(), Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost before=crate::state::pools_view(pools);let ghost original=*scratch;}
    #[cfg(verus_only)]
    proof! {assert(super::frontier::proof::frame_ready(before,scratch.frames@.last()));}
    #[cfg_attr(verus_only,verus_spec(
        invariant super::commit::proof::building(pools),crate::state::build_step(before,crate::state::pools_view(pools)),
            before==crate::state::pools_view(old(pools)),original==*old(scratch),
            proof::preserved(before,crate::state::pools_view(pools),_root,original.frames@,scratch.frames@),
            scratch.frames@.len()>0,scratch.frames@.last().pool_lod<10,edit.lod<10,super::ordered::proof::edit_shape(super::ordered::proof::edit_view(*edit)),
            super::resolve::proof::cache_same(&original,scratch),
        decreases scratch.frames@.len(),
    ))]
    while needs_close(&scratch.frames, edit.path, edit.lod) {
        #[cfg(verus_only)]
        proof_decl! {let ghost step=crate::state::pools_view(pools);let ghost frames=scratch.frames@;}
        #[cfg(verus_only)]
        proof! {proof::can_close(step,frames,*edit);}
        let result = finish_frame(pools, &mut scratch.frames);
        #[cfg(verus_only)]
        proof! {crate::balance::build_transitive(before,step,crate::state::pools_view(pools));}
        let _child = result?;
        #[cfg(verus_only)]
        proof! {proof::close_preserved(step,crate::state::pools_view(pools),_root,frames,scratch.frames@,_child);
        proof::preserved_transitive(before,step,crate::state::pools_view(pools),_root,original.frames@,frames,scratch.frames@);
        assert(super::frontier::proof::frame_ready(crate::state::pools_view(pools),scratch.frames@.last()));}
    }
    #[cfg(verus_only)]
    proof! {super::frontier::proof::contains_model(scratch.frames@.last(),edit.path);}
    Ok(())
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    requires frames@.len()>0 ==> frames@.last().pool_lod<10,
    ensures result==proof::needs_close(frames@,path,lod),
))]
fn needs_close(frames: &[BuildFrame], path: ZOrder20x3, lod: u8) -> bool {
    match frames.last() {
        Some(frame) => frame.pool_lod < lod || !contains(frame, path),
        None => false,
    }
}

#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    requires super::commit::proof::building(old(pools)),
        proof::ready(crate::state::pools_view(old(pools)),root,*edit,*replacement,old(scratch)),
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        super::resolve::proof::cache_same(old(scratch),final(scratch)),
        result is Ok ==> proof::routed(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools)),root,result->Ok_0,*edit,old(scratch),final(scratch)),
))]
pub(super) fn route_edit<P: LeafPatch>(
    pools: &mut [Pool; DAG_DEPTH as usize],
    root: Child,
    edit: &Edit<'_, P>,
    replacement: &Replacement<P>,
    scratch: &mut WriteScratch,
) -> Result<Child, Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost initial=crate::state::pools_view(pools);let ghost previous=*scratch;}
    if edit.lod == DAG_DEPTH {
        #[cfg(verus_only)]
        proof! {assert(previous.frames@==Seq::empty());assert(scratch.frames@==Seq::empty());}
        let result = replacement.resolve(pools, root);
        #[cfg(verus_only)]
        proof! {if result is Ok {proof::whole_effect(initial,crate::state::pools_view(pools),root,*edit,*replacement,result->Ok_0);}}
        return result;
    }
    if scratch.frames.is_empty() {
        #[cfg(verus_only)]
        proof! {assert(scratch.frames@==Seq::empty());assert(previous.frames@==Seq::empty());}
        if replacement.keeps_leaf(root) {
            #[cfg(verus_only)]
            proof! {proof::root_kept(initial,root,*edit,*replacement);}
            return Ok(root);
        }
        open_root(pools, root, scratch)?;
    }
    #[cfg(verus_only)]
    proof_decl! {let ghost opened=scratch.frames@;}
    close_until(pools, root, edit, scratch)?;
    #[cfg(verus_only)]
    proof! {proof::preserved_transitive(initial,initial,crate::state::pools_view(pools),root,previous.frames@,opened,scratch.frames@);
    proof::selected_frozen(initial,crate::state::pools_view(pools),*edit,*replacement);}
    #[cfg_attr(verus_only,verus_spec(
        invariant super::commit::proof::building(pools),crate::state::build_step(initial,crate::state::pools_view(pools)),
            initial==crate::state::pools_view(old(pools)),previous==*old(scratch),
            proof::preserved(initial,crate::state::pools_view(pools),root,previous.frames@,scratch.frames@),
            proof::at_edit(scratch.frames@,*edit),edit.lod<10,super::ordered::proof::edit_ready(*edit),
            super::ordered::proof::edit_shape(super::ordered::proof::edit_view(*edit)),
            super::ordered::proof::replacement_valid(*replacement),super::resolve::proof::selected(*edit,*replacement,crate::state::pools_view(pools)),
            super::resolve::proof::cache_same(&previous,scratch),
        decreases 10-scratch.frames@.len(),
    ))]
    loop {
        #[cfg(verus_only)]
        proof_decl! {let ghost step=crate::state::pools_view(pools);let ghost frames=scratch.frames@;}
        let frame = scratch.frames.last_mut().ok_or(Error::BuildFrameMissing)?;
        #[cfg(verus_only)]
        proof! {assert(*frame==frames.last());assert(super::frontier::proof::frame_ready(step,frames.last()));assert(frame.pool_lod<10);
        super::frontier::proof::slot_matches(edit.path,frame.pool_lod);}
        if frame.pool_lod == edit.lod {
            let old = frame.entry.child_at(edit.path, frame.pool_lod);
            let result = replacement.resolve(pools, old);
            #[cfg(verus_only)]
            proof! {crate::balance::build_transitive(initial,step,crate::state::pools_view(pools));}
            let child = result?;
            replace_child(frame, edit.path, child);
            #[cfg(verus_only)]
            proof! {proof::replaced_effect(step,crate::state::pools_view(pools),root,frames,scratch.frames@,*edit,*replacement,child);
            proof::effected_transitive(initial,step,crate::state::pools_view(pools),root,root,previous.frames@,frames,scratch.frames@,*edit);}
            return Ok(root);
        }
        let child = frame.entry.child_at(edit.path, frame.pool_lod);
        if replacement.keeps_leaf(child) {
            #[cfg(verus_only)]
            proof! {assert(scratch.frames@==frames);proof::kept::kept_candidate(step,root,frames,*edit,*replacement);
            proof::effected_transitive(initial,step,step,root,root,previous.frames@,frames,scratch.frames@,*edit);}
            return Ok(root);
        }
        let child_lod = frame.pool_lod - 1;
        let shift = u32::from(frame.pool_lod) * 6;
        #[cfg(verus_only)]
        proof! {assert((1u64<<shift)>0) by(bit_vector) requires shift<64u32;}
        let path = ZOrder20x3::from_value(edit.path.value() & !((1_u64 << shift) - 1))
            .ok_or(Error::InvalidInput)?;
        let entry = clone_or_synthesize(&pools[usize::from(child_lod)], child)?;
        let child_frame = BuildFrame {
            entry,
            original: child,
            path,
            any_modified: false,
            pool_lod: child_lod,
        };
        #[cfg(verus_only)]
        proof! {assert(scratch.frames@==frames);proof::aligned_child(step,frames,*edit,child_frame);}
        scratch.frames.push(child_frame);
        #[cfg(verus_only)]
        proof! {proof::opening_preserved(step,root,frames,scratch.frames@.last());
        proof::preserved_transitive(initial,step,step,root,previous.frames@,frames,scratch.frames@);}
        #[cfg(test)]
        scratch.record_frame();
    }
}
