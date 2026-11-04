use super::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

impl<T> View for Values<'_, T> {
    type V = Seq<T>;
    open(super) spec fn view(&self) -> Seq<T> {
        self.remaining@.filter_map(|slot: Slot<T>| slot.value)
    }
}

impl<T> Values<'_, T> {
    pub(crate) open(super) spec fn slots_left(&self) -> nat { self.remaining@.len() }
}

pub(super) proof fn values_step<T>(slots: Seq<Slot<T>>)
    requires slots.len() > 0,
    ensures slots.filter_map(|slot: Slot<T>| slot.value) ==
        match slots[0].value {
            Some(value) => seq![value] + slots.drop_first().filter_map(|slot: Slot<T>| slot.value),
            None => slots.drop_first().filter_map(|slot: Slot<T>| slot.value),
        },
    decreases slots.len(),
{
    reveal_with_fuel(Seq::filter_map, 2);
    if slots.len() > 1 {
        let prefix = slots.drop_last();
        values_step(prefix);
        assert(prefix.drop_first().push(slots.last()) =~= slots.drop_first());
        assert(prefix.push(slots.last()) =~= slots);
        assert(slots.drop_first().drop_last() =~= prefix.drop_first());
        assert(slots.drop_first().last() == slots.last());
        assert(prefix[0] == slots[0]);
    } else {
        assert(slots =~= seq![slots[0]]);
        assert(slots.drop_first() =~= Seq::<Slot<T>>::empty());
    }
}

impl Key {
    pub(crate) open(super) spec fn owned_by(self, owner: u64) -> bool { self.owner == owner }

    pub(crate) open(super) spec fn identifies(self, other: Key) -> bool {
        self.slot == other.slot && self.generation == other.generation
    }

    pub(crate) proof fn identity_with_owner(self, other: Key)
        requires self.owner == other.owner,
        ensures self.identifies(other) <==> self == other,
    {}

    pub(crate) broadcast proof fn identity_fields(self, other: Key)
        ensures #[trigger] self.identifies(other) <==> self.slot == other.slot && self.generation == other.generation,
    {}
}

impl<T> Arena<T> {
    pub(crate) open(super) spec fn values_view(&self) -> Seq<T> {
        self.slots@.filter_map(|slot: Slot<T>| slot.value)
    }

    /// 完整身份查到的原值必在实际槽位游标中。
    pub(crate) proof fn lookup_in_values(&self, key: Key)
        requires self.lookup(key) is Some,
        ensures self.values_view().contains(self.lookup(key)->Some_0),
    {
        self.lookup_bounds(key);
        value_survives(self.slots@, key.slot as int);
    }

    pub(crate) open(super) spec fn slot_count(&self) -> nat { self.slots@.len() }

    pub(crate) open(super) spec fn occupied(&self, slot: usize) -> bool {
        slot < self.slots@.len() && self.slots@[slot as int].value is Some
    }

    pub(crate) open(super) spec fn generation_at(&self, slot: usize) -> u32 {
        self.slots@[slot as int].generation
    }

    pub(super) open spec fn key_candidates(&self, owner: u64) -> Seq<Option<Key>> {
        Seq::new(self.slots@.len(), |slot: int| {
            if self.slots@[slot].value is Some {
                Some(Key { owner, slot: slot as u32, generation: self.slots@[slot].generation })
            } else { None }
        })
    }

    pub(crate) open(super) spec fn keys_view(&self, owner: u64) -> Seq<Key> {
        self.key_candidates(owner).filter_map(|key: Option<Key>| key)
    }

    pub(crate) open(super) spec fn keys_through(&self, owner: u64, count: nat) -> Seq<Key> {
        self.key_candidates(owner).take(count as int).filter_map(|key: Option<Key>| key)
    }

    pub(crate) proof fn slot_bound(&self)
        requires self.valid(),
        ensures self.slot_count() <= u32::MAX as int + 1,
    {}

    pub(crate) proof fn keys_through_empty(&self, owner: u64)
        ensures self.keys_through(owner, 0) == Seq::<Key>::empty(),
    {}

