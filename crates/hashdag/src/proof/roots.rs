use super::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

impl RootSlots {
    pub(crate) open spec fn valid(&self) -> bool {
        &&& self.slots@.len() > 0
        &&& self.slots@.len() <= usize::MAX
        &&& hash_dag_std::vec_model::capacity(self.free) >= self.slots@.len() - 1
        &&& self.slots@[0].child is Some
        &&& self.slots@[0].generation == 0
        &&& self.free@.no_duplicates()
        &&& (forall|i: int| #![auto] 0 <= i < self.free@.len() ==>
            self.free@[i] < self.slots@.len()
            && self.slots@[self.free@[i] as int].child is None)
        &&& (forall|i: int| #![auto] 0 <= i < self.slots@.len() ==>
            (self.slots@[i].child is None <==> self.free@.contains(i as usize)))
    }

    pub(crate) open spec fn lookup(&self, root: RootId) -> Option<Child> {
        if root.owner == self.owner && root.slot < self.slots@.len()
            && self.slots@[root.slot as int].generation == root.generation {
            self.slots@[root.slot as int].child
        } else { None }
    }

    pub(crate) open spec fn insertion_ready(&self) -> bool {
        self.free@.len() > 0 || (self.slots@.len() < usize::MAX
            && self.slots@.len() < hash_dag_std::vec_model::capacity(self.slots)
            && hash_dag_std::vec_model::capacity(self.free) >= self.slots@.len())
    }

    /// 已成功释放的身份总是严格小于当前代次；空槽的尚未发行代次不计作旧身份。
    pub(crate) open spec fn revoked(&self, root: RootId) -> bool {
        root.owner == self.owner && root.slot < self.slots@.len()
            && root.generation < self.slots@[root.slot as int].generation
    }

    pub(crate) open spec fn same_entries(&self, before: &Self) -> bool {
        self.owner == before.owner && self.slots@ =~= before.slots@ && self.free@ =~= before.free@
    }
}

pub(crate) open spec fn preserves_others(before: &RootSlots, after: &RootSlots, changed: RootId) -> bool {
    forall|root: RootId| #![auto] root != changed ==> after.lookup(root) == before.lookup(root)
}

pub(crate) open spec fn preserves_revoked(before: &RootSlots, after: &RootSlots) -> bool {
    forall|root: RootId| before.revoked(root) ==> after.revoked(root)
}

pub(crate) open spec fn inserted(before: &RootSlots, after: &RootSlots, child: Child, root: RootId) -> bool {
    &&& after.valid()
    &&& after.owner == before.owner
    &&& after.lookup(root) == Some(child)
    &&& root.owner == before.owner && root.slot != 0
    &&& before.lookup(root) is None
    &&& preserves_others(before, after, root)
    &&& preserves_revoked(before, after)
    &&& root.slot == if before.free@.len() > 0 { before.free@.last() } else { before.slots@.len() as usize }
    &&& after.slots@.len() == before.slots@.len() + if before.free@.len() > 0 { 0int } else { 1int }
    &&& after.slots@[root.slot as int].child == Some(child)
    &&& (forall|i: int| 0 <= i < before.slots@.len() && i != root.slot ==> after.slots@[i] == before.slots@[i])
}

pub(crate) open spec fn removed(before: &RootSlots, after: &RootSlots, root: RootId) -> bool {
    &&& after.valid()
    &&& after.owner == before.owner
    &&& after.revoked(root)
    &&& after.lookup(root) is None
    &&& preserves_others(before, after, root)
    &&& preserves_revoked(before, after)
    &&& after.slots@.len() == before.slots@.len()
    &&& hash_dag_std::vec_model::capacity(after.free) == hash_dag_std::vec_model::capacity(before.free)
    &&& after.slots@[root.slot as int].child is None
    &&& (forall|i: int| 0 <= i < before.slots@.len() && i != root.slot ==> after.slots@[i] == before.slots@[i])
}

pub(crate) open spec fn replaced(before: &RootSlots, after: &RootSlots, root: RootId, child: Child) -> bool {
    &&& after.valid() && after.owner == before.owner && after.free@ == before.free@
    &&& after.slots@.len() == before.slots@.len()
    &&& after.slots@[root.slot as int].child == Some(child)
    &&& after.slots@[root.slot as int].generation == before.slots@[root.slot as int].generation
    &&& (forall|i: int| 0 <= i < before.slots@.len() && i != root.slot ==> after.slots@[i] == before.slots@[i])
    &&& after.lookup(root) == Some(child)
    &&& preserves_others(before, after, root) && preserves_revoked(before, after)
}

