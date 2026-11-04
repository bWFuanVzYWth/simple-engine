use hashbrown::{DefaultHashBuilder, HashTable};

use crate::hashdag::Error;
use crate::node::{NodeIdx, NodeMask, NodeRef, NodeValues, PoolEntry};
#[cfg(not(verus_only))]
use crate::table_ops as table;
#[cfg(not(verus_only))]
use crate::vec_ops;
#[cfg(verus_only)]
use hash_dag_std::table_ops as table;
#[cfg(verus_only)]
use hash_dag_std::vec_ops;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
#[path = "proof/pool.rs"]
pub(crate) mod proof;

#[path = "pool_edges.rs"]
mod edges;

/// 节点 payload、类型 mask、引用计数采用分离数组；前两者可直接复制到设备。
/// 槽位稳定到回收复用，root 引用和构造持有负责生命周期，索引不是永久内容身份。
/// table 只包含有效节点，回收先移出 table；零引用的内部构建节点也可去重。
/// 公开写入的新节点额外保留一份构造引用，retained 只记录首次插入的节点。
#[derive(Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Pool {
    table: HashTable<NodeIdx>,
    values: Vec<NodeValues>,
    masks: Vec<NodeMask>,
    rcs: Vec<usize>,
    generations: Vec<u64>,
    next_generation: u64,
    free_list: Vec<NodeIdx>,
    constructing: bool,
    retained: Vec<NodeIdx>,
    hash_builder: DefaultHashBuilder,
    #[cfg(test)]
    hash_mask: u64,
}

#[derive(Debug)]
#[must_use]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum InsertResult {
    New(NodeIdx),
    Existing(NodeIdx),
}

