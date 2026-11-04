//! 替换提交的独占资源转移及旧消费者完成边界。
use super::*;

fn replacing() -> (State<Vec<u32>>, Handle, Patch) {
    let mut state = State::new(crate::Config::new(1, 4, 8).unwrap());
    let handle = state.reserve_root(Chunk::new([0; 3], 0)).unwrap();
    let build = state.request(handle).unwrap();
    state.complete(build.ticket, vec![7, 8]).unwrap();
    let patch = state.prepare_activate(handle).unwrap();
    state.publish(patch.token, 0).unwrap();
    let build = state.request_refresh(handle).unwrap();
    state.complete(build.ticket, Vec::new()).unwrap();
    let patch = state.prepare_replace(handle).unwrap();
    (state, handle, patch)
}

#[test]
fn transfer_moves_allocation_and_retires_remainder_at_last_use() {
    let (mut state, handle, patch) = replacing();
    let pointer = state.resource(handle).unwrap().as_ptr();
    state
        .publish_with(patch.token, 9, |old, next| {
            std::mem::swap(old, next);
        })
        .unwrap();
    assert_eq!(state.resource(handle).unwrap().as_ptr(), pointer);
    assert_eq!(state.resource(handle).unwrap(), &[7, 8]);
    assert!(state.take_retired(8).is_none());
    assert_eq!(state.take_retired(9), Some(Vec::new()));
}

#[test]
fn invalidated_candidate_does_not_call_transfer_or_change_old_resource() {
    let (mut state, handle, patch) = replacing();
    state.invalidate(handle).unwrap();
    let mut called = false;
    assert_eq!(
        state.publish_with(patch.token, 0, |_, _| called = true),
        Err(StateError::Transition)
    );
    assert!(!called);
    assert_eq!(state.resource(handle).unwrap(), &[7, 8]);
    assert!(state.replacement(handle).unwrap().is_empty());
    state.reject_patch(patch.token).unwrap();
    assert!(state.is_visible(handle));
}
