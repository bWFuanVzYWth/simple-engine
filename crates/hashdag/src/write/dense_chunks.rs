use crate::ZOrder20x3;
use crate::hashdag::{DAG_DEPTH, Error, HashDag, NODE_FANOUT, RootId};
use crate::node::{Child, EncodedChild, PoolEntry};
use crate::pool::Pool;
use crate::wide::{fold_leaf_slice, uniform_values};
use crate::write::commit::commit_node;
use crate::write::frames::WriteScratch;
use crate::write::ordered::{Edit, EditData, Edits, apply_ordered};
#[cfg(not(verus_only))]
use crate::{simd as simd_ops, vec_ops};
#[cfg(verus_only)]
use hash_dag_std::{simd_ops, vec_ops};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "../proof/write_dense.rs"]
pub(super) mod proof;

pub use super::input::DenseRegion;

#[cfg_attr(verus_only, verus_verify)]
struct DenseBatch<'a, 'v>(&'a [DenseRegion<'v>]);

#[cfg_attr(verus_only, verus_verify)]
impl super::Batch for DenseBatch<'_, '_> {
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
        set_dense_regions(pools, root, self.0, scratch)
    }
}

impl HashDag {
    /// 同步写入完整 Z-order 稠密区域；输入约束见 [`DenseRegion`]。
    ///
    /// 空批次保持原值。每个原生层保留一个 64 槽累积器和相邻节点缓存，
    /// 更高奇数空间 LOD 的八个原生子树按序构建，不保留整层子节点或展开描述。
    /// 同一原生叶内的 LOD0/1 条目合并修改一次，未覆盖的槽保持原值。
    /// 构造引用、池增长和去重存储仍与新唯一节点数量有关。
    /// root 身份与输入约束在资源准备前检查，输入错误不增加保留容量。
    ///
    /// # Errors
    ///
    /// root 失效或属于其他实例、LOD 越界、值数量不匹配、区域不合法、
    /// 节点缺失、容量或引用计数耗尽、分配失败时返回错误；失败保持原 root。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only,verus_spec(result =>
        requires old(self).valid(),ensures final(self).set_dense_regions_post(old(self),root,regions,result),
    ))]
    pub fn set_dense_regions(
        &mut self,
        root: RootId,
        regions: &[DenseRegion<'_>],
    ) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof! {crate::balance::stable_has_no_orphans(crate::state::pools_view(&self.pools),self.root_children());}
        if regions.is_empty() {
            return self.root_child(root).map(|_| ());
        }
        self.write_root(root, DenseBatch(regions))
    }
}

#[cfg_attr(verus_only, verus_verify)]
pub(super) struct DenseFrame {
    slots: [EncodedChild; NODE_FANOUT],
    len: usize,
    cache: DenseCommitCache,
}

#[cfg_attr(verus_only, verus_verify)]
struct DenseCommitCache {
    slots: [EncodedChild; NODE_FANOUT],
    child: Option<Child>,
}

