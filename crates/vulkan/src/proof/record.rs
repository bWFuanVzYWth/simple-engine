use super::*;
use crate::{
    lifecycle::resource::{Data, ResourceKind},
    owner::Arena,
};
use vstd::prelude::*;

verus! {
impl Op {
    pub(crate) open(super) spec fn root_view(&self) -> Option<Key> {
        match self {
            Self::Trace(p, _) => Some(p.key), Self::Dispatch(p, _) => Some(p.key),
            Self::Clear(buffer) => Some(buffer.key), Self::Keepalive(resource) => Some(resource.0),
            Self::Access(access) => Some(match access {
                ResourceAccess::Buffer(buffer) => buffer.key, ResourceAccess::Image(image) => image.key,
            }),
            Self::ResetTimestamps(q) | Self::Timestamp(q, _) => Some(q.key), Self::Barrier => None,
        }
    }
    pub(crate) open(super) spec fn access_view(&self) -> Option<Key> {
        match self {
            Self::Clear(buffer) => Some(buffer.key), Self::Access(access) => Some(match access {
                ResourceAccess::Buffer(buffer) => buffer.key, ResourceAccess::Image(image) => image.key,
            }), Self::ResetTimestamps(q) => Some(q.key), _ => None,
        }
    }
    pub(crate) open(super) spec fn reset_view(&self) -> Option<Key> {
        match self { Self::ResetTimestamps(q) => Some(q.key), _ => None }
    }
}

impl<P: Payloads, B> Life<P, B> {
    pub(crate) open(in crate::lifecycle) spec fn records_valid(&self) -> bool { self.records.valid() && self.timeline.valid() }
    pub(crate) open(in crate::lifecycle) spec fn record_lookup(&self, id: RecordId) -> Option<Record<B>> {
        if id.0.owner == self.owner { self.records.lookup(id.0) } else { None }
    }
    pub(crate) open(in crate::lifecycle) spec fn submitted_value(&self) -> u64 { self.timeline.submitted_view() }
    pub(crate) open(in crate::lifecycle) spec fn completed_value(&self) -> u64 { self.timeline.completed_view() }
}

impl<B> Record<B> {
    pub(crate) open(in crate::lifecycle) spec fn operations_value(&self) -> Vec<Op> { self.operations }
    pub(crate) open(in crate::lifecycle) spec fn uses_value(&self) -> Vec<Key> { self.uses }
    pub(crate) open(in crate::lifecycle) spec fn roots_value(&self) -> Vec<Key> { self.build_roots }
    pub(crate) open(in crate::lifecycle) spec fn buffers_value(&self) -> B { self.buffers }
    pub(crate) open(in crate::lifecycle) spec fn state_value(&self) -> RecordState { self.state }
}

pub(crate) open(in crate::lifecycle) spec fn created<P: Payloads, B>(life: &Life<P, B>, owner: u64) -> bool {
    &&& life.owner == owner
    &&& life.resources.valid()
    &&& life.records.valid()
    &&& life.timeline.valid()
    &&& life.resources.slot_count() == 0
    &&& life.records.slot_count() == 0
    &&& life.retiring@.len() == 0
    &&& life.retirements@.len() == 0
}

pub(crate) open(in crate::lifecycle) spec fn unchanged_resources<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>) -> bool {
    after.owner == before.owner && after.resources == before.resources
        && after.retiring == before.retiring && after.retirements == before.retirements && after.timeline == before.timeline
}

pub(crate) open(in crate::lifecycle) spec fn transitioned<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, id: RecordId,
    next: RecordState, result: Result<(), Error>) -> bool
{
    &&& after.records_valid()
    &&& unchanged_resources(before, after)
    &&& after.records.same_shape(&before.records)
    &&& (result is Ok <==> before.record_lookup(id) is Some && before.record_lookup(id)->Some_0.state is Pending)
    &&& forall|key: Key| #![auto] after.records.lookup(key) == match before.records.lookup(key) {
        Some(record) => Some(if result is Ok && key.identifies(id.0) { super::batch::proof::state(record, next) } else { record }),
        None => None,
    }
}

