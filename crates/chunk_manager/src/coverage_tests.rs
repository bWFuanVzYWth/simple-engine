//! 真实State的覆盖交换与完成值退役；不复制其转换算法。
use super::*;
use crate::{Bounds, Budget, Plan, plan};

fn ready(state: &mut State<u32>, handle: Handle, id: u32) {
    let build = state.request(handle).unwrap();
    state.complete(build.ticket, id).unwrap();
}
fn family(capacity: usize) -> (State<u32>, Handle, [Handle; 8]) {
    let config = Config::new(2, 4, capacity)
        .unwrap()
        .with_bounds(Some(Bounds::new([0; 3], [2; 3]).unwrap()));
    let mut state = State::new(config);
    let parent = state.reserve_root(Chunk::new([0; 3], 1)).unwrap();
    ready(&mut state, parent, 100);
    let patch = state.prepare_activate(parent).unwrap();
    state.publish(patch.token, 0).unwrap();
    let children = std::array::from_fn(|octant| state.reserve_child(parent, octant).unwrap());
    (state, parent, children)
}
fn finish_children(state: &mut State<u32>, children: [Handle; 8]) {
    for (id, child) in children.into_iter().enumerate() {
        ready(state, child, id as u32);
    }
}
fn split(state: &mut State<u32>, parent: Handle) {
    let patch = state.prepare_split(parent).unwrap();
    state.publish(patch.token, 7).unwrap();
}
fn ids(state: &State<u32>, prepared: bool) -> Vec<u32> {
    let mut result = if prepared {
        state
            .prepared_visible()
            .map(|(_, _, &id)| id)
            .collect::<Vec<_>>()
    } else {
        state.visible().map(|(_, _, &id)| id).collect::<Vec<_>>()
    };
    result.sort_unstable();
    result
}
fn complete_cover(state: &State<u32>) {
    // 每个LOD0单元应恰有一块可见资源覆盖；独立检查空间结果而非phase字段。
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                let point = [x, y, z];
                let count = state
                    .visible()
                    .filter(|(_, chunk, _)| {
                        let edge = 1i64 << chunk.lod;
                        (0..3).all(|a| {
                            chunk.coord[a] * edge <= point[a]
                                && point[a] < (chunk.coord[a] + 1) * edge
                        })
                    })
                    .count();
                assert_eq!(count, 1, "单元{point:?}覆盖必须完整且不重叠");
            }
        }
    }
}

#[test]
fn growth_keeps_a_locked_family_and_wrapped_retirement_order() {
    let (mut state, parent, children) = family(12);
    finish_children(&mut state, children);
    split(&mut state, parent);
    assert_eq!(state.take_retired(7), Some(100));
    // 移动队首后再填充队列，确保扩容必须保留环形顺序和完成值。
    for (index, child) in children.into_iter().enumerate() {
        let ticket = state.request_refresh(child).unwrap().ticket;
        state.complete(ticket, 200 + index as u32).unwrap();
        let patch = state.prepare_replace(child).unwrap();
        state.publish(patch.token, 20 + index as u64).unwrap();
    }
    for value in 400..404 {
        let ticket = state.request_refresh(children[0]).unwrap().ticket;
        state.complete(ticket, value).unwrap();
        let patch = state.prepare_replace(children[0]).unwrap();
        state.publish(patch.token, 40 + value as u64 - 400).unwrap();
    }
    ready(&mut state, parent, 300);
    let patch = state.prepare_merge(parent).unwrap_err();
    assert_eq!(patch, StateError::RetirementFull);
    for value in 0..8 {
        assert_eq!(state.take_retired(27), Some(value));
    }
    let pending = state.prepare_merge(parent).unwrap();
    state.grow_capacity(32).unwrap();
    assert_eq!(state.patch_resources(pending.token).unwrap().len(), 9);
    let mut expected = (201..208).collect::<Vec<_>>();
    expected.push(403);
    assert_eq!(ids(&state, false), expected);
    state.publish(pending.token, 100).unwrap();
    for value in [200, 400, 401, 402] {
        assert_eq!(state.take_retired(43), Some(value));
    }
    assert_eq!(state.take_retired(99), None);
    for value in [403, 201, 202, 203, 204, 205, 206, 207] {
        assert_eq!(state.take_retired(100), Some(value));
    }
    assert_eq!(state.take_retired(100), None);
    assert_eq!(ids(&state, false), [300]);
}