impl Default for DenseFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for DenseCommitCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl DenseFrame {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.len == 0, result.cache.child is None, no_unwind))]
    const fn new() -> Self {
        Self {
            slots: simd_ops::array_fill(EncodedChild::leaf(0)),
            len: 0,
            cache: DenseCommitCache::new(),
        }
    }

    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).len<64,
        ensures final(self).cache==old(self).cache,
            result == (old(self).len==63),
            final(self).len==if result {0usize} else {(old(self).len+1) as usize},
            proof::decoded_slots(final(self).slots).take((old(self).len+1) as int)==old(self).prefix().push(child),
        no_unwind
    ))]
    const fn append(&mut self, child: Child) -> bool {
        #[cfg(verus_only)]
        proof_decl! {let ghost previous=*self;}
        self.slots[self.len] = child.encode();
        self.len += 1;
        let full = self.len == NODE_FANOUT;
        if full {
            self.len = 0;
        }
        #[cfg(verus_only)]
        proof! {crate::node::proof::encoding_round_trip(child);
        assert(proof::decoded_slots(self.slots).take((previous.len+1) as int) =~= previous.prefix().push(child));}
        full
    }

    #[inline]
    #[cfg_attr(verus_only,verus_spec(result =>
        requires old(self).len<64,0<count<=64-old(self).len,
        ensures final(self).cache==old(self).cache,result==(old(self).len+count==64),
            final(self).len==if result {0usize} else {(old(self).len+count) as usize},
            proof::decoded_slots(final(self).slots).take((old(self).len+count) as int)
                ==old(self).prefix()+Seq::new(count as nat,|i:int|crate::node::proof::decode(encoded)),
        no_unwind
    ))]
    fn extend(&mut self, encoded: EncodedChild, count: usize) -> bool {
        #[cfg(verus_only)]
        proof_decl! {let ghost previous=*self;}
        let end = self.len + count;
        simd_ops::fill_range(&mut self.slots, self.len, end, encoded);
        self.len = end;
        let full = self.len == NODE_FANOUT;
        if full {
            self.len = 0;
        }
        #[cfg(verus_only)]
        proof! {assert(proof::decoded_slots(self.slots).take((previous.len+count) as int) =~= previous.prefix()+Seq::new(count as nat,|i:int|crate::node::proof::decode(encoded)));}
        full
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl DenseCommitCache {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.child is None, no_unwind))]
    const fn new() -> Self {
        Self {
            slots: simd_ops::array_fill(EncodedChild::leaf(0)),
            child: None,
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    ensures result is Ok <==> super::validate::proof::dense_prefix(regions@,regions@.len()),no_unwind
))]
pub(super) fn validate(regions: &[DenseRegion<'_>]) -> Result<(), Error> {
    super::validate::dense(regions)
}

/// 输入已在事务边界完整校验，此处只构建候选 root。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    requires super::commit::proof::building(old(pools)),old(scratch).routing_empty(),super::validate::proof::dense_prefix(regions@,regions@.len()),
        crate::state::child_valid(crate::state::pools_view(old(pools)),root,10),
    ensures super::commit::proof::building(final(pools)),crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        result is Ok ==> crate::state::child_valid(crate::state::pools_view(final(pools)),result->Ok_0,10)
            && forall|path:int|0<=path<crate::state::span(10) ==> #[trigger] crate::state::eval(crate::state::pools_view(final(pools)),result->Ok_0,10,path)
                ==proof::effect(regions@,|p:int|crate::state::eval(crate::state::pools_view(old(pools)),root,10,p),path,regions@.len()),
))]
pub(crate) fn set_dense_regions(
    pools: &mut [Pool; DAG_DEPTH as usize],
    root: Child,
    regions: &[DenseRegion<'_>],
    scratch: &mut WriteScratch,
) -> Result<Child, Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost before=crate::state::pools_view(pools);}
    #[cfg(verus_only)]
    proof! {proof::input_stream(regions@,|p:int|crate::state::eval(before,root,10,p),0);}
    let result = apply_ordered(
        pools,
        root,
        DenseEdits {
            pending: regions,
            part: 0,
        },
        scratch,
    );
    #[cfg(verus_only)]
    proof! {if result is Ok {assert forall|path:int|0<=path<crate::state::span(10) implies
    #[trigger] crate::state::eval(crate::state::pools_view(pools),result->Ok_0,10,path)
        ==proof::effect(regions@,|p:int|crate::state::eval(before,root,10,p),path,regions@.len()) by {
            proof::input_stream(regions@,|p:int|crate::state::eval(before,root,10,p),path);
        }}}
    result
}

#[cfg_attr(verus_only, verus_verify)]
struct DenseEdits<'a, 'v> {
    pending: &'a [DenseRegion<'v>],
    part: usize,
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a, 'v: 'a> Edits<'a, &'a [DenseRegion<'v>]> for DenseEdits<'a, 'v> {
    #[inline(always)]
    fn next(&mut self) -> Option<Result<Edit<'a, &'a [DenseRegion<'v>]>, Error>> {
        self.take_next()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a, 'v: 'a> DenseEdits<'a, 'v> {
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
    fn take_next(&mut self) -> Option<Result<Edit<'a, &'a [DenseRegion<'v>]>, Error>> {
        #[cfg(verus_only)]
        proof_decl! { let ghost source = self.pending@; }
        let (region, _) = self.pending.split_first()?;
        #[cfg(verus_only)]
        proof! { assert(super::validate::proof::dense_at(source, 0)); }
        let leaf = region.lod < 2;
        let prefix = region.path.value() >> 6;
        let mut end = 1;
        #[cfg_attr(verus_only, verus_spec(
            invariant source == self.pending@, source == old(self).pending@, self.part == old(self).part,
                self.valid(), super::validate::proof::dense_at(source, 0),
                region == &source[0], source.len() > 0, 0 < end <= self.pending@.len(),
                leaf == (source[0].lod < 2), prefix == crate::morton::proof::value(source[0].path) >> 6,
                !leaf ==> end == 1,
                leaf ==> forall|i:int| 0 <= i < end ==> #[trigger] source[i].lod < 2
                    && crate::morton::proof::value(source[i].path) >> 6 == prefix,
            decreases self.pending@.len() - end,
        ))]
        while leaf
            && end < self.pending.len()
            && self.pending[end].lod < 2
            && self.pending[end].path.value() >> 6 == prefix
        {
            end += 1;
        }
        #[cfg(verus_only)]
        proof! { proof::group_found(source, end as nat); super::validate::proof::dense_suffix(source, end as int); }
        let (group, rest) = self.pending.split_at(end);
        let lod = region.lod / 2;
        let parts = if leaf || region.lod % 2 == 0 { 1 } else { 8 };
        let part = self.part;
        #[cfg(verus_only)]
        proof! { proof::path_safe(crate::morton::proof::value(region.path), region.lod, part as u64); }
        self.part += 1;
        if self.part == parts {
            self.part = 0;
            self.pending = rest;
        }
        if leaf {
            #[cfg(verus_only)]
            proof! {proof::leaf_group_ready(source,end as nat);}
            return Some(Ok(Edit {
                path: region.path,
                lod: 1,
                data: EditData::Patch(group),
            }));
        }
        let count = region.values.len() / parts;
        #[cfg(verus_only)]
        proof! { proof::segment(region.values@.len(), parts as nat, part as nat);
        proof::segment_values(region.values@, parts as nat, part as nat);
        proof::source_region(source,0);proof::parts_proof::geometry(*region,part as nat);
        vstd::arithmetic::div_mod::lemma_div_by_multiple(crate::state::span(lod as nat) as int,parts as int);
        assert(count==crate::state::span(lod as nat)); }
        let offset = (part as u64) << (lod as u32 * 6);
        let result = match ZOrder20x3::from_value(region.path.value() + offset) {
            Some(path) => {
                let start = part * count;
                let values = region.values.split_at(start).1.split_at(count).0;
                Ok(Edit {
                    path,
                    lod,
                    data: EditData::Dense(values),
                })
            }
            None => Err(Error::InvalidInput),
        };
        Some(result)
    }
}