pub(super) proof fn insert_reused(before: RootSlots, after: RootSlots, index: usize)
    requires before.valid(), before.owner == after.owner,
        before.free@.len() > 0, index == before.free@.last(),
        after.free@ == before.free@.drop_last(),
        hash_dag_std::vec_model::capacity(after.free) == hash_dag_std::vec_model::capacity(before.free),
        after.slots@.len() == before.slots@.len(),
        after.slots@[index as int].generation == before.slots@[index as int].generation,
        after.slots@[index as int].child is Some,
        forall|i: int| 0 <= i < before.slots@.len() && i != index as int ==>
            after.slots@[i] == before.slots@[i],
    ensures after.valid(), preserves_revoked(&before, &after),
{
    let remaining = before.free@.len() - 1;
    assert(index != 0);
    assert forall|i: int, j: int| 0 <= i < remaining && 0 <= j < remaining && i != j
        implies after.free@[i] != after.free@[j] by {
        assert(before.free@[i] != before.free@[j]);
    }
    assert forall|i: int| #![auto] 0 <= i < after.free@.len() implies
        after.free@[i] < after.slots@.len()
            && after.slots@[after.free@[i] as int].child is None by {
        assert(before.free@[i] != before.free@[remaining]);
        assert(before.slots@[before.free@[i] as int].child is None);
    }
    assert forall|i: int| #![auto] 0 <= i < after.slots@.len()
        implies (after.slots@[i].child is None <==> after.free@.contains(i as usize)) by {
        if after.slots@[i].child is None {
            assert(i != index as int);
            assert(before.free@.contains(i as usize));
            let position = choose|position: int| 0 <= position < before.free@.len()
                && before.free@[position] == i as usize;
            assert(position < remaining);
            assert(after.free@[position] == i as usize);
        }
    }
}

pub(super) proof fn insert_new(before: RootSlots, after: RootSlots)
    requires before.valid(), before.insertion_ready(), before.owner == after.owner, before.free@.len() == 0,
        hash_dag_std::vec_model::capacity(after.free) == hash_dag_std::vec_model::capacity(before.free),
        before.slots@.len() < usize::MAX,
        after.free@ == before.free@,
        after.slots@.len() == before.slots@.len() + 1,
        forall|i: int| 0 <= i < before.slots@.len() ==> after.slots@[i] == before.slots@[i],
        after.slots@.last().generation == 0,
        after.slots@.last().child is Some,
    ensures after.valid(), preserves_revoked(&before, &after),
{}

pub(super) proof fn retired(before: RootSlots, after: RootSlots, root: RootId)
    requires before.valid(), before.lookup(root) is Some, root.slot != 0,
        root.generation < u64::MAX, before.owner == after.owner,
        after.slots@.len() == before.slots@.len(),
        after.slots@[root.slot as int].child is None,
        after.slots@[root.slot as int].generation == root.generation + 1,
        after.free@ == before.free@.push(root.slot),
        hash_dag_std::vec_model::capacity(after.free) == hash_dag_std::vec_model::capacity(before.free),
        forall|i: int| 0 <= i < before.slots@.len() && i != root.slot as int ==>
            after.slots@[i] == before.slots@[i],
    ensures removed(&before, &after, root),
{
    assert(!before.free@.contains(root.slot)) by {
        if before.free@.contains(root.slot) {
            let position = choose|position: int| 0 <= position < before.free@.len()
                && before.free@[position] == root.slot;
            assert(before.slots@[root.slot as int].child is None);
        }
    }
    assert forall|i: int, j: int| 0 <= i < after.free@.len()
        && 0 <= j < after.free@.len() && i != j
        implies after.free@[i] != after.free@[j] by {
        if i < before.free@.len() && j < before.free@.len() {
            assert(before.free@[i] != before.free@[j]);
        }
    }
    assert forall|i: int| #![auto] 0 <= i < after.free@.len() implies
        after.free@[i] < after.slots@.len()
            && after.slots@[after.free@[i] as int].child is None by {
        if i < before.free@.len() { assert(before.slots@[before.free@[i] as int].child is None); }
    }
    assert forall|i: int| #![auto] 0 <= i < after.slots@.len()
        implies (after.slots@[i].child is None <==> after.free@.contains(i as usize)) by {
        if i != root.slot as int && after.slots@[i].child is None {
            assert(before.free@.contains(i as usize));
        }
        if after.free@.contains(i as usize) {
            let position = choose|position: int| 0 <= position < after.free@.len()
                && after.free@[position] == i as usize;
            if position < before.free@.len() {
                assert(before.free@[position] == i as usize);
                assert(before.slots@[i].child is None);
            } else {
                assert(i == root.slot as int);
            }
        }
    }
}

pub(crate) proof fn revoked_absent(roots: &RootSlots, root: RootId)
    requires roots.revoked(root),
    ensures roots.lookup(root) is None,
{}

pub(super) proof fn release_capacity(roots: &RootSlots, root: RootId)
    requires roots.valid(), roots.lookup(root) is Some, root.slot != 0,
    ensures roots.free@.len() < hash_dag_std::vec_model::capacity(roots.free),
{
    let range = Set::<usize>::range(1usize, roots.slots@.len() as usize);
    vstd::set_lib::range_set_properties(1usize, roots.slots@.len() as usize);
    let available = range.remove(root.slot);
    roots.free@.unique_seq_to_set();
    assert forall|slot: usize| roots.free@.to_set().contains(slot) implies available.contains(slot) by {
        let pos = choose|pos: int| 0 <= pos < roots.free@.len() && roots.free@[pos] == slot;
        assert(roots.slots@[slot as int].child is None);
        assert(slot != 0 && slot != root.slot);
    }
    assert(roots.free@.to_set().subset_of(available));
    vstd::set_lib::lemma_len_subset(roots.free@.to_set(), available);
    assert(range.contains(root.slot));
    assert(available.len() + 1 == range.len());
}

}
