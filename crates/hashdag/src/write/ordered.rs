use std::convert::Infallible;

use crate::ZOrder20x3;
use crate::hashdag::{DAG_DEPTH, Error};
use crate::node::{Child, PoolEntry};
use crate::pool::Pool;
use crate::write::commit::commit_node;
use crate::write::dense_chunks::DenseRegion;
use crate::write::frames::{BuildFrame, WriteScratch};
use crate::write::frontier::replace_child;
use crate::write::set_cells::SetCell;
#[cfg(verus_only)]
use proof::{PatchModel, StreamModel};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(not(verus_only))]
pub(super) trait PatchModel {}
#[cfg(not(verus_only))]
impl<T> PatchModel for T {}
#[cfg(not(verus_only))]
pub(super) trait StreamModel {}
#[cfg(not(verus_only))]
impl<T> StreamModel for T {}

#[cfg(verus_only)]
#[path = "../proof/write_apply.rs"]
mod apply_proof;
#[cfg(verus_only)]
#[path = "../proof/write_ordered.rs"]
pub(super) mod proof;

#[cfg_attr(verus_only, verus_verify)]
pub(super) enum EditData<'a, P> {
    Uniform(u32),
    Dense(&'a [u32]),
    Built {
        tree: &'a super::built::LocalTree,
        part: usize,
    },
    /// 同一原生叶内的修改，目标 `lod` 为 1；输入类型由批内核确定。
    Patch(P),
}

/// 批内核只携带自身的借用输入，避免共享分派混入其他叶修改类型。
#[cfg_attr(verus_only, verus_verify)]
pub(super) trait LeafPatch: Copy + PatchModel {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires self.valid(),
        ensures result ==> self.effect(Seq::new(64, |i:int| value)) == Seq::new(64, |i:int| value),
    ))]
    fn keeps_leaf(self, value: u32) -> bool;
    #[cfg_attr(verus_only, verus_spec(result =>
        requires self.valid(),
        ensures result is Ok ==> final(values)@ == self.effect(old(values)@),
            result == Ok(false) ==> final(values)@ == old(values)@,
    ))]
    fn apply(self, values: &mut [u32; 64]) -> Result<bool, Error>;
}

/// 惰性借用输入；同一条目拆出的奇数 LOD 子树保持连续。
#[cfg_attr(verus_only, verus_verify)]
pub(super) trait Edits<'a, P: LeafPatch>: StreamModel {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).stream_valid(),
        ensures final(self).stream_valid(), result is None <==> old(self).remaining().len() == 0,
            result is None ==> final(self).remaining() == old(self).remaining(),
            result is Some ==> result->Some_0 is Ok
                && proof::edit_ready(result->Some_0->Ok_0)
                && proof::same_edit(proof::edit_view(result->Some_0->Ok_0), old(self).remaining()[0])
                && final(self).remaining() == old(self).remaining().drop_first(),
        no_unwind
    ))]
    fn next(&mut self) -> Option<Result<Edit<'a, P>, Error>>;
}

#[cfg(test)]
struct TestEdits<I>(I);
#[cfg(test)]
impl<'a, P: LeafPatch + 'a, I: Iterator<Item = Result<Edit<'a, P>, Error>>> Edits<'a, P>
    for TestEdits<I>
{
    fn next(&mut self) -> Option<Result<Edit<'a, P>, Error>> {
        self.0.next()
    }
}

#[cfg_attr(verus_only, verus_verify)]
pub(super) enum Replacement<P> {
    Value(Child),
    Patch(P),
}