    pub(crate) proof fn keys_through_step(&self, owner: u64, slot: usize, entry: Option<(Key, &T)>)
        requires self.valid(), slot < self.slot_count(), read_at(self, slot, owner, entry),
        ensures self.keys_through(owner, slot as nat + 1) == match entry {
            Some((key, _)) => self.keys_through(owner, slot as nat).push(key),
            None => self.keys_through(owner, slot as nat),
        },
    {
        let before = self.keys_through(owner, slot as nat);
        let after = match entry { Some((key, _)) => before.push(key), None => before };
        assert(keys_prefix(self, owner, slot as int, before));
        assert(after == match self.key_candidates(owner)[slot as int] {
            Some(key) => before.push(key), None => before,
        });
        keys_step(self, owner, slot, before, after);
    }

    pub(crate) proof fn keys_through_all(&self, owner: u64)
        ensures self.keys_through(owner, self.slot_count()) == self.keys_view(owner),
    {}

    pub(crate) proof fn keys_contains(&self, owner: u64, key: Key)
        requires self.valid(),
        ensures self.keys_view(owner).contains(key) <==> key.owner == owner && self.lookup(key) is Some,
    {
        let candidates = self.key_candidates(owner);
        let keep = |item: Option<Key>| item;
        if self.keys_view(owner).contains(key) {
            candidates.lemma_filter_map_contains(keep, key);
            assert(candidates.contains(Some(key)));
            let i = choose|i: int| 0 <= i < candidates.len() && candidates[i] == Some(key);
            assert(self.slots@[i].value is Some);
            assert(key.slot == i);
        }
        if key.owner == owner && self.lookup(key) is Some {
            let i = key.slot as int;
            assert(candidates[i] == Some(key));
            candidate_survives(candidates, key, i);
        }
    }

    pub(crate) proof fn lookup_bounds(&self, key: Key)
        requires self.lookup(key) is Some,
        ensures key.slot < self.slot_count(), self.occupied(key.slot as usize),
            self.generation_at(key.slot as usize) == key.generation,
    {}

    pub(crate) proof fn lookup_identity(&self, a: Key, b: Key)
        requires a.identifies(b),
        ensures self.lookup(a) == self.lookup(b),
    {}

    pub open(super) spec fn valid(&self) -> bool {
        self.slots@.len() <= u32::MAX as int + 1
            && self.free@.no_duplicates()
            && (forall|i: int| #![auto] 0 <= i < self.free@.len() ==>
                (self.free@[i] as int) < self.slots@.len()
                && self.slots@[self.free@[i] as int].value is None)
            && (forall|i: int| #![auto] 0 <= i < self.slots@.len()
                && self.slots@[i].value is None
                && self.slots@[i].generation < u32::MAX ==>
                    self.free@.contains(i as u32))
    }

    pub(crate) open(super) spec fn lookup(&self, key: Key) -> Option<T> {
        if (key.slot as int) < self.slots@.len()
            && self.slots@[key.slot as int].generation == key.generation {
            self.slots@[key.slot as int].value
        } else {
            None
        }
    }

    pub(crate) open(super) spec fn revoked(&self, key: Key) -> bool {
        (key.slot as int) < self.slots@.len()
            && (key.generation < self.slots@[key.slot as int].generation
                || (key.generation == self.slots@[key.slot as int].generation
                    && self.slots@[key.slot as int].value is None
                    && !self.free@.contains(key.slot)))
    }

    pub(crate) open(super) spec fn same_entries(&self, other: &Self) -> bool {
        self.slots@ =~= other.slots@ && self.free@ =~= other.free@
    }

    pub(crate) open(super) spec fn same_shape(&self, other: &Self) -> bool {
        &&& self.slots@.len() == other.slots@.len()
        &&& self.free@ == other.free@
        &&& (forall|i: int| 0 <= i < self.slots@.len() ==>
            self.slots@[i].generation == other.slots@[i].generation
            && (self.slots@[i].value is Some <==> other.slots@[i].value is Some))
    }

