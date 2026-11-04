//! 单层物理存储与 hashbrown 成员关系；不把节点唯一化作为外部假设。
use super::*;
use crate::node::Child;
use crate::{node::proof as node, state};
use hash_dag_std::table_model;
use vstd::prelude::*;

verus! {

pub(crate) open spec fn entry_edges(entry: PoolEntry) -> Seq<NodeIdx> {
    node::pointer_sequence(node::values(entry.values), node::mask(entry.mask))
}
pub(crate) open spec fn matched(pool: state::PoolState, entry: state::NodeModel) -> bool {
    exists|i: int| state::occupied(pool, i) && #[trigger] pool.nodes[i] == Some(entry)
}
pub(crate) open spec fn credited(before: state::PoolState, after: state::PoolState, edges: Seq<NodeIdx>) -> bool {
    &&& after.nodes == before.nodes && after.generations == before.generations
    &&& after.next_generation == before.next_generation && after.constructing == before.constructing
    &&& after.retained == before.retained && after.free == before.free
    &&& after.rcs.len() == before.rcs.len()
    &&& forall|i: int| 0 <= i < after.rcs.len() ==> #[trigger] after.rcs[i]
        == before.rcs[i] + state::occurrences(edges, node::at_index(i as u32))
}
pub(crate) open spec fn added(before: state::PoolState, after: state::PoolState, index: NodeIdx, amount:nat) -> bool {
    let i = node::idx(index);
    &&& after.nodes == before.nodes && after.generations == before.generations
    &&& after.next_generation == before.next_generation && after.constructing == before.constructing
    &&& after.retained == before.retained && after.free == before.free
    &&& after.rcs == before.rcs.update(i, (before.rcs[i] + amount) as usize)
    &&& after.rcs[i] == before.rcs[i] + amount
}

pub(crate) proof fn occurrences_concat<T>(a: Seq<T>, b: Seq<T>, value: T)
    ensures state::occurrences(a + b, value) == state::occurrences(a, value) + state::occurrences(b, value),
    decreases b.len(),
{
    reveal(Seq::filter);
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        occurrences_concat(a, b.drop_last(), value);
        assert((a + b).drop_last() =~= a + b.drop_last());
    }
}

pub(crate) proof fn occurrences_singleton<T>(item: T, value: T)
    ensures state::occurrences(seq![item], value) == if item == value { 1nat } else { 0nat },
{
    reveal_with_fuel(Seq::filter, 2);
}
pub(crate) proof fn occurrences_positive<T>(items: Seq<T>, value: T)
    requires items.contains(value),
    ensures state::occurrences(items,value) > 0,
{
    let i = choose|i:int| 0 <= i < items.len() && items[i] == value;
    items.lemma_filter_contains(|item:T| item == value, i);
}
pub(crate) proof fn occurrences_absent<T>(items: Seq<T>, value: T)
    requires !items.contains(value),
    ensures state::occurrences(items,value) == 0,
{
    items.lemma_all_neg_filter_empty(|item:T| item == value);
}

pub(super) proof fn pointer_suffix_occurrences(entry: PoolEntry, index: NodeIdx, start: nat)
    requires start <= 64,
    ensures state::occurrences(node::pointer_slots(node::values(entry.values), node::mask(entry.mask), start), index)
        == state::occurrences(node::children(entry).subrange(start as int, 64), Child::Ptr(index)),
    decreases 64 - start,
{
    let words = node::values(entry.values);
    let bits = node::mask(entry.mask);
    let children = node::children(entry);
    if start == 64 {
        reveal(Seq::filter);
        assert(children.subrange(64, 64) =~= Seq::empty());
    } else {
        pointer_suffix_occurrences(entry, index, start + 1);
        let tail = children.subrange((start + 1) as int, 64);
        assert(children.subrange(start as int, 64) =~= seq![children[start as int]] + tail);
        occurrences_concat(seq![children[start as int]], tail, Child::Ptr(index));
        occurrences_singleton(children[start as int], Child::Ptr(index));
        if bits & (1u64 << start) != 0 {
            let target = node::at_index(words[start as int]);
            assert(children[start as int] == Child::Ptr(target));
            let rest = node::pointer_slots(words, bits, start + 1);
            occurrences_concat(seq![target], rest, index);
            occurrences_singleton(target, index);
        } else {
            assert(children[start as int] is Leaf);
        }
    }
}

/// 物理指针枚举与 64 槽 Ptr 的精确重数相等；重复边不会被集合化。
pub(crate) proof fn pointer_occurrences(entry: PoolEntry, index: NodeIdx)
    ensures state::occurrences(entry_edges(entry), index)
        == state::occurrences(node::children(entry), Child::Ptr(index)),
{
    node::pointer_sequence_matches(node::values(entry.values), node::mask(entry.mask));
    pointer_suffix_occurrences(entry, index, 0);
    assert(node::children(entry).subrange(0, 64) =~= node::children(entry));
}

