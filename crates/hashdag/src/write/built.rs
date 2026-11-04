//! 调用方调度的局部构建与事务式批量导入；不读取应用材料语义，不创建线程或全局锁。
//!
//! worker 独占 [`LocalBuilder`] 深度帧和 dense 输入，结果槽独占 [`LocalTree`] 的分层 SoA 池。
//! 局部树不创建全局根或空间祖先；同层完整内容先在本地去重。导入每个区域时，其本地唯一节点
//! 只翻译一次，再参与目标 DAG 的全局去重；不展开共享子树或回读完整 dense。输入 0/MAX 都是普通值。
//! 局部池保留构造引用到整体重建／释放，不与全局引用计数共用；目标节点仅引用目标池。
//! 总工作集须包含所有并发 worker、结果槽、目标池和在途翻译表，不能只统计 dense scratch。

use super::dense_chunks::{DenseFrame, build_dense_subtree, dense_value_count};
use super::frames::WriteScratch;
use super::ordered::{Edit, EditData, Edits, LeafPatch, apply_ordered};
use crate::hashdag::{DAG_DEPTH, Error};
use crate::node::Child;
#[cfg(test)]
use crate::node::PoolEntry;
use crate::pool::Pool;
#[cfg(not(verus_only))]
use crate::simd as simd_ops;
#[cfg(not(verus_only))]
use crate::vec_ops;
use crate::{HashDag, RootId, ZOrder20x3};
#[cfg(verus_only)]
use hash_dag_std::simd_ops;
#[cfg(verus_only)]
use hash_dag_std::vec_ops;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "../proof/write_local.rs"]
pub(crate) mod local_proof;
#[cfg(verus_only)]
#[path = "../proof/write_built.rs"]
pub(super) mod proof;

/// 独占且可复用的局部 dense DAG；私有节点编号不能作为全局 [`crate::NodeIdx`] 使用。
///
/// 支持 `lod=0..=20`；偶数 LOD 持一个原生根，奇数 LOD 持 Morton 顺序的八个子根。
/// 不借用输入，也不引用目标 DAG；合并完成后可立即重建／丢弃。默认尚无有效内容。
///
/// ```
/// use hash_dag::{BuiltRegion, HashDag, LocalBuilder, LocalTree, ZOrder20x3};
/// let mut tree = LocalTree::new();
/// LocalBuilder::new().build(&mut tree, 2, &(0..64).collect::<Vec<u32>>())?;
/// let mut dag = HashDag::new()?;
/// dag.set_built_regions(dag.root(), &[BuiltRegion { path: ZOrder20x3::ZERO, tree: &tree }])?;
/// drop(tree); // 目标池不借用局部编号或输入。
/// assert_eq!(dag.value_at(dag.root(), ZOrder20x3::from_value(63).unwrap())?, 63);
/// # Ok::<(), hash_dag::Error>(())
/// ```
#[derive(Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct LocalTree {
    pub(super) pools: [Pool; DAG_DEPTH as usize],
    pub(super) roots: [Child; 8],
    pub(super) lod: Option<u8>,
}

#[cfg_attr(verus_only, verus_verify)]
impl Default for LocalTree {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.valid_local()))]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl LocalTree {
    /// 创建不分配节点存储的结果槽；首次成功构建前不能导入。
    #[cfg_attr(verus_only, verus_spec(result => ensures result.empty_local(), result.valid_local()))]
    pub fn new() -> Self {
        let tree = Self {
            pools: crate::layers::empty(),
            roots: [Child::Leaf(0); 8],
            lod: None,
        };
        #[cfg(verus_only)]
        proof! { local_proof::empty_valid(&tree); }
        tree
    }
}
impl LocalTree {
    /// 最近一次成功构建的空间 LOD；尚未构建或资源失败后为 None。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result==self.built_lod(),no_unwind
    ))]
    pub const fn lod(&self) -> Option<u8> {
        self.lod
    }
    /// 局部唯一非均匀节点总数，不计折叠叶和空间祖先。
    pub fn node_count(&self) -> usize {
        self.pools.iter().map(|pool| pool.arrays().0.len()).sum()
    }
    /// 池、局部去重表与构造持有的实际保留堆容量，不含 Self／allocator 元数据。
    pub fn retained_bytes(&self) -> usize {
        self.pools.iter().map(Pool::retained_bytes).sum()
    }
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(ensures final(self).empty_local(), final(self).valid_local()))]
    fn clear(&mut self) {
        self.lod = None;
        self.roots = simd_ops::array_fill(Child::Leaf(0));
        let mut level = 0;
        #[cfg_attr(verus_only, verus_spec(
            invariant level<=10, self.lod is None,
                forall|i:int|0<=i<8 ==> #[trigger] self.roots@[i]==Child::Leaf(0),
                forall|i:int|0<=i<level ==> #[trigger] self.pools@[i].valid() && self.pools@[i].state().nodes.len()==0,
            decreases 10-level,
        ))]
        while level < DAG_DEPTH as usize {
            #[cfg(verus_only)]
            proof_decl! { let ghost before = self.pools@; }
            self.pools[level].clear_local();
            #[cfg(verus_only)]
            proof! {
                assert forall|i: int| 0 <= i < level implies #[trigger] self.pools@[i] == before[i] by {}
            }
            level += 1;
        }
        #[cfg(verus_only)]
        proof! { proof::local_cleared(self); local_proof::empty_valid(self); }
    }
}