#[test]
fn taking_an_unpublished_edit_keeps_the_old_cover_and_returns_candidate_ownership() {
    let (mut state, parent, _) = family(16);
    let ticket = state.request_refresh(parent).unwrap().ticket;
    state.complete(ticket, 900).unwrap();
    assert_eq!(state.take_unpublished(parent).unwrap(), Some(900));
    assert_eq!(state.resource(parent), Some(&100));
    assert_eq!(state.replacement(parent), None);
    assert_eq!(state.retired_len(), 0);
    complete_cover(&state);
    let next = state.request_refresh(parent).unwrap().ticket;
    assert_ne!(next, ticket);
    assert_eq!(state.take_unpublished(parent).unwrap(), None);
    state.complete(next, 901).unwrap();
    let patch = state.prepare_replace(parent).unwrap();
    assert_eq!(
        state.take_unpublished(parent),
        Err(StateError::PendingPatch)
    );
    assert_eq!(state.replacement(parent), Some(&901));
    state.publish(patch.token, 15).unwrap();
    assert_eq!(state.resource(parent), Some(&901));
    assert_eq!(state.take_retired(14), None);
    assert_eq!(state.take_retired(15), Some(100));
}

#[test]
fn taking_a_ready_resource_keeps_its_slot_for_a_new_request() {
    let mut state = State::new(Config::new(1, 4, 4).unwrap());
    let handle = state.reserve_root(Chunk::new([0; 3], 0)).unwrap();
    let resource = vec![1, 2, 3];
    let allocation = resource.as_ptr();
    let old = state.request(handle).unwrap().ticket;
    state.complete(old, resource).unwrap();
    let returned = state.take_unpublished(handle).unwrap().unwrap();
    assert_eq!(returned.as_ptr(), allocation);
    assert_eq!(state.resource(handle), None);
    assert_eq!(state.find(Chunk::new([0; 3], 0)), Some(handle));
    assert_eq!(state.retired_len(), 0);
    let current = state.request(handle).unwrap().ticket;
    state.complete(current, returned).unwrap();
    assert_eq!(state.resource(handle).unwrap().as_ptr(), allocation);
}

#[test]
fn prepared_split_rejects_an_edit_of_the_outgoing_parent() {
    let (mut state, parent, children) = family(16);
    finish_children(&mut state, children);
    let patch = state.prepare_split(parent).unwrap();
    state.invalidate(parent).unwrap();
    assert_eq!(
        state.validate_patch(patch.token),
        Err(StateError::Transition)
    );
    assert_eq!(state.publish(patch.token, 20), Err(StateError::Transition));
    assert_eq!(ids(&state, false), [100]);
    complete_cover(&state);
    assert_eq!(state.retired_len(), 0);
    state.reject_patch(patch.token).unwrap();
    assert_eq!(ids(&state, true), [100]);
}

#[test]
fn prepared_merge_rejects_an_edit_of_an_outgoing_child() {
    let (mut state, parent, children) = family(16);
    finish_children(&mut state, children);
    split(&mut state, parent);
    assert_eq!(state.take_retired(7), Some(100));
    ready(&mut state, parent, 200);
    let patch = state.prepare_merge(parent).unwrap();
    state.invalidate(children[3]).unwrap();
    assert_eq!(
        state.validate_patch(patch.token),
        Err(StateError::Transition)
    );
    assert_eq!(state.publish(patch.token, 20), Err(StateError::Transition));
    assert_eq!(ids(&state, false), (0..8).collect::<Vec<_>>());
    complete_cover(&state);
    assert_eq!(state.retired_len(), 0);
    state.reject_patch(patch.token).unwrap();
    assert_eq!(ids(&state, true), (0..8).collect::<Vec<_>>());
}

