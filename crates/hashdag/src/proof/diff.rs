#[cfg(verus_only)]
use super::*;
#[cfg(verus_only)]
use vstd::prelude::verus;

#[cfg(not(verus_only))]
/// 普通构建仅保留擦除的关联类型，不保存图快照。
pub type GraphModel = ();

#[cfg(verus_only)]
verus! {
pub struct GraphModel {
    pub(crate) roots: crate::roots::RootSlots,
    pub(crate) nodes: Seq<Seq<Option<Seq<Child>>>>,
}

pub(crate) struct Context {
    pub(crate) base: ISet<NodeAddress>,
    pub(crate) root: Child,
    pub(crate) history: Seq<NodeAddress>,
    pub(crate) current: Option<NodeAddress>,
    pub(crate) done: nat,
    pub(crate) output: bool,
}

pub(super) open spec fn empty_context() -> Context {
    Context { base: ISet::empty(), root: Child::Leaf(0), history: Seq::empty(), current: None, done: 0, output: false }
}

pub(crate) mod model {
    use super::*;
    pub open(crate) spec fn pointer(child: Child) -> NodeIdx {
        match child { Child::Ptr(index) => index, Child::Leaf(_) => crate::node::proof::at_index(0) }
    }
    pub open(crate) spec fn has_node(graph: GraphModel, address: NodeAddress) -> bool {
        address.level < graph.nodes.len()
            && crate::node::proof::idx(address.index) < graph.nodes[address.level as int].len()
            && graph.nodes[address.level as int][crate::node::proof::idx(address.index)] is Some
    }
    pub open(crate) spec fn valid(graph: GraphModel) -> bool {
        graph.nodes.len() == 10 && graph.roots.valid()
            && (forall|level: int| 0 <= level < 10 ==> graph.nodes[level].len() <= u32::MAX as int + 1
                && graph.nodes[level].len() <= usize::MAX)
            && (forall|a: NodeAddress| has_node(graph, a) ==> {
                let children = graph.nodes[a.level as int][crate::node::proof::idx(a.index)]->Some_0;
                children.len() == 64 && forall|slot: int| 0 <= slot < 64 && children[slot] is Ptr ==>
                    a.level > 0 && has_node(graph, NodeAddress { level: (a.level - 1) as u8, index: pointer(children[slot]) })
            })
            && (forall|root: RootId| graph.roots.lookup(root) is Some ==>
                root_valid(graph, graph.roots.lookup(root)->Some_0))
    }
    pub open(crate) spec fn root_valid(graph: GraphModel, root: Child) -> bool {
        root is Ptr ==> has_node(graph, NodeAddress { level: 9, index: pointer(root) })
    }
    pub open(crate) spec fn dependency(graph: GraphModel, from: NodeAddress, to: NodeAddress) -> bool {
        has_node(graph, from) && from.level > 0 && to.level + 1 == from.level
            && graph.nodes[from.level as int][crate::node::proof::idx(from.index)]->Some_0.contains(Child::Ptr(to.index))
    }
    pub open(crate) spec fn seeded(root: Child, set: ISet<NodeAddress>) -> bool {
        root is Ptr ==> set.contains(NodeAddress { level: 9, index: pointer(root) })
    }
    pub open(crate) spec fn closed(graph: GraphModel, set: ISet<NodeAddress>) -> bool {
        forall|from: NodeAddress, to: NodeAddress| set.contains(from) && #[trigger] dependency(graph, from, to) ==> set.contains(to)
    }
    pub open(crate) spec fn reachable(graph: GraphModel, root: Child, address: NodeAddress) -> bool {
        forall|set: ISet<NodeAddress>| seeded(root, set) && closed(graph, set) ==> #[trigger] set.contains(address)
    }
}

broadcast use vstd::seq_lib::group_seq_properties;

pub(super) open spec fn bit_shape(graph: GraphModel, state: NodeDiff) -> bool {
    forall|level: int| 0 <= level < 10 ==>
        state.seen@[level]@.len() == (graph.nodes[level].len() + 63) / 64
}

pub open(crate) spec fn marked(graph: GraphModel, state: NodeDiff, address: NodeAddress) -> bool {
    address.level < 10 && crate::node::proof::idx(address.index) < graph.nodes[address.level as int].len()
        && crate::node::proof::idx(address.index) / 64 < state.seen@[address.level as int]@.len()
        && state.seen@[address.level as int]@[crate::node::proof::idx(address.index) / 64]
            & (1u64 << (crate::node::proof::idx(address.index) % 64)) != 0
}

pub(super) open spec fn marked_set(graph: GraphModel, state: NodeDiff) -> ISet<NodeAddress> {
    ISet::new(|address: NodeAddress| marked(graph, state, address))
}

pub(super) open spec fn row_closed(graph: GraphModel, state: NodeDiff, address: NodeAddress) -> bool {
    forall|child: NodeAddress| #[trigger] model::dependency(graph, address, child) ==> marked(graph, state, child)
}

pub(super) open spec fn visit(graph: GraphModel, state: NodeDiff) -> bool {
    let c = state.context@;
    &&& model::valid(graph) && model::root_valid(graph, c.root)
    &&& bit_shape(graph, state)
    &&& model::closed(graph, c.base)
    &&& (forall|a: NodeAddress| c.base.contains(a) ==> model::has_node(graph, a))
    &&& c.history.no_duplicates() && c.history.len() <= 10 * (u32::MAX as int + 1)
    &&& (forall|i: int| #![auto] 0 <= i < c.history.len() ==> model::has_node(graph, c.history[i])
        && model::reachable(graph, c.root, c.history[i]) && !c.base.contains(c.history[i]))
    &&& (forall|a: NodeAddress| marked(graph, state, a) <==> c.base.contains(a) || c.history.contains(a))
    &&& state.stack@.no_duplicates()
    &&& (forall|i: int| #![auto] 0 <= i < state.stack@.len() ==> c.history.contains(state.stack@[i]) && state.stack@[i].level > 0)
    &&& (c.current is Some ==> c.history.contains(c.current->Some_0) && !state.stack@.contains(c.current->Some_0))
    &&& c.history.len() == state.stack@.len() + c.done + if c.current is Some { 1nat } else { 0nat }
    &&& (forall|a: NodeAddress| #![auto] c.history.contains(a) && !state.stack@.contains(a) && c.current != Some(a)
        ==> row_closed(graph, state, a))
    &&& if c.output { state.nodes@ == c.history } else { state.nodes@.len() == 0 }
}

pub(super) open spec fn scope(before: Context, after: Context) -> bool {
    before.root == after.root && before.base == after.base && before.output == after.output
}

pub(super) open spec fn preserved(graph: GraphModel, before: NodeDiff, after: NodeDiff) -> bool {
    &&& scope(before.context@, after.context@)
    &&& before.context@.current == after.context@.current
    &&& before.context@.done <= after.context@.done
    &&& forall|a: NodeAddress| marked(graph, before, a) ==> marked(graph, after, a)
}

pub(super) open spec fn walk(graph: GraphModel, state: NodeDiff) -> bool {
    visit(graph, state) && state.context@.current is None
        && model::seeded(state.context@.root, marked_set(graph, state))
}

pub(super) open spec fn edges(graph: GraphModel, state: NodeDiff, node: NodeRef, remaining: u64) -> bool {
    let current = state.context@.current->Some_0;
    let original = crate::node::proof::mask(node.mask);
    &&& visit(graph, state) && state.context@.current is Some
    &&& crate::node::proof::ref_children(node)
        == graph.nodes[current.level as int][crate::node::proof::idx(current.index)]->Some_0
    &&& remaining & !original == 0
    &&& (forall|slot: u8| slot < 64 && original & (1u64 << slot) != 0 && remaining & (1u64 << slot) == 0 ==>
        marked(graph, state, NodeAddress { level: (current.level - 1) as u8,
            index: crate::node::proof::at_index(crate::node::proof::values(*node.values)[slot as int]) }))
}

pub(super) proof fn history_bound(history: Seq<NodeAddress>)
    requires history.no_duplicates(), forall|i: int| 0 <= i < history.len() ==> history[i].level < 10,
    ensures history.len() <= 10 * (u32::MAX as int + 1),
{
    let width = u32::MAX as int + 1;
    let universe = Seq::new((10 * width) as nat, |i: int| NodeAddress {
        level: (i / width) as u8, index: crate::node::proof::at_index((i % width) as u32) });
    assert(universe.no_duplicates()) by {
        assert forall|i: int, j: int| 0 <= i < universe.len() && 0 <= j < universe.len() && i != j
            implies universe[i] != universe[j] by {
            if universe[i] == universe[j] { vstd::arithmetic::div_mod::lemma_fundamental_div_mod(i, width); vstd::arithmetic::div_mod::lemma_fundamental_div_mod(j, width); }
        }
    }
    assert(history.to_set().subset_of(universe.to_set())) by {
        assert forall|a: NodeAddress| history.to_set().contains(a) implies universe.to_set().contains(a) by {
            let i = choose|i: int| 0 <= i < history.len() && history[i] == a;
            let rank = a.level as int * width + crate::node::proof::idx(a.index);
            assert(0 <= rank < universe.len());
            assert(universe[rank] == a);
        }
    }
    history.unique_seq_to_set(); universe.unique_seq_to_set();
    vstd::set_lib::lemma_len_subset(history.to_set(), universe.to_set());
}

pub(super) proof fn root_reachable(graph: GraphModel, root: Child)
    ensures root is Ptr ==> model::reachable(graph, root, NodeAddress { level: 9, index: model::pointer(root) }),
{}

pub(super) proof fn edge_reachable(graph: GraphModel, root: Child, from: NodeAddress, to: NodeAddress)
    requires model::reachable(graph, root, from), model::dependency(graph, from, to),
    ensures model::reachable(graph, root, to),
{
    assert forall|set: ISet<NodeAddress>| #![auto] model::seeded(root, set) && model::closed(graph, set)
        implies set.contains(to) by { assert(set.contains(from)); }
}

pub(super) proof fn reachable_valid(graph: GraphModel, root: Child, address: NodeAddress)
    requires model::valid(graph), model::root_valid(graph, root), model::reachable(graph, root, address),
    ensures model::has_node(graph, address),
{
    let live = ISet::new(|a: NodeAddress| model::has_node(graph, a));
    assert(model::seeded(root, live));
    assert(model::closed(graph, live)) by {
        assert forall|from: NodeAddress, to: NodeAddress| live.contains(from) && model::dependency(graph, from, to)
            implies live.contains(to) by {
            let children = graph.nodes[from.level as int][crate::node::proof::idx(from.index)]->Some_0;
            let slot = choose|slot: int| 0 <= slot < children.len() && children[slot] == Child::Ptr(to.index);
            assert(to.level == from.level - 1);
        }
    }
    assert(live.contains(address));
}

pub(super) proof fn reachable_closed(graph: GraphModel, root: Child)
    ensures model::closed(graph, ISet::new(|a: NodeAddress| model::reachable(graph, root, a))),
{
    assert forall|from: NodeAddress, to: NodeAddress| model::reachable(graph, root, from) && model::dependency(graph, from, to)
        implies model::reachable(graph, root, to) by { edge_reachable(graph, root, from, to); }
}

pub(super) proof fn complete(graph: GraphModel, state: NodeDiff)
    requires walk(graph, state), state.stack@.len() == 0,
    ensures forall|a: NodeAddress| marked(graph, state, a) <==>
        state.context@.base.contains(a) || model::reachable(graph, state.context@.root, a),
{
    let c = state.context@;
    let output = marked_set(graph, state);
    assert(model::closed(graph, output)) by {
        assert forall|from: NodeAddress, to: NodeAddress| output.contains(from) && model::dependency(graph, from, to)
            implies output.contains(to) by {
            if c.base.contains(from) { assert(c.base.contains(to)); }
            else { assert(row_closed(graph, state, from)); }
        }
    }
    assert forall|a: NodeAddress| marked(graph, state, a) <==> c.base.contains(a) || model::reachable(graph, c.root, a) by {
        if marked(graph, state, a) && !c.base.contains(a) { assert(c.history.contains(a)); }
        if model::reachable(graph, c.root, a) { assert(output.contains(a)); }
    }
}

impl NodeDiff {
    pub open(crate) spec fn ready(&self) -> bool { self.stack@.len() == 0 }
    pub open(crate) spec fn output(&self) -> Seq<NodeAddress> { self.nodes@ }
}

pub open(crate) spec fn exact(graph: GraphModel, before: RootId, after: RootId, output: Seq<NodeAddress>) -> bool {
    graph.roots.lookup(before) is Some && graph.roots.lookup(after) is Some
        && output.no_duplicates()
        && forall|a: NodeAddress| output.contains(a) <==>
            model::reachable(graph, graph.roots.lookup(after)->Some_0, a)
                && !model::reachable(graph, graph.roots.lookup(before)->Some_0, a)
}

pub open(crate) spec fn sorted(output: Seq<NodeAddress>) -> bool {
    forall|i: int, j: int| 0 <= i < j < output.len() ==>
        output[i].level < output[j].level || output[i].level == output[j].level
            && crate::node::proof::idx(output[i].index) <= crate::node::proof::idx(output[j].index)
}

pub(super) proof fn phase_initial(graph: GraphModel, state: NodeDiff)
    requires model::valid(graph), model::root_valid(graph, state.context@.root),
        bit_shape(graph, state), state.stack@.len() == 0, state.nodes@.len() == 0,
        state.context@.base == marked_set(graph, state),
        model::closed(graph, state.context@.base),
        forall|a: NodeAddress| state.context@.base.contains(a) ==> model::has_node(graph, a),
        state.context@.history == Seq::<NodeAddress>::empty(), state.context@.current is None, state.context@.done == 0,
    ensures visit(graph, state),
{}

pub(super) proof fn visited_new(graph: GraphModel, before: NodeDiff, after: NodeDiff, address: NodeAddress)
    requires visit(graph, before), model::has_node(graph, address),
        model::reachable(graph, before.context@.root, address), !marked(graph, before, address),
        bit_shape(graph, after),
        forall|a: NodeAddress| marked(graph, after, a) <==> marked(graph, before, a) || a == address,
        scope(before.context@, after.context@), before.context@.current == after.context@.current,
        after.context@.history == before.context@.history.push(address),
        after.context@.done == before.context@.done + if address.level == 0 { 1nat } else { 0nat },
        after.stack@ == if address.level == 0 { before.stack@ } else { before.stack@.push(address) },
        after.nodes@ == if after.context@.output { before.nodes@.push(address) } else { before.nodes@ },
    ensures visit(graph, after), preserved(graph, before, after),
{
    let history = after.context@.history;
    assert(!before.context@.history.contains(address));
    assert(history.no_duplicates());
    assert forall|i: int| 0 <= i < history.len() implies history[i].level < 10 by {
        assert(model::has_node(graph, history[i]));
    }
    history_bound(history);
    assert forall|a: NodeAddress| #![auto] history.contains(a) && !after.stack@.contains(a) && after.context@.current != Some(a)
        implies row_closed(graph, after, a) by {
        if a == address { assert(a.level == 0); }
        else {
            assert(row_closed(graph, before, a));
            assert forall|child: NodeAddress| model::dependency(graph, a, child) implies marked(graph, after, child) by {
                assert(marked(graph, before, child));
            }
        }
    }
}

pub(super) proof fn popped(graph: GraphModel, before: NodeDiff, after: NodeDiff, address: NodeAddress)
    requires walk(graph, before), before.stack@.len() > 0, address == before.stack@.last(),
        after.stack@ == before.stack@.drop_last(), after.nodes@ == before.nodes@,
        after.seen@ == before.seen@,
        scope(before.context@, after.context@), after.context@.history == before.context@.history,
        after.context@.done == before.context@.done, after.context@.current == Some(address),
    ensures visit(graph, after), address.level > 0, model::has_node(graph, address),
        model::reachable(graph, after.context@.root, address),
        model::seeded(after.context@.root, marked_set(graph, after)),
{
    assert(before.context@.history.contains(address));
    assert(!after.stack@.contains(address));
}

pub(super) proof fn node_done(graph: GraphModel, before: NodeDiff, after: NodeDiff, node: NodeRef)
    requires edges(graph, before, node, 0),
        after.stack@ == before.stack@, after.nodes@ == before.nodes@, after.seen@ == before.seen@,
        scope(before.context@, after.context@), after.context@.history == before.context@.history,
        after.context@.current is None, after.context@.done == before.context@.done + 1,
        model::seeded(before.context@.root, marked_set(graph, before)),
    ensures walk(graph, after),
{
    let current = before.context@.current->Some_0;
    assert(row_closed(graph, before, current)) by {
        assert forall|child: NodeAddress| model::dependency(graph, current, child) implies marked(graph, before, child) by {
            let children = crate::node::proof::ref_children(node);
            let slot = choose|slot: int| 0 <= slot < children.len() && children[slot] == Child::Ptr(child.index);
            assert(children.len() == 64);
            let shift = slot as u8;
            assert(shift as int == slot);
            assert(crate::node::proof::mask(node.mask) & (1u64 << shift) != 0);
            assert(0u64 & (1u64 << shift) == 0) by(bit_vector);
            assert(child.level == current.level - 1);
            assert(crate::node::proof::at_index(crate::node::proof::values(*node.values)[slot]) == child.index);
            assert(marked(graph, before, NodeAddress { level: (current.level - 1) as u8, index: child.index }));
        }
    }
}

pub(super) proof fn traversal_result(graph: GraphModel, state: NodeDiff)
    requires walk(graph, state), state.stack@.len() == 0,
    ensures forall|a: NodeAddress| marked(graph, state, a) <==>
        state.context@.base.contains(a) || model::reachable(graph, state.context@.root, a),
        state.context@.output ==> state.nodes@.no_duplicates()
            && forall|a: NodeAddress| state.nodes@.contains(a) <==>
                model::reachable(graph, state.context@.root, a) && !state.context@.base.contains(a),
{
    complete(graph, state);
    assert forall|a: NodeAddress| state.context@.output implies
        (state.nodes@.contains(a) <==> model::reachable(graph, state.context@.root, a) && !state.context@.base.contains(a)) by {
        if state.nodes@.contains(a) { assert(!state.context@.base.contains(a)); }
        if model::reachable(graph, state.context@.root, a) && !state.context@.base.contains(a) { assert(marked(graph, state, a)); }
    }
}

pub(super) proof fn early_empty(graph: GraphModel, before: RootId, after: RootId)
    requires graph.roots.lookup(before) is Some, graph.roots.lookup(after) is Some,
        graph.roots.lookup(before) == graph.roots.lookup(after) || graph.roots.lookup(after)->Some_0 is Leaf,
    ensures exact(graph, before, after, Seq::empty()), sorted(Seq::empty()),
{
    if graph.roots.lookup(after)->Some_0 is Leaf {
        assert(model::seeded(graph.roots.lookup(after)->Some_0, ISet::empty()));
        assert(model::closed(graph, ISet::empty()));
    }
}

pub(super) proof fn empty_bits(graph: GraphModel, state: NodeDiff)
    requires model::valid(graph),
        forall|level: int| 0 <= level < 10 ==> state.seen@[level]@.len() == (graph.nodes[level].len() + 63) / 64
            && forall|word: int| 0 <= word < state.seen@[level]@.len() ==> state.seen@[level]@[word] == 0,
    ensures bit_shape(graph, state), marked_set(graph, state) == ISet::<NodeAddress>::empty(),
        forall|a: NodeAddress| !marked(graph, state, a),
{
    assert forall|a: NodeAddress| !marked(graph, state, a) by {
        if a.level < 10 && crate::node::proof::idx(a.index) / 64 < state.seen@[a.level as int]@.len() {
            let shift = (crate::node::proof::idx(a.index) % 64) as u64;
            assert(0u64 & (1u64 << shift) == 0) by(bit_vector);
        }
    }
    assert(marked_set(graph, state) =~= ISet::<NodeAddress>::empty());
}

pub(super) proof fn traversal_closed(graph: GraphModel, root: Child, state: NodeDiff)
    requires model::valid(graph), model::root_valid(graph, root),
        forall|a: NodeAddress| marked(graph, state, a) <==> model::reachable(graph, root, a),
    ensures model::closed(graph, marked_set(graph, state)),
        forall|a: NodeAddress| marked(graph, state, a) ==> model::has_node(graph, a),
{
    reachable_closed(graph, root);
    assert(marked_set(graph, state) =~= ISet::new(|a: NodeAddress| model::reachable(graph, root, a)));
    assert forall|a: NodeAddress| marked(graph, state, a) implies model::has_node(graph, a) by {
        reachable_valid(graph, root, a);
    }
}

pub(super) proof fn set_bit(word: u64, shift: u64, other: u64)
    requires shift < 64, other < 64,
    ensures ((word | (1u64 << shift)) & (1u64 << other) != 0) <==>
        shift == other || word & (1u64 << other) != 0,
{
    assert(shift < 64 && other < 64 ==> (((word | (1u64 << shift)) & (1u64 << other) != 0)
        <==> shift == other || word & (1u64 << other) != 0)) by(bit_vector);
}

pub(super) proof fn already_marked(word: u64, shift: u64)
    ensures word & (1u64 << shift) != 0 ==> word | (1u64 << shift) == word,
{
    assert(word & (1u64 << shift) != 0 ==> word | (1u64 << shift) == word) by(bit_vector);
}

pub(super) proof fn start_edges(graph: GraphModel, state: NodeDiff, node: NodeRef, address: NodeAddress)
    requires visit(graph, state), state.context@.current == Some(address), model::has_node(graph, address),
        crate::node::proof::ref_children(node) == graph.nodes[address.level as int][crate::node::proof::idx(address.index)]->Some_0,
    ensures edges(graph, state, node, crate::node::proof::mask(node.mask)),
{
    and_not(crate::node::proof::mask(node.mask));
}

proof fn and_not(bits: u64)
    ensures bits & !bits == 0,
{ assert(bits & !bits == 0) by(bit_vector); }

pub(super) proof fn marked_one(graph: GraphModel, before: NodeDiff, after: NodeDiff, address: NodeAddress)
    requires model::valid(graph), bit_shape(graph, before), model::has_node(graph, address),
        after.seen@[address.level as int]@ == before.seen@[address.level as int]@.update(
            crate::node::proof::idx(address.index) / 64,
            before.seen@[address.level as int]@[crate::node::proof::idx(address.index) / 64]
                | (1u64 << (crate::node::proof::idx(address.index) % 64))),
        forall|level: int| 0 <= level < 10 && level != address.level ==> after.seen@[level]@ == before.seen@[level]@,
    ensures bit_shape(graph, after),
        forall|a: NodeAddress| marked(graph, after, a) <==> marked(graph, before, a) || a == address,
{
    assert forall|a: NodeAddress| marked(graph, after, a) <==> marked(graph, before, a) || a == address by {
        if a.level == address.level && crate::node::proof::idx(a.index) < graph.nodes[a.level as int].len() {
            let position = crate::node::proof::idx(a.index);
            let selected = crate::node::proof::idx(address.index);
            if position / 64 == selected / 64 {
                set_bit(before.seen@[a.level as int]@[position / 64], (selected % 64) as u64, (position % 64) as u64);
                if position % 64 == selected % 64 {
                    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(position, 64);
                    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(selected, 64);
                    assert(a == address);
                }
            }
        }
    }
}

pub(super) proof fn lowest_cleared(before: u64, after: u64, slot: u8)
    requires before != 0, after == before & ((before - 1) as u64), slot == before.trailing_zeros() as u8,
    ensures slot < 64, before & (1u64 << slot) != 0, after < before,
        forall|other: u8| other < 64 ==> (after & (1u64 << other) != 0 <==>
            before & (1u64 << other) != 0 && other != slot),
{
    crate::node::proof::nonzero_bit(before);
    vstd::std_specs::bits::axiom_u64_trailing_zeros(before);
    let shift = slot as u64;
    assert((before << ((64u64 - shift) as u64)) == 0);
    assert(shift < 64 && before << ((64u64 - shift) as u64) == 0 && before & (1u64 << shift) != 0 ==>
        before & ((before - 1) as u64) == before & !(1u64 << shift)) by(bit_vector);
    assert(before != 0 && after == before & ((before - 1) as u64) ==> after < before) by(bit_vector);
    assert forall|other: u8| other < 64 implies (after & (1u64 << other) != 0 <==>
        before & (1u64 << other) != 0 && other != slot) by {
        assert(slot < 64 && other < 64 ==> ((before & !(1u64 << slot)) & (1u64 << other) != 0
            <==> before & (1u64 << other) != 0 && other != slot)) by(bit_vector);
    }
}

pub(super) proof fn edge_done(graph: GraphModel, before: NodeDiff, after: NodeDiff, node: NodeRef, old_bits: u64, bits: u64, slot: u8)
    requires edges(graph, before, node, old_bits), old_bits != 0,
        bits == old_bits & ((old_bits - 1) as u64), slot == old_bits.trailing_zeros() as u8,
        visit(graph, after), preserved(graph, before, after),
        crate::node::proof::ref_children(node)[slot as int] is Ptr ==>
            marked(graph, after, NodeAddress { level: (after.context@.current->Some_0.level - 1) as u8,
                index: model::pointer(crate::node::proof::ref_children(node)[slot as int]) }),
    ensures edges(graph, after, node, bits),
{
    lowest_cleared(old_bits, bits, slot);
    assert forall|other: u8| other < 64 && crate::node::proof::mask(node.mask) & (1u64 << other) != 0
        && bits & (1u64 << other) == 0 implies marked(graph, after, NodeAddress {
            level: (after.context@.current->Some_0.level - 1) as u8,
            index: crate::node::proof::at_index(crate::node::proof::values(*node.values)[other as int]) }) by {
        if other != slot {
            assert(old_bits & (1u64 << other) == 0);
        } else {
            assert(crate::node::proof::ref_children(node)[other as int] is Ptr);
        }
    }
    clear_subset(crate::node::proof::mask(node.mask), old_bits, bits);
}

proof fn clear_subset(original: u64, before: u64, after: u64)
    requires before != 0, before & !original == 0, after == before & ((before - 1) as u64),
    ensures after & !original == 0,
{
    assert(before != 0 && before & !original == 0 && after == before & ((before - 1) as u64)
        ==> after & !original == 0) by(bit_vector);
}

pub(super) proof fn sorted_result(before: Seq<NodeAddress>, after: Seq<NodeAddress>)
    requires before.to_multiset() == after.to_multiset(),
        hash_dag_std::sort_model::ordered(after, NodeAddress::sort_key),
    ensures sorted(after), after.no_duplicates() == before.no_duplicates(),
        forall|a: NodeAddress| before.contains(a) <==> after.contains(a),
{
    broadcast use vstd::seq_lib::group_to_multiset_ensures;
    assert forall|i: int, j: int| 0 <= i < j < after.len() implies
        after[i].level < after[j].level || after[i].level == after[j].level
            && crate::node::proof::idx(after[i].index) <= crate::node::proof::idx(after[j].index) by {
        assert(hash_dag_std::sort_model::keys_le(after[i], after[j], NodeAddress::sort_key));
    }
    assert forall|a: NodeAddress| before.contains(a) <==> after.contains(a) by {
        vstd::seq_lib::to_multiset_contains(before, a);
        vstd::seq_lib::to_multiset_contains(after, a);
    }
    assert(before.to_set() =~= after.to_set());
    vstd::seq_lib::to_multiset_len(before);
    vstd::seq_lib::to_multiset_len(after);
    if before.no_duplicates() {
        before.unique_seq_to_set();
        after.lemma_no_dup_set_cardinality();
    }
    if after.no_duplicates() {
        after.unique_seq_to_set();
        before.lemma_no_dup_set_cardinality();
    }
}
}
