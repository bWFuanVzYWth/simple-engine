//! 批次状态与原值帧；所有数学快照均擦除。
use super::*;
use crate::owner::Key;
use vstd::prelude::*;

verus! {

broadcast use {vstd::seq_lib::group_seq_properties, Key::identity_fields};
pub(crate) open(in crate::lifecycle) spec fn state<B>(record: Record<B>, next: RecordState) -> Record<B> {
    Record { operations: record.operations, uses: record.uses, build_roots: record.build_roots, buffers: record.buffers, state: next }
}

pub(crate) open(in crate::lifecycle) spec fn selected(ids: Seq<RecordId>, count: int, owner: u64, key: Key) -> bool {
    exists|i: int| 0 <= i < count && ids[i].0.owner == owner && #[trigger] ids[i].0.identifies(key)
}

pub(crate) open(in crate::lifecycle) spec fn framed<B>(before: &Arena<Record<B>>, after: &Arena<Record<B>>) -> bool {
    &&& after.same_shape(before)
    &&& forall|key: Key| #![auto] before.lookup(key) is Some ==> {
        let a = before.lookup(key)->Some_0; let b = after.lookup(key)->Some_0;
        a.operations == b.operations && a.uses == b.uses && a.build_roots == b.build_roots && a.buffers == b.buffers
    }
}

pub(crate) open(in crate::lifecycle) spec fn written<B>(before: &Arena<Record<B>>, after: &Arena<Record<B>>, owner: u64,
    ids: Seq<RecordId>, count: int, next: RecordState) -> bool {
    &&& 0 <= count <= ids.len()
    &&& after.same_shape(before)
    &&& forall|key: Key| #![auto] after.lookup(key) == match before.lookup(key) {
        Some(record) => Some(if selected(ids, count, owner, key) { state(record, next) } else { record }), None => None,
    }
}

pub(crate) open(in crate::lifecycle) spec fn prefix<B>(records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, count: int, current: RecordState) -> bool {
    &&& 0 <= count <= ids.len()
    &&& forall|i: int| #![auto] 0 <= i < count ==> ids[i].0.owner == owner && records.lookup(ids[i].0) is Some
        && records.lookup(ids[i].0)->Some_0.state == current
    &&& forall|i: int, j: int| #![auto] 0 <= i < j < count ==> !ids[i].0.identifies(ids[j].0)
}

pub(crate) open(in crate::lifecycle) spec fn ready<B>(records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, serial: u64) -> bool {
    prefix(records, owner, ids, ids.len() as int, RecordState::Submitting(serial))
}

pub(crate) open(in crate::lifecycle) spec fn preparing<B>(original: &Arena<Record<B>>, records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, count: int, serial: u64) -> bool {
    &&& records.valid()
    &&& original.valid()
    &&& (count > 0 ==> serial > 0)
    &&& prefix(original, owner, ids, count, RecordState::Pending)
    &&& prefix(records, owner, ids, count, RecordState::Submitting(serial))
    &&& written(original, records, owner, ids, count, RecordState::Submitting(serial))
}

pub(crate) open(in crate::lifecycle) spec fn prepared<B>(before: &Arena<Record<B>>, after: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, serial: u64, result: Result<(), Error>) -> bool {
    &&& (result is Ok <==> input(before, owner, ids, serial))
    &&& framed(before, after)
    &&& (result is Err ==> identical(before, after))
    &&& (result is Ok ==> ready(after, owner, ids, serial)
        && prefix(before, owner, ids, ids.len() as int, RecordState::Pending)
        && written(before, after, owner, ids, ids.len() as int, RecordState::Submitting(serial)))
}

pub(crate) open(in crate::lifecycle) spec fn begun<B>(before: &Arena<Record<B>>, after: &Arena<Record<B>>, owner: u64, id: RecordId, serial: u64, result: Result<(), Error>) -> bool {
    &&& (result is Ok <==> allowed(before, owner, id, serial))
    &&& (result is Err ==> identical(before, after))
    &&& (result is Ok ==> id.0.owner == owner && before.lookup(id.0) is Some && before.lookup(id.0)->Some_0.state is Pending
        && serial > 0 && written(before, after, owner, seq![id], 1, RecordState::Submitting(serial)))
}

pub(crate) open(in crate::lifecycle) spec fn changing<B>(original: &Arena<Record<B>>, records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, count: int, i: int, current: RecordState, next: RecordState) -> bool {
    &&& records.valid()
    &&& prefix(original, owner, ids, count, current)
    &&& 0 <= i <= count
    &&& written(original, records, owner, ids, i, next)
    &&& forall|j: int| #![auto] i <= j < count ==> records.lookup(ids[j].0) is Some && records.lookup(ids[j].0)->Some_0.state == current
}