#[cfg_attr(verus_only, verus_verify)]
impl<P: LeafPatch> Replacement<P> {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::replacement_valid(*self),
        ensures result ==> proof::replacement_kept(*self,child),
    ))]
    pub(super) fn keeps_leaf(&self, child: Child) -> bool {
        let Child::Leaf(value) = child else {
            return false;
        };
        match self {
            Self::Value(replacement) => replacement.same(child),
            Self::Patch(patch) => patch.keeps_leaf(value),
        }
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires super::commit::proof::building(old(pools)),proof::replacement_valid(*self),
        ensures super::commit::proof::building(final(pools)),
            crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
            result is Err ==> crate::state::pools_view(final(pools))==crate::state::pools_view(old(pools)),
            proof::resolved(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools)),*self,original,result),
    ))]
    pub(super) fn resolve(
        &self,
        pools: &mut [Pool; DAG_DEPTH as usize],
        original: Child,
    ) -> Result<Child, Error> {
        match self {
            Self::Value(replacement) => Ok(*replacement),
            Self::Patch(patch) => patch_leaf(pools, original, *patch),
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl LeafPatch for &[SetCell] {
    fn keeps_leaf(self, value: u32) -> bool {
        super::patch::keeps_cells(self, value)
    }

    fn apply(self, values: &mut [u32; 64]) -> Result<bool, Error> {
        Ok(super::patch::apply_cells(values, self))
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl LeafPatch for &[DenseRegion<'_>] {
    fn keeps_leaf(self, value: u32) -> bool {
        let keeps = super::patch::keeps_dense(self, value);
        #[cfg(verus_only)]
        proof! { if keeps { super::patch::proof::dense_kept(self@, self@.len(), value); } }
        keeps
    }

    fn apply(self, values: &mut [u32; 64]) -> Result<bool, Error> {
        Ok(super::patch::apply_dense(values, self))
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl LeafPatch for Infallible {
    fn keeps_leaf(self, _: u32) -> bool {
        unreachable!()
    }

    fn apply(self, _: &mut [u32; 64]) -> Result<bool, Error> {
        unreachable!()
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires super::commit::proof::building(old(pools)),patch.valid(),
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        result is Err ==> crate::state::pools_view(final(pools))==crate::state::pools_view(old(pools)),
        result is Ok ==> patch.effect(proof::leaf_values(crate::state::pools_view(old(pools)),original)).len()==64
            && crate::state::child_valid(crate::state::pools_view(final(pools)),result->Ok_0,1)
            && super::commit::proof::image(crate::state::pools_view(final(pools))[0],result->Ok_0)
                == Seq::new(64,|i:int|Child::Leaf(patch.effect(proof::leaf_values(crate::state::pools_view(old(pools)),original))[i])),
))]
pub(super) fn patch_leaf<P: LeafPatch>(
    pools: &mut [Pool; DAG_DEPTH as usize],
    original: Child,
    patch: P,
) -> Result<Child, Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost before=crate::state::pools_view(pools);}
    // 原生叶的 64 槽都是数据；仅在父槽路由完成后复制一次 payload。
    let mut values = match original {
        Child::Leaf(value) => [value; 64],
        Child::Ptr(index) => {
            let node = pools[0].get(index).ok_or(Error::NodeMissing(index))?;
            #[cfg(verus_only)]
            proof! {proof::decoded_leaf(before,node,index);}
            *node.values.words()
        }
    };
    #[cfg(verus_only)]
    proof! {proof::leaf_image(before,original);assert(values@ =~= proof::leaf_values(before,original));}
    let changed = patch.apply(&mut values)?;
    #[cfg(verus_only)]
    proof! {assert(values@==patch.effect(proof::leaf_values(before,original)));}
    if !changed {
        #[cfg(verus_only)]
        proof! {assert(super::commit::proof::image(before[0],original)
        =~= Seq::new(64,|i:int|Child::Leaf(patch.effect(proof::leaf_values(before,original))[i])));}
        return Ok(original);
    }
    let entry = PoolEntry::new(&values, 0);
    #[cfg(verus_only)]
    proof! {super::commit::proof::data_image(entry);}
    let result = commit_node(pools, 0, &entry);
    #[cfg(verus_only)]
    proof! {if result is Ok {assert(super::commit::proof::image(crate::state::pools_view(pools)[0],result->Ok_0)
    =~= Seq::new(64,|i:int|Child::Leaf(patch.effect(proof::leaf_values(before,original))[i])));}}
    result
}

/// 已通过公开输入边界检查的单个原生子树修改；数据仅借用到本次调用结束。
#[cfg_attr(verus_only, verus_verify)]
pub(super) struct Edit<'a, P> {
    pub path: ZOrder20x3,
    pub lod: u8,
    pub data: EditData<'a, P>,
}

/// 有序修改共用的非递归内核。候选帧只保留当前祖先链，每个空间前缀至多打开一次。
/// 输入流惰性展开奇数空间 LOD，既不保留路由队列，也不累计整个批次的节点副本。
/// 原生叶修改在到达父槽后整组 patch，不为每个体素重复祖先路由。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires super::commit::proof::building(old(pools)),
        crate::state::child_valid(crate::state::pools_view(old(pools)), root, 10),
        old(scratch).frames@.len() == 0, edits.stream_valid(), proof::stream_ordered(edits.remaining()),
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)), crate::state::pools_view(final(pools))),
        result is Ok ==> final(scratch).frames@.len() == 0
            && crate::state::child_valid(crate::state::pools_view(final(pools)), result->Ok_0, 10)
            && forall|point: int| 0 <= point < crate::state::span(10) ==>
                #[trigger] crate::state::eval(crate::state::pools_view(final(pools)), result->Ok_0, 10, point)
                    == proof::stream_effect(edits.remaining(), |p: int| crate::state::eval(crate::state::pools_view(old(pools)), root, 10, p), point, edits.remaining().len()),
))]
pub(super) fn apply_ordered<'a, P: LeafPatch>(
    pools: &mut [Pool; DAG_DEPTH as usize],
    root: Child,
    edits: impl Edits<'a, P>,
    scratch: &mut WriteScratch,
) -> Result<Child, Error> {
    #[cfg(test)]
    {
        scratch.work = Default::default();
    }
    #[cfg(verus_only)]
    proof_decl! { let ghost initial = crate::state::pools_view(pools); let ghost source = edits.remaining();
    let ghost mut done = 0nat; let ghost mut last: Option<Edit<'a, P>> = None; }
    let mut cursor = edits;
    #[cfg(verus_only)]
    proof! { super::frontier::proof::span_bits(10); }
    let mut new_root = root;
    #[cfg_attr(verus_only, verus_spec(
        invariant super::commit::proof::building(pools), crate::state::build_step(initial, crate::state::pools_view(pools)),
            initial == crate::state::pools_view(old(pools)), source == edits.remaining(),
            crate::state::child_valid(crate::state::pools_view(pools), new_root, 10),
            proof::stream_ordered(source), cursor.stream_valid(), done <= source.len(),
            cursor.remaining() == source.subrange(done as int, source.len() as int),
            apply_proof::active(initial, root, source, done, crate::state::pools_view(pools), new_root, scratch.frames@),
            apply_proof::cache(last, crate::state::pools_view(pools), scratch),
            done == 0 ==> last is None && scratch.frames@.len() == 0,
            done > 0 ==> last is Some && proof::same_edit(proof::edit_view(last->Some_0), source[(done - 1) as int]),
        ensures done == source.len(),
        decreases source.len() - done,
    ))]
    loop {
        let next = cursor.next();
        let Some(result) = next else {
            break;
        };
        let edit = result?;
        #[cfg(verus_only)]
        proof! {
            assert(proof::same_edit(proof::edit_view(edit), source[done as int]));
            apply_proof::next_ready(source, done, last, edit, crate::state::pools_view(pools), scratch);
        }
        #[cfg(verus_only)]
        proof_decl! { let ghost before = crate::state::pools_view(pools); }
        let selected = super::resolve::resolve_edit(pools, &edit, scratch);
        #[cfg(verus_only)]
        proof! {
            crate::balance::build_transitive(initial, before, crate::state::pools_view(pools));
            apply_proof::frozen_active(initial, root, source, done, before, crate::state::pools_view(pools), new_root, scratch.frames@);
        }
        let replacement = selected?;
        #[cfg(verus_only)]
        proof! { apply_proof::route_ready(pools, new_root, source, done, edit, replacement, scratch); }
        #[cfg(verus_only)]
        proof_decl! { let ghost routed_from = crate::state::pools_view(pools); let ghost old_scratch = *scratch; let ghost previous_root = new_root; }
        let routed = super::route::route_edit(pools, new_root, &edit, &replacement, scratch);
        #[cfg(verus_only)]
        proof! { crate::balance::build_transitive(initial, routed_from, crate::state::pools_view(pools)); }
        new_root = routed?;
        #[cfg(verus_only)]
        proof! {
            apply_proof::advanced(initial, root, source, done, routed_from, crate::state::pools_view(pools), previous_root, new_root, edit, &old_scratch, scratch);
            apply_proof::cache_frozen(Some(edit), routed_from, crate::state::pools_view(pools), &old_scratch, scratch);
            last = Some(edit); done = done + 1;
        }
    }
    #[cfg_attr(verus_only, verus_spec(
        invariant super::commit::proof::building(pools), crate::state::build_step(initial, crate::state::pools_view(pools)),
            initial == crate::state::pools_view(old(pools)), source == edits.remaining(),
            apply_proof::active(initial, root, source, source.len(), crate::state::pools_view(pools), new_root, scratch.frames@),
            scratch.frames@.len() == 0 ==> crate::state::child_valid(crate::state::pools_view(pools), new_root, 10),
        decreases scratch.frames@.len(),
    ))]
    while !scratch.frames.is_empty() {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = crate::state::pools_view(pools); let ghost frames = scratch.frames@; let ghost previous_root = new_root; }
        #[cfg(verus_only)]
        proof! { assert(super::frontier::proof::frame_ready(before, frames[0])); assert(frames[0].pool_lod == 9); }
        let finished = finish_frame(pools, &mut scratch.frames);
        #[cfg(verus_only)]
        proof! { crate::balance::build_transitive(initial, before, crate::state::pools_view(pools)); }
        new_root = finished?;
        #[cfg(verus_only)]
        proof! {
            apply_proof::closed(initial, root, source, source.len(), before, crate::state::pools_view(pools), previous_root, new_root, frames, scratch.frames@);
            if scratch.frames@.len() == 0 { assert(frames.len() == 1); assert(frames[0].pool_lod == 9); }
        }
    }
    #[cfg(verus_only)]
    proof! { apply_proof::completed(initial, root, source, crate::state::pools_view(pools), new_root, scratch.frames@); }
    Ok(new_root)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires super::commit::proof::building(old(pools)),
        super::frontier::proof::chain_ready(crate::state::pools_view(old(pools)),old(frames)@),
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        super::frontier::proof::chain_ready(crate::state::pools_view(final(pools)),final(frames)@),
        result is Err ==> crate::state::pools_view(final(pools))==crate::state::pools_view(old(pools)),
        result is Ok ==> old(frames)@.len()>0
            && crate::state::child_valid(crate::state::pools_view(final(pools)),result->Ok_0,(old(frames)@.last().pool_lod+1) as nat)
            && super::commit::proof::image(crate::state::pools_view(final(pools))[old(frames)@.last().pool_lod as int],result->Ok_0)
                ==crate::node::proof::children(old(frames)@.last().entry)
            && if old(frames)@.len()==1 {final(frames)@.len()==0}
                else {super::frontier::proof::frames_closed(old(frames)@,final(frames)@,result->Ok_0)}
            && forall|code:u64|code<0x1000_0000_0000_0000u64 ==> #[trigger] super::frontier::proof::closed_value(
                crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools)),old(frames)@,final(frames)@,result->Ok_0,code),
))]
pub(super) fn finish_frame(
    pools: &mut [Pool; DAG_DEPTH as usize],
    frames: &mut Vec<BuildFrame>,
) -> Result<Child, Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost before_pools=crate::state::pools_view(pools);let ghost before_frames=frames@;}
    let frame = frames.pop().ok_or(Error::BuildFrameMissing)?;
    #[cfg(verus_only)]
    proof! {super::frontier::proof::chain_prefix(before_pools,before_frames,(before_frames.len()-1) as nat);
    assert(frames@ =~= before_frames.take((before_frames.len()-1) as int));
    assert(super::frontier::proof::frame_ready(before_pools,frame));}
    let unchanged = !frame.any_modified
        || match frame.original {
            Child::Leaf(value) => frame.entry.uniform_value() == Some(value),
            Child::Ptr(index) => pools[frame.pool_lod as usize]
                .get(index)
                .ok_or(Error::NodeMissing(index))?
                .same(&frame.entry),
        };
    let child = if unchanged {
        #[cfg(verus_only)]
        proof! {assert(super::commit::proof::image(before_pools[frame.pool_lod as int],frame.original)
        =~=crate::node::proof::children(frame.entry));}
        frame.original
    } else {
        commit_node(pools, frame.pool_lod, &frame.entry)?
    };
    #[cfg(verus_only)]
    proof! {super::frontier::proof::chain_frozen(before_pools,crate::state::pools_view(pools),before_frames);
    assert(crate::state::child_valid(crate::state::pools_view(pools),child,(frame.pool_lod+1) as nat));
    assert(super::commit::proof::image(crate::state::pools_view(pools)[frame.pool_lod as int],child)==crate::node::proof::children(frame.entry));}
    if let Some(parent) = frames.last_mut() {
        #[cfg(verus_only)]
        proof! {assert(*parent==before_frames[before_frames.len()-2]);
        assert(super::frontier::proof::frame_ready(crate::state::pools_view(pools),*parent));}
        replace_child(parent, frame.path, child);
        #[cfg(verus_only)]
        proof! {super::frontier::proof::closed_chain(crate::state::pools_view(pools),before_frames,frames@,child);}
    }
    #[cfg(verus_only)]
    proof! {super::frontier::proof::finish_semantics(before_pools,crate::state::pools_view(pools),before_frames,frames@,child);}
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HashDag, SetCell};

    #[test]
    fn batch_shares_prefix_work() -> Result<(), Error> {
        let edits: Vec<_> = (0..256_u64)
            .map(|code| SetCell {
                path: ZOrder20x3::from_value(code).expect("有序体素坐标"),
                data: 7,
            })
            .collect();
        let mut batched = HashDag::new()?;
        batched.set_cells(batched.root(), &edits)?;
        let batch_work = batched.scratch_write.work.opened_prefixes;
        let mut serial = HashDag::new()?;
        let mut serial_work = 0;
        for edit in &edits {
            serial.set_cells(serial.root(), std::slice::from_ref(edit))?;
            serial_work += serial.scratch_write.work.opened_prefixes;
        }
        assert!(batch_work < serial_work);
        Ok(())
    }

    #[test]
    fn candidate_depth_is_bounded_for_spread_batch() -> Result<(), Error> {
        let edits: Vec<_> = (0..512_u64)
            .map(|code| SetCell {
                path: ZOrder20x3::from_value(code << 50).expect("分散体素坐标"),
                data: 7,
            })
            .collect();
        let mut dag = HashDag::new()?;
        dag.set_cells(dag.root(), &edits)?;
        assert!((1..=usize::from(DAG_DEPTH)).contains(&dag.scratch_write.work.max_depth));
        Ok(())
    }

    #[test]
    fn failed_stream_releases_finished_prefix_and_preserves_roots() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let root = dag.root();
        dag.set_cells(
            root,
            &[SetCell {
                path: ZOrder20x3::ZERO,
                data: 7,
            }],
        )?;
        let snapshot = dag.fork_root(root)?;
        let original = dag.root_child(root)?;
        let lengths = dag.pools.each_ref().map(Pool::len);
        let distant = ZOrder20x3::from_value(1_u64 << 54).expect("不同顶层槽");
        let result = dag.write_root_test(
            root,
            || Ok(()),
            |pools, old_root, scratch| {
                let edits = [
                    Ok(Edit::<Infallible> {
                        path: ZOrder20x3::ZERO,
                        lod: 0,
                        data: EditData::Uniform(9),
                    }),
                    Ok(Edit::<Infallible> {
                        path: distant,
                        lod: 0,
                        data: EditData::Uniform(11),
                    }),
                    Err(Error::InvalidInput),
                ];
                apply_ordered(pools, old_root, TestEdits(edits.into_iter()), scratch)
            },
        );
        assert!(matches!(result, Err(Error::InvalidInput)));
        assert_eq!(dag.root_child(root)?, original);
        assert_eq!(dag.root_child(snapshot)?, original);
        assert_eq!(dag.pools.each_ref().map(Pool::len), lengths);
        assert_eq!(dag.value_at(root, ZOrder20x3::ZERO)?, 7);
        assert_eq!(dag.value_at(root, distant)?, 0);
        dag.release_root(snapshot)?;
        Ok(())
    }
}
