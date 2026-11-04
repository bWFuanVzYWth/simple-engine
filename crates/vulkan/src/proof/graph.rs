use super::*;
use vstd::prelude::verus;

/// 普通编译只保留 trait 的擦除关联类型。
#[cfg(not(verus_only))]
pub(crate) type Model = ();
/// 普通构建的邻接关联类型不携带规范序列。
#[cfg(not(verus_only))]
pub(crate) type EdgeModel = ();
#[cfg(verus_only)]
pub(crate) type EdgeModel = Seq<Option<Key>>;
/// 普通根源关联类型不携带规范数据。
#[cfg(not(verus_only))]
pub(crate) type SeedModel = ();
#[cfg(verus_only)]
verus! {
pub(crate) struct SeedModel { pub(crate) keys: Seq<Key>, pub(crate) position: nat }
}

verus! {

impl View for Keys<'_> {
    type V = SeedModel;
    open(super) spec fn view(&self) -> SeedModel { SeedModel { keys: self.keys@, position: self.position as nat } }
}

#[cfg(verus_only)]
pub(crate) struct NodeModel { pub(crate) generation: u32, pub(crate) edges: Seq<Option<Key>> }
#[cfg(verus_only)]
pub(crate) struct Model { pub(crate) owner: u64, pub(crate) nodes: Seq<Option<NodeModel>> }
#[cfg(verus_only)]
pub(super) struct Context {
    pub(super) roots: Seq<Key>, pub(super) extra: Seq<Key>, pub(super) base: Seq<bool>,
    // history 只记录实际入队的非叶节点；叶节点直接标记，不改变遍历分解。
    pub(super) history: Seq<usize>, pub(super) pending: Seq<usize>,
    pub(super) done: nat, pub(super) current: Option<usize>,
}