pub(crate) open spec fn decremented(before: state::PoolState, after: state::PoolState, index: NodeIdx) -> bool {
    let pos = node::idx(index);
    &&& state::live(before, pos)
    &&& after.nodes == if before.rcs[pos] == 1 { before.nodes.update(pos, None) } else { before.nodes }
    &&& after.rcs == before.rcs.update(pos, (before.rcs[pos] - 1) as usize)
    &&& after.generations == before.generations && after.next_generation == before.next_generation
    &&& after.constructing == before.constructing && after.retained == before.retained
    &&& after.free == if before.rcs[pos] == 1 { before.free.push(index) } else { before.free }
}

pub(crate) open spec fn interned(before: state::PoolState, after: state::PoolState, entry: state::NodeModel, rc: usize, result: InsertResult) -> bool {
    &&& state::valid_pool(after)
    &&& match result {
        InsertResult::Existing(index) => {
            let i = node::idx(index);
            &&& state::occupied(before, i) && before.nodes[i] == Some(entry)
            &&& after.nodes == before.nodes && after.generations == before.generations
            &&& after.next_generation == before.next_generation && after.constructing == before.constructing
            &&& after.retained == before.retained && after.free == before.free
            &&& after.rcs == before.rcs.update(i, (before.rcs[i] + rc) as usize)
            &&& after.rcs[i] == before.rcs[i] + rc
        },
        InsertResult::New(index) => {
            let i = node::idx(index);
            &&& !state::occupied(before, i) && state::occupied(after, i) && after.nodes[i] == Some(entry)
            &&& before.nodes.len() <= after.nodes.len() <= before.nodes.len() + 1
            &&& after.nodes.len() == before.nodes.len() + if before.free.len() == 0 { 1int } else { 0int }
            &&& after.rcs[i] == rc + if before.constructing { 1int } else { 0int }
            &&& after.generations[i] == before.next_generation + 1 && after.next_generation == before.next_generation + 1
            &&& after.constructing == before.constructing
            &&& after.retained == if before.constructing { before.retained.push(index) } else { before.retained }
            &&& after.free == if before.free.len() == 0 { before.free } else { before.free.drop_last() }
            &&& (before.free.len() == 0 ==> i == before.nodes.len())
            &&& (before.free.len() > 0 ==> index == before.free.last())
            &&& forall|j: int| #![trigger after.nodes[j]] 0 <= j < before.nodes.len() && j != i ==>
                after.nodes[j] == before.nodes[j] && after.rcs[j] == before.rcs[j] && after.generations[j] == before.generations[j]
        },
    }
}

pub(super) open spec fn row(values: Seq<NodeValues>, masks: Seq<NodeMask>, index: NodeIdx) -> state::NodeModel {
    node::children(PoolEntry { values: values[node::idx(index)], mask: masks[node::idx(index)] })
}

pub(super) open spec fn key_matches(values: Seq<NodeValues>, masks: Seq<NodeMask>, index: NodeIdx, entry: PoolEntry) -> bool {
    0 <= node::idx(index) < values.len() && node::idx(index) < masks.len()
        && row(values, masks, index) == node::children(entry)
}

pub(super) open spec fn hash_at(builder: DefaultHashBuilder, values: Seq<NodeValues>, masks: Seq<NodeMask>, index: NodeIdx) -> u64 {
    if 0 <= node::idx(index) < values.len() && node::idx(index) < masks.len() {
        table_model::hash(builder, node::values(values[node::idx(index)]), node::mask_words(masks[node::idx(index)]))
    } else { 0 }
}