pub(super) proof fn transition_done<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, id: RecordId, next: RecordState)
    requires before.records_valid(), after.records_valid(), unchanged_resources(before, after),
        id.0.owner == before.owner, before.records.lookup(id.0) is Some, before.records.lookup(id.0)->Some_0.state is Pending,
        after.records.same_shape(&before.records),
        after.records.lookup(id.0) == Some(super::batch::proof::state(before.records.lookup(id.0)->Some_0, next)),
        forall|key: Key| #![auto] !key.identifies(id.0) ==> after.records.lookup(key) == before.records.lookup(key),
    ensures transitioned(before, after, id, next, Ok(())),
{
    assert forall|key: Key| #![auto] after.records.lookup(key) == match before.records.lookup(key) {
        Some(record) => Some(if key.identifies(id.0) { super::batch::proof::state(record, next) } else { record }), None => None,
    } by {
        if key.identifies(id.0) { before.records.lookup_identity(key, id.0); after.records.lookup_identity(key, id.0); }
    }
}

pub(super) proof fn transition_rejected<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, id: RecordId, next: RecordState, error: Error)
    requires before.records_valid(), after.records_valid(), unchanged_resources(before, after),
        after.records.same_shape(&before.records),
        !(before.record_lookup(id) is Some && before.record_lookup(id)->Some_0.state is Pending),
        after.records.lookup(id.0) == before.records.lookup(id.0),
        forall|key: Key| #![auto] !key.identifies(id.0) ==> after.records.lookup(key) == before.records.lookup(key),
    ensures transitioned(before, after, id, next, Err(error)),
{
    assert forall|key: Key| #![auto] after.records.lookup(key) == before.records.lookup(key) by {
        if key.identifies(id.0) { before.records.lookup_identity(key, id.0); after.records.lookup_identity(key, id.0); }
    }
}

impl<'a, P: Payloads, B> Attempt<'a, P, B> {
    pub(crate) open(super) spec fn owner_ref(&self) -> &'a mut Life<P, B> { self.life }
    pub(crate) open(super) spec fn ids_value(&self) -> Seq<RecordId> { self.ids@ }
    pub(crate) open(super) spec fn serial_value(&self) -> u64 { self.serial }

    #[verifier::type_invariant]
    pub(super) open(super) spec fn valid(&self) -> bool {
        self.life.records_valid() && self.serial > 0
            && self.life.submitted_value() < u64::MAX && self.serial == self.life.submitted_value() + 1
            && super::batch::ready(&self.life.records, self.life.owner, self.ids@, self.serial)
    }
}

pub(crate) open(in crate::lifecycle) spec fn prepare_failed<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>) -> bool {
    after.records_valid() && unchanged_resources(before, after) && after.records.same_entries(&before.records)
}

pub(crate) open(super) spec fn attempt_started<P: Payloads, B>(before: &Life<P, B>, attempt: &Attempt<P, B>, ids: Seq<RecordId>) -> bool {
    &&& attempt.valid()
    &&& attempt.ids_value() == ids
    &&& unchanged_resources(before, attempt.life)
    &&& super::batch::framed(&before.records, &attempt.life.records)
    &&& super::batch::proof::prefix(&before.records, before.owner, ids, ids.len() as int, RecordState::Pending)
}

/// 外部兄弟模块只需退役组的精确帧，不展开 Attempt 的记录批次不变量。
pub(crate) proof fn attempt_groups_unchanged<P: Payloads, B>(before: &Life<P, B>, attempt: &Attempt<P, B>, ids: Seq<RecordId>)
    requires attempt_started(before, attempt, ids),
    ensures crate::lifecycle::proof::groups_unchanged(before, attempt.owner_ref()),
{}

