//! 借用真实资源图的无递归闭包计划。
//!
//! Graph/Adjacency 的适配须精确读取同一不可变数据；验证范围见各适配所属模块。
//! 内核自行检查归属、槽号和完整代次，再去重和展开直接依赖。
//! 只访问可达节点；空邻接节点直接标记，其余每槽至多入队一次。发布闭包作为保留计划种子，不重复展开其边。
//! Graph 定位/邻接须常数读取，Seeds 须单次线性扫描；此前提下计划为 O(V + E_reached + root_scan)，
//! root_scan 包含根源扫描的空槽及非资源项，不只产出的 Key。根直接借用，DFS 栈只保留待展开邻接，
//! 已处理历史只存在于擦除的规范中；不复制全图。

use crate::owner::Key;
use std::collections::TryReserveError;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(not(verus_only))]
use vstd::{
    prelude::{proof, proof_decl},
    view::View,
};

#[path = "proof/graph.rs"]
mod proof;
pub(crate) use proof::{EdgeModel, Model, SeedModel};
#[cfg(verus_only)]
pub(crate) use proof::{NodeModel, model};

/// 借用根序列，直接读取身份，不先复制临时 Key 表。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) trait Seeds: View<V = SeedModel> {
    /// 重置读位置；完整根序列不变，不要求调用前位置有效。
    #[cfg_attr(verus_only, verus_spec(
        ensures final(self)@.keys == old(self)@.keys, final(self)@.position == 0,
    ))]
    fn reset(&mut self);
    /// 读取下一根，耗尽后保持末尾；不改变完整序列。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self)@.position <= old(self)@.keys.len(),
        ensures final(self)@.keys == old(self)@.keys,
            result is Some <==> old(self)@.position < old(self)@.keys.len(),
            result is Some ==> result->Some_0 == old(self)@.keys[old(self)@.position as int]
                && final(self)@.position == old(self)@.position + 1,
            result is None ==> final(self)@.position == old(self)@.position,
    ))]
    fn next(&mut self) -> Option<Key>;
    /// 容量提示可为零，不影响根序列或正确性。
    fn hint(&self) -> usize;
}

/// 已有 Key 切片的零复制根适配。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Keys<'a> {
    keys: &'a [Key],
    position: usize,
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> Keys<'a> {
    /// 从头借用切片；不复制根。
    #[cfg_attr(verus_only, verus_spec(source => ensures source@.keys == keys@, source@.position == 0))]
    pub(crate) const fn new(keys: &'a [Key]) -> Self {
        Self { keys, position: 0 }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl Seeds for Keys<'_> {
    fn reset(&mut self) {
        self.position = 0;
    }
    fn next(&mut self) -> Option<Key> {
        if self.position >= self.keys.len() {
            return None;
        }
        let key = self.keys[self.position];
        self.position += 1;
        Some(key)
    }
    fn hint(&self) -> usize {
        self.keys.len()
    }
}

/// 借用一个现存版本的直接依赖；None 表示不携带资源的项，不过滤后再按序号查找。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) trait Adjacency: View<V = EdgeModel> {
    /// 可按常数时间索引的依赖项数。
    #[cfg_attr(verus_only, verus_spec(result => ensures result == self@.len()))]
    fn len(&self) -> usize;
    /// 直接读取一项；调用者先检查 index < len。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires index < self@.len(),
        ensures result == self@[index as int],
    ))]
    fn edge(&self, index: usize) -> Option<Key>;
}

/// 同一资源 owner 的只读图；Node 可直接借用 Graph 所持 Arena 引用的有效期。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) trait Graph: View<V = Model> {
    /// 不分配、不复制资源的邻接视图。
    type Node: Adjacency;
    /// 资源 owner。
    #[cfg_attr(verus_only, verus_spec(result => ensures result == self@.owner))]
    fn owner(&self) -> u64;
    /// 包含空槽的槽数；u32 槽域来自 Arena 的已有约束。
    #[cfg_attr(verus_only, verus_spec(result => ensures
        result == self@.nodes.len(), model::valid(self@),
    ))]
    fn len(&self) -> usize;
    /// 读取当前槽；返回身份由 owner、槽号和当前代次组成。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires slot < self@.nodes.len(),
        ensures
            result is Some <==> self@.nodes[slot as int] is Some,
            result is Some ==> result->Some_0.0 == model::key(self@, slot as int)
                && result->Some_0.1@ == self@.nodes[slot as int]->Some_0.edges,
    ))]
    fn node(&self, slot: usize) -> Option<(Key, Self::Node)>;

    /// 读取已由标记不变量确认存在的槽位身份；适配可仅访问代次。
    #[cfg_attr(verus_only, verus_spec(key =>
        requires slot < self@.nodes.len(), self@.nodes[slot as int] is Some,
        ensures key == model::key(self@, slot as int),
    ))]
    fn present_key(&self, slot: usize) -> Key {
        self.node(slot).expect("已标记槽位保持存在").0
    }
}