#[test]
fn planner_waits_for_a_refreshing_child_before_merging() {
    let (mut state, parent, children) = family(16);
    finish_children(&mut state, children);
    split(&mut state, parent);
    assert_eq!(state.take_retired(7), Some(100));
    ready(&mut state, parent, 200);
    let refresh = state.request_refresh(children[0]).unwrap();
    let target = Target::new([12, 0, 0]).unwrap();
    assert!(
        state
            .config()
            .contains_root(state.chunk(parent).unwrap(), target)
    );
    assert!(
        !state
            .config()
            .should_split(state.chunk(parent).unwrap(), target)
    );
    let budget = Budget {
        visits: 512,
        requests: 0,
        switches: 8,
    };
    let mut output = Plan::with_capacity(0, 8);
    plan(&mut state, target, budget, &mut output)
        .expect("子刷新在途应保旧覆盖等待，而不是报告非法合并");
    assert!(output.switches().is_empty());
    assert_eq!(ids(&state, false), (0..8).collect::<Vec<_>>());
    complete_cover(&state);
    state.complete(refresh.ticket, 300).unwrap();
    output.clear();
    plan(&mut state, target, budget, &mut output).unwrap();
    assert_eq!(output.switches().len(), 1);
    assert!(matches!(output.switches()[0].change, Change::Replace(_)));
    state.publish(output.switches()[0].token, 19).unwrap();
    output.clear();
    plan(&mut state, target, budget, &mut output).unwrap();
    assert_eq!(output.switches().len(), 1);
    assert!(matches!(output.switches()[0].change, Change::Merge { .. }));
    complete_cover(&state);
    state.publish(output.switches()[0].token, 30).unwrap();
    assert_eq!(ids(&state, false), [200]);
    complete_cover(&state);
}

#[test]
fn failed_or_cancelled_refresh_and_its_late_completion_preserve_the_old_cover() {
    for cancel in [false, true] {
        let (mut state, parent, _) = family(16);
        let refresh = state.request_refresh(parent).unwrap();
        if cancel {
            state.cancel(refresh.ticket).unwrap();
        } else {
            state.fail(refresh.ticket).unwrap();
        }
        let rejected = state.complete(refresh.ticket, 999).unwrap_err();
        assert_eq!(rejected.error, StateError::StaleTicket);
        assert_eq!(rejected.resource, 999);
        assert_eq!(ids(&state, false), [100]);
        assert!(state.take_retired(u64::MAX).is_none());
        complete_cover(&state);
        let next = state.request_refresh(parent).unwrap();
        assert_ne!(next.ticket, refresh.ticket);
        state.complete(next.ticket, 200).unwrap();
        let patch = state.prepare_replace(parent).unwrap();
        assert_eq!(ids(&state, false), [100]);
        assert_eq!(ids(&state, true), [200]);
        complete_cover(&state);
        state.reject_patch(patch.token).unwrap();
        assert_eq!(ids(&state, false), [100]);
        assert_eq!(ids(&state, true), [100]);
        let retry = state.prepare_replace(parent).unwrap();
        state.publish(retry.token, 41).unwrap();
        assert_eq!(ids(&state, false), [200]);
        assert_eq!(state.take_retired(40), None);
        assert_eq!(state.take_retired(41), Some(100));
    }
}