pub(crate) open(in crate::lifecycle) spec fn resolved<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, ids: Seq<RecordId>, serial: u64, accepted: bool) -> bool {
    &&& after.records_valid()
    &&& after.owner == before.owner && after.resources == before.resources
    &&& after.retiring == before.retiring && after.retirements == before.retirements
    &&& after.completed_value() == before.completed_value()
    &&& after.submitted_value() == if accepted { serial } else { before.submitted_value() }
    &&& super::batch::proof::written(&before.records, &after.records, before.owner, ids, ids.len() as int,
        if accepted { RecordState::Submitted(serial) } else { RecordState::Uncertain })
}

pub(crate) enum Take { Pending, External, Completed, Sync(bool), Shutdown }

pub(crate) open(in crate::lifecycle) spec fn selected_root(roots: Seq<Key>, count: int, owner: u64, key: Key) -> bool {
    exists|i: int| 0 <= i < count && roots[i].owner == owner && #[trigger] roots[i].identifies(key)
}

pub(crate) open(in crate::lifecycle) spec fn producer_action<P: Payloads>(value: Data<P>, id: RecordId, clear: bool) -> Data<P> {
    if clear {
        if value.producer_value() == Some(id) { value.with_producer(None) } else { value }
    } else { value.with_producer(Some(id)) }
}

pub(crate) open(in crate::lifecycle) spec fn producers_written<P: Payloads>(before: &Arena<Data<P>>, after: &Arena<Data<P>>, owner: u64,
    roots: Seq<Key>, count: int, id: RecordId, clear: bool) -> bool
{
    &&& before.valid() && after.valid() && after.same_shape(before)
    &&& 0 <= count <= roots.len()
    &&& forall|key: Key| #![auto] after.lookup(key) == match before.lookup(key) {
        Some(value) => Some(if selected_root(roots, count, owner, key) { producer_action(value, id, clear) } else { value }), None => None,
    }
}

pub(super) proof fn producers_empty<P: Payloads>(resources: &Arena<Data<P>>, owner: u64, roots: Seq<Key>, id: RecordId, clear: bool)
    requires resources.valid(),
    ensures producers_written(resources, resources, owner, roots, 0, id, clear),
{ resources.shape_reflexive(); }

pub(super) proof fn producer_action_twice<P: Payloads>(value: Data<P>, id: RecordId, clear: bool)
    ensures producer_action(producer_action(value, id, clear), id, clear) == producer_action(value, id, clear),
{
    if clear {
        if value.producer_value() == Some(id) {
            value.producer_is_build_output();
            value.producer_updated(None);
        }
    } else { value.producer_overwrite(Some(id), Some(id)); }
}

pub(super) proof fn producers_step<P: Payloads>(original: &Arena<Data<P>>, previous: &Arena<Data<P>>, resources: &Arena<Data<P>>, owner: u64,
    roots: Seq<Key>, count: int, id: RecordId, clear: bool)
    requires producers_written(original, previous, owner, roots, count, id, clear), count < roots.len(),
        resources.valid(), resources.same_shape(previous),
        (roots[count].owner == owner && previous.lookup(roots[count]) is Some ==>
            resources.lookup(roots[count]) == Some(producer_action(previous.lookup(roots[count])->Some_0, id, clear))),
        forall|key: Key| #![auto] (roots[count].owner != owner || !key.identifies(roots[count])) ==>
            resources.lookup(key) == previous.lookup(key),
    ensures producers_written(original, resources, owner, roots, count + 1, id, clear),
{
    broadcast use crate::arena::Key::identity_fields;
    resources.shape_transitive(previous, original);
    assert forall|key: Key| #![auto] selected_root(roots, count + 1, owner, key) <==>
        selected_root(roots, count, owner, key) || (roots[count].owner == owner && roots[count].identifies(key)) by {
        if selected_root(roots, count + 1, owner, key) {
            let index = choose|index: int| #![auto] 0 <= index < count + 1 && roots[index].owner == owner && roots[index].identifies(key);
            if index < count { assert(selected_root(roots, count, owner, key)); }
        }
        if roots[count].owner == owner && roots[count].identifies(key) {
            assert(0 <= count < count + 1);
            assert(selected_root(roots, count + 1, owner, key));
        }
    }
    assert forall|key: Key| #![auto] resources.lookup(key) == match original.lookup(key) {
        Some(value) => Some(if selected_root(roots, count + 1, owner, key) { producer_action(value, id, clear) } else { value }), None => None,
    } by {
        if roots[count].owner == owner && key.identifies(roots[count]) {
            original.lookup_identity(key, roots[count]);
            previous.lookup_identity(key, roots[count]);
            resources.lookup_identity(key, roots[count]);
            resources.shape_preserves_presence(previous, key);
            if original.lookup(key) is Some { producer_action_twice(original.lookup(key)->Some_0, id, clear); }
        }
    }
}