#[cfg_attr(verus_only, verus_verify)]
impl Pool {
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is Some <==> crate::node::proof::idx(idx) < values@.len() && crate::node::proof::idx(idx) < masks@.len(),
            result is Some ==> *result->Some_0.values == values@[crate::node::proof::idx(idx)]
                && result->Some_0.mask == masks@[crate::node::proof::idx(idx)],
        no_unwind
    ))]
    fn lookup_parts<'a>(
        values: &'a [NodeValues],
        masks: &[NodeMask],
        idx: NodeIdx,
    ) -> Option<NodeRef<'a>> {
        let pos = match idx.as_usize() {
            Ok(pos) => pos,
            Err(_) => return None,
        };
        if pos >= values.len() || pos >= masks.len() {
            return None;
        }
        Some(NodeRef {
            values: &values[pos],
            mask: masks[pos],
        })
    }

    /// 独占局部树整体重建；没有外部节点引用，不执行逐边回收，保留所有容器容量。
    #[cfg_attr(verus_only, verus_spec(
        ensures final(self).valid(), final(self).state().nodes.len() == 0,
            final(self).state().next_generation == old(self).state().next_generation,
    ))]
    pub(crate) fn clear_local(&mut self) {
        table::clear(&mut self.table);
        self.values.clear();
        self.masks.clear();
        self.rcs.clear();
        self.generations.clear();
        vec_ops::clear(&mut self.free_list);
        self.retained.clear();
        self.constructing = false;
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result.valid(), result.state() == (crate::state::PoolState {
            nodes: Seq::empty(), rcs: Seq::empty(), generations: Seq::empty(), next_generation: 0,
            constructing: false, retained: Seq::empty(), free: Seq::empty(),
        }),
    ))]
    pub(crate) fn new() -> Self {
        let pool = Self {
            table: table::new(),
            values: Vec::new(),
            masks: Vec::new(),
            rcs: Vec::new(),
            generations: Vec::new(),
            next_generation: 0,
            free_list: Vec::new(),
            constructing: false,
            retained: Vec::new(),
            hash_builder: table::builder(),
            #[cfg(test)]
            hash_mask: u64::MAX,
        };
        #[cfg(verus_only)]
        proof! { proof::empty_pool(&pool); }
        pool
    }

    /// 唯一化完整节点；哈希只筛选候选，完整比较决定相等。
    /// 仅新节点认领子边，认领失败时不得发布节点或改变 free list。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
            match children { Some(pool) => pool.valid() && pool.edge_domain(proof::entry_edges(*entry)), None => true },
        ensures final(self).valid(),
            result is Err ==> final(self).state() == old(self).state(),
            result is Ok ==> proof::interned(old(self).state(), final(self).state(), crate::node::proof::children(*entry), rc, result->Ok_0),
            result is Ok ==> (result->Ok_0 is Existing <==> proof::matched(old(self).state(), crate::node::proof::children(*entry))),
            match children {
                Some(pool) => final(pool).valid() && final(pool).same_storage(old(children->Some_0))
                    && if result is Ok && result->Ok_0 is New { final(pool).credits(old(children->Some_0), proof::entry_edges(*entry)) }
                        else { final(pool).counts() == old(children->Some_0).counts() },
                None => true,
            },
    ))]
    pub(crate) fn intern(
        &mut self,
        rc: usize,
        entry: &PoolEntry,
        children: Option<&mut Pool>,
    ) -> Result<InsertResult, Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; let ghost requested = rc; }
        let hash = table::hash(&self.hash_builder, entry.values.words(), entry.mask.words());
        #[cfg(test)]
        let hash = hash & self.hash_mask;
        let values = self.values.as_slice();
        let masks = self.masks.as_slice();
        let predicate = #[cfg_attr(verus_only, verus_spec(result: bool =>
            ensures result == proof::key_matches(values@, masks@, *index, *entry)
        ))]
        |index: &NodeIdx| match Self::lookup_parts(values, masks, *index) {
            Some(node) => node.same(entry),
            None => false,
        };
        let found = table::find(&self.table, hash, predicate);
        if let Some(idx) = found {
            let result = self.add_count(idx, rc);
            #[cfg(verus_only)]
            proof! {
                if result is Err { proof::reserved(&before, self); }
                else { proof::existing(&before, self, *entry, rc, idx); }
            }
            result?;
            return Ok(InsertResult::Existing(idx));
        }
        #[cfg(verus_only)]
        proof! { proof::no_matching_node(&before, *entry, hash, predicate); }
        let rc = rc
            .checked_add(self.constructing as usize)
            .ok_or(Error::RefCountOverflow)?;
        let generation = self
            .next_generation
            .checked_add(1)
            .ok_or(Error::NodeGenerationExhausted)?;
        let builder = &self.hash_builder;
        #[cfg(test)]
        let hash_mask = self.hash_mask;
        let rehash = #[cfg_attr(verus_only, verus_spec(result: u64 =>
            ensures result == proof::hash_at(*builder, values@, masks@, *index)
        ))]
        |index: &NodeIdx| {
            let hash = match Self::lookup_parts(values, masks, *index) {
                Some(node) => table::hash(builder, node.values.words(), node.mask.words()),
                None => 0,
            };
            #[cfg(test)]
            let hash = hash & hash_mask;
            hash
        };
        let result = table::reserve(&mut self.table, 1, rehash);
        #[cfg(verus_only)]
        proof! { proof::reserved(&before, self); }
        result?;
        if self.constructing {
            let result = self.retained.try_reserve(1);
            #[cfg(verus_only)]
            proof! { proof::reserved(&before, self); }
            result?;
        }
        let idx = if let Some(&idx) = self.free_list.last() {
            let pos = idx.as_usize()?;
            #[cfg(verus_only)]
            proof! { proof::reserved(&before, self); }
            let payload = self.values.get_mut(pos).ok_or(Error::NodeMissing(idx))?;
            let mask = self.masks.get_mut(pos).ok_or(Error::NodeMissing(idx))?;
            let count = self.rcs.get_mut(pos).ok_or(Error::NodeMissing(idx))?;
            let version = self
                .generations
                .get_mut(pos)
                .ok_or(Error::NodeMissing(idx))?;
            if let Some(children) = children {
                let result = children.retain_edges(entry);
                // 失败返回前须结束槽借用并连接原池不变的擦除证明。
                #[allow(clippy::question_mark)]
                if let Err(error) = result {
                    #[cfg(verus_only)]
                    proof! {
                        assert(*payload == before.values@[pos as int]);
                        assert(*mask == before.masks@[pos as int]);
                        assert(*count == before.rcs@[pos as int]);
                        assert(*version == before.generations@[pos as int]);
                    }
                    #[cfg(verus_only)]
                    proof! {
                        assert(self.values@ =~= before.values@);
                        assert(self.masks@ =~= before.masks@);
                        assert(self.rcs@ =~= before.rcs@);
                        assert(self.generations@ =~= before.generations@);
                        proof::reserved(&before, self);
                    }
                    return Err(error);
                }
            }
            *payload = entry.values;
            *mask = entry.mask;
            *count = rc;
            *version = generation;
            vec_ops::pop(&mut self.free_list);
            idx
        } else {
            let idx =
                NodeIdx::new(u64::try_from(self.values.len()).map_err(|_| Error::IndexExhausted)?)
                    .ok_or(Error::IndexExhausted)?;
            let count = self
                .values
                .len()
                .checked_add(1)
                .ok_or(Error::CapacityOverflow)?;
            let result = self.values.try_reserve(1);
            #[cfg(verus_only)]
            proof! { proof::reserved(&before, self); }
            result?;
            let result = self.masks.try_reserve(1);
            #[cfg(verus_only)]
            proof! { proof::reserved(&before, self); }
            result?;
            let result = self.rcs.try_reserve(1);
            #[cfg(verus_only)]
            proof! { proof::reserved(&before, self); }
            result?;
            let result = self.generations.try_reserve(1);
            #[cfg(verus_only)]
            proof! { proof::reserved(&before, self); }
            result?;
            let additional = count.saturating_sub(self.free_list.len());
            let result = vec_ops::reserve(&mut self.free_list, additional);
            #[cfg(verus_only)]
            proof! { proof::reserved(&before, self); }
            result?;
            if let Some(children) = children {
                let result = children.retain_edges(entry);
                #[cfg(verus_only)]
                proof! { proof::reserved(&before, self); }
                result?;
            }
            self.values.push(entry.values);
            self.masks.push(entry.mask);
            self.rcs.push(rc);
            self.generations.push(generation);
            idx
        };
        self.next_generation = generation;
        let values = self.values.as_slice();
        let masks = self.masks.as_slice();
        let rehash = #[cfg_attr(verus_only, verus_spec(result: u64 =>
            ensures result == proof::hash_at(*builder, values@, masks@, *index)
        ))]
        |index: &NodeIdx| {
            let hash = match Self::lookup_parts(values, masks, *index) {
                Some(node) => table::hash(builder, node.values.words(), node.mask.words()),
                None => 0,
            };
            #[cfg(test)]
            let hash = hash & hash_mask;
            hash
        };
        table::insert(&mut self.table, hash, idx, rehash);
        if self.constructing {
            self.retained.push(idx);
        }
        #[cfg(verus_only)]
        proof! { proof::allocated(&before, self, *entry, requested, idx); }
        Ok(InsertResult::New(idx))
    }

    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(),
        ensures final(self).valid(), final(self).state() == (crate::state::PoolState { constructing: true, ..old(self).state() }),
        no_unwind
    ))]
    pub(crate) const fn begin_write(&mut self) {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        self.constructing = true;
        #[cfg(verus_only)]
        proof! { proof::metadata(&before, self); }
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).valid(),
            result == if old(self).state().retained.len() == 0 { None } else { Some(old(self).state().retained.last()) },
            final(self).state() == (crate::state::PoolState {
                retained: if old(self).state().retained.len() == 0 { old(self).state().retained } else { old(self).state().retained.drop_last() },
                ..old(self).state()
            }),
        no_unwind
    ))]
    pub(crate) fn pop_retained(&mut self) -> Option<NodeIdx> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        let result = vec_ops::pop(&mut self.retained);
        #[cfg(verus_only)]
        proof! { proof::metadata(&before, self); }
        result
    }

    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(),
        ensures final(self).valid(), final(self).state() == (crate::state::PoolState { constructing: false, ..old(self).state() }),
        no_unwind
    ))]
    pub(crate) const fn end_write(&mut self) {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        self.constructing = false;
        #[cfg(verus_only)]
        proof! { proof::metadata(&before, self); }
    }

    #[cfg(test)]
    pub(crate) fn rc_add_or_insert(
        &mut self,
        rc: usize,
        entry: &PoolEntry,
    ) -> Result<InsertResult, Error> {
        self.intern(rc, entry, None)
    }

    #[cfg(test)]
    pub(crate) fn ref_count(&self, idx: NodeIdx) -> Option<usize> {
        self.rcs.get(idx.as_usize().ok()?).copied()
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(), old(self).count_domain(crate::node::proof::idx(idx)),
        ensures final(self).valid(), final(self).same_storage(old(self)),
            result is Err ==> final(self).counts() == old(self).counts(),
            result is Err ==> final(self).state() == old(self).state(),
            result is Ok ==> final(self).count_change(old(self), crate::node::proof::idx(idx), 1),
            result is Ok ==> proof::added(old(self).state(),final(self).state(),idx,1),
            result is Ok ==> 0 <= crate::node::proof::idx(idx) < old(self).counts().len()
                && old(self).counts()[crate::node::proof::idx(idx)] + 1 <= usize::MAX,
        no_unwind
    ))]
    pub(crate) fn rc_inc(&mut self, idx: NodeIdx) -> Result<(), Error> {
        self.add_count(idx, 1)
    }

    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(), old(self).count_domain(crate::node::proof::idx(idx)),
        ensures final(self).valid(), final(self).same_storage(old(self)),
            result is Err ==> final(self).counts() == old(self).counts(),
            result is Err ==> final(self).state() == old(self).state(),
            result is Ok ==> final(self).count_change(old(self), crate::node::proof::idx(idx), amount as nat),
            result is Ok ==> proof::added(old(self).state(),final(self).state(),idx,amount as nat),
            result is Ok ==> 0 <= crate::node::proof::idx(idx) < old(self).counts().len()
                && old(self).counts()[crate::node::proof::idx(idx)] + amount <= usize::MAX,
        no_unwind
    ))]
    fn add_count(&mut self, idx: NodeIdx, amount: usize) -> Result<(), Error> {
        let pos = idx.as_usize()?;
        if pos >= self.rcs.len() {
            return Err(Error::NodeMissing(idx));
        }
        let count = match self.rcs[pos].checked_add(amount) {
            Some(count) => count,
            None => return Err(Error::RefCountOverflow),
        };
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        self.rcs[pos] = count;
        #[cfg(verus_only)]
        proof! { proof::count_updated(&before, self, crate::node::proof::idx(idx), amount as nat); }
        Ok(())
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
            crate::state::live(old(self).state(), crate::node::proof::idx(idx))
                && old(self).counts()[crate::node::proof::idx(idx)] == 1 ==>
                !old(self).state().retained.contains(idx),
        ensures final(self).valid(),
            result is Ok <==> crate::state::live(old(self).state(), crate::node::proof::idx(idx)),
            result is Err ==> final(self).state() == old(self).state(),
            result is Ok ==> proof::decremented(old(self).state(), final(self).state(), idx),
            result is Ok ==> (result->Ok_0 is Some <==> old(self).counts()[crate::node::proof::idx(idx)] == 1),
            result is Ok && result->Ok_0 is Some ==> crate::node::proof::children(result->Ok_0->Some_0)
                == old(self).state().nodes[crate::node::proof::idx(idx)]->Some_0,
    ))]
    pub(crate) fn rc_dec_or_remove(&mut self, idx: NodeIdx) -> Result<Option<PoolEntry>, Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        let pos = idx.as_usize()?;
        let count = *self.rcs.get(pos).ok_or(Error::NodeMissing(idx))?;
        if count == 0 {
            return Err(Error::NodeMissing(idx));
        }
        if count == 1 {
            #[cfg(verus_only)]
            proof! { self.free_room(idx); }
            let node = self.get_live(idx).ok_or(Error::NodeMissing(idx))?;
            let hash = table::hash(&self.hash_builder, node.values.words(), node.mask.words());
            #[cfg(test)]
            let hash = hash & self.hash_mask;
            let entry = node.to_entry();
            let predicate = #[cfg_attr(verus_only, verus_spec(result: bool => ensures result == (*index == idx)))]
            |index: &NodeIdx| index.get() == idx.get();
            #[cfg(verus_only)]
            proof! {
                assert(hash_dag_std::table_model::entries(self.table).contains_key(idx));
                assert(hash_dag_std::table_model::entries(self.table)[idx] == hash);
                assert(!predicate.ensures((&idx,), false));
            }
            let removed = table::remove(&mut self.table, hash, predicate);
            #[cfg(verus_only)]
            proof! { assert(removed is Some); }
            removed.ok_or(Error::NodeMissing(idx))?;
            *self.rcs.get_mut(pos).ok_or(Error::NodeMissing(idx))? = 0;
            vec_ops::push(&mut self.free_list, idx);
            #[cfg(verus_only)]
            proof! { proof::removed_node(&before, self, idx); }
            Ok(Some(entry))
        } else {
            *self.rcs.get_mut(pos).ok_or(Error::NodeMissing(idx))? = count - 1;
            #[cfg(verus_only)]
            proof! { proof::count_replaced(&before, self, crate::node::proof::idx(idx), (count - 1) as usize); }
            Ok(None)
        }
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is Some <==> self.readable(crate::node::proof::idx(idx)),
            result is Some ==> crate::node::proof::ref_children(result->Some_0) == self.raw_node(crate::node::proof::idx(idx)),
            result is Some ==> crate::node::proof::values(*result->Some_0.values) == self.raw_values(crate::node::proof::idx(idx))
                && crate::node::proof::mask_words(result->Some_0.mask) == self.raw_mask(crate::node::proof::idx(idx)),
            self.valid() ==> (result is Some <==> crate::state::live(self.state(), crate::node::proof::idx(idx))),
            self.valid() && result is Some ==> self.state().nodes[crate::node::proof::idx(idx)] == Some(crate::node::proof::ref_children(result->Some_0)),
        no_unwind
    ))]
    pub(crate) fn get(&self, idx: NodeIdx) -> Option<NodeRef<'_>> {
        #[cfg(verus_only)]
        proof! { if self.valid() { self.read_model(crate::node::proof::idx(idx)); } }
        let pos = match idx.as_usize() {
            Ok(pos) => pos,
            Err(_) => return None,
        };
        if pos >= self.rcs.len() || self.rcs[pos] == 0 {
            return None;
        }
        self.get_live(idx)
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == self.raw_generation(crate::node::proof::idx(idx)),
            self.valid() ==> (result is Some <==> crate::state::live(self.state(), crate::node::proof::idx(idx))),
            self.valid() && result is Some ==> result->Some_0 == self.state().generations[crate::node::proof::idx(idx)],
        no_unwind
    ))]
    pub(crate) fn generation(&self, idx: NodeIdx) -> Option<u64> {
        #[cfg(verus_only)]
        proof! { if self.valid() { self.read_model(crate::node::proof::idx(idx)); } }
        let pos = match idx.as_usize() {
            Ok(pos) => pos,
            Err(_) => return None,
        };
        if pos >= self.rcs.len() || self.rcs[pos] == 0 || pos >= self.generations.len() {
            None
        } else {
            Some(self.generations[pos])
        }
    }

    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is Some <==> self.payload_present(crate::node::proof::idx(idx)),
            result is Some ==> crate::node::proof::ref_children(result->Some_0) == self.raw_node(crate::node::proof::idx(idx)),
            result is Some ==> crate::node::proof::values(*result->Some_0.values) == self.raw_values(crate::node::proof::idx(idx))
                && crate::node::proof::mask_words(result->Some_0.mask) == self.raw_mask(crate::node::proof::idx(idx)),
        no_unwind
    ))]
    pub(crate) fn get_live(&self, idx: NodeIdx) -> Option<NodeRef<'_>> {
        let pos = match idx.as_usize() {
            Ok(pos) => pos,
            Err(_) => return None,
        };
        if pos >= self.values.len() || pos >= self.masks.len() {
            return None;
        }
        Some(NodeRef {
            values: &self.values[pos],
            mask: self.masks[pos],
        })
    }

    #[cfg(test)]
    pub(crate) const fn slot_count(&self) -> usize {
        self.values.len()
    }
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result.0@==self.raw_arrays().0,result.1@==self.raw_arrays().1,
            result.0@.len() == self.read_nodes().len(),
            self.valid() ==> result.0@.len()==self.state().nodes.len() && result.1@.len()==self.state().nodes.len(),
            self.valid() ==> forall|i:int|crate::state::occupied(self.state(),i) ==>
                #[trigger] self.state().nodes[i]==Some(crate::node::proof::children(PoolEntry{values:result.0@[i],mask:result.1@[i]})),
        no_unwind
    ))]
    pub(crate) fn arrays(&self) -> (&[NodeValues], &[NodeMask]) {
        #[cfg(verus_only)]
        proof! {if self.valid() {assert forall|i:int|crate::state::occupied(self.state(),i) implies
        #[trigger] self.state().nodes[i]==Some(crate::node::proof::children(PoolEntry{values:self.values@[i],mask:self.masks@[i]})) by {self.read_model(i);}}}
        (self.values.as_slice(), self.masks.as_slice())
    }
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.rcs.iter().filter(|&&v| v != 0).count()
    }
}