pub(super) proof fn prepared_step<B>(original: &Arena<Record<B>>, previous: &Arena<Record<B>>, records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, count: int, serial: u64)
    requires preparing(original, previous, owner, ids, count, serial), count < ids.len(), records.valid(),
        begun(previous, records, owner, ids[count], serial, Ok(())),
    ensures preparing(original, records, owner, ids, count + 1, serial),
{
    let key = ids[count].0;
    assert(!selected(ids, count, owner, key)) by {
        if selected(ids, count, owner, key) {
            let i = choose|i: int| #![auto] 0 <= i < count && ids[i].0.owner == owner && ids[i].0.identifies(key);
            previous.lookup_identity(ids[i].0, key);
            assert(previous.lookup(key)->Some_0.state == RecordState::Submitting(serial));
        }
    }
    assert(original.lookup(key) == previous.lookup(key));
    assert forall|i: int, j: int| #![auto] 0 <= i < j < count + 1 implies !ids[i].0.identifies(ids[j].0) by {
        if j == count { assert(!ids[i].0.identifies(key)); }
    }
    assert forall|i: int| #![auto] 0 <= i < count + 1 implies ids[i].0.owner == owner
        && original.lookup(ids[i].0) is Some && original.lookup(ids[i].0)->Some_0.state is Pending by {}
    assert(seq![ids[count]][0] == ids[count]);
    assert(ids[count].0.identifies(key));
    assert(selected(seq![ids[count]], 1, owner, key));
    assert(records.lookup(key) == Some(state(previous.lookup(key)->Some_0, RecordState::Submitting(serial))));
    extend_write(original, previous, records, owner, ids, count, RecordState::Submitting(serial));
    assert forall|i: int| #![auto] 0 <= i < count + 1 implies ids[i].0.owner == owner
        && records.lookup(ids[i].0) is Some && records.lookup(ids[i].0)->Some_0.state == RecordState::Submitting(serial) by {
        assert(ids[i].0.owner == owner);
        assert(ids[i].0.identifies(ids[i].0));
        assert(selected(ids, count + 1, owner, ids[i].0));
    }
}

pub(super) proof fn rolled_back<B>(original: &Arena<Record<B>>, previous: &Arena<Record<B>>, records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, count: int, serial: u64)
    requires preparing(original, previous, owner, ids, count, serial), records.valid(),
        written(previous, records, owner, ids, count, RecordState::Pending),
    ensures identical(original, records), framed(original, records),
{
    records.shape_transitive(previous, original);
    assert forall|key: Key| #![auto] records.lookup(key) == original.lookup(key) by {
        if selected(ids, count, owner, key) && original.lookup(key) is Some {
            let i = choose|i: int| #![auto] 0 <= i < count && ids[i].0.owner == owner && ids[i].0.identifies(key);
            original.lookup_identity(ids[i].0, key);
            assert(original.lookup(key)->Some_0.state is Pending);
        }
    }
    records.same_values_implies_same_entries(original);
}

pub(super) proof fn changed_step<B>(original: &Arena<Record<B>>, previous: &Arena<Record<B>>, records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, count: int, i: int, current: RecordState, next: RecordState)
    requires changing(original, previous, owner, ids, count, i, current, next), i < count,
        records.valid(), records.same_shape(previous),
        records.lookup(ids[i].0) == Some(state(previous.lookup(ids[i].0)->Some_0, next)),
        forall|key: Key| #![auto] !key.identifies(ids[i].0) ==> records.lookup(key) == previous.lookup(key),
    ensures changing(original, records, owner, ids, count, i + 1, current, next),
{
    extend_write(original, previous, records, owner, ids, i, next);
    assert forall|j: int| #![auto] i + 1 <= j < count implies records.lookup(ids[j].0) is Some
        && records.lookup(ids[j].0)->Some_0.state == current by {
        assert(!ids[j].0.identifies(ids[i].0));
    }
}

pub(super) proof fn quarantined_step<B>(original: &Arena<Record<B>>, previous: &Arena<Record<B>>, records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, i: int)
    requires written(original, previous, owner, ids, i, RecordState::Uncertain), i < ids.len(), records.valid(),
        written(previous, records, owner, seq![ids[i]], 1, RecordState::Uncertain),
    ensures written(original, records, owner, ids, i + 1, RecordState::Uncertain),
{
    assert(seq![ids[i]][0] == ids[i]);
    assert(ids[i].0.identifies(ids[i].0));
    if ids[i].0.owner == owner { assert(selected(seq![ids[i]], 1, owner, ids[i].0)); }
    extend_write(original, previous, records, owner, ids, i, RecordState::Uncertain);
}