impl Pool {
    pub(crate) proof fn edge_domain_model(&self, entry: PoolEntry)
        requires self.valid(), forall|i:int| 0 <= i < 64 ==> match #[trigger] node::children(entry)[i] {
            Child::Leaf(_) => true, Child::Ptr(index) => state::occupied(self.state(), node::idx(index)),
        },
        ensures self.edge_domain(entry_edges(entry)),
    {
        let edges = entry_edges(entry);
        let row = node::children(entry);
        assert forall|i:int| 0 <= i < edges.len() implies self.count_domain(node::idx(#[trigger] edges[i])) by {
            let key = edges[i];
            occurrences_positive(edges, key);
            assert(state::occurrences(edges, key) > 0);
            pointer_occurrences(entry, key);
            if !row.contains(Child::Ptr(key)) {
                occurrences_absent(row, Child::Ptr(key));
                assert(false);
            }
            let slot = choose|slot:int| 0 <= slot < 64 && row[slot] == Child::Ptr(key);
            assert(state::occupied(self.state(), node::idx(key)));
        }
    }
    pub(crate) proof fn valid_model(&self)
        requires self.valid(),
        ensures state::valid_pool(self.state()), self.counts() == self.state().rcs,
    {}
    pub(crate) proof fn live_model(&self, index:int)
        requires self.valid(), state::live(self.state(),index),
        ensures self.count_domain(index), self.counts()[index] > 0,
            0 <= index < self.counts().len(),
    {}
    pub(super) proof fn free_room(&self, index: NodeIdx)
        requires self.valid(), state::occupied(self.state(), node::idx(index)),
        ensures self.free_list@.len() < hash_dag_std::vec_model::capacity(self.free_list),
    {
        assert(table_model::entries(self.table).contains_key(index));
        table_model::entries(self.table).lemma_remove_key_len(index);
    }
    pub(crate) open(super) spec fn count_domain(&self, index: int) -> bool {
        0 <= index < self.state().nodes.len() ==> state::occupied(self.state(), index)
    }

    pub(crate) open(super) spec fn count_change(&self, before: &Self, index: int, amount: nat) -> bool {
        self.same_storage(before) && self.rcs@ == before.rcs@.update(index, (before.rcs@[index] + amount) as usize)
    }

    pub(crate) open(super) spec fn counts(&self) -> Seq<usize> { self.rcs@ }

    pub(crate) open(super) spec fn edge_domain(&self, edges: Seq<NodeIdx>) -> bool {
        forall|i: int| 0 <= i < edges.len() ==> self.count_domain(node::idx(#[trigger] edges[i]))
    }

    pub(crate) open(super) spec fn credits(&self, before: &Self, edges: Seq<NodeIdx>) -> bool {
        self.same_storage(before) && self.rcs@.len() == before.rcs@.len()
            && forall|i: int| 0 <= i < self.rcs@.len() ==> #[trigger] self.rcs@[i]
                == before.rcs@[i] + state::occurrences(edges, node::at_index(i as u32))
    }
    pub(crate) open(super) spec fn payload_present(&self, index: int) -> bool {
        0 <= index < self.values@.len() && index < self.masks@.len()
    }

    pub(crate) open(super) spec fn readable(&self, index: int) -> bool {
        0 <= index < self.values@.len() && index < self.masks@.len()
            && index < self.rcs@.len() && self.rcs@[index] > 0
    }

    pub(crate) open(super) spec fn read_nodes(&self) -> Seq<Option<state::NodeModel>> {
        Seq::new(self.values@.len(), |i: int| if self.readable(i) { Some(self.raw_node(i)) } else { None })
    }

    pub(crate) proof fn read_node_at(&self, index: int)
        ensures self.readable(index) <==> 0 <= index < self.read_nodes().len() && self.read_nodes()[index] is Some,
            index < self.read_nodes().len() && index >= 0 ==>
            self.read_nodes()[index] == if self.readable(index) { Some(self.raw_node(index)) } else { None },
    {}

    pub(crate) proof fn read_model(&self, index: int)
        requires self.valid(),
        ensures self.readable(index) <==> state::live(self.state(), index),
            state::occupied(self.state(), index) ==> self.state().nodes[index] == Some(self.raw_node(index)),
    {
        if self.readable(index) {
            assert(!self.free_list@.contains(node::at_index(index as u32)));
        }
    }

    pub(crate) proof fn read_projection(&self)
        requires self.valid(),
        ensures self.read_nodes().len() == self.state().nodes.len(),
            self.read_nodes().len() <= usize::MAX,
            forall|i: int| 0 <= i < self.read_nodes().len() ==>
                #[trigger] self.read_nodes()[i] == if state::live(self.state(), i) { self.state().nodes[i] } else { None },
    {
        assert forall|i: int| 0 <= i < self.read_nodes().len() implies
            #[trigger] self.read_nodes()[i] == if state::live(self.state(), i) { self.state().nodes[i] } else { None } by {
            self.read_node_at(i);
            self.read_model(i);
        }
    }

    pub(crate) open(super) spec fn raw_node(&self, index: int) -> state::NodeModel {
        node::children(PoolEntry { values: self.values@[index], mask: self.masks@[index] })
    }
    pub(crate) open(super) spec fn raw_arrays(&self)->(Seq<NodeValues>,Seq<NodeMask>) {(self.values@,self.masks@)}
    pub(crate) open(super) spec fn raw_values(&self, index: int) -> Seq<u32> { node::values(self.values@[index]) }
    pub(crate) open(super) spec fn raw_mask(&self, index: int) -> Seq<u32> { node::mask_words(self.masks@[index]) }
    pub(crate) open(super) spec fn raw_generation(&self, index: int) -> Option<u64> {
        if 0 <= index < self.rcs@.len() && index < self.generations@.len() && self.rcs@[index] > 0 {
            Some(self.generations@[index])
        } else { None }
    }

    pub(crate) open(super) spec fn state(&self) -> state::PoolState {
        state::PoolState {
            nodes: Seq::new(self.values@.len(), |i: int|
                if self.free_list@.contains(node::at_index(i as u32)) { None } else { Some(self.raw_node(i)) }),
            rcs: self.rcs@,
            generations: self.generations@,
            next_generation: self.next_generation,
            constructing: self.constructing,
            retained: self.retained@,
            free: self.free_list@,
        }
    }

    pub(crate) open(super) spec fn valid(&self) -> bool {
        &&& self.values@.len() == self.masks@.len() == self.rcs@.len() == self.generations@.len()
        &&& hash_dag_std::vec_model::capacity(self.free_list) >= self.values@.len()
        &&& self.free_list@.len() + table_model::entries(self.table).len() == self.values@.len()
        &&& state::valid_pool(self.state())
        &&& forall|index: NodeIdx| #[trigger] table_model::entries(self.table).contains_key(index)
            <==> state::occupied(self.state(), node::idx(index))
        &&& forall|index: NodeIdx| #[trigger] table_model::entries(self.table).contains_key(index) ==>
            table_model::entries(self.table)[index] == table_model::hash(self.hash_builder,
                node::values(self.values@[node::idx(index)]), node::mask_words(self.masks@[node::idx(index)]))
    }

    pub(crate) open(super) spec fn same_storage(&self, before: &Self) -> bool {
        self.values@ == before.values@ && self.masks@ == before.masks@
            && self.generations@ == before.generations@ && self.free_list@ == before.free_list@
            && self.retained@ == before.retained@ && self.constructing == before.constructing
            && self.next_generation == before.next_generation
            && table_model::entries(self.table) == table_model::entries(before.table)
            && self.hash_builder == before.hash_builder
            && hash_dag_std::vec_model::capacity(self.free_list) == hash_dag_std::vec_model::capacity(before.free_list)
    }

    pub(crate) open(super) spec fn same_data(&self, before: &Self) -> bool {
        self.values@ == before.values@ && self.masks@ == before.masks@ && self.rcs@ == before.rcs@
            && self.generations@ == before.generations@ && self.free_list@ == before.free_list@
            && self.retained@ == before.retained@ && self.constructing == before.constructing
            && self.next_generation == before.next_generation
            && table_model::entries(self.table) == table_model::entries(before.table)
            && self.hash_builder == before.hash_builder
    }
}

pub(crate) proof fn credits_model(before: &Pool, after: &Pool, edges: Seq<NodeIdx>)
    requires after.credits(before, edges),
    ensures credited(before.state(), after.state(), edges),
{
    assert(after.state().nodes == before.state().nodes);
}

pub(crate) proof fn unchanged_model(before: &Pool, after: &Pool)
    requires after.same_storage(before), after.counts() == before.counts(),
    ensures after.state() == before.state(),
{
    assert(after.state().nodes == before.state().nodes);
}

pub(super) proof fn reserved(before: &Pool, after: &Pool)
    requires before.valid(), after.same_data(before),
        hash_dag_std::vec_model::capacity(after.free_list) >= hash_dag_std::vec_model::capacity(before.free_list),
    ensures after.valid(), after.state() == before.state(),
{
    assert(after.state().nodes =~= before.state().nodes);
    assert(after.state() == before.state());
}

pub(super) proof fn empty_pool(pool: &Pool)
    requires pool.values@.len() == 0, pool.masks@.len() == 0, pool.rcs@.len() == 0,
        pool.generations@.len() == 0, pool.free_list@.len() == 0, pool.retained@.len() == 0,
        !pool.constructing, pool.next_generation == 0,
    ensures pool.state() == (state::PoolState {
        nodes: Seq::empty(), rcs: Seq::empty(), generations: Seq::empty(), next_generation: 0,
        constructing: false, retained: Seq::empty(), free: Seq::empty(),
    }),
{
    assert(pool.state().nodes =~= Seq::empty());
    assert(pool.rcs@ =~= Seq::empty());
    assert(pool.generations@ =~= Seq::empty());
    assert(pool.free_list@ =~= Seq::empty());
    assert(pool.retained@ =~= Seq::empty());
}

pub(super) proof fn metadata(before: &Pool, after: &Pool)
    requires before.valid(), after.values@ == before.values@, after.masks@ == before.masks@,
        after.rcs@ == before.rcs@, after.generations@ == before.generations@,
        after.next_generation == before.next_generation, after.free_list@ == before.free_list@,
        after.retained@.is_prefix_of(before.retained@), after.hash_builder == before.hash_builder,
        table_model::entries(after.table) == table_model::entries(before.table),
        hash_dag_std::vec_model::capacity(after.free_list) == hash_dag_std::vec_model::capacity(before.free_list),
    ensures after.valid(), after.state() == (state::PoolState {
        constructing: after.constructing, retained: after.retained@, ..before.state()
    }),
{
    assert(after.state().nodes == before.state().nodes);
    assert(after.state() == (state::PoolState { constructing: after.constructing, retained: after.retained@, ..before.state() }));
    assert(after.retained@.no_duplicates());
    assert forall|i: int| 0 <= i < after.retained@.len() implies state::live(after.state(), node::idx(after.retained@[i])) by {
        assert(after.retained@[i] == before.retained@[i]);
        assert(state::live(before.state(), node::idx(before.retained@[i])));
    }
    assert forall|i: int, j: int| state::occupied(after.state(), i) && state::occupied(after.state(), j)
        && after.state().nodes[i] == after.state().nodes[j] implies i == j by {
        assert(state::occupied(before.state(), i)); assert(state::occupied(before.state(), j));
    }
    assert forall|i: int, j: int| state::occupied(after.state(), i) && state::occupied(after.state(), j)
        && after.state().generations[i] == after.state().generations[j] implies i == j by {
        assert(state::occupied(before.state(), i)); assert(state::occupied(before.state(), j));
    }
    assert(state::valid_pool(after.state()));
}

pub(super) proof fn existing(before: &Pool, after: &Pool, entry: PoolEntry, rc: usize, index: NodeIdx)
    requires before.valid(), after.valid(), state::occupied(before.state(), node::idx(index)),
        before.raw_node(node::idx(index)) == node::children(entry),
        after.count_change(before, node::idx(index), rc as nat),
        before.rcs@[node::idx(index)] + rc <= usize::MAX,
    ensures interned(before.state(), after.state(), node::children(entry), rc, InsertResult::Existing(index)),
        matched(before.state(), node::children(entry)),
{
    before.read_model(node::idx(index));
    assert(after.state().nodes == before.state().nodes);
    assert(before.state().nodes[node::idx(index)] == Some(node::children(entry)));
}

pub(super) proof fn no_matching_node<F: Fn(&NodeIdx) -> bool>(before: &Pool, entry: PoolEntry, query: u64, predicate: F)
    requires before.valid(), query == table_model::hash(before.hash_builder, node::values(entry.values), node::mask_words(entry.mask)),
        table_model::found(table_model::entries(before.table), query, predicate, None),
        forall|key: NodeIdx, result: bool| predicate.ensures((&key,), result) ==>
            result == key_matches(before.values@, before.masks@, key, entry),
    ensures forall|i: int| #![trigger before.state().nodes[i]] state::occupied(before.state(), i) ==>
        before.state().nodes[i] != Some(node::children(entry)),
{
    assert forall|i: int| #![trigger before.state().nodes[i]] state::occupied(before.state(), i) implies
        before.state().nodes[i] != Some(node::children(entry)) by {
        let key = node::at_index(i as u32);
        assert(node::idx(key) == i);
        before.read_model(i);
        assert(table_model::entries(before.table).contains_key(key));
        if before.state().nodes[i] == Some(node::children(entry)) {
            let own = PoolEntry { values: before.values@[i], mask: before.masks@[i] };
            node::entries_same(own, entry);
            assert(table_model::entries(before.table)[key] == query);
            assert(predicate.ensures((&key,), false));
        }
    }
}

pub(super) proof fn count_updated(before: &Pool, after: &Pool, index: int, amount: nat)
    requires before.valid(), 0 <= index < before.rcs@.len(), state::occupied(before.state(), index),
        before.rcs@[index] + amount <= usize::MAX,
        after.count_change(before, index, amount),
    ensures after.valid(), after.state().nodes == before.state().nodes,
        after.state().generations == before.state().generations,
{
    count_replaced(before, after, index, (before.rcs@[index] + amount) as usize);
}

pub(super) proof fn count_replaced(before: &Pool, after: &Pool, index: int, replacement: usize)
    requires before.valid(), 0 <= index < before.rcs@.len(), state::occupied(before.state(), index),
        before.rcs@[index] > 0 ==> replacement > 0,
        after.same_storage(before), after.rcs@ == before.rcs@.update(index, replacement),
    ensures after.valid(), after.state().nodes == before.state().nodes,
        after.state().generations == before.state().generations,
{
    assert(state::valid_pool(before.state()));
    assert(after.state().nodes == before.state().nodes);
    assert(after.state().free == before.state().free);
    assert(after.state().retained == before.state().retained);
    assert(after.state().generations == before.state().generations);
    assert(after.state().next_generation == before.state().next_generation);
    assert(after.state().nodes.len() == after.state().rcs.len() == after.state().generations.len());
    assert(after.state().nodes.len() <= u32::MAX as int + 1);
    assert(after.state().free.no_duplicates());
    assert(after.state().retained.no_duplicates());
    assert forall|i: int| #![trigger after.state().nodes[i]] 0 <= i < after.state().nodes.len() implies
        (after.state().nodes[i] is None <==> after.state().free.contains(node::at_index(i as u32)))
        && (after.state().nodes[i] is None ==> after.state().rcs[i] == 0)
        && after.state().generations[i] <= after.state().next_generation
        && (after.state().nodes[i] is Some ==> after.state().nodes[i]->Some_0.len() == 64
            && 0 < after.state().generations[i]) by {
        assert(0 <= i < before.state().nodes.len());
        assert(before.raw_node(i).len() == 64);
        before.read_model(i);
        assert(before.state().generations[i] <= before.state().next_generation);
        assert(before.state().nodes[i] is None ==> before.state().rcs[i] == 0);
        assert(before.state().nodes[i] is Some ==> before.state().nodes[i]->Some_0.len() == 64
            && 0 < before.state().generations[i] <= before.state().next_generation);
        if i == index { assert(after.state().nodes[i] is Some); }
    }
    assert forall|i: int| #![trigger after.state().free[i]] 0 <= i < after.state().free.len() implies
        0 <= node::idx(after.state().free[i]) < after.state().nodes.len() by {
        assert(after.state().free[i] == before.state().free[i]);
    }
    assert forall|i: int, j: int| state::occupied(after.state(), i) && state::occupied(after.state(), j)
        && after.state().nodes[i] == after.state().nodes[j] implies i == j by {
        assert(state::occupied(before.state(), i)); assert(state::occupied(before.state(), j));
    }
    assert forall|i: int, j: int| state::occupied(after.state(), i) && state::occupied(after.state(), j)
        && after.state().generations[i] == after.state().generations[j] implies i == j by {
        assert(state::occupied(before.state(), i)); assert(state::occupied(before.state(), j));
    }
    assert forall|i: int| 0 <= i < after.retained@.len() implies
        state::live(after.state(), node::idx(after.retained@[i])) by {
        let k = node::idx(after.retained@[i]);
        assert(state::live(before.state(), k));
    }
    assert(state::valid_pool(after.state()));
    assert(after.valid());
}

pub(super) proof fn removed_node(before: &Pool, after: &Pool, index: NodeIdx)
    requires before.valid(), state::live(before.state(), node::idx(index)),
        before.rcs@[node::idx(index)] == 1, !before.retained@.contains(index),
        after.values@ == before.values@, after.masks@ == before.masks@,
        after.generations@ == before.generations@, after.next_generation == before.next_generation,
        after.constructing == before.constructing, after.retained@ == before.retained@,
        after.hash_builder == before.hash_builder,
        table_model::entries(after.table) == table_model::entries(before.table).remove(index),
        after.rcs@ == before.rcs@.update(node::idx(index), 0),
        after.free_list@ == before.free_list@.push(index),
        hash_dag_std::vec_model::capacity(after.free_list) == hash_dag_std::vec_model::capacity(before.free_list),
    ensures after.valid(), decremented(before.state(), after.state(), index),
{
    broadcast use vstd::seq_lib::lemma_seq_contains_after_push;
    let pos = node::idx(index);
    assert(!before.free_list@.contains(index));
    assert(after.free_list@.no_duplicates());
    assert forall|i: int| 0 <= i < after.state().nodes.len() implies
        #[trigger] after.state().nodes[i] == before.state().nodes.update(pos, None)[i] by {
        assert(node::idx(node::at_index(i as u32)) == i);
        assert(after.raw_node(i) == before.raw_node(i));
        let key = node::at_index(i as u32);
        if i == pos {
            assert(key == index); assert(after.free_list@.contains(key));
            assert(after.state().nodes[i] is None);
        } else {
            assert(key != index);
            assert(after.free_list@.contains(key) <==> before.free_list@.contains(key));
            if before.free_list@.contains(key) {
                assert(after.state().nodes[i] is None); assert(before.state().nodes[i] is None);
            } else {
                assert(after.state().nodes[i] == Some(after.raw_node(i)));
                assert(before.state().nodes[i] == Some(before.raw_node(i)));
            }
        }
    }
    assert(after.state().nodes =~= before.state().nodes.update(pos, None));
    assert(decremented(before.state(), after.state(), index));
    assert(state::valid_pool(before.state()));
    assert forall|i: int| #![trigger after.state().free[i]] 0 <= i < after.state().free.len() implies
        0 <= node::idx(after.state().free[i]) < after.state().nodes.len() by {
        if i < before.free_list@.len() { assert(after.free_list@[i] == before.free_list@[i]); }
        else { assert(after.free_list@[i] == index); }
    }
    assert forall|i: int| #![trigger after.state().nodes[i]] 0 <= i < after.state().nodes.len() implies
        (after.state().nodes[i] is None <==> after.state().free.contains(node::at_index(i as u32)))
        && (after.state().nodes[i] is None ==> after.state().rcs[i] == 0)
        && after.state().generations[i] <= after.state().next_generation
        && (after.state().nodes[i] is Some ==> after.state().nodes[i]->Some_0.len() == 64 && 0 < after.state().generations[i]) by {
        before.read_model(i);
        assert(before.state().generations[i] <= before.state().next_generation);
        assert(before.state().nodes[i] is Some ==> before.state().nodes[i]->Some_0.len() == 64 && 0 < before.state().generations[i]);
    }
    assert forall|i: int| 0 <= i < after.retained@.len() implies state::live(after.state(), node::idx(after.retained@[i])) by {
        let key = after.retained@[i];
        assert(key != index); assert(state::live(before.state(), node::idx(key)));
    }
    assert forall|i: int, j: int| state::occupied(after.state(), i) && state::occupied(after.state(), j)
        && after.state().nodes[i] == after.state().nodes[j] implies i == j by {
        assert(state::occupied(before.state(), i)); assert(state::occupied(before.state(), j));
    }
    assert forall|i: int, j: int| state::occupied(after.state(), i) && state::occupied(after.state(), j)
        && after.state().generations[i] == after.state().generations[j] implies i == j by {
        assert(state::occupied(before.state(), i)); assert(state::occupied(before.state(), j));
    }
    assert(state::valid_pool(after.state()));
    table_model::entries(before.table).lemma_remove_key_len(index);
    assert forall|key: NodeIdx| #[trigger] table_model::entries(after.table).contains_key(key)
        <==> state::occupied(after.state(), node::idx(key)) by {
        assert(table_model::entries(before.table).contains_key(key) <==> state::occupied(before.state(), node::idx(key)));
    }
    assert forall|key: NodeIdx| #[trigger] table_model::entries(after.table).contains_key(key) implies
        table_model::entries(after.table)[key] == table_model::hash(after.hash_builder,
            node::values(after.values@[node::idx(key)]), node::mask_words(after.masks@[node::idx(key)])) by {
        assert(table_model::entries(before.table).contains_key(key));
    }
}