pub(crate) open(in crate::lifecycle) spec fn root_finished<P: Payloads>(before: &Arena<Data<P>>, after: &Arena<Data<P>>, owner: u64, root: Key, id: RecordId) -> bool {
    &&& after.valid() && after.same_shape(before) && after.slot_count() == before.slot_count()
    &&& (root.owner == owner && before.lookup(root) is Some ==> after.lookup(root) == Some(producer_action(before.lookup(root)->Some_0, id, true)))
    &&& forall|key: Key| #![trigger after.lookup(key)] (root.owner != owner || !key.identifies(root)) ==> after.lookup(key) == before.lookup(key)
}

pub(super) proof fn finish_step<P: Payloads>(original: &Arena<Data<P>>, previous: &Arena<Data<P>>, resources: &Arena<Data<P>>, owner: u64,
    roots: Seq<Key>, count: int, id: RecordId)
    requires producers_written(original, previous, owner, roots, count, id, true), count < roots.len(),
        root_finished(previous, resources, owner, roots[count], id),
    ensures producers_written(original, resources, owner, roots, count + 1, id, true),
{
    producers_step(original, previous, resources, owner, roots, count, id, true);
}

pub(crate) open(in crate::lifecycle) spec fn producers_finished<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, roots: Seq<Key>, id: RecordId) -> bool {
    after.owner == before.owner && after.records == before.records && after.timeline == before.timeline
        && after.retiring == before.retiring && after.retirements == before.retirements
        && after.resources_valid()
        && producers_written(&before.resources, &after.resources, before.owner, roots, roots.len() as int, id, true)
}

pub(crate) open(in crate::lifecycle) spec fn finishing<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, roots: Seq<Key>, count: int, id: RecordId) -> bool {
    after.owner == before.owner && after.records == before.records && after.timeline == before.timeline
        && after.retiring == before.retiring && after.retirements == before.retirements
        && after.resources_valid()
        && producers_written(&before.resources, &after.resources, before.owner, roots, count, id, true)
}

pub(crate) open(in crate::lifecycle) spec fn roots_valid<P: Payloads>(resources: &Arena<Data<P>>, owner: u64, roots: Seq<Key>, count: int) -> bool {
    0 <= count <= roots.len() && forall|i: int| #![auto] 0 <= i < count ==> roots[i].owner == owner
        && resources.lookup(roots[i]) is Some && (resources.lookup(roots[i])->Some_0.kind_value() == ResourceKind::Blas
            || resources.lookup(roots[i])->Some_0.kind_value() == ResourceKind::Acceleration)
}

pub(crate) open(in crate::lifecycle) spec fn build_inputs<P: Payloads, B>(life: &Life<P, B>, record: &Record<B>) -> bool {
    roots_valid(&life.resources, life.owner, record.build_roots@, record.build_roots@.len() as int)
}

