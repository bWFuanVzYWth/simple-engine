//! 验证生产提交状态核，不导入 Vulkan FFI。

#![allow(dead_code)]

use vstd::prelude::*;

#[path = "../src/submission.rs"]
mod submission;

verus! {

#[cfg(reject_unaccepted_completion)]
fn reject_unaccepted_completion(state: submission::RecordState)
    requires !(state is Submitted),
{
    let completed = state.completed(u64::MAX);
    assert(completed);
}

#[cfg(reject_failed_device_reuse)]
fn reject_failed_device_reuse() {
    let mut state = submission::DeviceState::new();
    state.fail(submission::FailureKind::Validation);
    let failure = state.failure();
    assert(failure.is_none());
}

#[cfg(reject_submission_serial_wrap)]
fn reject_submission_serial_wrap(timeline: submission::Timeline)
    requires timeline.valid(), timeline.submitted_view() == u64::MAX,
{
    let next = timeline.next();
    assert(next.is_some());
}

}

fn main() {}