impl Pool {
    /// 按实际 capacity 统计池的堆分配，包含哈希控制区和对齐，不含 allocator 元数据。
    pub(crate) fn retained_bytes(&self) -> usize {
        table::retained(&self.table)
            + self.values.capacity() * size_of::<NodeValues>()
            + self.masks.capacity() * size_of::<NodeMask>()
            + self.rcs.capacity() * size_of::<usize>()
            + self.generations.capacity() * size_of::<u64>()
            + self.free_list.capacity() * size_of::<NodeIdx>()
            + self.retained.capacity() * size_of::<NodeIdx>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ZOrder20x3;
    use crate::node::{Child, slot_at};
    use proptest::prelude::*;

    fn arb_entry() -> impl Strategy<Value = PoolEntry> {
        crate::test_util::arb_entry()
    }

    fn arb_pool_with_nodes(
        count: core::ops::Range<usize>,
    ) -> impl Strategy<Value = (Pool, Vec<(NodeIdx, PoolEntry)>)> {
        prop::collection::vec((arb_entry(), 1usize..=5usize), count).prop_map(|entries_with_rc| {
            let mut pool = Pool::new();
            let mut inserted: Vec<(NodeIdx, PoolEntry)> = Vec::new();
            for (entry, rc) in entries_with_rc {
                if let Ok(InsertResult::New(idx)) = pool.rc_add_or_insert(rc, &entry) {
                    inserted.push((idx, entry));
                }
            }
            (pool, inserted)
        })
    }

    #[test]
    fn new_pool_is_empty() {
        let pool = Pool::new();
        assert_eq!(pool.len(), 0);
    }

    proptest! {
        // node 原测试
        #[test]
        fn new_preserves_values(
            slots in prop::collection::vec(any::<u32>(), 64),
            mixed in any::<u64>(),
        ) {
            let mut arr = [0u32; 64];
            arr.copy_from_slice(&slots);
            let entry = PoolEntry::new(&arr, mixed);
            let all = entry.children();
            for (idx, (child, slot)) in all.into_iter().zip(slots).enumerate() {
                if (mixed & (1u64 << idx)) != 0 {
                    assert_eq!(child, Child::Ptr(NodeIdx::new(u64::from(slot)).unwrap_or(NodeIdx::ZERO)));
                } else {
                    assert_eq!(child, Child::Leaf(slot));
                }
            }
        }

        #[test]
        fn all_children_preserves_order(
            slots in prop::collection::vec(any::<u32>(), 64),
            mixed in any::<u64>(),
        ) {
            let mut arr = [0u32; 64];
            arr.copy_from_slice(&slots);
            let entry = PoolEntry::new(&arr, mixed);
            let children = entry.children();
            for (slot, child) in (0u8..).zip(children) {
                assert_eq!(child, entry.child(slot));
            }
        }

        #[test]
        fn child_mixed_flag_matches(
            slots in prop::collection::vec(any::<u32>(), 64),
            mixed in any::<u64>(),
            slot in 0u8..64u8,
        ) {
            let mut arr = [0u32; 64];
            arr.copy_from_slice(&slots);
            let entry = PoolEntry::new(&arr, mixed);
            let expected_mixed = (mixed & (1u64 << slot)) != 0;
            match entry.child(slot) {
                Child::Ptr(_) => assert!(expected_mixed),
                Child::Leaf(_) => assert!(!expected_mixed),
            }
        }

        #[test]
        fn equality_matches_same_encoded_key(
            slots in prop::collection::vec(any::<u32>(), 64),
            mixed in any::<u64>(),
        ) {
            let mut arr = [0u32; 64];
            arr.copy_from_slice(&slots);
            let a = PoolEntry::new(&arr, mixed);
            let b = PoolEntry::new(&arr, mixed);
            assert_eq!(a, b);
        }

        #[test]
        fn inequality_different_mixed(
            slots in prop::collection::vec(any::<u32>(), 64),
            a_mixed in any::<u64>(),
            b_mixed in any::<u64>(),
        ) {
            prop_assume!(a_mixed != b_mixed);
            let mut arr = [0u32; 64];
            arr.copy_from_slice(&slots);
            let a = PoolEntry::new(&arr, a_mixed);
            let b = PoolEntry::new(&arr, b_mixed);
            assert_ne!(a, b);
        }

        #[test]
        fn clone_preserves_equality(entry in arb_entry()) {
            let cloned = entry.clone();
            assert_eq!(entry, cloned);
        }

        #[test]
        fn child_at_matches_octant_at_then_child(
            slots in prop::collection::vec(any::<u32>(), 64),
            mixed in any::<u64>(),
            x in 0u32..16u32,
            y in 0u32..16u32,
            z in 0u32..16u32,
            lod in 0u8..4u8,
        ) {
            let mut arr = [0u32; 64];
            arr.copy_from_slice(&slots);
            let entry = PoolEntry::new(&arr, mixed);
            let path = ZOrder20x3::encode_unchecked(x, y, z);
            let slot = slot_at(path, lod);
            assert_eq!(entry.child_at(path, lod), entry.child(slot));
        }

        #[test]
        fn set_child_at_round_trip(
            x in 0u32..16u32,
            y in 0u32..16u32,
            z in 0u32..16u32,
            lod in 0u8..4u8,
            data in any::<u32>(),
        ) {
            let mut entry = PoolEntry::new(&[0u32; 64], 0);
            let path = ZOrder20x3::encode_unchecked(x, y, z);
            entry.set_child_at(path, lod, Child::Leaf(data));
            assert_eq!(entry.child_at(path, lod), Child::Leaf(data));
        }

        // pool 原测试
        #[test]
        fn insert_new_node_increases_len(entry in arb_entry()) {
            let mut pool = Pool::new();
            let old_len = pool.len();

            let result = pool.rc_add_or_insert(1, &entry);
            prop_assert!(matches!(result, Ok(InsertResult::New(_))), "{result:?}");
            if let Ok(InsertResult::New(idx)) = result {
                prop_assert_eq!(pool.len(), old_len + 1);
                prop_assert_eq!(pool.get(idx).map(crate::node::NodeRef::to_entry), Some(entry.clone()));
            }
        }

        #[test]
        fn insert_duplicate_returns_existing(entry in arb_entry()) {
            let mut pool = Pool::new();

            let first = pool.rc_add_or_insert(1, &entry);
            prop_assert!(matches!(first, Ok(InsertResult::New(_))), "{first:?}");
            let len_after_first = pool.len();

            let second = pool.rc_add_or_insert(1, &entry);
            prop_assert!(matches!(second, Ok(InsertResult::Existing(_))), "{second:?}");
            prop_assert_eq!(pool.len(), len_after_first);
        }

        #[test]
        fn insert_distinct_nodes_grows_correctly(
            entries in prop::collection::vec(arb_entry(), 0..50)
        ) {
            let mut pool = Pool::new();
            let mut seen = 0usize;

            for entry in &entries {
                if let Ok(InsertResult::New(_)) = pool.rc_add_or_insert(1, entry) { seen += 1 }
            }

            assert_eq!(pool.len(), seen);
        }

        #[test]
        fn rc_inc_valid_index(
            (mut pool, inserted) in arb_pool_with_nodes(1..10)
        ) {
            prop_assume!(!inserted.is_empty());
            for &(idx, ref entry) in &inserted {
                let result = pool.rc_inc(idx);
                assert!(result.is_ok(), "rc_inc should succeed for idx={idx:?}");
                prop_assert_eq!(pool.get(idx).map(crate::node::NodeRef::to_entry), Some(entry.clone()));
            }
        }

        #[test]
        fn rc_inc_invalid_index(
            (mut pool, inserted) in arb_pool_with_nodes(0..5),
            bogus_idx in any::<u32>(),
        ) {
            let bogus_idx = NodeIdx::new(u64::from(bogus_idx)).unwrap_or(NodeIdx::ZERO);
            let bogus_not_used = inserted.iter().all(|&(idx, _)| idx != bogus_idx);
            prop_assume!(bogus_not_used);

            let result = pool.rc_inc(bogus_idx);
            assert!(result.is_err(), "rc_inc on bogus index should fail");
        }

        #[test]
        fn rc_dec_keeps_node_when_rc_gt_1(entry in arb_entry()) {
            let mut pool = Pool::new();
            let insert = pool.rc_add_or_insert(3, &entry);
            prop_assert!(matches!(insert, Ok(InsertResult::New(_))), "{insert:?}");
            if let Ok(InsertResult::New(idx)) = insert {
                let len_before = pool.len();
                let result = pool.rc_dec_or_remove(idx);
                prop_assert!(matches!(result, Ok(None)), "expected Ok(None), got {result:?}");
                prop_assert_eq!(pool.len(), len_before);
                prop_assert!(pool.get(idx).is_some());
            }
        }

        #[test]
        fn rc_dec_removes_node_when_rc_eq_1(entry in arb_entry()) {
            let mut pool = Pool::new();
            let insert = pool.rc_add_or_insert(1, &entry);
            prop_assert!(matches!(insert, Ok(InsertResult::New(_))), "{insert:?}");
            if let Ok(InsertResult::New(idx)) = insert {
                let len_before = pool.len();
                let result = pool.rc_dec_or_remove(idx);
                prop_assert_eq!((result).ok(), Some(Some(entry)));
                prop_assert_eq!(pool.len(), len_before - 1);
                prop_assert!(pool.get(idx).is_none());
            }
        }

        #[test]
        fn rc_dec_invalid_index(
            (mut pool, inserted) in arb_pool_with_nodes(0..5),
            bogus_idx in any::<u32>(),
        ) {
            let bogus_idx = NodeIdx::new(u64::from(bogus_idx)).unwrap_or(NodeIdx::ZERO);
            let bogus_not_used = inserted.iter().all(|&(idx, _)| idx != bogus_idx);
            prop_assume!(bogus_not_used);

            let result = pool.rc_dec_or_remove(bogus_idx);
            assert!(result.is_err());
        }

        #[test]
        fn remove_all_returns_to_empty(
            (mut pool, inserted) in arb_pool_with_nodes(1..20)
        ) {
            prop_assume!(!inserted.is_empty());
            for &(idx, _) in &inserted {
                while pool.rc_dec_or_remove(idx).is_ok_and(|r| r.is_none()) {}
            }
            assert_eq!(pool.len(), 0);
        }
    }

    #[test]
    fn rc_merges_on_duplicate() {
        let mut pool = Pool::new();
        let entry = PoolEntry::new(&[0u32; 64], 0);
        let result = pool.rc_add_or_insert(2, &entry);
        assert!(matches!(result, Ok(InsertResult::New(_))), "{result:?}");
        let Ok(InsertResult::New(idx)) = result else {
            return;
        };
        let duplicate = pool.rc_add_or_insert(3, &entry);
        assert!(
            matches!(duplicate, Ok(InsertResult::Existing(_))),
            "{duplicate:?}"
        );
        assert!(pool.rc_dec_or_remove(idx).is_ok_and(|r| r.is_none()));
        assert!(pool.rc_dec_or_remove(idx).is_ok_and(|r| r.is_none()));
        assert!(pool.rc_dec_or_remove(idx).is_ok_and(|r| r.is_none()));
        assert!(pool.rc_dec_or_remove(idx).is_ok_and(|r| r.is_none()));
        assert!(pool.rc_dec_or_remove(idx).is_ok_and(|r| r.is_some()));
        assert!(pool.get(idx).is_none());
    }

    #[test]
    fn dedup_after_remove_and_reinsert() {
        let mut pool = Pool::new();
        let entry = PoolEntry::new(&[0u32; 64], 0);
        let result = pool.rc_add_or_insert(1, &entry);
        assert!(matches!(result, Ok(InsertResult::New(_))), "{result:?}");
        let Ok(InsertResult::New(idx)) = result else {
            return;
        };
        assert!(pool.rc_dec_or_remove(idx).is_ok_and(|r| r.is_some()));
        assert!(pool.get(idx).is_none());

        let reinsert = pool.rc_add_or_insert(1, &entry);
        assert!(matches!(reinsert, Ok(InsertResult::New(_))), "{reinsert:?}");
        let Ok(InsertResult::New(idx2)) = reinsert else {
            return;
        };
        assert_eq!(pool.len(), 1);
        assert!(pool.get(idx2).is_some());
    }

    #[test]
    fn free_list_reuses_slots() {
        let mut pool = Pool::new();
        let a = PoolEntry::new(&[0u32; 64], 0);
        let b = PoolEntry::new(&[1u32; 64], 0);

        let result = pool.rc_add_or_insert(1, &a);
        assert!(matches!(result, Ok(InsertResult::New(_))), "{result:?}");
        let Ok(InsertResult::New(idx_a)) = result else {
            return;
        };
        assert!(pool.rc_dec_or_remove(idx_a).is_ok_and(|r| r.is_some()));

        let insert_b = pool.rc_add_or_insert(1, &b);
        assert!(matches!(insert_b, Ok(InsertResult::New(_))), "{insert_b:?}");
        let Ok(InsertResult::New(idx_b)) = insert_b else {
            return;
        };
        assert_eq!(idx_b, idx_a);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn overflowing_edge_acquisition_restores_previous_edges() -> Result<(), Error> {
        let mut pool = Pool::new();
        let leaf = PoolEntry::new(&[3; 64], 0);
        let InsertResult::New(index) = pool.rc_add_or_insert(usize::MAX - 1, &leaf)? else {
            return Err(Error::BuildFrameMissing);
        };
        let mut children = [Child::Leaf(0); 64];
        children[0] = Child::Ptr(index);
        children[1] = Child::Ptr(index);
        assert!(matches!(
            pool.retain_edges(&PoolEntry::from_children(&children)),
            Err(Error::RefCountOverflow)
        ));
        assert_eq!(pool.ref_count(index), Some(usize::MAX - 1));
        Ok(())
    }

    #[test]
    fn reclaimed_slots_keep_prepared_storage() -> Result<(), Error> {
        let mut pool = Pool::new();
        let InsertResult::New(index) = pool.rc_add_or_insert(1, &PoolEntry::new(&[3; 64], 0))?
        else {
            return Err(Error::BuildFrameMissing);
        };
        let retained = pool.retained_bytes();
        assert!(retained > size_of::<NodeValues>() + size_of::<NodeMask>());
        pool.rc_dec_or_remove(index)?;
        assert_eq!(pool.retained_bytes(), retained);
        assert_eq!(pool.len(), 0);
        Ok(())
    }

    #[test]
    fn hash_collisions_preserve_content_identity_and_independent_release() -> Result<(), Error> {
        let mut pool = Pool::new();
        pool.hash_mask = 0;
        let entries = std::array::from_fn::<_, 8, _>(|index| {
            let mut values = [0; 64];
            values[0] = index as u32 + 1;
            PoolEntry::new(&values, 0)
        });
        let mut indices = [NodeIdx::ZERO; 8];
        for (slot, entry) in indices.iter_mut().zip(&entries) {
            let InsertResult::New(index) = pool.rc_add_or_insert(1, entry)? else {
                return Err(Error::BuildFrameMissing);
            };
            *slot = index;
        }
        assert_eq!(pool.len(), entries.len());
        assert!(
            matches!(pool.rc_add_or_insert(1, &entries[0])?, InsertResult::Existing(index) if index == indices[0])
        );
        assert!(pool.rc_dec_or_remove(indices[0])?.is_none());
        assert!(pool.rc_dec_or_remove(indices[0])?.is_some());
        assert!(pool.get(indices[0]).is_none());
        for (&index, entry) in indices[1..].iter().zip(&entries[1..]) {
            assert!(
                matches!(pool.rc_add_or_insert(1, entry)?, InsertResult::Existing(found) if found == index)
            );
            assert_eq!(pool.get(index).map(NodeRef::to_entry).as_ref(), Some(entry));
        }
        Ok(())
    }
}