pub(crate) open(in crate::lifecycle) spec fn inserting<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, value: Record<B>, id: RecordId, count: int) -> bool {
    &&& after.owner == before.owner && after.timeline == before.timeline
    &&& after.retiring == before.retiring && after.retirements == before.retirements
    &&& after.records_valid() && after.resources_valid()
    &&& value.state is Pending && id.0.owner == before.owner
    &&& before.record_lookup(id) is None && after.record_lookup(id) == Some(value)
    &&& roots_valid(&before.resources, before.owner, value.build_roots@, value.build_roots@.len() as int)
    &&& producers_written(&before.resources, &after.resources, before.owner, value.build_roots@, count, id, false)
    &&& forall|key: Key| #![auto] !key.identifies(id.0) ==> after.records.lookup(key) == before.records.lookup(key)
    &&& forall|key: Key| #![auto] before.records.revoked(key) ==> after.records.revoked(key)
}

pub(crate) open(in crate::lifecycle) spec fn record_inserted<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, value: Record<B>, result: Result<RecordId, (Error, Record<B>)>) -> bool {
    (forall|key: Key| #![auto] before.records.revoked(key) ==> after.records.revoked(key)) && match result {
        Ok(id) => inserting(before, after, value, id, value.build_roots@.len() as int),
        Err((_, returned)) => returned == value && after.records_valid() && after.resources_valid()
            && unchanged_resources(before, after) && after.records.same_entries(&before.records),
    }
}

pub(crate) open(in crate::lifecycle) spec fn may_take<P: Payloads, B>(life: &Life<P, B>, id: RecordId, mode: Take) -> bool {
    life.record_lookup(id) is Some && {
        let state = life.record_lookup(id)->Some_0.state;
        match mode {
            Take::Pending => state is Pending, Take::External => state is External,
            Take::Completed => state matches RecordState::Submitted(n) && n <= life.completed_value(),
            Take::Sync(done) => state is Synchronous || (!done && state is Pending), Take::Shutdown => true,
        }
    }
}

pub(crate) open(in crate::lifecycle) spec fn taken<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, id: RecordId,
    mode: Take, result: Result<Record<B>, Error>) -> bool
{
    &&& after.records_valid() && after.resources_valid()
    &&& after.owner == before.owner && after.timeline == before.timeline
    &&& after.retiring == before.retiring && after.retirements == before.retirements
    &&& (result is Ok <==> may_take(before, id, mode))
    &&& (result is Ok ==> result->Ok_0 == before.record_lookup(id)->Some_0
        && after.record_lookup(id) is None && after.records.revoked(id.0))
    &&& (result is Err ==> after.records.same_entries(&before.records) && after.resources == before.resources
        && forall|key: Key| after.records.lookup(key) == before.records.lookup(key))
    &&& (result is Ok ==> {
        let roots = result->Ok_0.build_roots@;
        let clears = match mode { Take::Pending => false, Take::Sync(done) => done, _ => true };
        if clears { producers_written(&before.resources, &after.resources, before.owner, roots, roots.len() as int, id, true) }
        else { after.resources == before.resources }
    })
    &&& forall|key: Key| #![auto] !key.identifies(id.0) ==> after.records.lookup(key) == before.records.lookup(key)
    &&& forall|key: Key| #![auto] before.records.revoked(key) ==> after.records.revoked(key)
}

fn record_buffers_are_taken_once<P: Payloads, B>(life: &mut Life<P, B>, id: RecordId)
    requires old(life).records_valid(), old(life).resources_valid(),
        old(life).record_lookup(id) is Some, old(life).record_lookup(id)->Some_0.state is Pending,
{
    let ghost expected = life.record_lookup(id)->Some_0.buffers;
    let first = life.take_pending(id);
    assert(first is Ok);
    let record = match first { Ok(record) => record, Err(_) => return };
    let buffers = record.into_buffers();
    assert(buffers == expected);
    let second = life.take_pending(id);
    assert(second is Err);
    #[cfg(reject_record_double_take)]
    assert(second is Ok);
}

fn removed_record_cannot_revive<P: Payloads, B>(life: &mut Life<P, B>, id: RecordId, buffers: B)
    requires old(life).records_valid(), old(life).resources_valid(),
        old(life).record_lookup(id) is Some, old(life).record_lookup(id)->Some_0.state is Pending,
{
    let first = life.take_pending(id);
    assert(first is Ok);
    let record = Record::new(Vec::new(), Vec::new(), Vec::new(), buffers);
    let _inserted = life.insert_record(record);
    proof { crate::arena::revoked_absent(&life.records, id.0); }
    let lookup = life.record(id);
    assert(lookup is Err);
    #[cfg(reject_record_generation_revival)]
    assert(lookup is Ok);
}