#[cfg(verus_only)]
pub(crate) mod model {
    use super::*;
    pub(crate) open spec fn valid(graph: Model) -> bool { graph.nodes.len() <= u32::MAX as int + 1 }
    pub(crate) open spec fn key(graph: Model, slot: int) -> Key {
        Key { owner: graph.owner, slot: slot as u32, generation: graph.nodes[slot]->Some_0.generation }
    }
    pub(crate) open spec fn has_key(graph: Model, id: Key) -> bool {
        id.owner == graph.owner && (id.slot as int) < graph.nodes.len()
            && graph.nodes[id.slot as int] is Some && key(graph, id.slot as int) == id
    }
    pub(crate) open spec fn dependency(graph: Model, from: Key, to: Key) -> bool {
        has_key(graph, from) && graph.nodes[from.slot as int]->Some_0.edges.contains(Some(to))
    }
    pub(crate) open spec fn closed(graph: Model, keys: ISet<Key>) -> bool {
        forall|from: Key, to: Key| keys.contains(from) && #[trigger] dependency(graph, from, to) ==> keys.contains(to)
    }
    pub(crate) open spec fn seeded(roots: Seq<Key>, extra: Seq<Key>, keys: ISet<Key>) -> bool {
        (forall|i: int| #![auto] 0 <= i < roots.len() ==> keys.contains(roots[i]))
            && (forall|i: int| #![auto] 0 <= i < extra.len() ==> keys.contains(extra[i]))
    }
    pub(crate) open spec fn reachable(graph: Model, roots: Seq<Key>, extra: Seq<Key>, id: Key) -> bool {
        forall|keys: ISet<Key>| seeded(roots, extra, keys) && closed(graph, keys) ==> #[trigger] keys.contains(id)
    }
    pub(crate) open spec fn marked(graph: Model, bits: Seq<bool>, id: Key) -> bool {
        has_key(graph, id) && (id.slot as int) < bits.len() && bits[id.slot as int]
    }
    pub(crate) open spec fn matches(graph: Model, bits: Seq<bool>, roots: Seq<Key>, extra: Seq<Key>) -> bool {
        bits.len() == graph.nodes.len()
            && (forall|i: int| #![auto] 0 <= i < bits.len() && bits[i] ==> graph.nodes[i] is Some)
            && (forall|id: Key| #![auto] marked(graph, bits, id) <==> reachable(graph, roots, extra, id))
    }
}

impl Plan {
    /// 同槽数、发布必保留且不会标记空槽。
    pub(crate) open spec fn valid<G: Graph>(&self, graph: &G) -> bool {
        self.published@.len() == graph@.nodes.len() && self.keep@.len() == graph@.nodes.len()
            && (forall|i: int| #![auto] 0 <= i < graph@.nodes.len() ==>
                (self.published@[i] ==> self.keep@[i]) && (self.keep@[i] ==> graph@.nodes[i] is Some))
    }
    /// 精确最小闭包：published 从发布根开始，keep 另加全部录制根。
    pub(crate) open spec fn matches<G: Graph>(&self, graph: &G, roots: Seq<Key>, extra: Seq<Key>) -> bool {
        model::matches(graph@, self.published@, roots, Seq::empty()) && model::matches(graph@, self.keep@, roots, extra)
    }
}

broadcast use vstd::seq_lib::group_seq_properties;

pub(super) open spec fn mask_matches(count: usize, seed: Option<&[bool]>, bits: Seq<bool>) -> bool {
    bits.len() == count && (seed is Some ==> seed->Some_0@.len() == count)
        && (forall|j: int| #![auto] 0 <= j < bits.len() ==> bits[j] ==
            if seed is Some { seed->Some_0@[j] } else { false })
}
pub(super) open spec fn scope(before: Context, after: Context) -> bool {
    before.roots == after.roots && before.extra == after.extra && before.base == after.base
}
pub(super) open spec fn prefix<A>(before: Seq<A>, after: Seq<A>) -> bool {
    before.len() <= after.len() && forall|i: int| #![auto] 0 <= i < before.len() ==> before[i] == after[i]
}
pub(super) open spec fn preserved(before: Seq<bool>, after: Seq<bool>) -> bool {
    before.len() == after.len() && forall|i: int| #![auto] 0 <= i < before.len() && before[i] ==> after[i]
}
pub(super) open spec fn keys_reachable(graph: Model, context: Context, keys: Seq<Key>) -> bool {
    forall|i: int| #![auto] 0 <= i < keys.len() ==> model::reachable(graph, context.roots, context.extra, keys[i])
}
pub(super) open spec fn keys_marked(graph: Model, bits: Seq<bool>, keys: Seq<Key>) -> bool {
    forall|i: int| #![auto] 0 <= i < keys.len() ==> model::marked(graph, bits, keys[i])
}
pub(super) open spec fn row_closed(graph: Model, bits: Seq<bool>, slot: usize) -> bool {
    forall|e: int| #![auto] 0 <= e < graph.nodes[slot as int]->Some_0.edges.len()
        && graph.nodes[slot as int]->Some_0.edges[e] is Some ==>
            model::marked(graph, bits, graph.nodes[slot as int]->Some_0.edges[e]->Some_0)
}
pub(super) open spec fn base_valid(graph: Model, roots: Seq<Key>, extra: Seq<Key>, base: Seq<bool>) -> bool {
    model::valid(graph) && base.len() == graph.nodes.len()
        && (forall|i: int| #![auto] 0 <= i < base.len() && base[i] ==>
            graph.nodes[i] is Some && model::reachable(graph, roots, extra, model::key(graph, i)) && row_closed(graph, base, i as usize))
}
pub(super) open spec fn visit<N: Adjacency>(graph: Model, state: Visit<N>) -> bool {
    let context = state.context@;
    let history = context.history;
    &&& base_valid(graph, context.roots, context.extra, context.base)
    &&& state.bits@.len() == graph.nodes.len() && state.bits@.len() <= usize::MAX
    &&& history.no_duplicates() && history.len() <= graph.nodes.len()
    &&& (forall|i: int| #![trigger history[i]] 0 <= i < history.len() ==> history[i] < state.bits@.len() && !context.base[history[i] as int]
        && graph.nodes[history[i] as int]->Some_0.edges.len() > 0)
    &&& (forall|i: int| #![trigger state.bits@[i]] 0 <= i < state.bits@.len()
        && (context.base[i] || history.contains(i as usize)) ==> state.bits@[i])
    &&& (forall|i: int| #![trigger state.bits@[i]] 0 <= i < state.bits@.len() && state.bits@[i]
        ==> graph.nodes[i] is Some && model::reachable(graph, context.roots, context.extra, model::key(graph, i)))
    &&& (forall|i: int| #![trigger state.bits@[i]] 0 <= i < state.bits@.len() && state.bits@[i]
        && !context.base[i] && !history.contains(i as usize) ==> graph.nodes[i]->Some_0.edges.len() == 0)
    &&& context.pending.len() == state.work@.len() && context.pending.no_duplicates()
    &&& (forall|i: int| #![auto] 0 <= i < state.work@.len() ==> history.contains(context.pending[i])
        && state.work@[i]@ == graph.nodes[context.pending[i] as int]->Some_0.edges)
    &&& (context.current is Some ==> history.contains(context.current->Some_0) && !context.pending.contains(context.current->Some_0))
    &&& history.len() == state.work@.len() + context.done + if context.current is Some { 1nat } else { 0nat }
    &&& (forall|slot: usize| #![auto] history.contains(slot) && !context.pending.contains(slot) && context.current != Some(slot)
        ==> row_closed(graph, state.bits@, slot))
}
pub(super) open spec fn visit_preserved<N: Adjacency>(before: Visit<N>, after: Visit<N>) -> bool {
    scope(before.context@, after.context@) && before.context@.done == after.context@.done
        && before.context@.current == after.context@.current
        && preserved(before.bits@, after.bits@) && prefix(before.work@, after.work@)
        && prefix(before.context@.pending, after.context@.pending)
        && prefix(before.context@.history, after.context@.history)
        && after.work@.len() - before.work@.len() == after.context@.history.len() - before.context@.history.len()
}
pub(super) open spec fn seed_visit_loop<N: Adjacency>(graph: Model, keys: Seq<Key>, initial: Visit<N>, state: Visit<N>, r: usize) -> bool {
    visit(graph, state) && visit_preserved(initial, state)
        && keys_reachable(graph, state.context@, keys) && r <= keys.len()
        && (forall|j: int| #![auto] 0 <= j < r ==> model::marked(graph, state.bits@, keys[j]))
}
pub(super) open spec fn seed_cursor_loop<N: Adjacency>(graph: Model, original: SeedModel, source: SeedModel, initial: Visit<N>, state: Visit<N>) -> bool {
    visit(graph, state) && visit_preserved(initial, state)
        && source.keys == original.keys && source.position <= source.keys.len()
        && keys_reachable(graph, state.context@, source.keys)
        && (forall|j: int| #![auto] 0 <= j < source.position ==> model::marked(graph, state.bits@, source.keys[j]))
}
pub(super) open spec fn closure_input(graph: Model, roots: SeedModel, extra: SeedModel, seed: Option<&[bool]>) -> bool {
    roots.position <= roots.keys.len() && extra.position == 0
        && if seed is Some { model::matches(graph, seed->Some_0@, roots.keys, Seq::empty()) } else { roots.position == 0 }
}
pub(super) open spec fn sources_preserved(roots_before: SeedModel, roots_after: SeedModel, extra_before: SeedModel, extra_after: SeedModel) -> bool {
    roots_after.keys == roots_before.keys && extra_after.keys == extra_before.keys
        && roots_after.position <= roots_after.keys.len() && extra_after.position <= extra_after.keys.len()
}
pub(super) open spec fn walk_visit<N: Adjacency>(graph: Model, state: Visit<N>) -> bool {
    visit(graph, state) && state.context@.current is None
        && keys_marked(graph, state.bits@, state.context@.roots) && keys_marked(graph, state.bits@, state.context@.extra)
}
pub(super) open spec fn walk_iteration<N: Adjacency>(graph: Model, initial: Context, state: Visit<N>) -> bool {
    walk_visit(graph, state) && scope(initial, state.context@)
}
pub(super) open spec fn edges_visit<N: Adjacency>(graph: Model, state: Visit<N>, node: EdgeModel, e: int, edges: int) -> bool {
    visit(graph, state) && state.context@.current is Some
        && node == graph.nodes[state.context@.current->Some_0 as int]->Some_0.edges
        && keys_marked(graph, state.bits@, state.context@.roots) && keys_marked(graph, state.bits@, state.context@.extra)
        && edges == node.len() && 0 <= e <= edges
        && (forall|p: int| #![auto] 0 <= p < e && node[p] is Some ==> model::marked(graph, state.bits@, node[p]->Some_0))
}
pub(super) open spec fn edge_iteration<N: Adjacency>(graph: Model, initial: Visit<N>, state: Visit<N>, node: EdgeModel, e: int, edges: int) -> bool {
    edges_visit(graph, state, node, e, edges) && visit_preserved(initial, state)
}

pub(super) proof fn initial_state(graph: Model, context: Context, seed: Option<&[bool]>, bits: Seq<bool>)
    requires model::valid(graph), context.base == bits,
        context.history == Seq::<usize>::empty(), context.pending == Seq::<usize>::empty(), context.done == 0, context.current is None,
        mask_matches(graph.nodes.len() as usize, seed, bits), graph.nodes.len() <= usize::MAX,
        seed is Some ==> model::matches(graph, seed->Some_0@, context.roots, Seq::empty()),
    ensures base_valid(graph, context.roots, context.extra, bits),
        keys_reachable(graph, context, context.roots), keys_reachable(graph, context, context.extra),
        seed is Some ==> keys_marked(graph, bits, context.roots),
{
    if seed is Some {
        assert(bits =~= seed->Some_0@);
        seed_valid(graph, context.roots, context.extra, bits);
        seed_roots(graph, context.roots, bits);
    }
    assert forall|i: int| #![auto] 0 <= i < context.roots.len()
        implies model::reachable(graph, context.roots, context.extra, context.roots[i]) by {
        root_reachable(graph, context.roots, context.extra, context.roots[i]);
    }
    assert forall|i: int| #![auto] 0 <= i < context.extra.len()
        implies model::reachable(graph, context.roots, context.extra, context.extra[i]) by {
        root_reachable(graph, context.roots, context.extra, context.extra[i]);
    }
}

pub(super) open spec fn enqueue_context(graph: Model, before: Context, slot: usize) -> Context {
    if graph.nodes[slot as int]->Some_0.edges.len() == 0 { before }
    else { Context { history: before.history.push(slot), pending: before.pending.push(slot), ..before } }
}

pub(super) proof fn enqueued<N: Adjacency>(graph: Model, before: Visit<N>, after: &Visit<N>, id: Key)
    requires visit(graph, before), model::has_key(graph, id),
        model::reachable(graph, before.context@.roots, before.context@.extra, id), !before.bits@[id.slot as int],
        after.context@ == enqueue_context(graph, before.context@, id.slot as usize),
        after.bits@ == before.bits@.update(id.slot as int, true),
        if graph.nodes[id.slot as int]->Some_0.edges.len() == 0 {
            after.work@ == before.work@
        } else {
            after.work@.len() > 0 && after.work@ == before.work@.push(after.work@.last())
                && after.work@.last()@ == graph.nodes[id.slot as int]->Some_0.edges
        },
    ensures visit(graph, *after), visit_preserved(before, *after),
{
    if graph.nodes[id.slot as int]->Some_0.edges.len() == 0 { visited_leaf(graph, before, after, id); }
    else { visited_new(graph, before, after, id); }
}

pub(super) proof fn visited_new<N: Adjacency>(graph: Model, before: Visit<N>, after: &Visit<N>, id: Key)
    requires visit(graph, before), model::has_key(graph, id),
        graph.nodes[id.slot as int]->Some_0.edges.len() > 0,
        model::reachable(graph, before.context@.roots, before.context@.extra, id), !before.bits@[id.slot as int],
        scope(before.context@, after.context@), after.context@.done == before.context@.done, after.context@.current == before.context@.current,
        after.context@.history == before.context@.history.push(id.slot as usize),
        after.context@.pending == before.context@.pending.push(id.slot as usize),
        after.bits@ == before.bits@.update(id.slot as int, true),
        after.work@.len() > 0, after.work@ == before.work@.push(after.work@.last()),
        after.work@.last()@ == graph.nodes[id.slot as int]->Some_0.edges,
    ensures visit(graph, *after), visit_preserved(before, *after),
{
    let previous = before.context@.history;
    let history = after.context@.history;
    assert(!previous.contains(id.slot as usize));
    assert(history.no_duplicates());
    assert forall|i: int| #![auto] 0 <= i < history.len() implies history[i] < after.bits@.len() by {
        if i < previous.len() { assert(history[i] == previous[i]); }
    }
    vstd::std_specs::vec::axiom_spec_len(&after.bits);
    work_bound(history, after.bits@.len());
    assert forall|i: int| #![auto] 0 <= i < after.bits@.len() implies
        ((after.context@.base[i] || history.contains(i as usize)) ==> after.bits@[i])
            && (after.bits@[i] ==> graph.nodes[i] is Some && model::reachable(graph, after.context@.roots, after.context@.extra, model::key(graph, i)))
            && (after.bits@[i] && !after.context@.base[i] && !history.contains(i as usize) ==> graph.nodes[i]->Some_0.edges.len() == 0) by {
        if i == id.slot { assert(model::key(graph, i) == id); }
        else { assert(after.bits@[i] == before.bits@[i]); }
    }
    assert forall|slot: usize| #![auto] history.contains(slot) && !after.context@.pending.contains(slot) && after.context@.current != Some(slot)
        implies row_closed(graph, after.bits@, slot) by {
        assert(slot != id.slot);
        assert(row_closed(graph, before.bits@, slot));
    }
}

pub(super) proof fn visited_leaf<N: Adjacency>(graph: Model, before: Visit<N>, after: &Visit<N>, id: Key)
    requires visit(graph, before), model::has_key(graph, id),
        model::reachable(graph, before.context@.roots, before.context@.extra, id),
        !before.bits@[id.slot as int], graph.nodes[id.slot as int]->Some_0.edges.len() == 0,
        after.context@ == before.context@, after.work@ == before.work@,
        after.bits@ == before.bits@.update(id.slot as int, true),
    ensures visit(graph, *after), visit_preserved(before, *after),
{
    assert forall|i: int| #![auto] 0 <= i < after.bits@.len() implies
        ((after.context@.base[i] || after.context@.history.contains(i as usize)) ==> after.bits@[i])
            && (after.bits@[i] ==> graph.nodes[i] is Some && model::reachable(graph, after.context@.roots, after.context@.extra, model::key(graph, i)))
            && (after.bits@[i] && !after.context@.base[i] && !after.context@.history.contains(i as usize)
                ==> graph.nodes[i]->Some_0.edges.len() == 0) by {
        if i == id.slot { assert(model::key(graph, i) == id); }
        else { assert(after.bits@[i] == before.bits@[i]); }
    }
    assert forall|slot: usize| #![auto] after.context@.history.contains(slot)
        && !after.context@.pending.contains(slot) && after.context@.current != Some(slot)
        implies row_closed(graph, after.bits@, slot) by {
        assert(row_closed(graph, before.bits@, slot));
    }
}

pub(super) proof fn popped<N: Adjacency>(graph: Model, before: Visit<N>, after: &Visit<N>, index: usize, edges: EdgeModel)
    requires walk_visit(graph, before), before.work@.len() > 0,
        index == before.context@.pending.last(), edges == before.work@.last()@,
        after.work@ == before.work@.drop_last(), after.bits@ == before.bits@,
        scope(before.context@, after.context@), after.context@.history == before.context@.history,
        after.context@.pending == before.context@.pending.drop_last(),
        after.context@.done == before.context@.done, after.context@.current == Some(index),
    ensures visit(graph, *after), keys_marked(graph, after.bits@, after.context@.roots), keys_marked(graph, after.bits@, after.context@.extra),
        model::has_key(graph, model::key(graph, index as int)),
        edges == graph.nodes[index as int]->Some_0.edges,
{
    queued_node_valid(graph, before, index);
    assert(!after.context@.pending.contains(index));
    assert forall|slot: usize| #![auto] after.context@.history.contains(slot) && !after.context@.pending.contains(slot) && after.context@.current != Some(slot)
        implies row_closed(graph, after.bits@, slot) by {
        assert(!before.context@.pending.contains(slot));
    }
}

pub(super) proof fn queued_node_valid<N: Adjacency>(graph: Model, state: Visit<N>, slot: usize)
    requires visit(graph, state), state.context@.history.contains(slot),
    ensures model::has_key(graph, model::key(graph, slot as int)),
        model::reachable(graph, state.context@.roots, state.context@.extra, model::key(graph, slot as int)),
{
    let history = state.context@.history;
    let index = choose|index: int| 0 <= index < history.len() && history[index] == slot;
    assert(state.bits@[slot as int]);
}

pub(super) open spec fn pop_context(before: Context, has_entry: bool) -> Context {
    if has_entry && before.pending.len() > 0 {
        Context { pending: before.pending.drop_last(), current: Some(before.pending.last()), ..before }
    } else { before }
}

pub(super) open spec fn stack_popped<N: Adjacency>(before: Visit<N>, after: Visit<N>, entry: Option<N>) -> bool {
    (entry is Some <==> before.work@.len() > 0)
        && (entry is None ==> after.work@.len() == 0)
        && (forall|graph: Model| #![trigger walk_visit(graph, before)] walk_visit(graph, before) ==>
            if entry is Some {
                visit(graph, after) && scope(before.context@, after.context@)
                    && after.context@.done == before.context@.done && after.context@.current is Some
                    && keys_marked(graph, after.bits@, after.context@.roots) && keys_marked(graph, after.bits@, after.context@.extra)
                    && entry->Some_0@ == graph.nodes[after.context@.current->Some_0 as int]->Some_0.edges
            } else {
                walk_visit(graph, after) && scope(before.context@, after.context@) && after.work@.len() == 0
            })
}

pub(super) proof fn stack_popped_for_all<N: Adjacency>(before: Visit<N>, after: &Visit<N>, entry: Option<N>)
    requires
        entry == if before.work@.len() > 0 { Some(before.work@.last()) } else { None },
        after.work@ == if before.work@.len() > 0 { before.work@.drop_last() } else { before.work@ },
        after.bits@ == before.bits@,
        after.context@ == pop_context(before.context@, entry is Some),
    ensures stack_popped(before, *after, entry),
{
    assert forall|graph: Model| #![trigger walk_visit(graph, before)] walk_visit(graph, before) implies
        if entry is Some {
            visit(graph, *after) && scope(before.context@, after.context@)
                && after.context@.done == before.context@.done && after.context@.current is Some
                && keys_marked(graph, after.bits@, after.context@.roots) && keys_marked(graph, after.bits@, after.context@.extra)
                && entry->Some_0@ == graph.nodes[after.context@.current->Some_0 as int]->Some_0.edges
        } else {
            walk_visit(graph, *after) && scope(before.context@, after.context@) && after.work@.len() == 0
        } by {
        if let Some(node) = entry { popped(graph, before, after, before.context@.pending.last(), node@); }
    }
}

pub(super) proof fn node_done<N: Adjacency>(graph: Model, before: Visit<N>, after: &Visit<N>, node: EdgeModel)
    requires edges_visit(graph, before, node, node.len() as int, node.len() as int),
        after.work@ == before.work@, after.bits@ == before.bits@,
        scope(before.context@, after.context@), after.context@.history == before.context@.history,
        after.context@.pending == before.context@.pending,
        after.context@.done == before.context@.done + 1, after.context@.current is None,
    ensures walk_visit(graph, *after),
{
    let index = before.context@.current->Some_0;
    assert(row_closed(graph, after.bits@, index));
    assert forall|slot: usize| #![auto] after.context@.history.contains(slot) && !after.context@.pending.contains(slot)
        implies row_closed(graph, after.bits@, slot) by {
        if slot == index { assert(row_closed(graph, after.bits@, index)); }
        else { assert(row_closed(graph, before.bits@, slot)); }
    }
}
pub(super) proof fn root_reachable(graph: Model, roots: Seq<Key>, extra: Seq<Key>, id: Key)
    requires roots.contains(id) || extra.contains(id),
    ensures model::reachable(graph, roots, extra, id),
{
    assert forall|keys: ISet<Key>| #![auto] model::seeded(roots, extra, keys) && model::closed(graph, keys)
        implies keys.contains(id) by {
        if roots.contains(id) { let i = choose|i: int| 0 <= i < roots.len() && roots[i] == id; }
        else { let i = choose|i: int| 0 <= i < extra.len() && extra[i] == id; }
    }
}

pub(super) proof fn edge_reachable(graph: Model, roots: Seq<Key>, extra: Seq<Key>, from: Key, edge: int)
    requires model::valid(graph), model::has_key(graph, from), model::reachable(graph, roots, extra, from),
        0 <= edge < graph.nodes[from.slot as int]->Some_0.edges.len(),
        graph.nodes[from.slot as int]->Some_0.edges[edge] is Some,
    ensures model::reachable(graph, roots, extra, graph.nodes[from.slot as int]->Some_0.edges[edge]->Some_0),
{
    let to = graph.nodes[from.slot as int]->Some_0.edges[edge]->Some_0;
    assert(model::dependency(graph, from, to));
    assert forall|keys: ISet<Key>| #![auto] model::seeded(roots, extra, keys) && model::closed(graph, keys)
        implies keys.contains(to) by { assert(keys.contains(from)); }
}

pub(super) proof fn visit_edge_reachable<N: Adjacency>(graph: Model, state: Visit<N>, node: EdgeModel, edge: int)
    requires edges_visit(graph, state, node, edge, node.len() as int), edge < node.len(), node[edge] is Some,
    ensures model::reachable(graph, state.context@.roots, state.context@.extra, node[edge]->Some_0),
{
    let context = state.context@;
    queued_node_valid(graph, state, context.current->Some_0);
    edge_reachable(graph, context.roots, context.extra, model::key(graph, context.current->Some_0 as int), edge);
}

pub(super) proof fn work_bound(work: Seq<usize>, n: nat)
    requires n <= usize::MAX, work.no_duplicates(), forall|i: int| #![auto] 0 <= i < work.len() ==> work[i] < n,
    ensures work.len() <= n,
{
    let universe = Seq::new(n, |i: int| i as usize);
    assert(universe.no_duplicates());
    assert(work.to_set().subset_of(universe.to_set())) by {
        assert forall|x: usize| work.to_set().contains(x) implies universe.to_set().contains(x) by {
            let i = choose|i: int| 0 <= i < work.len() && work[i] == x;
            assert(0 <= (x as int) < n); assert(universe[x as int] == x);
        }
    }
    work.unique_seq_to_set(); universe.unique_seq_to_set();
    vstd::set_lib::lemma_len_subset(work.to_set(), universe.to_set());
}

pub(super) proof fn seed_valid(graph: Model, roots: Seq<Key>, extra: Seq<Key>, base: Seq<bool>)
    requires model::valid(graph), model::matches(graph, base, roots, Seq::empty()),
    ensures base_valid(graph, roots, extra, base),
{
    assert forall|i: int| #![auto] 0 <= i < base.len() && base[i] implies
        graph.nodes[i] is Some && model::reachable(graph, roots, extra, model::key(graph, i)) by {
        let id = model::key(graph, i);
        assert(model::marked(graph, base, id));
        assert(model::reachable(graph, roots, Seq::empty(), id));
        assert forall|keys: ISet<Key>| #![auto] model::seeded(roots, extra, keys) && model::closed(graph, keys)
            implies keys.contains(id) by { assert(model::seeded(roots, Seq::empty(), keys)); }
    }
    assert forall|i: int, e: int| #![trigger base[i], graph.nodes[i]->Some_0.edges[e]]
        0 <= i < base.len() && base[i] && 0 <= e < graph.nodes[i]->Some_0.edges.len()
            && graph.nodes[i]->Some_0.edges[e] is Some
        implies model::marked(graph, base, graph.nodes[i]->Some_0.edges[e]->Some_0) by {
        let id = model::key(graph, i);
        assert(model::marked(graph, base, id));
        edge_reachable(graph, roots, Seq::empty(), id, e);
    }
}

pub(super) proof fn seed_roots(graph: Model, roots: Seq<Key>, base: Seq<bool>)
    requires model::matches(graph, base, roots, Seq::empty()),
    ensures forall|i: int| #![auto] 0 <= i < roots.len() ==> model::marked(graph, base, roots[i]),
{
    assert forall|i: int| #![auto] 0 <= i < roots.len() implies model::marked(graph, base, roots[i]) by {
        root_reachable(graph, roots, Seq::empty(), roots[i]);
    }
}

pub(super) proof fn closure_complete<N: Adjacency>(graph: Model, state: Visit<N>)
    requires walk_visit(graph, state), state.work@.len() == 0,
    ensures model::matches(graph, state.bits@, state.context@.roots, state.context@.extra),
{
    assert(visit(graph, state));
    let context = state.context@;
    let roots = context.roots; let extra = context.extra; let base = context.base; let bits = state.bits@;
    assert(bits.len() == graph.nodes.len());
    assert forall|i: int| #![auto] 0 <= i < bits.len() && bits[i] implies graph.nodes[i] is Some by {}
    let output = ISet::new(|id: Key| model::marked(graph, bits, id));
    assert(model::seeded(roots, extra, output));
    assert(model::closed(graph, output)) by {
        assert forall|from: Key, to: Key| output.contains(from) && model::dependency(graph, from, to)
            implies output.contains(to) by {
            let e = choose|e: int| 0 <= e < graph.nodes[from.slot as int]->Some_0.edges.len()
                && graph.nodes[from.slot as int]->Some_0.edges[e] == Some(to);
            if base[from.slot as int] { assert(model::marked(graph, base, to)); }
            else if context.history.contains(from.slot as usize) {
                assert(row_closed(graph, bits, from.slot as usize));
            } else {
                assert(graph.nodes[from.slot as int]->Some_0.edges.len() == 0);
            }
        }
    }
    assert forall|id: Key| #![auto] model::marked(graph, bits, id) <==> model::reachable(graph, roots, extra, id) by {
        if model::marked(graph, bits, id) {
            assert(bits[id.slot as int]);
            assert(model::key(graph, id.slot as int) == id);
            assert(model::reachable(graph, roots, extra, model::key(graph, id.slot as int)));
        }
        if model::reachable(graph, roots, extra, id) { assert(output.contains(id)); }
    }
}

pub(super) proof fn plan_valid(graph: Model, roots: Seq<Key>, records: Seq<Key>, plan: &Plan)
    requires model::valid(graph), model::matches(graph, plan.published@, roots, Seq::empty()), model::matches(graph, plan.keep@, roots, records),
    ensures plan.published@.len() == graph.nodes.len() && plan.keep@.len() == graph.nodes.len()
        && (forall|i: int| #![auto] 0 <= i < graph.nodes.len() ==>
            (plan.published@[i] ==> plan.keep@[i]) && (plan.keep@[i] ==> graph.nodes[i] is Some)),
{
    assert forall|id: Key| model::reachable(graph, roots, Seq::empty(), id) implies model::reachable(graph, roots, records, id) by {
        assert forall|keys: ISet<Key>| #![auto] model::seeded(roots, records, keys) && model::closed(graph, keys)
            implies keys.contains(id) by { assert(model::seeded(roots, Seq::empty(), keys)); }
    }
    assert forall|i: int| #![auto] 0 <= i < graph.nodes.len() implies
        (plan.published@[i] ==> plan.keep@[i]) && (plan.keep@[i] ==> graph.nodes[i] is Some) by {
        if plan.published@[i] {
            let id = model::key(graph, i);
            assert(model::marked(graph, plan.published@, id));
            assert(model::reachable(graph, roots, Seq::empty(), id));
            assert(model::reachable(graph, roots, records, id));
            assert(model::marked(graph, plan.keep@, id));
        }
    }
}

}