/// 完整闭包成功后才供所有者提交的发布与保留掩码。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Plan {
    pub(crate) published: Vec<bool>,
    pub(crate) keep: Vec<bool>,
}

/// 输入或局部分配失败；不更新资源、记录或退役状态。
#[cfg_attr(verus_only, verus_verify)]
#[derive(Debug)]
pub(crate) enum PlanError {
    /// 根或依赖属于其他 owner。
    ForeignOwner,
    /// 完整 Key 不匹配现存槽。
    InvalidKey,
    /// 邻接适配返回了不对应当前槽的身份。
    InvalidShape,
    /// 保留标准库分配错误来源。
    Allocation(TryReserveError),
}

/// 检查当前完整身份；退役计划也使用同一入口。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => ensures result == model::has_key(graph@, key)))]
pub(crate) fn current<G: Graph>(graph: &G, key: Key) -> bool {
    if key.owner != graph.owner() || key.slot as usize >= graph.len() {
        return false;
    }
    match graph.node(key.slot as usize) {
        Some((found, _)) => found.same(key),
        None => false,
    }
}

/// 重置读器，计算完整发布根和全部根的精确最小闭包。
/// 失败只丢弃本地计划；读位置可推进，原始根序列及资源所有者保持不变。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result is Ok ==> model::valid(graph@) && result->Ok_0.valid(graph)
        && result->Ok_0.matches(graph, old(roots)@.keys, old(record_roots)@.keys),
        final(roots)@.keys == old(roots)@.keys,
        final(record_roots)@.keys == old(record_roots)@.keys,
))]
pub(crate) fn plan<G: Graph, S: Seeds, R: Seeds>(
    graph: &G,
    roots: &mut S,
    record_roots: &mut R,
) -> Result<Plan, PlanError> {
    roots.reset();
    record_roots.reset();
    let mut empty = Keys::new(&[]);
    let published = closure(graph, roots, &mut empty, None)?;
    let keep = closure(graph, roots, record_roots, Some(&published))?;
    let result = Plan { published, keep };
    proof! { proof::plan_valid(graph@, roots@.keys, record_roots@.keys, &result); }
    Ok(result)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => requires model::valid(graph@),
    ensures result is Ok ==> model::has_key(graph@, key)
        && result->Ok_0.0 == key.slot && result->Ok_0.1@ == graph@.nodes[key.slot as int]->Some_0.edges,
))]
fn resolve<G: Graph>(graph: &G, key: Key) -> Result<(usize, G::Node), PlanError> {
    if key.owner != graph.owner() {
        return Err(PlanError::ForeignOwner);
    }
    let index = key.slot as usize;
    if index >= graph.len() {
        return Err(PlanError::InvalidKey);
    }
    match graph.node(index) {
        Some((found, node)) => {
            if found.owner != graph.owner() || found.slot as usize != index {
                return Err(PlanError::InvalidShape);
            }
            if !found.same(key) {
                return Err(PlanError::InvalidKey);
            }
            Ok((index, node))
        }
        None => Err(PlanError::InvalidKey),
    }
}