/// worker 独占的构建 scratch；只有按树深度增长的帧，不持有结果或跨调用节点引用。
#[derive(Default)]
#[cfg_attr(verus_only, verus_verify)]
pub struct LocalBuilder {
    frames: Vec<DenseFrame>,
}

#[cfg_attr(verus_only, verus_verify)]
impl LocalBuilder {
    /// 创建空 scratch；调用方可在线程或任务之间显式移动所有权。
    #[cfg_attr(verus_only, verus_spec(result => ensures result.empty_builder(), no_unwind))]
    pub const fn new() -> Self {
        Self {
            frames: vec_ops::new(),
        }
    }
}
impl LocalBuilder {
    /// 帧的保留堆容量，不含结果槽、输入或 Self。
    pub fn retained_bytes(&self) -> usize {
        self.frames.capacity() * size_of::<DenseFrame>()
    }
    /// 把局部 Morton dense 构建到结果槽，输入长度须为 `2^(3*lod)`。
    ///
    /// 校验失败保留原结果；开始构建后遇到资源失败会使结果失效并清空逻辑节点，保留容量。
    /// 内核不调度线程、不触碰全局 DAG；调用返回后不保留 values 的借用。
    /// # Errors
    /// LOD／值数量无效，或节点、引用计数／容量耗尽、分配失败时返回对应 Error 并保留来源。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(tree).valid_local(),
        ensures final(tree).valid_local(),
            result is Ok ==> final(tree).built_from(lod,values@),
            result is Err && proof::input_valid(lod,values.len()) ==> final(tree).empty_local(),
            !proof::input_valid(lod,values.len()) ==> *final(tree)==*old(tree) && *final(self)==*old(self),
            proof::input_valid(lod,values.len()) ==> final(self).empty_builder(),
    ))]
    pub fn build(&mut self, tree: &mut LocalTree, lod: u8, values: &[u32]) -> Result<(), Error> {
        if lod > ZOrder20x3::BIT_DEPTH {
            return Err(Error::InvalidLod(lod));
        }
        if Some(values.len()) != dense_value_count(lod) {
            return Err(Error::InvalidDenseValues);
        }
        tree.clear();
        #[cfg(verus_only)]
        proof! {proof::empty_graph(&tree.pools);proof::input_parts(lod,values.len());}
        #[cfg(verus_only)]
        proof_decl! {let ghost empty=crate::state::pools_view(&tree.pools);}
        crate::layers::set_phase::<true>(&mut tree.pools);
        #[cfg(verus_only)]
        proof! {crate::layers::proof::phase_balance(empty,crate::state::pools_view(&tree.pools),Seq::empty(),true);}
        #[cfg(verus_only)]
        proof_decl! {let ghost initial=crate::state::pools_view(&tree.pools);}
        let parts = if lod % 2 == 0 { 1 } else { 8 };
        let count = values.len() / parts;
        let result = build_parts(
            &mut tree.pools,
            &mut tree.roots,
            lod,
            values,
            parts,
            count,
            &mut self.frames,
        );
        #[cfg(verus_only)]
        proof! {crate::balance::build_keeps_balance(initial,crate::state::pools_view(&tree.pools),Seq::empty());
        proof::no_free(initial,crate::state::pools_view(&tree.pools));}
        self.frames.clear();
        if result.is_err() {
            tree.clear();
        } else {
            #[cfg(verus_only)]
            proof_decl! {let ghost built=crate::state::pools_view(&tree.pools);}
            crate::layers::set_phase::<false>(&mut tree.pools);
            #[cfg(verus_only)]
            proof! {crate::layers::proof::phase_balance(built,crate::state::pools_view(&tree.pools),Seq::empty(),false);
            proof::parts_phase(built,crate::state::pools_view(&tree.pools),tree.roots@,values@,(lod/2) as nat,parts as nat,count as nat);}
            tree.lod = Some(lod);
            #[cfg(verus_only)]
            proof! { local_proof::built_valid(tree, lod, values@); }
        }
        result
    }
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires super::commit::proof::building(old(pools)),lod<=20,0<parts<=8,
        count==crate::state::span((lod/2) as nat),values@.len()==parts*count,
    ensures super::commit::proof::building(final(pools)),
        crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        result is Ok ==> proof::built_parts(crate::state::pools_view(final(pools)),final(roots)@,values@,(lod/2) as nat,parts as nat,count as nat),
))]
fn build_parts(
    pools: &mut [Pool; DAG_DEPTH as usize],
    roots: &mut [Child; 8],
    lod: u8,
    values: &[u32],
    parts: usize,
    count: usize,
    frames: &mut Vec<DenseFrame>,
) -> Result<(), Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost initial=crate::state::pools_view(pools);}
    let mut part = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant part<=parts,parts<=8,lod<=20,count==crate::state::span((lod/2) as nat),values@.len()==parts*count,
            super::commit::proof::building(pools),crate::state::build_step(initial,crate::state::pools_view(pools)),
            initial==crate::state::pools_view(old(pools)),
            proof::built_parts(crate::state::pools_view(pools),roots@,values@,(lod/2) as nat,part as nat,count as nat),
        decreases parts-part,
    ))]
    while part < parts {
        #[cfg(verus_only)]
        proof_decl! {let ghost step=crate::state::pools_view(pools);let ghost old_roots=roots@;}
        #[cfg(verus_only)]
        proof! {assert(values.len()==values@.len());proof::part_bounds(values@.len(),parts as nat,count as nat,part as nat);}
        let start = part * count;
        let result = build_dense_subtree(pools, lod / 2, &values[start..start + count], frames);
        #[cfg(verus_only)]
        proof! {crate::balance::build_transitive(initial,step,crate::state::pools_view(pools));}
        let child = result?;
        #[cfg(verus_only)]
        proof! {proof::parts_frozen(step,crate::state::pools_view(pools),old_roots,values@,(lod/2) as nat,part as nat,count as nat);}
        roots[part] = child;
        #[cfg(verus_only)]
        proof! {proof::part_added(crate::state::pools_view(pools),old_roots,roots@,values@,(lod/2) as nat,part as nat,count as nat,child);}
        part += 1;
    }
    Ok(())
}

