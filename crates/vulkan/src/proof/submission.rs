//! 故障不可逆与乱序完成的组合证明。

use super::{DeviceState, FailureKind, RecordState, Timeline};
use vstd::prelude::*;

verus! {

impl DeviceState {
    pub(crate) open(super) spec fn failure_view(self) -> Option<FailureKind> {
        match self {
            Self::Active(_) => None,
            Self::Failed(failed) => Some(failed.kind),
        }
    }
}

impl Timeline {
    pub(crate) open(super) spec fn submitted_view(&self) -> u64 { self.submitted }

    pub(crate) open(super) spec fn completed_view(&self) -> u64 { self.completed }

    pub(crate) open(super) spec fn valid(&self) -> bool {
        self.completed <= self.submitted
    }
}

pub(crate) open spec fn failed(before: DeviceState, after: DeviceState, kind: FailureKind) -> bool {
    &&& after is Failed
    &&& (before is Failed ==> after == before)
    &&& after.failure_view() == match before.failure_view() {
        Some(first) => Some(first),
        None => Some(kind),
    }
}

pub(crate) open spec fn record_completed(state: RecordState, serial: u64) -> bool {
    state matches RecordState::Submitted(n) && n <= serial
}

pub(crate) open spec fn begin(state: RecordState, serial: u64) -> Option<RecordState> {
    if state is Pending && serial > 0 { Some(RecordState::Submitting(serial)) } else { None }
}

pub(crate) open spec fn begin_sync(state: RecordState) -> Option<RecordState> {
    if state is Pending { Some(RecordState::Synchronous) } else { None }
}

pub(crate) open spec fn accept_record(state: RecordState, serial: u64) -> Option<RecordState> {
    match state {
        RecordState::Submitting(n) => if n == serial { Some(RecordState::Submitted(serial)) } else { None },
        _ => None,
    }
}

pub(crate) open spec fn empty(value: &Timeline) -> bool {
    value.valid() && value.submitted_view() == 0 && value.completed_view() == 0
}

pub(crate) open spec fn next_serial(value: &Timeline) -> Option<u64> {
    if value.submitted_view() < u64::MAX {
        Some((value.submitted_view() + 1) as u64)
    } else { None }
}

pub(crate) open spec fn accepted_timeline(before: &Timeline, after: &Timeline, serial: u64, accepted: bool) -> bool {
    &&& after.valid()
    &&& accepted == (before.submitted_view() < u64::MAX && serial == before.submitted_view() + 1)
    &&& (accepted ==> after.submitted_view() == serial
        && serial > before.submitted_view()
        && after.completed_view() == before.completed_view())
    &&& (!accepted ==> *after == *before)
}

pub(crate) open spec fn observed(before: &Timeline, after: &Timeline, serial: u64, accepted: bool) -> bool {
    &&& after.valid()
    &&& after.submitted_view() == before.submitted_view()
    &&& after.completed_view() >= before.completed_view()
    &&& accepted == (serial <= before.submitted_view())
    &&& (accepted ==> after.completed_view() == if serial > before.completed_view() {
        serial
    } else { before.completed_view() })
    &&& (!accepted ==> *after == *before)
}

fn failure_is_terminal(kind: FailureKind, later: FailureKind) {
    let mut state = DeviceState::new();
    state.fail(kind);
    state.fail(later);
    let failure = state.failure();
    assert(failure == Some(kind));
}

fn completion_does_not_close_unaccepted() {
    let pending = RecordState::Pending.completed(u64::MAX);
    let external = RecordState::External.completed(u64::MAX);
    let synchronous = RecordState::Synchronous.completed(u64::MAX);
    let submitting = RecordState::Submitting(1).completed(u64::MAX);
    let uncertain = RecordState::Uncertain.completed(u64::MAX);
    assert(!pending && !external && !synchronous && !submitting && !uncertain);
}

fn completion_is_monotonic() {
    let mut timeline = Timeline::new();
    let first = timeline.accept(1);
    let second = timeline.accept(2);
    let completed = timeline.observe(2);
    let older = timeline.observe(1);
    let before_invalid = timeline.completed();
    let future = timeline.observe(3);
    let after_invalid = timeline.completed();
    assert(first && second && completed && older);
    assert(before_invalid == 2 && after_invalid == 2 && !future);
}

fn no_serial_wrap(mut timeline: Timeline)
    requires timeline.valid(), timeline.submitted_view() == u64::MAX,
{
    let next = timeline.next();
    let accepted = timeline.accept(0);
    let submitted = timeline.submitted();
    assert(next.is_none() && !accepted && submitted == u64::MAX);
}

fn acceptance_requires_matching_attempt(serial: u64, other: u64)
    requires serial > 0, other != serial,
{
    let attempt = RecordState::Pending.begin(serial);
    assert(attempt == Some(RecordState::Submitting(serial)));
    let Some(attempt) = attempt else { return; };
    let mismatched = attempt.accept(other);
    let accepted = attempt.accept(serial);
    let unaccepted_done = attempt.completed(serial);
    assert(mismatched.is_none() && !unaccepted_done);
    assert(accepted == Some(RecordState::Submitted(serial)));
    let Some(accepted) = accepted else { return; };
    let before = accepted.completed(serial - 1);
    let after = accepted.completed(serial);
    assert(!before && after);
}

fn quarantine_cannot_complete(state: RecordState, serial: u64) {
    let quarantined = state.quarantine();
    let done = quarantined.completed(serial);
    let begin = quarantined.begin(serial);
    let accept = quarantined.accept(serial);
    assert(!done && begin.is_none() && accept.is_none());
}

fn only_pending_can_begin(state: RecordState, serial: u64)
    requires !(state is Pending),
{
    let next = state.begin(serial);
    assert(next.is_none());
}

fn synchronous_requires_pending(state: RecordState) {
    let next = state.begin_sync();
    assert(next.is_some() == (state is Pending));
    if let Some(next) = next {
        let completed = next.completed(u64::MAX);
        let accepted = next.accept(1);
        let restarted = next.begin_sync();
        assert(next is Synchronous);
        assert(!completed && accepted.is_none() && restarted.is_none());
    }
}

fn duplicate_serial_is_rejected() {
    let mut timeline = Timeline::new();
    let first = timeline.accept(1);
    let duplicate = timeline.accept(1);
    let skipped = timeline.accept(3);
    let submitted = timeline.submitted();
    assert(first && !duplicate && !skipped && submitted == 1);
}

}