    pub(crate) proof fn shape_preserves_keys(&self, other: &Self, owner: u64)
        requires self.same_shape(other),
        ensures self.keys_view(owner) == other.keys_view(owner),
    {
        assert(self.key_candidates(owner) =~= other.key_candidates(owner));
    }

    pub(crate) proof fn shape_reflexive(&self)
        ensures self.same_shape(self), self.same_entries(self),
    {}

    pub(crate) proof fn entries_preserve_values(&self, other: &Self)
        requires self.same_entries(other),
        ensures forall|key: Key| self.lookup(key) == other.lookup(key),
    {}

    pub(crate) proof fn shape_transitive(&self, middle: &Self, other: &Self)
        requires self.same_shape(middle), middle.same_shape(other),
        ensures self.same_shape(other),
    {}

    pub(crate) proof fn shape_preserves_presence(&self, other: &Self, key: Key)
        requires self.same_shape(other),
        ensures self.lookup(key) is Some <==> other.lookup(key) is Some,
    {}

    pub(crate) proof fn shape_slots(&self, other: &Self)
        requires self.same_shape(other),
        ensures self.slot_count() == other.slot_count(),
            forall|slot: usize| slot < self.slot_count() ==> self.occupied(slot) == other.occupied(slot)
                && self.generation_at(slot) == other.generation_at(slot),
    {}

    pub(crate) proof fn occupied_lookup(&self, slot: usize, owner: u64)
        requires self.valid(), slot < self.slot_count(), self.occupied(slot),
        ensures self.lookup(Key { owner, slot: slot as u32, generation: self.generation_at(slot) }) is Some,
    {}

    pub(crate) proof fn same_values_implies_same_entries(&self, other: &Self)
        requires self.valid(), other.valid(), self.same_shape(other),
            forall|key: Key| self.lookup(key) == other.lookup(key),
        ensures self.same_entries(other),
    {
        assert(self.slots@ =~= other.slots@) by {
            assert forall|i: int| 0 <= i < self.slots@.len() implies
                self.slots@[i] == other.slots@[i] by {
                if self.slots@[i].value is Some {
                    let key = Key { owner: 0, slot: i as u32, generation: self.slots@[i].generation };
                    assert(key.slot == i);
                    assert(self.lookup(key) == self.slots@[i].value);
                    assert(other.lookup(key) == other.slots@[i].value);
                }
            }
        }
    }
}

proof fn value_survives<T>(slots: Seq<Slot<T>>, index: int)
    requires 0 <= index < slots.len(), slots[index].value is Some,
    ensures slots.filter_map(|slot: Slot<T>| slot.value).contains(slots[index].value->Some_0),
    decreases slots.len(),
{
    reveal(Seq::filter_map);
    if index + 1 < slots.len() { value_survives(slots.drop_last(), index); }
}

proof fn candidate_survives(candidates: Seq<Option<Key>>, key: Key, i: int)
    requires 0 <= i < candidates.len(), candidates[i] == Some(key),
    ensures candidates.filter_map(|item: Option<Key>| item).contains(key),
    decreases candidates.len(),
{
    reveal(Seq::filter_map);
    if i + 1 < candidates.len() {
        candidate_survives(candidates.drop_last(), key, i);
    }
}

pub(super) open spec fn keys_prefix<T>(arena: &Arena<T>, owner: u64, slot: int, keys: Seq<Key>) -> bool {
    arena.valid() && 0 <= slot <= arena.slots@.len()
        && keys == arena.key_candidates(owner).take(slot).filter_map(|key: Option<Key>| key)
}

pub(super) proof fn keys_step<T>(arena: &Arena<T>, owner: u64, slot: usize, before: Seq<Key>, after: Seq<Key>)
    requires
        keys_prefix(arena, owner, slot as int, before), slot < arena.slots@.len(),
        after == match arena.key_candidates(owner)[slot as int] {
            Some(key) => before.push(key), None => before,
        },
    ensures keys_prefix(arena, owner, slot as int + 1, after),
{
    let candidates = arena.key_candidates(owner);
    let keep = |key: Option<Key>| key;
    candidates.lemma_filter_map_take_succ(keep, slot as int);
    assert(after == candidates.take(slot as int + 1).filter_map(keep));
}

