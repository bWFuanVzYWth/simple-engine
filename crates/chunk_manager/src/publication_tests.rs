//! 整批覆盖发布的取消、输入失败和代号耗尽边界。
use super::*;
fn pending_roots() -> (State<u32>, [Patch; 2]) {
    let mut state = State::new(crate::Config::new(1, 4, 8).unwrap());
    let patches = [0, 1].map(|x| {
        let handle = state.reserve_root(Chunk::new([x, 0, 0], 0)).unwrap();
        let build = state.request(handle).unwrap();
        state.complete(build.ticket, x as u32).unwrap();
        state.prepare_activate(handle).unwrap()
    });
    (state, patches)
}
#[test]
fn canceled_publication_preserves_pending_cover_and_generation() {
    let (mut state, patches) = pending_roots();
    let generation = state.generation;
    {
        let _uncommitted = state.prepare_publication(&patches, 0).unwrap();
    }
    assert_eq!(state.generation, generation);
    assert_eq!(state.visible().count(), 0);
    state.prepare_publication(&patches, 0).unwrap().commit();
    assert_eq!(state.visible().count(), 2);
}
#[test]
fn one_bad_candidate_keeps_the_whole_cover_unpublished() {
    let (mut state, patches) = pending_roots();
    state.invalidate(patches[1].token.handle).unwrap();
    let generation = state.generation;
    assert!(matches!(
        state.prepare_publication(&patches, 0),
        Err(StateError::Transition)
    ));
    assert_eq!(state.generation, generation);
    assert_eq!(state.visible().count(), 0);
    assert!(state.pending(patches[0].token).is_ok());
}
#[test]
fn duplicate_tokens_are_rejected_before_publishing_anything() {
    let (mut state, patches) = pending_roots();
    let generation = state.generation;
    assert!(matches!(
        state.prepare_publication(&[patches[0], patches[0]], 0),
        Err(StateError::Transition)
    ));
    assert_eq!(state.generation, generation);
    assert_eq!(state.visible().count(), 0);
    state.prepare_publication(&patches, 0).unwrap().commit();
    assert_eq!(state.visible().count(), 2);
}
#[test]
fn generation_exhaustion_is_checked_for_the_entire_batch() {
    let (mut state, patches) = pending_roots();
    state.generation = u64::MAX - 1;
    assert!(matches!(
        state.prepare_publication(&patches, 0),
        Err(StateError::GenerationExhausted)
    ));
    assert_eq!(state.generation, u64::MAX - 1);
    assert_eq!(state.visible().count(), 0);
    assert!(state.pending(patches[0].token).is_ok());
}
#[test]
fn forged_child_token_cannot_publish_the_same_family_twice() {
    let mut state = State::new(crate::Config::new(2, 4, 16).unwrap());
    let parent = state.reserve_root(Chunk::new([0; 3], 1)).unwrap();
    let build = state.request(parent).unwrap();
    state.complete(build.ticket, 1).unwrap();
    let active = state.prepare_activate(parent).unwrap();
    state.publish(active.token, 0).unwrap();
    let children: [_; 8] = std::array::from_fn(|octant| {
        let child = state.reserve_child(parent, octant).unwrap();
        let build = state.request(child).unwrap();
        state.complete(build.ticket, 2).unwrap();
        child
    });
    let split = state.prepare_split(parent).unwrap();
    let forged = Patch {
        token: PatchToken {
            handle: children[0],
            generation: split.token.generation,
        },
        change: Change::Activate(Chunk::new([0; 3], 0)),
    };
    let generation = state.generation;
    assert!(state.prepare_publication(&[split, forged], 0).is_err());
    assert_eq!(state.generation, generation);
    assert_eq!(state.visible().count(), 1);
    state.prepare_publication(&[split], 0).unwrap().commit();
    assert_eq!(state.visible().count(), 8);
}