pub(super) proof fn allocated(before: &Pool, after: &Pool, entry: PoolEntry, rc: usize, index: NodeIdx)
    requires before.valid(), before.next_generation < u64::MAX,
        after.values@.len() == if before.free_list@.len() == 0 { before.values@.len() + 1 } else { before.values@.len() },
        after.values@.len() == after.masks@.len() == after.rcs@.len() == after.generations@.len(),
        0 <= node::idx(index) < after.values@.len(),
        before.free_list@.len() == 0 ==> node::idx(index) == before.values@.len(),
        before.free_list@.len() > 0 ==> index == before.free_list@.last(),
        after.free_list@ == if before.free_list@.len() == 0 { before.free_list@ } else { before.free_list@.drop_last() },
        after.values@[node::idx(index)] == entry.values, after.masks@[node::idx(index)] == entry.mask,
        after.rcs@[node::idx(index)] == rc + if before.constructing { 1int } else { 0int },
        after.generations@[node::idx(index)] == before.next_generation + 1,
        after.next_generation == before.next_generation + 1, after.constructing == before.constructing,
        after.retained@ == if before.constructing { before.retained@.push(index) } else { before.retained@ },
        forall|j: int| #![trigger after.values@[j]] 0 <= j < before.values@.len() && j != node::idx(index) ==>
            after.values@[j] == before.values@[j] && after.masks@[j] == before.masks@[j]
                && after.rcs@[j] == before.rcs@[j] && after.generations@[j] == before.generations@[j],
        after.hash_builder == before.hash_builder,
        table_model::entries(after.table) == table_model::entries(before.table).insert(index,
            table_model::hash(before.hash_builder, node::values(entry.values), node::mask_words(entry.mask))),
        hash_dag_std::vec_model::capacity(after.free_list) >= after.values@.len(),
        forall|j: int| #![trigger before.state().nodes[j]] state::occupied(before.state(), j) ==>
            before.state().nodes[j] != Some(node::children(entry)),
    ensures after.valid(), interned(before.state(), after.state(), node::children(entry), rc, InsertResult::New(index)),
{
    broadcast use vstd::seq_lib::lemma_seq_contains_after_push;
    let pos = node::idx(index);
    if before.free_list@.len() > 0 {
        let last = before.free_list@.len() - 1;
        assert(before.free_list@.contains(index));
        assert(0 <= pos < before.values@.len());
        assert(before.free_list@ =~= after.free_list@.push(index));
        assert forall|i: int, j: int| 0 <= i < after.free_list@.len() && 0 <= j < after.free_list@.len() && i != j
            implies after.free_list@[i] != after.free_list@[j] by {
            assert(before.free_list@[i] != before.free_list@[j]);
        }
        assert(!after.free_list@.contains(index)) by {
            if after.free_list@.contains(index) {
                let i = choose|i: int| 0 <= i < after.free_list@.len() && after.free_list@[i] == index;
                assert(before.free_list@[i] != before.free_list@[last]);
            }
        }
    } else {
        assert(after.free_list@.len() == 0);
    }
    assert(!state::occupied(before.state(), pos));
    assert(!table_model::entries(before.table).contains_key(index));
    assert(after.values@.len() <= u32::MAX as int + 1);
    assert forall|i: int| #![trigger after.state().nodes[i]] 0 <= i < after.state().nodes.len() implies
        after.state().nodes[i] == if i == pos { Some(node::children(entry)) } else { before.state().nodes[i] } by {
        let key = node::at_index(i as u32);
        assert(node::idx(key) == i);
        if i == pos {
            assert(key == index);
            assert(!after.free_list@.contains(key));
        } else {
            assert(i < before.values@.len());
            assert(key != index);
            assert(after.free_list@.contains(key) <==> before.free_list@.contains(key));
            assert(after.raw_node(i) == before.raw_node(i));
        }
    }
    assert(after.state().nodes[pos] == Some(node::children(entry)));
    assert(!before.retained@.contains(index)) by {
        if before.retained@.contains(index) {
            let i = choose|i: int| 0 <= i < before.retained@.len() && before.retained@[i] == index;
            assert(state::live(before.state(), node::idx(before.retained@[i])));
        }
    }
    assert(after.retained@.no_duplicates());
    assert forall|i: int| #![trigger after.state().free[i]] 0 <= i < after.state().free.len() implies
        0 <= node::idx(after.state().free[i]) < after.state().nodes.len() by {
        assert(after.free_list@[i] == before.free_list@[i]);
    }
    assert forall|i: int| #![trigger after.state().nodes[i]] 0 <= i < after.state().nodes.len() implies
        (after.state().nodes[i] is None <==> after.state().free.contains(node::at_index(i as u32)))
        && (after.state().nodes[i] is None ==> after.state().rcs[i] == 0)
        && after.state().generations[i] <= after.state().next_generation
        && (after.state().nodes[i] is Some ==> after.state().nodes[i]->Some_0.len() == 64 && 0 < after.state().generations[i]) by {
        if i != pos {
            assert(after.values@[i] == before.values@[i]);
            assert(after.masks@[i] == before.masks@[i]);
            assert(after.rcs@[i] == before.rcs@[i]);
            assert(after.generations@[i] == before.generations@[i]);
            before.read_model(i);
            assert(before.state().generations[i] <= before.state().next_generation);
            assert(before.state().nodes[i] is Some ==> before.state().nodes[i]->Some_0.len() == 64 && 0 < before.state().generations[i]);
        } else {
            assert(node::children(entry).len() == 64);
            assert(after.state().generations[i] == before.next_generation + 1);
        }
    }
    assert forall|i: int| 0 <= i < after.retained@.len() implies state::live(after.state(), node::idx(after.retained@[i])) by {
        if i < before.retained@.len() {
            let key = before.retained@[i];
            assert(state::live(before.state(), node::idx(key)));
            assert(node::idx(key) != pos);
        } else { assert(after.retained@[i] == index); }
    }
    assert forall|i: int, j: int| state::occupied(after.state(), i) && state::occupied(after.state(), j)
        && after.state().nodes[i] == after.state().nodes[j] implies i == j by {
        if i != pos { assert(state::occupied(before.state(), i)); }
        if j != pos { assert(state::occupied(before.state(), j)); }
    }
    assert forall|i: int, j: int| state::occupied(after.state(), i) && state::occupied(after.state(), j)
        && after.state().generations[i] == after.state().generations[j] implies i == j by {
        if i != pos {
            assert(state::occupied(before.state(), i));
            assert(before.state().generations[i] <= before.state().next_generation);
        }
        if j != pos {
            assert(state::occupied(before.state(), j));
            assert(before.state().generations[j] <= before.state().next_generation);
        }
    }
    assert(state::valid_pool(after.state()));
    table_model::entries(after.table).lemma_remove_key_len(index);
    assert(table_model::entries(after.table).remove(index) =~= table_model::entries(before.table));
    assert forall|key: NodeIdx| #[trigger] table_model::entries(after.table).contains_key(key)
        <==> state::occupied(after.state(), node::idx(key)) by {
        assert(table_model::entries(before.table).contains_key(key) <==> state::occupied(before.state(), node::idx(key)));
    }
    assert forall|key: NodeIdx| #[trigger] table_model::entries(after.table).contains_key(key) implies
        table_model::entries(after.table)[key] == table_model::hash(after.hash_builder,
            node::values(after.values@[node::idx(key)]), node::mask_words(after.masks@[node::idx(key)])) by {
        if key != index { assert(table_model::entries(before.table).contains_key(key)); }
    }
    assert(after.valid());
    assert forall|j: int| #![trigger after.state().nodes[j]] 0 <= j < before.state().nodes.len() && j != pos implies
        after.state().nodes[j] == before.state().nodes[j] && after.state().rcs[j] == before.state().rcs[j]
            && after.state().generations[j] == before.state().generations[j] by {
        assert(after.values@[j] == before.values@[j]);
        assert(after.rcs@[j] == before.rcs@[j]);
        assert(after.generations@[j] == before.generations@[j]);
    }
    let old_state=before.state();let new_state=after.state();
    assert(!state::occupied(old_state,pos) && state::occupied(new_state,pos));
    assert(new_state.nodes[pos]==Some(node::children(entry)));
    assert(old_state.nodes.len()<=new_state.nodes.len()<=old_state.nodes.len()+1);
    assert(new_state.nodes.len()==old_state.nodes.len()+if old_state.free.len()==0{1int}else{0int});
    assert(new_state.rcs[pos]==rc+if old_state.constructing{1int}else{0int});
    assert(new_state.generations[pos]==old_state.next_generation+1 && new_state.next_generation==old_state.next_generation+1);
    assert(new_state.constructing==old_state.constructing);
    assert(new_state.retained==if old_state.constructing{old_state.retained.push(index)}else{old_state.retained});
    assert(new_state.free==if old_state.free.len()==0{old_state.free}else{old_state.free.drop_last()});
    assert(old_state.free.len()==0 ==> pos==old_state.nodes.len());
    assert(old_state.free.len()>0 ==> index==old_state.free.last());
    assert(interned(old_state,new_state,node::children(entry),rc,InsertResult::New(index)));
}

}