/// 将已构建结果覆盖到全局空间；按 path 升序、按 tree.lod 对齐，且批内互不重叠。
///
/// tree 借用只持续到合并返回；相同局部树可用于多个不相交位置，不需要 Clone 或 sentinel。
#[derive(Clone, Copy, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct BuiltRegion<'a> {
    /// 目标空间的 Morton 起点。
    pub path: ZOrder20x3,
    /// 成功完成构建的独占结果槽，只读共享给导入内核。
    pub tree: &'a LocalTree,
}

#[cfg_attr(verus_only, verus_verify)]
struct BuiltBatch<'a, 'v>(&'a [BuiltRegion<'v>]);

#[cfg_attr(verus_only, verus_verify)]
impl super::Batch for BuiltBatch<'_, '_> {
    fn validate(&self) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof! {use_type_invariant(self);}
        validate(self.0)
    }
    #[inline(always)]
    fn build(
        self,
        pools: &mut [Pool; DAG_DEPTH as usize],
        root: Child,
        scratch: &mut WriteScratch,
    ) -> Result<Child, Error> {
        #[cfg(verus_only)]
        proof! {use_type_invariant(&self);}
        merge(pools, root, self.0, scratch)
    }
}

impl HashDag {
    /// 导入已并行构建的局部结果并一次发布；不重新扫描 dense，也不隐藏调度。
    ///
    /// 每个区域内的局部唯一节点自底向上翻译一次；同一 LocalTree 用于多个区域时分别翻译。
    /// 全局完整内容去重、折叠和 RC 规则与其他写入一致。
    /// 有序区域共用祖先路由。旧 root／快照保持存活到候选完成；失败不改变原 root，已增长
    /// 的容量不回滚。导入翻译 scratch 归 HashDag 所有，并计入 retained_bytes。
    /// # Errors
    /// root 无效、局部结果未成功构建、输入顺序／对齐／重叠不合法，或节点、容量／引用计数
    /// 耗尽和分配失败时返回对应 Error。root 与全部输入约束在资源准备前检查。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),proof::healthy(regions@),
        ensures final(self).set_built_post(old(self),root,regions,result),
    ))]
    pub fn set_built_regions(
        &mut self,
        root: RootId,
        regions: &[BuiltRegion<'_>],
    ) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof! {crate::balance::stable_has_no_orphans(crate::state::pools_view(&self.pools),self.root_children());}
        if regions.is_empty() {
            return self.root_child(root).map(|_| ());
        }
        self.write_root(root, BuiltBatch(regions))
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::healthy(regions@),
    ensures result is Ok <==> proof::batch_prefix(regions@,regions@.len()),no_unwind
))]
fn validate(regions: &[BuiltRegion<'_>]) -> Result<(), Error> {
    let mut end = 0;
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index<=regions@.len(),proof::healthy(regions@),proof::batch_prefix(regions@,index as nat),
            end==if index==0 {0int}else{proof::region_end(regions@[(index-1) as int])},
        decreases regions@.len()-index,
    ))]
    while index < regions.len() {
        let region = &regions[index];
        let lod = match region.tree.lod {
            Some(lod) => lod,
            None => {
                #[cfg(verus_only)]
                proof! {proof::batch_failure(regions@,index as nat);}
                return Err(Error::SubtreeNotBuilt);
            }
        };
        #[cfg(verus_only)]
        proof! {assert(region.tree.valid_local());assert(super::import::proof::source_valid(region.tree));super::validate::proof::span_safe(lod);}
        let span = 1_u64 << (lod as u32 * 3);
        let code = region.path.value();
        if code & (span - 1) != 0 || code < end {
            #[cfg(verus_only)]
            proof! {proof::batch_failure(regions@,index as nat);}
            return Err(Error::InvalidInput);
        }
        end = match code.checked_add(span) {
            Some(end) => end,
            None => {
                #[cfg(verus_only)]
                proof! {proof::batch_failure(regions@,index as nat);}
                return Err(Error::CapacityOverflow);
            }
        };
        #[cfg(verus_only)]
        proof! {assert(proof::batch_at(regions@,index as nat));proof::batch_validated(regions@,index as nat);}
        index += 1;
    }
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires super::commit::proof::building(old(pools)),proof::healthy(regions@),proof::batch_prefix(regions@,regions@.len()),
        old(scratch).routing_empty(),crate::state::child_valid(crate::state::pools_view(old(pools)),root,10),
    ensures super::commit::proof::building(final(pools)),crate::state::build_step(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        result is Ok ==> crate::state::child_valid(crate::state::pools_view(final(pools)),result->Ok_0,10)
            && forall|path:int|0<=path<crate::state::span(10) ==> #[trigger] crate::state::eval(crate::state::pools_view(final(pools)),result->Ok_0,10,path)
                ==proof::batch_effect(regions@,|p:int|crate::state::eval(crate::state::pools_view(old(pools)),root,10,p),path,regions@.len()),
))]
fn merge(
    pools: &mut [Pool; DAG_DEPTH as usize],
    root: Child,
    regions: &[BuiltRegion<'_>],
    scratch: &mut WriteScratch,
) -> Result<Child, Error> {
    #[cfg(verus_only)]
    proof_decl! {let ghost before=crate::state::pools_view(pools);}
    #[cfg(verus_only)]
    proof! {
        assert forall|i:int|0<=i<regions@.len() implies #[trigger] proof::ready(regions@[i])
            && super::import::proof::source_valid(regions@[i].tree) by {
            assert(proof::batch_at(regions@,i as nat));assert(regions@[i].tree.valid_local());
        }
        proof::stream_proof::input_stream(regions@,|p:int|crate::state::eval(before,root,10,p),0);
    }
    let result = apply_ordered(
        pools,
        root,
        BuiltEdits {
            pending: regions,
            part: 0,
        },
        scratch,
    );
    #[cfg(verus_only)]
    proof! {if result is Ok {assert forall|path:int|0<=path<crate::state::span(10) implies
    #[trigger] crate::state::eval(crate::state::pools_view(pools),result->Ok_0,10,path)
        ==proof::batch_effect(regions@,|p:int|crate::state::eval(before,root,10,p),path,regions@.len()) by {
            proof::stream_proof::input_stream(regions@,|p:int|crate::state::eval(before,root,10,p),path);
        }}}
    result
}

#[cfg_attr(verus_only, verus_verify)]
struct BuiltEdits<'a, 'v> {
    pending: &'a [BuiltRegion<'v>],
    part: usize,
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a, 'v: 'a> Edits<'a, &'a [BuiltRegion<'v>]> for BuiltEdits<'a, 'v> {
    #[inline(always)]
    fn next(&mut self) -> Option<Result<Edit<'a, &'a [BuiltRegion<'v>]>, Error>> {
        self.take_next()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a, 'v: 'a> BuiltEdits<'a, 'v> {
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
    fn take_next(&mut self) -> Option<Result<Edit<'a, &'a [BuiltRegion<'v>]>, Error>> {
        #[cfg(verus_only)]
        proof_decl! { let ghost source = self.pending@; }
        let (region, _) = self.pending.split_first()?;
        #[cfg(verus_only)]
        proof! { assert(proof::ready(*region)); }
        let lod = match region.tree.lod {
            Some(lod) => lod,
            None => panic!("边界已校验"),
        };
        let leaf = lod < 2;
        let prefix = region.path.value() >> 6;
        let mut end = 1;
        #[cfg_attr(verus_only, verus_spec(
            invariant source == self.pending@, source == old(self).pending@, self.part == old(self).part,
                self.valid(), proof::ready(*region), region == &source[0], source.len() > 0,
                region.tree.lod == Some(lod), leaf == (lod < 2),
                prefix == crate::morton::proof::value(source[0].path) >> 6, 0 < end <= self.pending@.len(),
                !leaf ==> end == 1,
                leaf ==> forall|i:int| 0 <= i < end ==> proof::small(#[trigger] source[i].tree)
                    && crate::morton::proof::value(source[i].path) >> 6 == prefix,
            decreases self.pending@.len() - end,
        ))]
        while leaf
            && end < self.pending.len()
            && matches!(self.pending[end].tree.lod, None | Some(0 | 1))
            && self.pending[end].path.value() >> 6 == prefix
        {
            #[cfg(verus_only)]
            proof! { assert(proof::ready(source[end as int])); }
            end += 1;
        }
        #[cfg(verus_only)]
        proof! { proof::group_found(source, end as nat); }
        #[cfg(verus_only)]
        proof! {assert forall|i:int|0<=i<source.len() implies #[trigger] proof::ready(source[i])
        && super::import::proof::source_valid(source[i].tree) by {assert(proof::ready(self.pending@[i]));}
        proof::ready_suffix(source,end as nat);if leaf {proof::leaf_group_ready(source,end as nat);}}
        let (group, rest) = self.pending.split_at(end);
        let parts = if leaf || lod % 2 == 0 { 1 } else { 8 };
        let part = self.part;
        #[cfg(verus_only)]
        proof! { super::dense_chunks::proof::path_safe(crate::morton::proof::value(region.path), lod, part as u64); }
        self.part += 1;
        if self.part == parts {
            self.part = 0;
            self.pending = rest;
        }
        Some(
            match ZOrder20x3::from_value(
                region.path.value() + ((part as u64) << ((lod / 2) as u32 * 6)),
            ) {
                Some(path) => Ok(Edit {
                    path,
                    lod: if leaf { 1 } else { lod / 2 },
                    data: if leaf {
                        EditData::Patch(group)
                    } else {
                        EditData::Built {
                            tree: region.tree,
                            part,
                        }
                    },
                }),
                None => Err(Error::InvalidInput),
            },
        )
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl LeafPatch for &[BuiltRegion<'_>] {
    fn keeps_leaf(self, value: u32) -> bool {
        let mut index = 0;
        #[cfg_attr(verus_only, verus_spec(
            invariant index <= self@.len(), proof::leaf_regions(self@),
                proof::kept(self@, index as nat, value),
            decreases self@.len() - index,
        ))]
        while index < self.len() {
            let region = &self[index];
            let count = if region.tree.lod == Some(0) { 1 } else { 8 };
            let mut position = 0;
            #[cfg_attr(verus_only, verus_spec(
                invariant index < self@.len(), region == &self@[index as int],
                    count == proof::count(region.tree), count <= 8, position <= count,
                    forall|i:int| 0 <= i < position ==> #[trigger] region.tree.roots@[i] == Child::Leaf(value),
                decreases count - position,
            ))]
            while position < count {
                if !region.tree.roots[position].same(Child::Leaf(value)) {
                    return false;
                }
                position += 1;
            }
            #[cfg(verus_only)]
            proof! { proof::kept_step(self@, index as nat, value); }
            index += 1;
        }
        #[cfg(verus_only)]
        proof! { proof::keeps(self@, self@.len(), value); }
        true
    }
    fn apply(self, values: &mut [u32; 64]) -> Result<bool, Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = values@; }
        let mut changed = false;
        let mut index = 0;
        #[cfg_attr(verus_only, verus_spec(
            invariant index <= self@.len(), proof::leaf_regions(self@), before.len() == 64,
                values@ == proof::apply(before, self@, index as nat),
                !changed ==> values@ == before,
            decreases self@.len() - index,
        ))]
        while index < self.len() {
            let region = &self[index];
            #[cfg(verus_only)]
            proof! { super::patch::proof::slot_bits(crate::morton::proof::value(region.path)); }
            let start = (region.path.value() & 63) as usize;
            let count = if region.tree.lod == Some(0) { 1 } else { 8 };
            #[cfg(verus_only)]
            proof_decl! { let ghost previous = values@; }
            let mut position = 0;
            #[cfg_attr(verus_only, verus_spec(
                invariant index < self@.len(), region == &self@[index as int],
                    proof::leaf_region(*region), before.len() == 64, previous.len() == 64,
                    previous == proof::apply(before, self@, index as nat),
                    start as int == proof::start(*region), count == proof::count(region.tree),
                    count <= 8, position <= count,
                    values@ == proof::region_apply(previous, *region, position as nat),
                    !changed ==> values@ == before,
                decreases count - position,
            ))]
            while position < count {
                let Child::Leaf(data) = region.tree.roots[position] else {
                    return Err(Error::BuildFrameMissing);
                };
                changed |= values[start + position] != data;
                values[start + position] = data;
                #[cfg(verus_only)]
                proof! { proof::region_step(previous, *region, position as nat, values@); }
                position += 1;
            }
            index += 1;
        }
        Ok(changed)
    }
}

