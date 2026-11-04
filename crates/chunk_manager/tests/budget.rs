//! 工作额度、输出背压与异步 ticket 的公开契约。

use chunk_manager::{Budget, Config, Plan, PlanError, State, Target, plan};

fn budget(visits: usize) -> Budget {
    Budget {
        visits,
        requests: 8,
        switches: 4,
    }
}

#[test]
fn zero_work_budget_only_changes_the_target() {
    let mut state = State::<()>::new(Config::new(3, 4, 32).unwrap());
    let mut output = Plan::with_capacity(8, 4);
    let stats = plan(
        &mut state,
        Target::new([0; 3]).unwrap(),
        budget(0),
        &mut output,
    )
    .unwrap();
    assert_eq!(stats.visits, 0);
    assert!(state.is_empty());
    assert!(output.is_empty());
    assert!(stats.pending);
}

#[test]
fn visit_budget_is_independent_of_teleport_distance_and_radius() {
    for radius in [4, 1_000_000] {
        let mut state = State::<()>::new(Config::new(20, radius, 128).unwrap());
        let mut output = Plan::with_capacity(8, 4);
        for position in [[0; 3], [Target::LIMIT; 3], [-Target::LIMIT; 3]] {
            let before = state.len();
            let stats = plan(
                &mut state,
                Target::new(position).unwrap(),
                budget(1),
                &mut output,
            )
            .unwrap();
            assert!(stats.visits <= 1);
            assert!(state.len() <= before + 1);
            assert!(output.requests().len() <= 1);
            for build in output.requests() {
                state.complete(build.ticket, ()).unwrap();
            }
            for patch in output.switches() {
                state.reject_patch(patch.token).unwrap();
            }
            output.clear();
        }
    }
}

#[test]
fn output_capacity_applies_backpressure_without_issuing_hidden_work() {
    let mut state = State::<()>::new(Config::new(1, 4, 128).unwrap());
    let target = Target::new([0; 3]).unwrap();
    let mut output = Plan::with_capacity(0, 0);
    let stats = plan(&mut state, target, budget(256), &mut output).unwrap();
    assert!(stats.visits <= 256);
    assert!(stats.blocked > 0);
    assert_eq!(state.pending_requests(), 0);
    assert_eq!(state.pending_patches(), 0);
    let mut output = Plan::with_capacity(8, 4);
    let stats = plan(&mut state, target, budget(256), &mut output).unwrap();
    assert!(stats.requests > 0);
}

#[test]
fn unconsumed_output_rejects_the_next_plan_without_state_changes() {
    let mut state = State::<()>::new(Config::new(1, 4, 128).unwrap());
    let mut output = Plan::with_capacity(8, 4);
    plan(
        &mut state,
        Target::new([0; 3]).unwrap(),
        budget(64),
        &mut output,
    )
    .unwrap();
    assert!(!output.is_empty());
    let before = (
        state.len(),
        state.pending_requests(),
        state.pending_patches(),
    );
    let error = plan(
        &mut state,
        Target::new([100; 3]).unwrap(),
        budget(64),
        &mut output,
    )
    .unwrap_err();
    assert!(matches!(error, PlanError::NotEmpty));
    assert_eq!(
        before,
        (
            state.len(),
            state.pending_requests(),
            state.pending_patches()
        )
    );
}

#[test]
fn nearby_movement_preserves_an_inflight_ticket() {
    let mut state = State::<()>::new(Config::new(3, 4, 128).unwrap());
    let mut output = Plan::with_capacity(8, 4);
    plan(
        &mut state,
        Target::new([0; 3]).unwrap(),
        budget(32),
        &mut output,
    )
    .unwrap();
    let build = output.requests()[0];
    output.clear();
    plan(
        &mut state,
        Target::new([1, 0, 0]).unwrap(),
        budget(64),
        &mut output,
    )
    .unwrap();
    state.complete(build.ticket, ()).unwrap();
}

#[test]
fn obsolete_requests_emit_cancellation_and_return_late_resources() {
    let mut state = State::<u32>::new(Config::new(1, 4, 128).unwrap());
    let mut output = Plan::with_capacity(8, 4);
    plan(
        &mut state,
        Target::new([0; 3]).unwrap(),
        budget(64),
        &mut output,
    )
    .unwrap();
    let old = output.requests()[0].ticket;
    output.clear();
    let target = Target::new([1_000; 3]).unwrap();
    let mut cancelled = false;
    for _ in 0..1_000 {
        let stats = plan(
            &mut state,
            target,
            Budget {
                visits: 128,
                requests: 0,
                switches: 0,
            },
            &mut output,
        )
        .unwrap();
        assert!(stats.visits <= 128);
        assert!(stats.cancellations <= 8);
        cancelled |= output.cancellations().contains(&old);
        output.clear();
        if cancelled {
            break;
        }
    }
    assert!(cancelled);
    let rejected = state.complete(old, 77).unwrap_err();
    assert_eq!(rejected.resource, 77);
    assert_eq!(rejected.error, chunk_manager::StateError::StaleTicket);
}
