//! 根可达物理节点的精确集合差；显式栈 DFS，物理槽位图去重。

#[cfg(not(verus_only))]
use crate::diff_ops;
use crate::node::NodeRef;
use crate::{Child, Error, HashDag, NodeIdx, RootId};
#[cfg(verus_only)]
use hash_dag_std::diff_ops;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(not(verus_only))]
use vstd::view::View;

#[path = "proof/diff.rs"]
pub(crate) mod proof;
pub(crate) use proof::GraphModel;

#[cfg_attr(verus_only, verus_verify)]
pub(crate) const DIFF_DEPTH: usize = 10;

/// 借用同一 DAG 的真实根、物理槽和节点；适配不复制节点或分配。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) trait DiffGraph: View<V = GraphModel> {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::model::valid(self@),
        ensures result is Ok <==> self@.roots.lookup(root) is Some,
            result is Ok ==> result->Ok_0 == self@.roots.lookup(root)->Some_0,
    ))]
    fn root_child(&self, root: RootId) -> Result<Child, Error>;

    #[cfg_attr(verus_only, verus_spec(result => requires proof::model::valid(self@), level < 10,
        ensures result == self@.nodes[level as int].len(),
    ))]
    fn slots(&self, level: u8) -> usize;

    #[cfg_attr(verus_only, verus_spec(result => requires proof::model::valid(self@), level < 10,
        ensures result is Some <==> proof::model::has_node(self@, NodeAddress { level, index }),
            result is Some ==> crate::node::proof::ref_children(result->Some_0)
                == self@.nodes[level as int][crate::node::proof::idx(index)]->Some_0,
    ))]
    fn node(&self, level: u8, index: NodeIdx) -> Option<NodeRef<'_>>;
}

/// 原生节点的物理地址；只在所属 DAG 和保活根的生命周期内有效。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(verus_only, verus_verify)]
pub struct NodeAddress {
    /// 原生池层级 `0..HashDag::DEPTH`，不是空间 LOD。
    pub level: u8,
    /// 层内物理槽；节点释放后该位置可复用。
    pub index: NodeIdx,
}

#[cfg_attr(verus_only, verus_verify)]
impl NodeAddress {
    #[cfg_attr(verus_only, verus_spec(key => ensures key == (self.level, self.index.0), no_unwind))]
    const fn sort_key(&self) -> (u8, u32) {
        (self.level, self.index.0)
    }
}

/// 消费者持有的差分输出与临时缓冲，不保存 DAG 历史或节点引用。
///
/// 同层节点已由 DAG 唯一化，按物理槽标记即可判定是否访问过；每次遍历前重置位图，
/// 不缓存跨调用的节点身份。两个根由调用方保活，复制 [`RootId`] 不能代替
/// [`HashDag::fork_root`]。遍历不复制节点 payload，也不修改 DAG 的结构引用。
#[cfg_attr(verus_only, verus_verify)]
pub struct NodeDiff {
    pub(crate) nodes: Vec<NodeAddress>,
    pub(crate) stack: Vec<NodeAddress>,
    pub(crate) seen: [Vec<u64>; DIFF_DEPTH],
    #[cfg(verus_only)]
    pub(crate) context: Ghost<proof::Context>,
}