pub(super) proof fn keys_complete<T>(arena: &Arena<T>, owner: u64, keys: Seq<Key>)
    requires keys_prefix(arena, owner, arena.slots@.len() as int, keys),
    ensures keys == arena.keys_view(owner),
{}

pub(crate) open spec fn read_at<T>(arena: &Arena<T>, slot: usize, owner: u64, entry: Option<(Key, &T)>) -> bool {
    &&& (entry is Some <==> arena.occupied(slot))
    &&& (entry is Some ==> {
        let (key, value) = entry->Some_0;
        &&& slot < arena.slot_count()
        &&& key.owner == owner
        &&& key.slot == slot
        &&& key.generation == arena.generation_at(slot)
        &&& arena.lookup(key) == Some(*value)
    })
    &&& (forall|key: Key| #![auto] key.owned_by(owner) && key.slot == slot
        && arena.lookup(key) is Some ==> entry is Some && entry->Some_0.0 == key)
}

pub(crate) open spec fn preserves_others<T>(before: &Arena<T>, after: &Arena<T>, key: Key) -> bool {
    forall|previous: Key| #![auto] !previous.identifies(key) ==>
        after.lookup(previous) == before.lookup(previous)
}

pub(crate) open spec fn preserves_revoked<T>(before: &Arena<T>, after: &Arena<T>) -> bool {
    forall|key: Key| before.revoked(key) ==> after.revoked(key)
}

pub(crate) open spec fn inserted<T>(
    before: &Arena<T>, after: &Arena<T>, owner: u64, value: T, result: Result<Key, (InsertError, T)>,
) -> bool {
    &&& after.valid()
    &&& before.slot_count() <= after.slot_count() <= before.slot_count() + 1
    &&& (result is Err ==> after.slot_count() == before.slot_count())
    &&& (result is Ok ==> result->Ok_0.slot < after.slot_count())
    &&& (result is Ok ==> result->Ok_0.owner == owner)
    &&& (result is Ok ==> after.lookup(result->Ok_0) == Some(value))
    &&& (result is Ok ==> result->Ok_0.owned_by(owner) && before.lookup(result->Ok_0) is None)
    &&& (result is Err ==> result->Err_0.1 == value)
    &&& (result is Err ==> after.same_entries(before))
    &&& (result is Ok ==> preserves_others(before, after, result->Ok_0))
    &&& preserves_revoked(before, after)
}

pub(crate) open spec fn removed_entry<T>(before: &Arena<T>, after: &Arena<T>, key: Key, result: Option<T>) -> bool {
    &&& after.valid()
    &&& after.slot_count() == before.slot_count()
    &&& after.lookup(key) is None
    &&& result == before.lookup(key)
    &&& preserves_others(before, after, key)
    &&& (result is Some ==> after.revoked(key))
    &&& preserves_revoked(before, after)
}

pub(crate) open spec fn borrowed_state<T>(before: &Arena<T>, after: &Arena<T>, key: Key, found: bool) -> bool {
    &&& after.valid()
    &&& after.slot_count() == before.slot_count()
    &&& after.same_shape(before)
    &&& (found <==> before.lookup(key) is Some)
    &&& (!found ==> after.same_entries(before))
    &&& (!found ==> forall|other: Key| after.lookup(other) == before.lookup(other))
    &&& preserves_others(before, after, key)
    &&& preserves_revoked(before, after)
}

