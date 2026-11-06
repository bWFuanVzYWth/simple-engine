//! 历史事务、票据身份和设置的行为边界。
use pt::history::{GlobalReset, Histories, History, Invalidation};
use restir_pt::{ColorOutput, Error, FrameParams, FrameState, Settings};

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
    let request = Invalidation::all(GlobalReset::ExplicitRequested);
    assert_eq!(state.invalidate_history(request), Err(Error::PendingFrame));
    state.reject(token).unwrap();
    state.invalidate_history(request).unwrap();
    assert!(!state.next_frame_params().unwrap().history_valid);
}

#[test]
fn path_invalidation_preserves_absolute_frame_identity() {
    let mut state = FrameState::new().unwrap();
    let (token, _) = state.begin().unwrap();
    state
        .commit_with_color(token, ColorOutput::Produced)
        .unwrap();
    state
        .invalidate_history(Invalidation::new(
            Histories::one(History::Paths),
            GlobalReset::ExplicitRequested,
        ))
        .unwrap();
    assert_eq!(
        state.next_frame_params().unwrap(),
        FrameParams {
            frame_index: 1,
            history_valid: false,
        }
    );
    assert!(state.color_history_valid());
}

#[test]
fn color_invalidation_preserves_path_history() {
    let mut state = FrameState::new().unwrap();
    let (token, _) = state.begin().unwrap();
    state
        .commit_with_color(token, ColorOutput::Produced)
        .unwrap();
    assert!(state.color_history_valid());
    let before = state.next_frame_params().unwrap();
    state
        .invalidate_history(Invalidation::new(
            Histories::one(History::Color),
            GlobalReset::ExplicitRequested,
        ))
        .unwrap();
    assert_eq!(state.next_frame_params().unwrap(), before);
    assert!(!state.color_history_valid());
}

#[test]
fn produced_color_becomes_readable_only_after_commit() {
    let mut state = FrameState::new().unwrap();
    let (token, _) = state.begin().unwrap();
    assert!(!state.color_history_valid());
    state
        .commit_with_color(token, ColorOutput::Produced)
        .unwrap();
    assert!(state.color_history_valid());
}

#[test]
fn omitted_color_breaks_only_color_continuity() {
    let mut state = FrameState::new().unwrap();
    let (first, _) = state.begin().unwrap();
    state
        .commit_with_color(first, ColorOutput::Produced)
        .unwrap();
    let (second, _) = state.begin().unwrap();
    state
        .commit_with_color(second, ColorOutput::Omitted)
        .unwrap();
    assert!(!state.color_history_valid());
    assert_eq!(
        state.next_frame_params().unwrap(),
        FrameParams {
            frame_index: 2,
            history_valid: true,
        }
    );
}

#[test]
fn rejected_frame_preserves_previously_produced_color() {
    let mut state = FrameState::new().unwrap();
    let (first, _) = state.begin().unwrap();
    state
        .commit_with_color(first, ColorOutput::Produced)
        .unwrap();
    let before = state.next_frame_params().unwrap();
    let (cancelled, _) = state.begin().unwrap();
    state.reject(cancelled).unwrap();
    assert!(state.color_history_valid());
    assert_eq!(state.next_frame_params().unwrap(), before);
}

#[test]
fn rejected_producer_cannot_publish_color_with_stale_token() {
    let mut state = FrameState::new().unwrap();
    let (cancelled, _) = state.begin().unwrap();
    state.reject(cancelled).unwrap();
    assert_eq!(
        state.commit_with_color(cancelled, ColorOutput::Produced),
        Err(Error::StaleFrame)
    );
    assert!(!state.color_history_valid());
    assert_eq!(state.next_frame_params().unwrap().frame_index, 0);
}

#[test]
fn legacy_commit_explicitly_omits_color() {
    let mut state = FrameState::new().unwrap();
    let (first, _) = state.begin().unwrap();
    state
        .commit_with_color(first, ColorOutput::Produced)
        .unwrap();
    let (second, _) = state.begin().unwrap();
    state.commit(second).unwrap();
    assert!(!state.color_history_valid());
    assert!(state.next_frame_params().unwrap().history_valid);
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
fn settings_default_keeps_twelve_surface_bounces() {
    let settings = Settings::default().validate().unwrap();
    assert_eq!(settings.max_bounces, 12);
    assert_eq!(settings.history_length, 20);
}