#[cfg_attr(verus_only, verus_verify)]
impl NodeDiff {
    /// 创建未分配缓冲的差分器。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(state => ensures state.ready(), state.output().len() == 0))]
    pub const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            stack: Vec::new(),
            seen: [
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            #[cfg(verus_only)]
            context: verus_exec_expr!(Ghost(proof::empty_context())),
        }
    }

    /// 最近一次成功差分的排序结果；初始或失败后为空。
    ///
    /// 借用和节点地址的有效期同 [`Self::between`]；读取不会重复遍历或分配。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures result@ == self.output()))]
    pub const fn nodes(&self) -> &[NodeAddress] {
        self.nodes.as_slice()
    }

    /// 精确返回 `Reach(after) \ Reach(before)`，按层级、槽编号排序且不重复。
    ///
    /// `Reach` 按 `(level, index)` 计唯一物理节点；旧根在任何空间位置已引用的节点
    /// 均不输出，新叶也不输出。调用方保存旧根完整物理表示后，只需上传这些节点并独立
    /// 更新 root 描述；叶根变化即使差集为空也必须更新描述。
    ///
    /// 先 DFS 标记旧根的全部可达节点，再遍历新根；命中旧集合即跳过整个子图。
    /// 每个旧节点和新增节点最多展开一次。设物理槽总数为 `p`、旧根唯一节点数为 `n`、
    /// 输出数为 `k`，成本为 `O(p/64 + 64 × (n+k) + k log k)`；即使连续编辑也重建旧集合。
    /// 根内容相等或新根为叶时直接返回空集。这里只计算根差集，
    /// 不管理设备驻留历史、容量、在途访问或提交顺序。
    ///
    /// 输出借用到下一次修改差分器为止，节点地址还依赖原 DAG 的有效根；
    /// 调用后若释放根或编辑使节点失活，不得再用旧输出读取节点。
    ///
    /// # Errors
    ///
    /// 两个 root 都验证所属实例及存活状态；跨实例、失效句柄、遍历发现的内部节点缺失或
    /// 容量耗尽时返回错误。保留底层分配来源，失败清空输出，不修改 DAG。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires dag.valid(), old(self).ready(),
        ensures final(self).ready(), result is Err ==> final(self).output().len() == 0,
            result is Ok ==> result->Ok_0@ == final(self).output()
                && proof::exact(dag@, before, after, result->Ok_0@) && proof::sorted(result->Ok_0@),
    ))]
    pub fn between(
        &mut self,
        dag: &HashDag,
        before: RootId,
        after: RootId,
    ) -> Result<&[NodeAddress], Error> {
        #[cfg(verus_only)]
        proof! { crate::hashdag::view_proof::valid(dag); }
        self.between_graph(dag, before, after)
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::model::valid(dag@), old(self).ready(),
        ensures final(self).ready(),
            result is Err ==> final(self).nodes@.len() == 0,
            result is Ok ==> result->Ok_0@ == final(self).nodes@
                && proof::exact(dag@, before, after, result->Ok_0@)
                && proof::sorted(result->Ok_0@),
    ))]
    pub(crate) fn between_graph<G: DiffGraph>(
        &mut self,
        dag: &G,
        before: RootId,
        after: RootId,
    ) -> Result<&[NodeAddress], Error> {
        let result = self.collect(dag, before, after);
        self.stack.clear();
        if let Err(error) = result {
            self.nodes.clear();
            return Err(error);
        }
        Ok(&self.nodes)
    }
}