pub(super) use super::validate::dense_value_count;

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    requires super::commit::proof::building(old(pools)),lod<=10,values@.len()==crate::state::span(lod as nat),
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        result is Ok ==> crate::state::child_valid(crate::state::pools_view(final(pools)),result->Ok_0,lod as nat)
            && proof::content(crate::state::pools_view(final(pools)),result->Ok_0,lod as nat)==values@,
))]
pub(super) fn build_dense_subtree(
    pools: &mut [Pool; DAG_DEPTH as usize],
    lod: u8,
    values: &[u32],
    frames: &mut Vec<DenseFrame>,
) -> Result<Child, Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost initial=crate::state::pools_view(pools);}
    frames.clear();
    if lod == 0 {
        #[cfg(verus_only)]
        proof! {assert(values@.len()==1);assert(proof::content(initial,Child::Leaf(values@[0]),0) =~= values@);}
        return match values.first() {
            Some(value) => Ok(Child::Leaf(*value)),
            None => Err(Error::InvalidDenseValues),
        };
    }
    if let Some(value) = uniform_values(values) {
        #[cfg(verus_only)]
        proof! {proof::uniform_content(initial,values@,lod as nat,value);}
        return Ok(Child::Leaf(value));
    }
    let depth = (lod - 1) as usize;
    vec_ops::reserve_exact(frames, depth)?;
    let factory = #[cfg_attr(verus_only,verus_spec(result:DenseFrame => ensures result.len==0,result.cache.child is None))]
    || DenseFrame::new();
    vec_ops::resize_with(frames, depth, factory);
    let groups = values.as_chunks::<NODE_FANOUT>().0;
    #[cfg(verus_only)]
    proof! {proof::groups_shape(values@,groups@,lod as nat);
    assert forall|i:int|0<=i<frames@.len() implies #[trigger] frames@[i].valid(initial,(i+1) as nat) by {}
    proof::pending_empty(initial,frames@,frames@.len());}
    let mut position = 0;
    #[cfg_attr(verus_only,verus_spec(
        invariant initial==crate::state::pools_view(old(pools)),1<=lod<=10,frames@.len()==lod-1,
            super::commit::proof::building(pools),crate::state::build_step(initial,crate::state::pools_view(pools)),
            proof::frames_valid(crate::state::pools_view(pools),frames@),proof::groups_match(values@,groups@),
            position<=groups@.len(),values@.len()==crate::state::span(lod as nat),lod==1 ==> groups@.len()==1,
            proof::pending(crate::state::pools_view(pools),frames@,frames@.len())==values@.take((position*64) as int),
        decreases groups@.len()-position,
    ))]
    while position < groups.len() {
        #[cfg(verus_only)]
        proof_decl! {let ghost before=crate::state::pools_view(pools);let ghost start=position;}
        let run_start = position;
        let group = &groups[position];
        position += 1;
        let result = commit_leaf_value_group(pools, group);
        #[cfg(verus_only)]
        proof! {proof::frames_frozen(before,crate::state::pools_view(pools),frames@,frames@.len());
        crate::balance::build_transitive(initial,before,crate::state::pools_view(pools));}
        let child = result?;
        #[cfg(verus_only)]
        proof! {proof::leaf_content(crate::state::pools_view(pools),child,group@);}
        if matches!(child, Child::Ptr(_)) {
            #[cfg_attr(verus_only,verus_spec(
                invariant start<position<=groups@.len(),run_start==start,group==&groups@[start as int],
                    forall|i:int|start<=i<position ==> #[trigger] groups@[i]@==group@,
                decreases groups@.len()-position,
            ))]
            while position < groups.len()
                && groups[position][0] == group[0]
                && groups[position][NODE_FANOUT - 1] == group[NODE_FANOUT - 1]
                && crate::wide::equal_groups64(&groups[position], group)
            {
                position += 1;
            }
        }
        let run = position - run_start;
        #[cfg(verus_only)]
        proof! {proof::input_run(values@,groups@,start as nat,run as nat,group@);}
        #[cfg(verus_only)]
        proof_decl! {let ghost middle=crate::state::pools_view(pools);let ghost current=frames@;}
        if run == 1 {
            let result = push_dense_child(pools, lod, 1, child, frames);
            #[cfg(verus_only)]
            proof! {crate::balance::build_transitive(initial,middle,crate::state::pools_view(pools));}
            if let Some(root) = result? {
                #[cfg(verus_only)]
                proof! {assert(position*64==values@.len());assert(values@.take((position*64) as int) =~= values@);}
                return Ok(root);
            }
            continue;
        }
        #[cfg(verus_only)]
        proof! {assert(proof::pending(middle,current,current.len())+proof::repeat(proof::content(middle,child,1),run as nat)==values@.take((position*64) as int));}
        let result = push_dense_run(pools, lod, child, run, frames);
        #[cfg(verus_only)]
        proof! {crate::balance::build_transitive(initial,middle,crate::state::pools_view(pools));}
        if let Some(root) = result? {
            #[cfg(verus_only)]
            proof! {assert(proof::content(crate::state::pools_view(pools),root,lod as nat)==values@.take((position*64) as int));
            assert(position*64==values@.len());assert(values@.take((position*64) as int) =~= values@);}
            return Ok(root);
        }
    }
    #[cfg(verus_only)]
    proof! {proof::pending_bound(crate::state::pools_view(pools),frames@,frames@.len());assert(false);}
    Err(Error::InvalidDenseValues)
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    requires super::commit::proof::building(old(pools)),1<=level<=lod<=10,
        old(frames)@.len()==lod-1,proof::frames_valid(crate::state::pools_view(old(pools)),old(frames)@),
        proof::empty_below(old(frames)@,(level-1) as nat),
        crate::state::child_valid(crate::state::pools_view(old(pools)),child,level as nat),
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        final(frames)@.len()==old(frames)@.len(),proof::frames_valid(crate::state::pools_view(final(pools)),final(frames)@),
        result is Ok ==> match result->Ok_0 {
            Some(root)=>crate::state::child_valid(crate::state::pools_view(final(pools)),root,lod as nat)
                && proof::content(crate::state::pools_view(final(pools)),root,lod as nat)
                    ==proof::pending(crate::state::pools_view(old(pools)),old(frames)@,old(frames)@.len())
                        +proof::content(crate::state::pools_view(old(pools)),child,level as nat),
            None=>proof::pending(crate::state::pools_view(final(pools)),final(frames)@,final(frames)@.len())
                ==proof::pending(crate::state::pools_view(old(pools)),old(frames)@,old(frames)@.len())
                    +proof::content(crate::state::pools_view(old(pools)),child,level as nat),
        },
))]
fn push_dense_child(
    pools: &mut [Pool; DAG_DEPTH as usize],
    lod: u8,
    level: u8,
    child: Child,
    frames: &mut [DenseFrame],
) -> Result<Option<Child>, Error> {
    let mut current_level = level;
    let mut current_child = child;
    #[cfg(verus_only)]
    proof_decl! {let ghost initial=crate::state::pools_view(pools);
    let ghost target=proof::pending(initial,frames@,frames@.len())+proof::content(initial,child,level as nat);}
    #[cfg_attr(verus_only,verus_spec(
        invariant initial==crate::state::pools_view(old(pools)),
            target==proof::pending(initial,old(frames)@,old(frames)@.len())+proof::content(initial,child,level as nat),
            super::commit::proof::building(pools),crate::state::build_step(initial,crate::state::pools_view(pools)),
            1<=current_level<=lod<=10,frames@.len()==lod-1,old(frames)@.len()==lod-1,proof::frames_valid(crate::state::pools_view(pools),frames@),
            proof::empty_below(frames@,(current_level-1) as nat),crate::state::child_valid(crate::state::pools_view(pools),current_child,current_level as nat),
            proof::pending(crate::state::pools_view(pools),frames@,frames@.len())+proof::content(crate::state::pools_view(pools),current_child,current_level as nat)==target,
        decreases lod-current_level,
    ))]
    loop {
        if current_level == lod {
            #[cfg(verus_only)]
            proof! {proof::pending_empty(crate::state::pools_view(pools),frames@,frames@.len());}
            return Ok(Some(current_child));
        }
        #[cfg(verus_only)]
        proof_decl! {let ghost before=crate::state::pools_view(pools);let ghost previous=frames@;let ghost appended=proof::content(before,current_child,current_level as nat);}
        let index = (current_level - 1) as usize;
        if index >= frames.len() {
            return Err(Error::BuildFrameMissing);
        }
        #[cfg(verus_only)]
        proof! {assert(frames@[index as int].valid(before,((index as int)+1) as nat));}
        let frame = &mut frames[index];
        if !frame.append(current_child) {
            #[cfg(verus_only)]
            proof! {proof::append_pending(before,previous,frames@,current_level as nat,appended);}
            return Ok(None);
        }
        #[cfg(verus_only)]
        proof_decl! {let ghost row=proof::decoded_slots(frame.slots);}
        #[cfg(verus_only)]
        proof! {
            assert(row==previous[index as int].prefix().push(current_child));
            assert forall|slot:int|0<=slot<64 implies #[trigger] crate::state::child_valid(before,row[slot],current_level as nat) by {
                if slot<63 {assert(row[slot]==previous[index as int].prefix()[slot]);}
                else {assert(row[slot]==current_child);}
            }
        }
        let result = commit_child_group(pools, current_level, &frame.slots, &mut frame.cache);
        #[cfg(verus_only)]
        proof! {proof::reset_valid(before,crate::state::pools_view(pools),previous,frames@,current_level);
        crate::balance::build_transitive(initial,before,crate::state::pools_view(pools));}
        current_child = result?;
        #[cfg(verus_only)]
        proof! {proof::carry_pending(before,crate::state::pools_view(pools),previous,frames@,current_level,appended,current_child,row);
        crate::balance::build_transitive(initial,before,crate::state::pools_view(pools));}
        current_level += 1;
    }
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(result =>
    requires super::commit::proof::building(old(pools)),2<=lod<=10,
        old(frames)@.len()==lod-1,proof::frames_valid(crate::state::pools_view(old(pools)),old(frames)@),
        crate::state::child_valid(crate::state::pools_view(old(pools)),child,1),
        proof::pending(crate::state::pools_view(old(pools)),old(frames)@,old(frames)@.len()).len()+run*64<=crate::state::span(lod as nat),
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        final(frames)@.len()==old(frames)@.len(),proof::frames_valid(crate::state::pools_view(final(pools)),final(frames)@),
        result is Ok ==> match result->Ok_0 {
            Some(root)=>crate::state::child_valid(crate::state::pools_view(final(pools)),root,lod as nat)
                && proof::content(crate::state::pools_view(final(pools)),root,lod as nat)
                    ==proof::pending(crate::state::pools_view(old(pools)),old(frames)@,old(frames)@.len())
                        +proof::repeat(proof::content(crate::state::pools_view(old(pools)),child,1),run as nat),
            None=>proof::pending(crate::state::pools_view(final(pools)),final(frames)@,final(frames)@.len())
                ==proof::pending(crate::state::pools_view(old(pools)),old(frames)@,old(frames)@.len())
                    +proof::repeat(proof::content(crate::state::pools_view(old(pools)),child,1),run as nat),
        },
))]
fn push_dense_run(
    pools: &mut [Pool; DAG_DEPTH as usize],
    lod: u8,
    child: Child,
    run: usize,
    frames: &mut [DenseFrame],
) -> Result<Option<Child>, Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost initial=crate::state::pools_view(pools);let ghost source=proof::content(initial,child,1);
    let ghost target=proof::pending(initial,frames@,frames@.len())+proof::repeat(source,run as nat);}
    #[cfg(verus_only)]
    proof! {reveal_with_fuel(crate::state::span,2);assert(crate::state::span(1)==64);assert(source.len()==64);
    proof::repeat_length(source,run as nat);assert(target.len()==proof::pending(initial,frames@,frames@.len()).len()+run*64);}
    let encoded = child.encode();
    #[cfg(verus_only)]
    proof! {crate::node::proof::encoding_round_trip(child);}
    let mut remaining = run;
    #[cfg_attr(verus_only,verus_spec(
        invariant initial==crate::state::pools_view(old(pools)),source==proof::content(initial,child,1),
            target==proof::pending(initial,old(frames)@,old(frames)@.len())+proof::repeat(source,run as nat),target.len()<=crate::state::span(lod as nat),
            super::commit::proof::building(pools),crate::state::build_step(initial,crate::state::pools_view(pools)),
            2<=lod<=10,source.len()==64,frames@.len()==lod-1,old(frames)@.len()==lod-1,proof::frames_valid(crate::state::pools_view(pools),frames@),
            crate::state::child_valid(crate::state::pools_view(pools),child,1),proof::content(crate::state::pools_view(pools),child,1)==source,
            crate::node::proof::decode(encoded)==child,remaining<=run,
            proof::pending(crate::state::pools_view(pools),frames@,frames@.len())+proof::repeat(source,remaining as nat)==target,
        decreases remaining,
    ))]
    while remaining != 0 {
        #[cfg(verus_only)]
        proof_decl! {let ghost before=crate::state::pools_view(pools);let ghost previous=frames@;let ghost old_remaining=remaining;}
        if frames.is_empty() {
            return Err(Error::BuildFrameMissing);
        }
        #[cfg(verus_only)]
        proof! {assert(frames@[0].valid(before,(0int+1) as nat));}
        let frame = &mut frames[0];
        let count = remaining.min(NODE_FANOUT - frame.len);
        let full = frame.extend(encoded, count);
        remaining -= count;
        if !full {
            #[cfg(verus_only)]
            proof! {proof::fill_pending(before,previous,frames@,child,count as nat);
            proof::append_repeat(proof::pending(before,previous,previous.len()),source,old_remaining as nat,count as nat);}
            continue;
        }
        #[cfg(verus_only)]
        proof_decl! {let ghost row=proof::decoded_slots(frame.slots);}
        #[cfg(verus_only)]
        proof! {
            assert(row==previous[0].prefix()+Seq::new(count as nat,|i:int|child));
            assert forall|slot:int|0<=slot<64 implies #[trigger] crate::state::child_valid(before,row[slot],1) by {
                if slot<previous[0].len {assert(row[slot]==previous[0].prefix()[slot]);}
                else {assert(row[slot]==child);}
            }
        }
        let result = commit_child_group(pools, 1, &frame.slots, &mut frame.cache);
        #[cfg(verus_only)]
        proof! {proof::reset_valid(before,crate::state::pools_view(pools),previous,frames@,1);
        crate::balance::build_transitive(initial,before,crate::state::pools_view(pools));}
        let parent = result?;
        #[cfg(verus_only)]
        proof! {proof::fill_carry(before,crate::state::pools_view(pools),previous,frames@,child,count as nat,parent,row);
        proof::content_frozen(before,crate::state::pools_view(pools),child,1);}
        #[cfg(verus_only)]
        proof_decl! {let ghost middle=crate::state::pools_view(pools);let ghost current=frames@;}
        let result = push_dense_child(pools, lod, 2, parent, frames);
        #[cfg(verus_only)]
        proof! {crate::balance::build_transitive(initial,middle,crate::state::pools_view(pools));
        proof::content_frozen(middle,crate::state::pools_view(pools),child,1);
        proof::append_repeat(proof::pending(before,previous,previous.len()),source,old_remaining as nat,count as nat);}
        if let Some(root) = result? {
            #[cfg(verus_only)]
            proof! {assert(proof::content(crate::state::pools_view(pools),root,lod as nat)+proof::repeat(source,remaining as nat)==target);
            proof::repeat_length(source,remaining as nat);assert(remaining==0);}
            return Ok(Some(root));
        }
    }
    Ok(None)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires super::commit::proof::building(old(pools)),
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        result is Err ==> crate::state::pools_view(final(pools)) == crate::state::pools_view(old(pools)),
        result is Ok ==> proof::represented(crate::state::pools_view(final(pools)),0,result->Ok_0,proof::leaf_row(values@)),
))]
fn commit_leaf_value_group(
    pools: &mut [Pool; DAG_DEPTH as usize],
    values: &[u32; NODE_FANOUT],
) -> Result<Child, Error> {
    if let Some(data) = fold_leaf_slice(values.as_slice()) {
        #[cfg(verus_only)]
        proof! { proof::uniform_leaf(values@, data); }
        return Ok(Child::Leaf(data));
    }
    let node = PoolEntry::new(values, 0);
    #[cfg(verus_only)]
    proof! { proof::leaf_entry(node); }
    commit_node(pools, 0, &node)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires 0 < pool_lod < 10,super::commit::proof::building(old(pools)),
        old(cache).valid(crate::state::pools_view(old(pools)),pool_lod),
        proof::row_domain(crate::state::pools_view(old(pools)),pool_lod,proof::decoded_slots(*slots)),
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        final(cache).valid(crate::state::pools_view(final(pools)),pool_lod),
        result is Err ==> crate::state::pools_view(final(pools)) == crate::state::pools_view(old(pools)),
        result is Ok ==> proof::represented(crate::state::pools_view(final(pools)),pool_lod,result->Ok_0,proof::decoded_slots(*slots)),
))]
fn commit_child_group(
    pools: &mut [Pool; DAG_DEPTH as usize],
    pool_lod: u8,
    slots: &[EncodedChild; NODE_FANOUT],
    cache: &mut DenseCommitCache,
) -> Result<Child, Error> {
    if let Some(value) = slots[0].leaf_value() {
        let predicate = #[cfg_attr(verus_only, verus_spec(result:bool =>
            ensures result == (crate::node::proof::decode(*slot) == Child::Leaf(value))
        ))]
        |slot: &EncodedChild| slot.leaf_value() == Some(value);
        if slots.iter().all(predicate) {
            #[cfg(verus_only)]
            proof! {
                assert(forall|i:int| 0 <= i < 64 ==> #[trigger] crate::node::proof::decode(slots@[i]) == Child::Leaf(value));
                assert(proof::decoded_slots(*slots) =~= Seq::new(64,|i:int|Child::Leaf(value)));
            }
            return Ok(Child::Leaf(value));
        }
    }
    // 固定 Verus 尚不支持 let-chain。
    #[allow(clippy::collapsible_if)]
    if let Some(child) = cache.child {
        if cache.slots == *slots {
            #[cfg(verus_only)]
            proof! { assert(cache.slots@ == slots@); }
            return Ok(child);
        }
    }
    let node = PoolEntry::from_encoded_slots(slots);
    #[cfg(verus_only)]
    proof! {
        assert(crate::node::proof::children(node) =~= proof::decoded_slots(*slots));
    }
    let child = commit_node(pools, pool_lod, &node)?;
    cache.slots = *slots;
    cache.child = Some(child);
    Ok(child)
}