pub(super) proof fn extend_write<B>(original: &Arena<Record<B>>, previous: &Arena<Record<B>>, records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, i: int, next: RecordState)
    requires written(original, previous, owner, ids, i, next), i < ids.len(), records.same_shape(previous),
        (ids[i].0.owner == owner && previous.lookup(ids[i].0) is Some ==> records.lookup(ids[i].0) == Some(state(previous.lookup(ids[i].0)->Some_0, next))),
        forall|key: Key| #![auto] ids[i].0.owner != owner || !key.identifies(ids[i].0) ==> records.lookup(key) == previous.lookup(key),
    ensures written(original, records, owner, ids, i + 1, next),
{
    records.shape_transitive(previous, original);
    assert forall|key: Key| #![auto] selected(ids, i + 1, owner, key) <==>
        selected(ids, i, owner, key) || (ids[i].0.owner == owner && ids[i].0.identifies(key)) by {
        if selected(ids, i + 1, owner, key) {
            let j = choose|j: int| #![auto] 0 <= j < i + 1 && ids[j].0.owner == owner && ids[j].0.identifies(key);
            if j < i { assert(selected(ids, i, owner, key)); }
        }
    }
    assert forall|key: Key| #![auto] records.lookup(key) == match original.lookup(key) {
        Some(record) => Some(if selected(ids, i + 1, owner, key) { state(record, next) } else { record }), None => None,
    } by {
        records.shape_preserves_presence(previous, key);
        if ids[i].0.owner == owner && ids[i].0.identifies(key) {
            previous.lookup_identity(ids[i].0, key);
            records.lookup_identity(ids[i].0, key);
            if let Some(record) = original.lookup(key) {
                assert(previous.lookup(key) is Some);
                assert(records.lookup(key) == Some(state(previous.lookup(key)->Some_0, next)));
                assert(state(previous.lookup(key)->Some_0, next) == state(record, next));
            }
        } else {
            assert(selected(ids, i + 1, owner, key) == selected(ids, i, owner, key));
        }
    }
}

pub(super) proof fn empty_write<B>(records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, next: RecordState)
    ensures written(records, records, owner, ids, 0, next),
{
    records.shape_reflexive();
}

pub(super) proof fn write_framed<B>(before: &Arena<Record<B>>, after: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, count: int, next: RecordState)
    requires written(before, after, owner, ids, count, next),
    ensures framed(before, after),
{}

pub(super) proof fn begun_done<B>(before: &Arena<Record<B>>, after: &Arena<Record<B>>, owner: u64, id: RecordId, serial: u64)
    requires id.0.owner == owner, serial > 0, before.lookup(id.0) is Some, before.lookup(id.0)->Some_0.state is Pending,
        after.same_shape(before), after.lookup(id.0) == Some(state(before.lookup(id.0)->Some_0, RecordState::Submitting(serial))),
        forall|key: Key| #![auto] !key.identifies(id.0) ==> after.lookup(key) == before.lookup(key),
    ensures begun(before, after, owner, id, serial, Ok(())),
{
    empty_write(before, owner, seq![id], RecordState::Submitting(serial));
    extend_write(before, before, after, owner, seq![id], 0, RecordState::Submitting(serial));
}

pub(super) proof fn unchanged<B>(before: &Arena<Record<B>>, after: &Arena<Record<B>>)
    requires before.valid(), after.valid(), after.same_shape(before), forall|key: Key| #![auto] after.lookup(key) == before.lookup(key),
    ensures identical(before, after), framed(before, after),
{
    after.same_values_implies_same_entries(before);
}

pub(crate) open(in crate::lifecycle) spec fn identical<B>(before: &Arena<Record<B>>, after: &Arena<Record<B>>) -> bool {
    after.same_shape(before) && after.same_entries(before)
        && forall|key: Key| #![auto] after.lookup(key) == before.lookup(key)
}

pub(super) proof fn unchanged_one<B>(before: &Arena<Record<B>>, after: &Arena<Record<B>>, key: Key)
    requires before.valid(), after.valid(), after.same_shape(before), after.lookup(key) == before.lookup(key),
        forall|other: Key| #![auto] !other.identifies(key) ==> after.lookup(other) == before.lookup(other),
    ensures identical(before, after), framed(before, after),
{
    assert forall|other: Key| #![auto] after.lookup(other) == before.lookup(other) by {
        if other.identifies(key) {
            before.lookup_identity(other, key);
            after.lookup_identity(other, key);
        }
    }
    unchanged(before, after);
}