/// 只存局部编号到目标 Child 的翻译；目标构造引用由外层事务保活。
#[cfg_attr(verus_only, verus_verify)]
pub(super) struct ImportScratch {
    pub(super) maps: [Vec<Child>; DAG_DEPTH as usize],
    pub roots: [Child; 8],
}
impl Default for ImportScratch {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg_attr(verus_only, verus_verify)]
impl ImportScratch {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.empty(), no_unwind))]
    pub(super) const fn new() -> Self {
        Self {
            maps: [
                vec_ops::new(),
                vec_ops::new(),
                vec_ops::new(),
                vec_ops::new(),
                vec_ops::new(),
                vec_ops::new(),
                vec_ops::new(),
                vec_ops::new(),
                vec_ops::new(),
                vec_ops::new(),
            ],
            roots: [
                Child::Leaf(0),
                Child::Leaf(0),
                Child::Leaf(0),
                Child::Leaf(0),
                Child::Leaf(0),
                Child::Leaf(0),
                Child::Leaf(0),
                Child::Leaf(0),
            ],
        }
    }
}
impl ImportScratch {
    pub fn retained_bytes(&self) -> usize {
        self.maps
            .iter()
            .map(|map| map.capacity() * size_of::<Child>())
            .sum()
    }
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(ensures final(self).empty(), no_unwind))]
    pub(crate) fn clear(&mut self) {
        let mut level = 0;
        #[cfg_attr(verus_only, verus_spec(
            invariant level<=10, forall|i:int|0<=i<level ==> #[trigger] self.maps@[i]@.len()==0,
            decreases 10-level,
        ))]
        while level < DAG_DEPTH as usize {
            vec_ops::clear(&mut self.maps[level]);
            level += 1;
        }
        self.roots = simd_ops::array_fill(Child::Leaf(0));
    }
}

#[cfg(test)]
#[path = "built_tests.rs"]
mod tests;
