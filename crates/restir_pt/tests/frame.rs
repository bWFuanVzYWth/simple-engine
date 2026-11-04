//! 历史事务、票据身份和设置的行为边界。
use restir_pt::{Error, FrameParams, FrameState, Settings};

#[test]
fn rejected_frame_preserves_history_and_sequence() {
    let mut state = FrameState::new().unwrap();
    let (first, _) = state.begin().unwrap();
    state.commit(first).unwrap();
    let before = state.next_frame_params().unwrap();
    let (cancelled, _) = state.begin().unwrap();
    state.reject(cancelled).unwrap();
    assert_eq!(state.next_frame_params().unwrap(), before);
}

#[test]
fn submitted_frame_alone_enables_temporal_history() {
    let mut state = FrameState::new().unwrap();
    assert_eq!(
        state.next_frame_params().unwrap(),
        FrameParams {
            frame_index: 0,
            history_valid: false
        }
    );
    let (token, _) = state.begin().unwrap();
    state.commit(token).unwrap();
    assert_eq!(
        state.next_frame_params().unwrap(),
        FrameParams {
            frame_index: 1,
            history_valid: true
        }
    );
}

#[test]
fn pending_frame_excludes_record_and_reset() {
    let mut state = FrameState::new().unwrap();
    let (token, _) = state.begin().unwrap();
    assert!(matches!(state.begin(), Err(Error::PendingFrame)));
    assert_eq!(state.reset_history(), Err(Error::PendingFrame));
    state.reject(token).unwrap();
    state.reset_history().unwrap();
    assert!(!state.next_frame_params().unwrap().history_valid);
}

#[test]
fn token_cannot_confirm_another_owner_or_another_transaction() {
    let mut a = FrameState::new().unwrap();
    let mut b = FrameState::new().unwrap();
    let (old, _) = a.begin().unwrap();
    let (other, _) = b.begin().unwrap();
    assert_eq!(a.commit(other), Err(Error::StaleFrame));
    a.reject(old).unwrap();
    let (current, _) = a.begin().unwrap();
    assert_eq!(a.commit(old), Err(Error::StaleFrame));
    a.commit(current).unwrap();
    b.reject(other).unwrap();
}

#[test]
fn settings_reject_non_finite_shift_inputs() {
    let invalid = Settings {
        near_field_distance_sigma: f32::NAN,
        ..Settings::default()
    };
    assert_eq!(invalid.validate(), Err(Error::ShiftThreshold));
}

#[test]
fn settings_default_matches_nv_point_profile() {
    let settings = Settings::default().validate().unwrap();
    assert_eq!(settings.max_vertices, 10);
    assert_eq!(settings.history_length, 20);
}