pub(super) proof fn write_identical<B>(before: &Arena<Record<B>>, same: &Arena<Record<B>>, after: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, count: int, next: RecordState)
    requires identical(before, same), written(same, after, owner, ids, count, next),
    ensures written(before, after, owner, ids, count, next),
{
    after.shape_transitive(same, before);
}

pub(crate) open(in crate::lifecycle) spec fn input<B>(records: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, serial: u64) -> bool {
    prefix(records, owner, ids, ids.len() as int, RecordState::Pending) && (ids.len() == 0 || serial > 0)
}

pub(crate) open(in crate::lifecycle) spec fn allowed<B>(records: &Arena<Record<B>>, owner: u64, id: RecordId, serial: u64) -> bool {
    id.0.owner == owner && records.lookup(id.0) is Some && records.lookup(id.0)->Some_0.state is Pending && serial > 0
}

pub(super) proof fn rejected_input<B>(original: &Arena<Record<B>>, previous: &Arena<Record<B>>, owner: u64, ids: Seq<RecordId>, count: int, serial: u64)
    requires preparing(original, previous, owner, ids, count, serial), count < ids.len(), !allowed(previous, owner, ids[count], serial),
    ensures !input(original, owner, ids, serial),
{
    if input(original, owner, ids, serial) {
        let key = ids[count].0;
        assert(!selected(ids, count, owner, key)) by {
            if selected(ids, count, owner, key) {
                let i = choose|i: int| #![auto] 0 <= i < count && ids[i].0.owner == owner && ids[i].0.identifies(key);
                assert(!ids[i].0.identifies(ids[count].0));
            }
        }
        assert(previous.lookup(key) == original.lookup(key));
    }
}

fn duplicate_batch_is_atomic() {
    let mut records = Arena::new();
    let record = Record::new(Vec::new(), Vec::new(), Vec::new(), 42u64);
    let key = match records.try_insert(7, record) { Ok(key) => key, Err(_) => return };
    let id = RecordId::new(key);
    let ids = [id, id];
    let result = prepare(&mut records, 7, &ids, 1);
    assert(result is Err);
    assert(records.lookup(key) is Some && records.lookup(key)->Some_0.state is Pending);
    assert(records.lookup(key)->Some_0.buffers == 42);
    #[cfg(reject_batch_duplicate_acceptance)]
    assert(result is Ok);
    #[cfg(reject_batch_failed_prefix)]
    assert(records.lookup(key)->Some_0.state is Submitting);
}

fn accepted_batch_preserves_payload() {
    let mut records = Arena::new();
    let record = Record::new(Vec::new(), Vec::new(), Vec::new(), 42u64);
    let key = match records.try_insert(7, record) { Ok(key) => key, Err(_) => return };
    let id = RecordId::new(key);
    let ids = [id];
    let result = prepare(&mut records, 7, &ids, 1);
    assert(result is Ok);
    accept(&mut records, 7, &ids, 1);
    assert(records.lookup(key) is Some && records.lookup(key)->Some_0.state == RecordState::Submitted(1));
    assert(records.lookup(key)->Some_0.buffers == 42);
}

fn quarantine_does_not_accept() {
    let mut records = Arena::new();
    let record = Record::new(Vec::new(), Vec::new(), Vec::new(), 42u64);
    let key = match records.try_insert(7, record) { Ok(key) => key, Err(_) => return };
    let id = RecordId::new(key);
    let ids = [id];
    let result = prepare(&mut records, 7, &ids, 1);
    assert(result is Ok);
    quarantine(&mut records, 7, &ids);
    assert(records.lookup(key) is Some && records.lookup(key)->Some_0.state is Uncertain);
    assert(records.lookup(key)->Some_0.buffers == 42);
    #[cfg(reject_batch_quarantine_acceptance)]
    assert(records.lookup(key)->Some_0.state is Submitted);
}

pub(super) proof fn single_quarantined<B>(before: &Arena<Record<B>>, after: &Arena<Record<B>>, owner: u64, id: RecordId)
    requires written(before, before, owner, seq![id], 0, RecordState::Uncertain), after.same_shape(before),
        (id.0.owner == owner && before.lookup(id.0) is Some ==> after.lookup(id.0) == Some(state(before.lookup(id.0)->Some_0, RecordState::Uncertain))),
        forall|key: Key| #![auto] id.0.owner != owner || !key.identifies(id.0) ==> after.lookup(key) == before.lookup(key),
    ensures written(before, after, owner, seq![id], 1, RecordState::Uncertain),
{
    extend_write(before, before, after, owner, seq![id], 0, RecordState::Uncertain);
}

}