/// 遍历状态独占局部掩码和工作表；证明上下文在普通构建中完全移除。
#[cfg_attr(verus_only, verus_verify)]
struct Visit<N> {
    bits: Vec<bool>,
    work: Vec<N>,
    #[cfg(verus_only)]
    context: Ghost<proof::Context>,
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::visit(graph@, *old(visit)),
        model::reachable(graph@, old(visit).context@.roots, old(visit).context@.extra, key),
    ensures proof::visit(graph@, *final(visit)),
        proof::visit_preserved(*old(visit), *final(visit)),
        result is Ok ==> model::marked(graph@, final(visit).bits@, key),
))]
#[inline(always)]
fn enqueue<G: Graph>(
    graph: &G,
    key: Key,
    reserve_hint: usize,
    visit: &mut Visit<G::Node>,
) -> Result<(), PlanError> {
    if key.owner != graph.owner() {
        return Err(PlanError::ForeignOwner);
    }
    let index = key.slot as usize;
    if index >= graph.len() {
        return Err(PlanError::InvalidKey);
    }
    if visit.bits[index] {
        let found = graph.present_key(index);
        if found.owner != graph.owner() || found.slot as usize != index {
            return Err(PlanError::InvalidShape);
        }
        if !found.same(key) {
            return Err(PlanError::InvalidKey);
        }
        return Ok(());
    }
    let (index, node) = resolve(graph, key)?;
    if !visit.bits[index] {
        proof_decl! { let ghost before = *visit; }
        if node.len() != 0 {
            // 工作表只保存待展开节点；空栈也可复用先前容量。
            let additional = if visit.work.is_empty() {
                reserve_hint.max(1).min(visit.bits.len())
            } else {
                1
            };
            if let Err(error) = visit.work.try_reserve(additional) {
                return Err(PlanError::Allocation(error));
            }
            visit.work.push(node);
        }
        visit.bits[index] = true;
        proof! {
            *visit.context.borrow_mut() = proof::enqueue_context(graph@, before.context@, index);
            proof::enqueued(graph@, before, visit, key);
        }
    }
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::closure_input(graph@, old(roots)@, old(extra)@, seed),
    ensures model::valid(graph@),
        proof::sources_preserved(old(roots)@, final(roots)@, old(extra)@, final(extra)@),
        result is Ok ==> model::matches(graph@, result->Ok_0@, old(roots)@.keys, old(extra)@.keys),
))]
fn closure<G: Graph, S: Seeds, R: Seeds>(
    graph: &G,
    roots: &mut S,
    extra: &mut R,
    seed: Option<&[bool]>,
) -> Result<Vec<bool>, PlanError> {
    let count = graph.len();
    let bits = initial_mask(count, seed)?;
    proof_decl! { let ghost context = proof::Context { roots: roots@.keys, extra: extra@.keys, base: bits@,
    history: Seq::empty(), pending: Seq::empty(), done: 0, current: None }; }
    proof! { proof::initial_state(graph@, context, seed, bits@); }
    let mut visit = Visit {
        bits,
        work: Vec::new(),
        #[cfg(verus_only)]
        context: verus_exec_expr!(Ghost(context)),
    };
    if seed.is_none() {
        seed_keys(graph, roots, &mut visit)?;
    }
    seed_keys(graph, extra, &mut visit)?;
    expand(graph, &mut visit)?;
    Ok(visit.bits)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires seed is Some ==> seed->Some_0@.len() == count,
    ensures result is Ok ==> proof::mask_matches(count, seed, result->Ok_0@),
))]
fn initial_mask(count: usize, seed: Option<&[bool]>) -> Result<Vec<bool>, PlanError> {
    let mut bits = Vec::new();
    if let Err(error) = bits.try_reserve(count) {
        return Err(PlanError::Allocation(error));
    }
    match seed {
        Some(base) => bits.extend_from_slice(base),
        None => bits.resize(count, false),
    }
    Ok(bits)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::visit(graph@, *old(visit)),
        old(keys)@.position == 0, proof::keys_reachable(graph@, old(visit).context@, old(keys)@.keys),
    ensures proof::visit(graph@, *final(visit)),
        proof::visit_preserved(*old(visit), *final(visit)),
        final(keys)@.keys == old(keys)@.keys, final(keys)@.position <= final(keys)@.keys.len(),
        result is Ok ==> proof::keys_marked(graph@, final(visit).bits@, old(keys)@.keys),
))]
fn seed_keys<G: Graph, S: Seeds>(
    graph: &G,
    keys: &mut S,
    visit: &mut Visit<G::Node>,
) -> Result<(), PlanError> {
    let reserve_hint = keys.hint().min(visit.bits.len());
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::seed_cursor_loop(graph@, old(keys)@, keys@, *old(visit), *visit),
        decreases keys@.keys.len() - keys@.position,
    ))]
    loop {
        let Some(key) = keys.next() else {
            return Ok(());
        };
        enqueue(graph, key, reserve_hint, visit)?;
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::walk_visit(graph@, *old(visit)),
    ensures result is Ok ==> model::matches(graph@, final(visit).bits@,
        old(visit).context@.roots, old(visit).context@.extra),
))]
fn expand<G: Graph>(graph: &G, visit: &mut Visit<G::Node>) -> Result<(), PlanError> {
    proof_decl! { let ghost initial_context = visit.context@; }
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::walk_iteration(graph@, initial_context, *visit),
        ensures visit.work@.len() == 0,
        decreases graph@.nodes.len() - visit.context@.done,
    ))]
    while let Some(node) = pop(visit) {
        expand_node(graph, &node, visit)?;
    }
    proof! { proof::closure_complete(graph@, *visit); }
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(entry =>
    ensures proof::stack_popped(*old(visit), *final(visit), entry),
))]
fn pop<N: Adjacency>(visit: &mut Visit<N>) -> Option<N> {
    proof_decl! { let ghost before = *visit; }
    let entry = visit.work.pop();
    proof! {
        *visit.context.borrow_mut() = proof::pop_context(before.context@, entry is Some);
        proof::stack_popped_for_all(before, visit, entry);
    }
    entry
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::edges_visit(graph@, *old(visit), node@, 0, node@.len() as int),
    ensures result is Ok ==> proof::walk_visit(graph@, *final(visit))
        && proof::scope(old(visit).context@, final(visit).context@)
        && final(visit).context@.done == old(visit).context@.done + 1,
))]
fn expand_node<G: Graph>(
    graph: &G,
    node: &G::Node,
    visit: &mut Visit<G::Node>,
) -> Result<(), PlanError> {
    proof_decl! { let ghost initial = *visit; }
    let edges = node.len();
    let mut e = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::edge_iteration(graph@, initial, *visit, node@, e as int, edges as int),
        decreases edges - e,
    ))]
    while e < edges {
        if let Some(key) = node.edge(e) {
            proof! {
                proof::visit_edge_reachable(graph@, *visit, node@, e as int);
            }
            enqueue(graph, key, 1, visit)?;
        }
        e += 1;
    }
    proof! {
        let before = *visit;
        let context = visit.context@;
        *visit.context.borrow_mut() = proof::Context { done: context.done + 1, current: None, ..context };
        proof::node_done(graph@, before, visit, node@);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../verify/graph_fixture.rs"]
mod fixture;

#[cfg(test)]
mod tests {
    use super::fixture::{Fixture, Stored};
    use super::*;

    const fn key(slot: u32, generation: u32) -> Key {
        Key {
            owner: 7,
            slot,
            generation,
        }
    }

    fn nodes(edges: &[Key], ranges: &[(usize, usize)]) -> [Option<Stored>; 4] {
        std::array::from_fn(|slot| {
            ranges.get(slot).map(|&(start, end)| Stored {
                generation: 0,
                edges: edges[start..end].iter().copied().map(Some).collect(),
            })
        })
    }

    fn plan_for(
        nodes: &[Option<Stored>; 4],
        roots: &[Key],
        records: &[Key],
    ) -> Result<Plan, PlanError> {
        plan(
            &Fixture { nodes },
            &mut Keys::new(roots),
            &mut Keys::new(records),
        )
    }

    #[test]
    fn cycle_closure_excludes_disconnected_node() {
        let nodes = nodes(&[key(1, 0), key(0, 0)], &[(0, 1), (1, 2), (2, 2)]);
        let plan = plan_for(&nodes, &[key(0, 0)], &[]).unwrap();
        assert_eq!(plan.published, [true, true, false, false]);
        assert_eq!(plan.keep, plan.published);
    }

    #[test]
    fn stale_generation_is_rejected() {
        let nodes = nodes(&[], &[(0, 0)]);
        assert!(matches!(
            plan_for(&nodes, &[key(0, 1)], &[]),
            Err(PlanError::InvalidKey)
        ));
    }

    #[test]
    fn marked_slot_rejects_old_generation() {
        let mut nodes = nodes(&[], &[(0, 0)]);
        nodes[0].as_mut().unwrap().generation = 1;
        assert!(matches!(
            plan_for(&nodes, &[key(0, 1), key(0, 0)], &[]),
            Err(PlanError::InvalidKey)
        ));
    }

    #[test]
    fn unreachable_invalid_edge_does_not_reject_plan() {
        let nodes = nodes(&[key(0, 1)], &[(0, 0), (0, 1)]);
        let plan = plan_for(&nodes, &[key(0, 0)], &[]).unwrap();
        assert_eq!(plan.keep, [true, false, false, false]);
    }

    #[test]
    fn record_roots_extend_published_closure() {
        let nodes = nodes(
            &[key(1, 0), key(1, 0), key(3, 0)],
            &[(0, 1), (1, 1), (1, 3), (3, 3)],
        );
        let plan = plan_for(&nodes, &[key(0, 0)], &[key(2, 0)]).unwrap();
        assert_eq!(plan.published, [true, true, false, false]);
        assert_eq!(plan.keep, [true; 4]);
    }

    #[test]
    fn planning_restarts_consumed_root_sources() {
        let nodes = nodes(&[], &[(0, 0), (0, 0)]);
        let first = [key(0, 0)];
        let second = [key(1, 0)];
        let mut roots = Keys::new(&first);
        let mut records = Keys::new(&second);
        roots.next();
        records.next();
        let plan = plan(&Fixture { nodes: &nodes }, &mut roots, &mut records).unwrap();
        assert_eq!(plan.published, [true, false, false, false]);
        assert_eq!(plan.keep, [true, true, false, false]);
    }
}