pub(super) proof fn insert_reused<T>(before: Arena<T>, after: Arena<T>, index: u32)
    requires
        before.valid(),
        before.free@.len() > 0,
        index == before.free@.last(),
        after.free@ == before.free@.drop_last(),
        after.slots@.len() == before.slots@.len(),
        after.slots@[index as int].generation == before.slots@[index as int].generation,
        after.slots@[index as int].value is Some,
        forall|i: int| 0 <= i < before.slots@.len() && i != index as int ==>
            after.slots@[i] == before.slots@[i],
    ensures
        after.valid(),
        forall|key: Key| before.revoked(key) ==> after.revoked(key),
{
    let remaining = before.free@.len() - 1;
    assert forall|i: int, j: int| 0 <= i < remaining && 0 <= j < remaining && i != j
        implies after.free@[i] != after.free@[j] by {
        assert(before.free@[i] != before.free@[j]);
    }
    assert forall|i: int| #![auto] 0 <= i < after.free@.len() implies
        (after.free@[i] as int) < after.slots@.len()
            && after.slots@[after.free@[i] as int].value is None by {
        assert(before.free@[i] != before.free@[remaining]);
        assert(before.slots@[before.free@[i] as int].value is None);
    }
    assert forall|i: int| #![auto] 0 <= i < after.slots@.len()
        && after.slots@[i].value is None && after.slots@[i].generation < u32::MAX
        implies after.free@.contains(i as u32) by {
        assert(i != index as int);
        assert(before.free@.contains(i as u32));
        let position = choose|position: int| 0 <= position < before.free@.len()
            && before.free@[position] == i as u32;
        assert(position < remaining);
        assert(after.free@[position] == i as u32);
    }
}

pub(super) proof fn insert_new<T>(before: Arena<T>, after: Arena<T>)
    requires
        before.valid(), before.free@.len() == 0,
        before.slots@.len() <= u32::MAX,
        after.free@ == before.free@,
        after.slots@.len() == before.slots@.len() + 1,
        forall|i: int| 0 <= i < before.slots@.len() ==> after.slots@[i] == before.slots@[i],
        after.slots@.last().generation == 0,
        after.slots@.last().value is Some,
    ensures
        after.valid(),
        forall|key: Key| before.revoked(key) ==> after.revoked(key),
{}

pub(super) proof fn removed<T>(before: Arena<T>, after: Arena<T>, key: Key)
    requires
        before.valid(), before.lookup(key) is Some,
        after.slots@.len() == before.slots@.len(),
        after.slots@[key.slot as int].value is None,
        forall|i: int| 0 <= i < before.slots@.len() && i != key.slot as int ==>
            after.slots@[i] == before.slots@[i],
        key.generation < u32::MAX ==> after.slots@[key.slot as int].generation == key.generation + 1
            && after.free@ == before.free@.push(key.slot),
        key.generation == u32::MAX ==> after.slots@[key.slot as int].generation == u32::MAX
            && after.free@ == before.free@,
    ensures
        after.valid(), after.revoked(key),
        forall|previous: Key| before.revoked(previous) ==> after.revoked(previous),
{
    assert(!before.free@.contains(key.slot)) by {
        if before.free@.contains(key.slot) {
            let position = choose|position: int| 0 <= position < before.free@.len()
                && before.free@[position] == key.slot;
            assert(before.slots@[key.slot as int].value is None);
        }
    }
    if key.generation < u32::MAX {
        assert forall|i: int, j: int| 0 <= i < after.free@.len()
            && 0 <= j < after.free@.len() && i != j
            implies after.free@[i] != after.free@[j] by {
            if i < before.free@.len() && j < before.free@.len() {
                assert(before.free@[i] != before.free@[j]);
            }
        }
    }
    assert forall|i: int| #![auto] 0 <= i < after.free@.len() implies
        (after.free@[i] as int) < after.slots@.len()
            && after.slots@[after.free@[i] as int].value is None by {
        if i < before.free@.len() {
            assert(before.slots@[before.free@[i] as int].value is None);
        }
    }
    assert forall|i: int| #![auto] 0 <= i < after.slots@.len()
        && after.slots@[i].value is None && after.slots@[i].generation < u32::MAX
        implies after.free@.contains(i as u32) by {
        if i != key.slot as int {
            assert(before.free@.contains(i as u32));
        }
    }
}

pub(crate) proof fn revoked_absent<T>(arena: &Arena<T>, key: Key)
    requires arena.revoked(key),
    ensures arena.lookup(key) is None,
{}

}