#[test]
fn split_waits_for_every_child_and_only_retires_the_parent_after_publication() {
    let (mut state, parent, children) = family(16);
    for (id, child) in children[..7].iter().copied().enumerate() {
        ready(&mut state, child, id as u32);
        assert_eq!(state.prepare_split(parent), Err(StateError::Transition));
        complete_cover(&state);
    }
    let last = state.request(children[7]).unwrap();
    state.fail(last.ticket).unwrap();
    assert_eq!(state.prepare_split(parent), Err(StateError::Transition));
    assert_eq!(ids(&state, false), [100]);
    let retry = state.request(children[7]).unwrap();
    assert_eq!(state.complete(last.ticket, 999).unwrap_err().resource, 999);
    state.complete(retry.ticket, 7).unwrap();
    let patch = state.prepare_split(parent).unwrap();
    assert_eq!(ids(&state, false), [100]);
    assert_eq!(ids(&state, true), (0..8).collect::<Vec<_>>());
    assert_eq!(state.retired_len(), 0);
    state.reject_patch(patch.token).unwrap();
    assert_eq!(ids(&state, false), [100]);
    complete_cover(&state);
    let patch = state.prepare_split(parent).unwrap();
    state.publish(patch.token, 51).unwrap();
    complete_cover(&state);
    assert_eq!(ids(&state, false), (0..8).collect::<Vec<_>>());
    assert_eq!(state.take_retired(50), None);
    assert_eq!(state.take_retired(51), Some(100));
}

#[test]
fn merge_waits_for_the_parent_and_defers_all_eight_children_until_last_use() {
    let (mut state, parent, children) = family(16);
    finish_children(&mut state, children);
    split(&mut state, parent);
    assert_eq!(state.take_retired(7), Some(100));
    let build = state.request(parent).unwrap();
    assert_eq!(state.prepare_merge(parent), Err(StateError::Transition));
    state.fail(build.ticket).unwrap();
    assert_eq!(state.prepare_merge(parent), Err(StateError::Transition));
    complete_cover(&state);
    let retry = state.request(parent).unwrap();
    assert_eq!(state.complete(build.ticket, 999).unwrap_err().resource, 999);
    state.complete(retry.ticket, 200).unwrap();
    let patch = state.prepare_merge(parent).unwrap();
    assert_eq!(ids(&state, false), (0..8).collect::<Vec<_>>());
    assert_eq!(ids(&state, true), [200]);
    assert_eq!(state.retired_len(), 0);
    state.reject_patch(patch.token).unwrap();
    complete_cover(&state);
    assert_eq!(ids(&state, true), (0..8).collect::<Vec<_>>());
    let patch = state.prepare_merge(parent).unwrap();
    state.publish(patch.token, 61).unwrap();
    assert_eq!(ids(&state, false), [200]);
    complete_cover(&state);
    assert_eq!(state.retired_len(), 8);
    assert_eq!(state.take_retired(60), None);
    let mut retired = (0..8)
        .map(|_| state.take_retired(61).unwrap())
        .collect::<Vec<_>>();
    retired.sort_unstable();
    assert_eq!(retired, (0..8).collect::<Vec<_>>());
    assert_eq!(state.take_retired(61), None);
}

#[test]
fn a_full_retirement_queue_blocks_merge_without_removing_old_children() {
    let (mut state, parent, children) = family(9);
    finish_children(&mut state, children);
    split(&mut state, parent);
    let refresh = state.request_refresh(children[0]).unwrap();
    state.complete(refresh.ticket, 300).unwrap();
    let patch = state.prepare_replace(children[0]).unwrap();
    state.publish(patch.token, 8).unwrap();
    ready(&mut state, parent, 200);
    assert_eq!(state.prepare_merge(parent), Err(StateError::RetirementFull));
    complete_cover(&state);
    assert_eq!(state.visible().count(), 8);
    assert_eq!(state.take_retired(6), None);
    assert_eq!(state.take_retired(7), Some(100));
    let patch = state.prepare_merge(parent).unwrap();
    complete_cover(&state);
    state.publish(patch.token, 9).unwrap();
    assert_eq!(ids(&state, false), [200]);
    assert_eq!(state.take_retired(7), None);
    assert_eq!(state.take_retired(8), Some(0));
    assert_eq!(state.take_retired(8), None);
    let mut retired = (0..8)
        .map(|_| state.take_retired(9).unwrap())
        .collect::<Vec<_>>();
    retired.sort_unstable();
    assert_eq!(retired, [1, 2, 3, 4, 5, 6, 7, 300]);
}

