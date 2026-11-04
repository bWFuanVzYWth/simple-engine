//! 装配生产 SDK 恢复核及其证明；不加载 SDK 或运行 GPU。

#![allow(dead_code)]

#[path = "../src/recovery.rs"]
mod recovery;

use recovery::{Recovery, RecoveryCause};
use vstd::prelude::*;

verus! {

#[cfg(reject_reset_loss)]
fn reject_reset_loss() {
    let mut state = Recovery::new();
    state.invalidate(RecoveryCause::RecordingCancelled);
    state.reconfigured();
    assert(!state.reset_required());
}

#[cfg(reject_unconfigured_recording)]
fn reject_unconfigured_recording() {
    let mut state = Recovery::new();
    state.invalidate(RecoveryCause::EvaluationFailed);
    state.recorded();
}

}

fn main() {}