#[cfg(test)]
#[path = "dense_chunks_lod0_tests.rs"]
mod lod0_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{self, Point};
    use crate::{HashDag, SetCell};
    use hashbrown::HashMap;
    use proptest::prelude::*;
    use proptest::test_runner::TestCaseError;

    fn path(x: u32, y: u32, z: u32) -> ZOrder20x3 {
        ZOrder20x3::encode_unchecked(x, y, z)
    }

    fn checked<T>(result: Result<T, Error>) -> Result<T, TestCaseError> {
        result.map_err(|error| TestCaseError::fail(format!("HashDag 返回意外错误： {error:?}")))
    }

    fn total_len(dag: &HashDag) -> usize {
        crate::test_util::total_len(&dag.pools)
    }

    fn dense_values(lod: u8, f: impl Fn(u32, u32, u32) -> u32) -> Vec<u32> {
        let count = 1usize << (u32::from(lod) * 3);
        let mut values = Vec::with_capacity(count);
        for index in 0..count {
            let point = test_util::local_point(index, lod);
            values.push(f(point.x, point.y, point.z));
        }
        values
    }

    #[test]
    fn dense_chunk_roundtrip_reads_values() {
        let Some(mut dag) = test_util::ok(HashDag::new()) else {
            return;
        };
        let values: Vec<u32> = (0..64u32).map(|v| v + 1).collect();
        let chunks = [DenseRegion {
            path: path(0, 0, 0),
            lod: 2,
            values: &values,
        }];

        let _ = test_util::ok(dag.set_dense_regions(dag.root(), &chunks));

        for (index, value) in values.iter().copied().enumerate() {
            let point = test_util::local_point(index, 2);
            assert_eq!(
                dag.value_at(dag.root(), path(point.x, point.y, point.z))
                    .expect("读取写入值"),
                value
            );
        }
    }

    #[test]
    fn uniform_dense_chunk_folds() {
        let Some(mut dag) = test_util::ok(HashDag::new()) else {
            return;
        };
        let values = [7u32; 64];
        let chunks = [DenseRegion {
            path: path(0, 0, 0),
            lod: 2,
            values: &values,
        }];

        let _ = test_util::ok(dag.set_dense_regions(dag.root(), &chunks));

        assert_eq!(
            dag.subtree_at(dag.root(), path(0, 0, 0), 2)
                .expect("读取子树"),
            crate::SubtreeState::Uniform { value: 7 }
        );
    }

    #[test]
    fn multiple_dense_chunks_write_disjoint_ranges() {
        let Some(mut dag) = test_util::ok(HashDag::new()) else {
            return;
        };
        let a = [3u32; 64];
        let b = [5u32; 64];
        let chunks = [
            DenseRegion {
                path: path(0, 0, 0),
                lod: 2,
                values: &a,
            },
            DenseRegion {
                path: path(4, 0, 0),
                lod: 2,
                values: &b,
            },
        ];

        let _ = test_util::ok(dag.set_dense_regions(dag.root(), &chunks));

        assert_eq!(
            dag.value_at(dag.root(), path(1, 0, 0)).expect("读取写入值"),
            3
        );
        assert_eq!(
            dag.value_at(dag.root(), path(5, 0, 0)).expect("读取写入值"),
            5
        );
        assert_eq!(
            dag.value_at(dag.root(), path(8, 0, 0)).expect("读取邻区"),
            0
        );
    }

    #[test]
    fn adjacent_leaf_groups_share_nodes_and_release() -> Result<(), TestCaseError> {
        let mut dag = checked(HashDag::new())?;
        let root = checked(dag.fork_root(dag.root()))?;
        let values = dense_values(6, |x, y, z| (x ^ y ^ z) & 1);
        checked(dag.set_dense_regions(
            root,
            &[DenseRegion {
                path: path(0, 0, 0),
                lod: 6,
                values: &values,
            }],
        ))?;
        prop_assert_eq!(checked(dag.pool_slot_count(0))?, 1);
        for x in 0..64 {
            prop_assert_eq!(checked(dag.value_at(root, path(x, 31, 47)))?, x & 1);
        }
        checked(dag.release_root(root))?;
        prop_assert_eq!(total_len(&dag), 0);
        Ok(())
    }

    #[test]
    fn repeated_leaf_runs_preserve_spatial_order() -> Result<(), Error> {
        let values: Vec<_> = (0..1_usize << 18)
            .map(|code| {
                let leaf = code / NODE_FANOUT;
                match code % NODE_FANOUT {
                    0 => 3,
                    63 => 5,
                    31 if leaf < 5 => 7,
                    31 if leaf < 137 => 11,
                    31 => 13,
                    slot => slot as u32,
                }
            })
            .collect();
        let mut dag = HashDag::new()?;
        let root = dag.root();
        dag.set_dense_regions(
            root,
            &[DenseRegion {
                path: ZOrder20x3::ZERO,
                lod: 6,
                values: &values,
            }],
        )?;
        let mut actual = vec![0; values.len()];
        dag.decode_region(root, ZOrder20x3::ZERO, 6, &mut actual)?;
        for (index, &value) in actual.iter().enumerate() {
            let code = ZOrder20x3::encode(
                (index % 64) as u32,
                (index / 64 % 64) as u32,
                (index / 4096) as u32,
            )
            .expect("区域内 Morton 坐标")
            .value() as usize;
            assert_eq!(value, values[code], "线性索引 {index}，Morton 路径 {code}");
        }
        Ok(())
    }

    #[test]
    fn repeated_dense_chunk_releases_cached_nodes() {
        let Some(mut dag) = test_util::ok(HashDag::new()) else {
            return;
        };
        let Some(root) = test_util::ok(dag.fork_root(dag.root())) else {
            return;
        };
        let values = dense_values(3, |x, _, _| x & 7);
        let chunks = [DenseRegion {
            path: path(0, 0, 0),
            lod: 3,
            values: &values,
        }];

        let _ = test_util::ok(dag.set_dense_regions(root, &chunks));
        let _ = test_util::ok(dag.release_root(root));

        assert_eq!(total_len(&dag), 0);
    }

    proptest! {
        #[test]
        fn dense_chunks_match_reference(
            left in prop::collection::vec(0u32..8u32, 64),
            right in prop::collection::vec(0u32..8u32, 64),
        ) {
            let mut dag = checked(HashDag::new())?;
            let chunks = [
                DenseRegion { path: path(0, 0, 0), lod: 2, values: &left },
                DenseRegion { path: path(4, 0, 0), lod: 2, values: &right },
            ];
            checked(dag.set_dense_regions(dag.root(), &chunks))?;

            let mut reference = HashMap::new();
            for (index, value) in left.iter().copied().enumerate() {
                let point = test_util::local_point(index, 2);
                reference.insert(point, value);
            }
            for (index, value) in right.iter().copied().enumerate() {
                let point = test_util::local_point(index, 2);
                reference.insert(Point { x: point.x + 4, y: point.y, z: point.z }, value);
            }

            for (point, value) in reference {
                prop_assert_eq!(checked(dag.value_at(dag.root(), path(point.x, point.y, point.z)))?, value);
            }
        }
    }

    #[test]
    fn dense_chunk_can_overlay_set_cells() {
        let Some(mut dag) = test_util::ok(HashDag::new()) else {
            return;
        };
        let _ = test_util::ok(dag.set_cells(
            dag.root(),
            &[SetCell {
                path: path(1, 0, 0),
                data: 9,
            }],
        ));

        let values = [4u32; 64];
        let chunks = [DenseRegion {
            path: path(0, 0, 0),
            lod: 2,
            values: &values,
        }];
        let _ = test_util::ok(dag.set_dense_regions(dag.root(), &chunks));

        assert_eq!(
            dag.value_at(dag.root(), path(1, 0, 0)).expect("读取覆盖值"),
            4
        );
    }

    #[test]
    fn dense_values_helper_has_expected_order() {
        let values = dense_values(1, |x, y, z| x | (y << 1) | (z << 2));
        assert_eq!(values, (0u32..8u32).collect::<Vec<_>>());
    }

    #[test]
    fn mixed_dense_depths_follow_local_order_and_preserve_gaps() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let specifications = [(0, 1_u8), (64, 2), (512, 3), (4096, 4), (32768, 5)];
        let values: Vec<Vec<u32>> = specifications
            .iter()
            .map(|&(_, lod)| {
                (0..1_u32 << (u32::from(lod) * 3))
                    .map(|index| index.wrapping_mul(0x9e37_79b9) ^ u32::from(lod))
                    .collect()
            })
            .collect();
        let regions: Vec<_> = specifications
            .iter()
            .zip(&values)
            .map(|(&(base, lod), values)| DenseRegion {
                path: ZOrder20x3::from_value(base).expect("对齐起点"),
                lod,
                values,
            })
            .collect();
        dag.set_dense_regions(dag.root(), &regions)?;
        for region in regions {
            for (index, &value) in region.values.iter().enumerate() {
                let path =
                    ZOrder20x3::from_value(region.path.value() + index as u64).expect("区域坐标");
                assert_eq!(dag.value_at(dag.root(), path)?, value);
            }
            let after = ZOrder20x3::from_value(region.path.value() + region.values.len() as u64)
                .expect("相邻坐标");
            assert_eq!(dag.value_at(dag.root(), after)?, 0);
        }
        Ok(())
    }
}
