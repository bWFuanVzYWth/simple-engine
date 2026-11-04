//! SDK 恢复核的转换性质；实际 configure/evaluate 仍由适配层负责。

#[cfg(verus_only)]
use super::{Recovery, RecoveryCause};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn reconfigure_reason(state: Recovery) -> Option<RecoveryCause> {
    match state {
        Recovery::NeedsReconfigure(cause) => Some(cause),
        _ => None,
    }
}

#[cfg(verus_only)]
fn reconfiguration_preserves_reset_request(cause: RecoveryCause) {
    let mut state = Recovery::new();
    state.invalidate(cause);
    assert(state.reconfigure_reason() == Some(cause));
    state.reconfigured();
    assert(state.reset_required());
    assert(state.reconfigure_reason() is None);
    state.recorded();
    assert(!state.reset_required());
}

#[cfg(verus_only)]
fn cancelled_record_requires_reconfiguration() {
    let mut state = Recovery::new();
    state.invalidate(RecoveryCause::RecordingFailed);
    state.reconfigured();
    state.recorded();
    state.invalidate(RecoveryCause::RecordingCancelled);
    assert(state.reconfigure_reason() == Some(RecoveryCause::RecordingCancelled));
}

}