fn external_record_requires_external_completion<P: Payloads, B>(life: &mut Life<P, B>, id: RecordId)
    requires old(life).records_valid(), old(life).resources_valid(),
        old(life).record_lookup(id) is Some, old(life).record_lookup(id)->Some_0.state is Pending,
{
    let marked = life.mark_external(id);
    assert(marked is Ok);
    let cancelled = life.take_pending(id);
    assert(cancelled is Err);
    let completed = life.take_external(id);
    assert(completed is Ok);
    let second = life.take_external(id);
    assert(second is Err);
    #[cfg(reject_record_external_cancel)]
    assert(cancelled is Ok);
}

fn cancelled_build_keeps_producer_tombstone<P: Payloads, B>(life: &mut Life<P, B>, id: RecordId, root: Key)
    requires old(life).records_valid(), old(life).resources_valid(),
        old(life).record_lookup(id) is Some, old(life).record_lookup(id)->Some_0.state is Pending,
        old(life).resource_lookup(root) is Some, old(life).resource_lookup(root)->Some_0.producer_value() == Some(id),
{
    let removed = life.take_pending(id);
    assert(removed is Ok);
    let active = life.active(root);
    assert(active is Err);
    #[cfg(reject_record_cancelled_producer)]
    assert(active is Ok);
}

fn managed_attempt_updates_real_owner<P: Payloads, B>(life: &mut Life<P, B>, id: RecordId)
    requires old(life).records_valid(), old(life).resources_valid(), old(life).submitted_value() < u64::MAX,
        old(life).record_lookup(id) is Some, old(life).record_lookup(id)->Some_0.state is Pending,
{
    let ghost expected = life.record_lookup(id)->Some_0.buffers;
    let ids = [id];
    let attempt = match life.prepare_batch(&ids) { Ok(attempt) => attempt, Err(_) => return };
    proof { id.0.identity_with_owner(id.0); }
    let serial = attempt.accept();
    assert(ids@[0] == id);
    assert(super::batch::proof::selected(ids@, 1, life.owner, id.0));
    assert(life.submitted_value() == serial);
    assert(life.record_lookup(id) is Some && life.record_lookup(id)->Some_0.state == RecordState::Submitted(serial));
    assert(life.record_lookup(id)->Some_0.buffers == expected);
    #[cfg(reject_record_attempt_reversion)]
    assert(life.record_lookup(id)->Some_0.state is Pending);
}

fn completed_build_clears_matching_producer<P: Payloads, B>(life: &mut Life<P, B>, id: RecordId, root: Key)
    requires old(life).records_valid(), old(life).resources_valid(),
        old(life).record_lookup(id) is Some, old(life).record_lookup(id)->Some_0.state is External,
        old(life).record_lookup(id)->Some_0.build_roots@.contains(root),
        old(life).resource_lookup(root) is Some, old(life).resource_lookup(root)->Some_0.producer_value() == Some(id),
{
    let ghost before = *life;
    let completed = life.take_external(id);
    assert(completed is Ok);
    let ghost roots = completed->Ok_0.build_roots@;
    proof {
        root.identity_with_owner(root);
        let index = choose|i: int| 0 <= i < roots.len() && roots[i] == root;
        assert(selected_root(roots, roots.len() as int, life.owner, root));
        before.resource_lookup(root)->Some_0.producer_is_build_output();
        before.resource_lookup(root)->Some_0.producer_updated(None);
    }
    assert(life.resource_lookup(root) is Some && life.resource_lookup(root)->Some_0.producer_value() is None);
    #[cfg(reject_record_completed_producer)]
    assert(life.resource_lookup(root)->Some_0.producer_value() == Some(id));
}
}