#[test]
fn late_refresh_completion_after_an_edit_cannot_publish_the_stale_candidate() {
    let (mut state, parent, _) = family(16);
    let old = state.request_refresh(parent).unwrap();
    state.invalidate(parent).unwrap();
    state.complete(old.ticket, 200).unwrap();
    assert_eq!(state.prepare_replace(parent), Err(StateError::Transition));
    assert_eq!(ids(&state, false), [100]);
    complete_cover(&state);
    state.discard_replacement(parent).unwrap();
    assert_eq!(state.take_retired(0), Some(200));
    assert_eq!(ids(&state, false), [100]);
    let current = state.request_refresh(parent).unwrap();
    assert_ne!(current.ticket, old.ticket);
    state.complete(current.ticket, 300).unwrap();
    let patch = state.prepare_replace(parent).unwrap();
    state.publish(patch.token, 67).unwrap();
    assert_eq!(ids(&state, false), [300]);
    assert_eq!(state.take_retired(66), None);
    assert_eq!(state.take_retired(67), Some(100));
}

#[test]
fn old_resource_destruction_requires_both_completion_and_explicit_owner_return() {
    use std::cell::Cell;
    #[derive(Debug)]
    struct Probe<'a> {
        id: usize,
        drops: &'a [Cell<u32>; 2],
    }
    impl Drop for Probe<'_> {
        fn drop(&mut self) {
            self.drops[self.id].set(self.drops[self.id].get() + 1);
        }
    }
    let drops = [Cell::new(0), Cell::new(0)];
    let mut state = State::new(Config::new(1, 4, 2).unwrap());
    let handle = state.reserve_root(Chunk::new([0; 3], 0)).unwrap();
    let build = state.request(handle).unwrap();
    state
        .complete(
            build.ticket,
            Probe {
                id: 0,
                drops: &drops,
            },
        )
        .unwrap();
    let patch = state.prepare_activate(handle).unwrap();
    state.publish(patch.token, 0).unwrap();
    let build = state.request_refresh(handle).unwrap();
    state
        .complete(
            build.ticket,
            Probe {
                id: 1,
                drops: &drops,
            },
        )
        .unwrap();
    let patch = state.prepare_replace(handle).unwrap();
    state.publish(patch.token, 73).unwrap();
    assert_eq!(drops[0].get(), 0);
    assert!(state.take_retired(72).is_none());
    assert_eq!(drops[0].get(), 0);
    let old = state.take_retired(73).unwrap();
    assert_eq!(old.id, 0);
    assert_eq!(drops[0].get(), 0);
    drop(old);
    assert_eq!(drops[0].get(), 1);
    assert_eq!(drops[1].get(), 0);
    drop(state);
    assert_eq!(drops[1].get(), 1);
}

#[test]
fn planning_capacity_pressure_cannot_remove_a_parent_needed_by_the_target() {
    let config = Config::new(2, 4, 8)
        .unwrap()
        .with_bounds(Some(Bounds::new([0; 3], [2; 3]).unwrap()));
    let mut state = State::new(config);
    let parent = state.reserve_root(Chunk::new([0; 3], 1)).unwrap();
    ready(&mut state, parent, 100);
    let patch = state.prepare_activate(parent).unwrap();
    state.publish(patch.token, 0).unwrap();
    let mut output = Plan::with_capacity(16, 16);
    let mut blocked = false;
    for _ in 0..16 {
        let stats = plan(
            &mut state,
            Target::new([0; 3]).unwrap(),
            Budget {
                visits: 128,
                requests: 16,
                switches: 16,
            },
            &mut output,
        )
        .unwrap();
        blocked |= stats.blocked != 0;
        for build in output.requests() {
            state.complete(build.ticket, 200).unwrap();
        }
        assert!(output.switches().is_empty());
        assert_eq!(ids(&state, false), [100]);
        complete_cover(&state);
        assert_eq!(state.retired_len(), 0);
        output.clear();
    }
    assert!(blocked);
    assert_eq!(state.len(), 8);
    assert_eq!(state.pending_requests(), 0);
}