impl NodeDiff {
    /// 输出、DFS 工作栈及槽位图保留的堆后备字节。
    ///
    /// 位图按各层物理槽数分配，含空闲槽；工作栈按深度有界，最低层不入栈。
    /// 输出及缓冲容量保留历史峰值。
    /// 不含自身、allocator 元数据或 DAG 的根与节点，每个并发消费者单独计费。
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<NodeAddress>()
            + self.stack.capacity() * size_of::<NodeAddress>()
            + self
                .seen
                .iter()
                .map(|bits| bits.capacity() * size_of::<u64>())
                .sum::<usize>()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl NodeDiff {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::model::valid(dag@), old(self).ready(),
        ensures result is Ok ==> final(self).ready() && proof::exact(dag@, before, after, final(self).nodes@)
            && proof::sorted(final(self).nodes@),
    ))]
    fn collect<G: DiffGraph>(
        &mut self,
        dag: &G,
        before: RootId,
        after: RootId,
    ) -> Result<(), Error> {
        self.nodes.clear();
        #[cfg(verus_only)]
        proof_decl! { let ghost before_id = before; let ghost after_id = after; }
        let before = dag.root_child(before)?;
        let after = dag.root_child(after)?;
        if before.same(after) || matches!(after, Child::Leaf(_)) {
            #[cfg(verus_only)]
            proof! { proof::early_empty(dag@, before_id, after_id); }
            return Ok(());
        }
        let mut level = 0usize;
        #[cfg_attr(verus_only, verus_spec(
            invariant level <= 10, proof::model::valid(dag@), self.ready(), self.nodes@.len() == 0,
                dag@.roots.lookup(before_id) == Some(before), dag@.roots.lookup(after_id) == Some(after),
                forall|k: int| 0 <= k < level ==> self.seen@[k]@.len() == (dag@.nodes[k].len() + 63) / 64
                    && forall|word: int| 0 <= word < self.seen@[k]@.len() ==> self.seen@[k]@[word] == 0,
            decreases 10 - level,
        ))]
        while level < DIFF_DEPTH {
            let words = dag.slots(level as u8).div_ceil(64);
            let bits = &mut self.seen[level];
            if words > bits.len() {
                bits.try_reserve(words - bits.len())?;
            }
            bits.resize(words, 0);
            diff_ops::clear(bits.as_mut_slice());
            level += 1;
        }
        #[cfg(verus_only)]
        proof! { proof::empty_bits(dag@, *self); }
        self.traverse(dag, before, false)?;
        #[cfg(verus_only)]
        proof! { proof::traversal_closed(dag@, before, *self); }
        self.traverse(dag, after, true)?;
        #[cfg(verus_only)]
        proof_decl! { let ghost unsorted = self.nodes@; }
        diff_ops::sort(self.nodes.as_mut_slice(), NodeAddress::sort_key);
        #[cfg(verus_only)]
        proof! { proof::sorted_result(unsorted, self.nodes@); }
        Ok(())
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::model::valid(dag@), proof::model::root_valid(dag@, root),
            old(self).ready(), old(self).nodes@.len() == 0, proof::bit_shape(dag@, *old(self)),
            proof::model::closed(dag@, proof::marked_set(dag@, *old(self))),
            forall|a: NodeAddress| proof::marked(dag@, *old(self), a) ==> proof::model::has_node(dag@, a),
        ensures result is Ok ==> final(self).ready() && proof::bit_shape(dag@, *final(self))
            && (forall|a: NodeAddress| proof::marked(dag@, *final(self), a) <==>
                proof::marked(dag@, *old(self), a) || proof::model::reachable(dag@, root, a))
            && if output { final(self).nodes@.no_duplicates() && forall|a: NodeAddress| final(self).nodes@.contains(a) <==>
                proof::model::reachable(dag@, root, a) && !proof::marked(dag@, *old(self), a)
            } else { final(self).nodes@.len() == 0 },
    ))]
    fn traverse<G: DiffGraph>(&mut self, dag: &G, root: Child, output: bool) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost base = proof::marked_set(dag@, *self); }
        #[cfg(verus_only)]
        proof! {
            *self.context.borrow_mut() = proof::Context { base, root,
                history: Seq::empty(), current: None, done: 0, output };
            assert(proof::marked_set(dag@, *self) =~= base);
            proof::phase_initial(dag@, *self);
        }
        if let Child::Ptr(index) = root {
            #[cfg(verus_only)]
            proof! { proof::root_reachable(dag@, root); }
            self.enqueue(dag, 9, index, output)?;
        }
        #[cfg_attr(verus_only, verus_spec(
            invariant proof::walk(dag@, *self), self.context@.root == root, self.context@.output == output,
                self.context@.base == base,
            ensures self.stack@.len() == 0,
            decreases 10 * (u32::MAX as int + 1) - self.context@.done,
        ))]
        while let Some(address) = self.pop(dag) {
            self.expand(dag, address, output)?;
        }
        #[cfg(verus_only)]
        proof! { proof::traversal_result(dag@, *self); }
        Ok(())
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::walk(_dag@, *old(self)),
        ensures result is Some ==> proof::visit(_dag@, *final(self))
            && final(self).context@.current == Some(result->Some_0)
            && result->Some_0.level > 0 && proof::model::has_node(_dag@, result->Some_0)
            && proof::model::reachable(_dag@, final(self).context@.root, result->Some_0),
            result is None ==> proof::walk(_dag@, *final(self)) && final(self).ready(),
            final(self).context@.done == old(self).context@.done,
            proof::scope(old(self).context@, final(self).context@),
            proof::model::seeded(final(self).context@.root, proof::marked_set(_dag@, *final(self))),
    ))]
    #[inline(always)]
    fn pop<G: DiffGraph>(&mut self, _dag: &G) -> Option<NodeAddress> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        let address = self.stack.pop();
        #[cfg(verus_only)]
        proof! {
            if address is Some {
                let context = self.context@;
                *self.context.borrow_mut() = proof::Context { current: address, ..context };
                proof::popped(_dag@, before, *self, address->Some_0);
            }
        }
        address
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::visit(dag@, *old(self)), old(self).context@.current == Some(address),
            old(self).context@.output == output, address.level > 0,
            proof::model::has_node(dag@, address),
            proof::model::seeded(old(self).context@.root, proof::marked_set(dag@, *old(self))),
        ensures result is Ok ==> proof::walk(dag@, *final(self))
            && final(self).context@.done > old(self).context@.done
            && proof::scope(old(self).context@, final(self).context@),
    ))]
    #[inline(always)]
    fn expand<G: DiffGraph>(
        &mut self,
        dag: &G,
        address: NodeAddress,
        output: bool,
    ) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost initial = self.context@; }
        let NodeAddress { level, index } = address;
        let node = dag.node(level, index).ok_or(Error::NodeMissing(index))?;
        let mut bits = node.mask.bits();
        #[cfg(verus_only)]
        proof! { proof::start_edges(dag@, *self, node, address); }
        #[cfg_attr(verus_only, verus_spec(
            invariant proof::edges(dag@, *self, node, bits), self.context@.output == output,
                self.context@.current == Some(address), proof::scope(initial, self.context@), initial.done <= self.context@.done,
                address.level == level, level > 0, level < 10,
                proof::model::seeded(self.context@.root, proof::marked_set(dag@, *self)),
            decreases bits,
        ))]
        while bits != 0 {
            #[cfg(verus_only)]
            proof_decl! { let ghost before = *self; let ghost previous = bits; }
            let slot = bits.trailing_zeros() as u8;
            bits &= bits - 1;
            #[cfg(verus_only)]
            proof! { proof::lowest_cleared(previous, bits, slot); }
            if let Child::Ptr(child) = node.child(slot) {
                #[cfg(verus_only)]
                proof! {
                    assert(crate::node::proof::ref_children(node)[slot as int] == Child::Ptr(child));
                    proof::edge_reachable(dag@, self.context@.root, address, NodeAddress { level: (level - 1) as u8, index: child });
                }
                self.enqueue(dag, level - 1, child, output)?;
            }
            #[cfg(verus_only)]
            proof! { proof::edge_done(dag@, before, *self, node, previous, bits, slot); }
        }
        #[cfg(verus_only)]
        proof! {
            let before = *self;
            let context = self.context@;
            *self.context.borrow_mut() = proof::Context { current: None, done: context.done + 1, ..context };
            proof::node_done(dag@, before, *self, node);
        }
        Ok(())
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::visit(dag@, *old(self)), old(self).context@.output == output,
            proof::model::has_node(dag@, NodeAddress { level, index }),
            proof::model::reachable(dag@, old(self).context@.root, NodeAddress { level, index }),
        ensures result is Ok ==> proof::visit(dag@, *final(self))
            && proof::preserved(dag@, *old(self), *final(self))
            && proof::marked(dag@, *final(self), NodeAddress { level, index }),
    ))]
    #[inline(always)]
    fn enqueue<G: DiffGraph>(
        &mut self,
        dag: &G,
        level: u8,
        index: NodeIdx,
        output: bool,
    ) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        if self.mark(dag, level, index)? {
            if level == 0 {
                dag.node(0, index).ok_or(Error::NodeMissing(index))?;
            } else {
                self.stack.try_reserve(1)?;
                self.stack.push(NodeAddress { level, index });
            }
            if output {
                self.nodes.try_reserve(1)?;
                self.nodes.push(NodeAddress { level, index });
            }
            #[cfg(verus_only)]
            proof! {
                let context = self.context@;
                *self.context.borrow_mut() = proof::Context {
                    history: context.history.push(NodeAddress { level, index }),
                    done: context.done + if level == 0 { 1nat } else { 0nat }, ..context };
                proof::visited_new(dag@, before, *self, NodeAddress { level, index });
            }
        }
        Ok(())
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::model::valid(_dag@), proof::bit_shape(_dag@, *old(self)),
            proof::model::has_node(_dag@, NodeAddress { level, index }),
        ensures result is Ok && result->Ok_0 == !proof::marked(_dag@, *old(self), NodeAddress { level, index }),
            proof::bit_shape(_dag@, *final(self)),
            final(self).stack@ == old(self).stack@, final(self).nodes@ == old(self).nodes@,
            final(self).context@ == old(self).context@,
            forall|a: NodeAddress| proof::marked(_dag@, *final(self), a) <==>
                proof::marked(_dag@, *old(self), a) || a == (NodeAddress { level, index }),
    ))]
    #[inline(always)]
    fn mark<G: DiffGraph>(&mut self, _dag: &G, level: u8, index: NodeIdx) -> Result<bool, Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        let position = index.as_usize()?;
        let word = self.seen[usize::from(level)]
            .get_mut(position / 64)
            .ok_or(Error::NodeMissing(index))?;
        #[cfg(verus_only)]
        proof_decl! { let ghost previous_word = *word; }
        let bit = 1_u64 << (position % 64);
        let fresh = *word & bit == 0;
        if fresh {
            *word |= bit;
        }
        #[cfg(verus_only)]
        proof! { proof::already_marked(previous_word, (position % 64) as u64); }
        #[cfg(verus_only)]
        proof! { proof::marked_one(_dag@, before, *self, NodeAddress { level, index }); }
        Ok(fresh)
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl Default for NodeDiff {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.ready(), result.output().len() == 0))]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
