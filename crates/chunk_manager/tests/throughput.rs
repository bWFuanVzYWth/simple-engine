//! 真实访问预算下的进展、背压、差分、扩容和增量消费者。

use std::collections::BTreeMap;

use chunk_manager::{
    Bounds, Budget, Chunk, Config, Handle, Plan, State, Target, VisibleChange, plan,
};

fn config(minimum: [i64; 3], maximum: [i64; 3], capacity: usize) -> Config {
    Config::new(1, 1_000, capacity)
        .unwrap()
        .with_bounds(Some(Bounds::new(minimum, maximum).unwrap()))
}

struct Host {
    state: State<u64>,
    output: Plan,
    view: BTreeMap<Chunk, (Handle, u64)>,
    sequence: u64,
    requests: usize,
    switches: usize,
    visits: usize,
}

impl Host {
    fn new(config: Config) -> Self {
        Self {
            state: State::new(config),
            output: Plan::with_capacity(32, 32),
            view: BTreeMap::new(),
            sequence: 0,
            requests: 0,
            switches: 0,
            visits: 0,
        }
    }

    fn step(&mut self, target: Target, visits: usize) -> bool {
        let stats = plan(
            &mut self.state,
            target,
            Budget {
                visits,
                requests: 32,
                switches: 32,
            },
            &mut self.output,
        )
        .unwrap();
        assert!(stats.visits <= visits);
        self.visits += stats.visits;
        for build in self.output.requests() {
            self.sequence += 1;
            self.requests += 1;
            self.state.complete(build.ticket, self.sequence).unwrap();
        }
        for patch in self.output.switches() {
            let changes = self.state.patch_resources(patch.token).unwrap();
            assert!(changes.len() <= 9);
            for change in changes {
                match change {
                    VisibleChange::Remove { chunk, .. } => {
                        self.view.remove(&chunk);
                    }
                    VisibleChange::Upsert {
                        handle,
                        chunk,
                        resource,
                    } => {
                        self.view.insert(chunk, (handle, *resource));
                    }
                }
            }
            self.state.publish(patch.token, self.sequence).unwrap();
            self.switches += 1;
        }
        self.output.clear();
        while self.state.take_retired(self.sequence).is_some() {}
        stats.pending
    }

    fn settle(&mut self, target: Target, visits: usize, frames: usize) -> usize {
        for frame in 1..=frames {
            if !self.step(target, visits) {
                let actual: BTreeMap<_, _> = self
                    .state
                    .visible()
                    .map(|(handle, chunk, resource)| (chunk, (handle, *resource)))
                    .collect();
                assert_eq!(self.view, actual);
                return frame;
            }
        }
        panic!(
            "预算内未取得进展：nodes={}, requests={}, patches={}",
            self.state.len(),
            self.state.pending_requests(),
            self.state.pending_patches()
        );
    }
}

#[test]
fn exhausted_output_quota_sleeps_until_a_later_call_can_use_it() {
    let mut state = State::new(config([0; 3], [1; 3], 4));
    let target = Target::new([0; 3]).unwrap();
    let mut output = Plan::with_capacity(1, 1);
    let paused = Budget {
        visits: 10_000,
        requests: 0,
        switches: 0,
    };
    let first = plan(&mut state, target, paused, &mut output).unwrap();
    assert!(first.pending);
    assert!(first.visits < 100, "一个配额阻塞节点不应消耗整批预算");
    assert_eq!(
        plan(&mut state, target, paused, &mut output)
            .unwrap()
            .visits,
        0
    );
    plan(
        &mut state,
        target,
        Budget {
            requests: 1,
            ..paused
        },
        &mut output,
    )
    .unwrap();
    state.complete(output.requests()[0].ticket, 7).unwrap();
    output.clear();
    let ready = plan(&mut state, target, paused, &mut output).unwrap();
    assert!(ready.visits <= 2);
    assert_eq!(
        plan(&mut state, target, paused, &mut output)
            .unwrap()
            .visits,
        0
    );
    plan(
        &mut state,
        target,
        Budget {
            switches: 1,
            ..paused
        },
        &mut output,
    )
    .unwrap();
    state.publish(output.switches()[0].token, 0).unwrap();
    assert_eq!(state.visible().count(), 1);
}

#[test]
fn a_single_visit_per_frame_eventually_builds_and_publishes_every_required_leaf() {
    let mut host = Host::new(config([0; 3], [8, 2, 2], 48));
    let frames = host.settle(Target::new([0; 3]).unwrap(), 1, 512);
    assert_eq!(host.view.len(), 32);
    assert_eq!(host.requests, 32);
    assert_eq!(host.switches, 32);
    assert!(frames <= 320, "正预算不能长期只搬队列而不发布：{frames}");
}

#[test]
fn sparse_edit_cost_does_not_depend_on_reserved_empty_slots() {
    let mut costs = Vec::new();
    for capacity in [32, 65_536] {
        let mut host = Host::new(config([0; 3], [2; 3], capacity));
        let target = Target::new([0; 3]).unwrap();
        host.settle(target, 128, 100);
        let handle = host.state.find(Chunk::new([0; 3], 0)).unwrap();
        host.state.invalidate(handle).unwrap();
        let before = (host.requests, host.switches, host.visits);
        host.settle(target, 16, 10);
        costs.push((
            host.requests - before.0,
            host.switches - before.1,
            host.visits - before.2,
        ));
    }
    assert_eq!(costs[0], costs[1]);
    assert_eq!(costs[0].0, 1);
    assert_eq!(costs[0].1, 1);
    assert!(costs[0].2 <= 4);
}

#[test]
fn unchanged_rectangular_demand_ignores_target_and_radius_changes() {
    let mut host = Host::new(config([-2; 3], [2; 3], 80));
    host.settle(Target::new([0; 3]).unwrap(), 256, 100);
    let version = host.state.coverage_version();
    for x in 1..=20 {
        host.state.set_radius(1_000 + x as u32).unwrap();
        let before = host.visits;
        assert!(!host.step(Target::new([x, -x, x]).unwrap(), 1));
        assert_eq!(host.visits, before);
    }
    assert_eq!(host.state.coverage_version(), version);
}

#[test]
fn rectangular_move_visits_only_the_changed_slabs() {
    let mut host = Host::new(config([0; 3], [16, 4, 4], 512));
    let target = Target::new([0; 3]).unwrap();
    host.settle(target, 256, 100);
    let keep: Vec<_> = host
        .view
        .iter()
        .filter(|(chunk, _)| chunk.coord[0] > 0)
        .map(|(chunk, value)| (*chunk, *value))
        .collect();
    let before = (host.requests, host.switches, host.visits);
    host.state
        .reconfigure(config([1, 0, 0], [17, 4, 4], 512))
        .unwrap();
    host.settle(Target::new([1, 0, 0]).unwrap(), 16, 30);
    assert_eq!(host.requests - before.0, 16);
    assert_eq!(host.switches - before.1, 32);
    assert!(
        host.visits - before.2 <= 144,
        "不应遍历256个重叠节点或512个槽"
    );
    for (chunk, value) in keep {
        assert_eq!(host.view.get(&chunk), Some(&value));
    }
}

#[test]
fn capacity_growth_preserves_inflight_identity_and_resumes_the_blocked_root() {
    let mut state = State::new(config([0; 3], [2; 3], 1));
    let mut output = Plan::with_capacity(8, 8);
    let target = Target::new([0; 3]).unwrap();
    let budget = Budget {
        visits: 256,
        requests: 8,
        switches: 8,
    };
    plan(&mut state, target, budget, &mut output).unwrap();
    let build = output.requests()[0];
    let handle = state.find(build.chunk).unwrap();
    output.clear();
    let blocked = plan(&mut state, target, budget, &mut output).unwrap();
    assert!(blocked.visits < budget.visits);
    assert!(blocked.pending);
    state.grow_capacity(16).unwrap();
    assert_eq!(state.find(build.chunk), Some(handle));
    state.complete(build.ticket, 99).unwrap();
    let mut host = Host {
        state,
        output,
        view: BTreeMap::new(),
        sequence: 100,
        requests: 1,
        switches: 0,
        visits: 0,
    };
    host.settle(target, 8, 100);
    assert_eq!(host.view.len(), 8);
    assert_eq!(host.view[&build.chunk], (handle, 99));
    assert_eq!(host.requests, 8);
}

#[test]
fn superseded_large_initial_cursor_does_not_delay_the_latest_target() {
    let mut host = Host::new(Config::new(1, 100_000, 16).unwrap());
    host.step(Target::new([0; 3]).unwrap(), 1);
    let new_target = Target::new([1_000_000; 3]).unwrap();
    let mut found = false;
    for _ in 0..32 {
        host.step(new_target, 1);
        found |= host
            .view
            .keys()
            .any(|chunk| chunk.coord.iter().all(|&value| value > 900_000));
    }
    assert!(found, "不能等待旧的巨大根球游标耗尽才开始新目标");
}

#[test]
fn sparse_fallback_cost_depends_on_live_nodes_instead_of_reserved_capacity() {
    let mut costs = Vec::new();
    for capacity in [32, 65_536] {
        let configuration = Config::new(2, 4, capacity)
            .unwrap()
            .with_bounds(Some(Bounds::new([0; 3], [2; 3]).unwrap()));
        let mut host = Host::new(configuration);
        host.settle(Target::new([0; 3]).unwrap(), 128, 100);
        let before = host.visits;
        host.settle(Target::new([10_000; 3]).unwrap(), 8, 100);
        assert!(host.view.is_empty());
        costs.push(host.visits - before);
    }
    assert_eq!(costs[0], costs[1]);
    assert!(costs[0] <= 80);
}

#[test]
fn tiny_budget_preserves_and_eventually_replaces_a_complete_lod_family() {
    let configuration = Config::new(2, 4, 32)
        .unwrap()
        .with_bounds(Some(Bounds::new([0; 3], [2; 3]).unwrap()));
    let mut host = Host::new(configuration);
    let origin = Target::new([0; 3]).unwrap();
    host.settle(origin, 1, 256);
    assert_eq!(host.view.len(), 8);
    let far = Target::new([12, 0, 0]).unwrap();
    for _ in 0..128 {
        let pending = host.step(far, 1);
        let mut cell_owners = [0_u8; 8];
        for chunk in host.view.keys() {
            if chunk.lod == 1 {
                for owner in &mut cell_owners {
                    *owner += 1;
                }
            } else {
                let index = (chunk.coord[0] + chunk.coord[1] * 2 + chunk.coord[2] * 4) as usize;
                cell_owners[index] += 1;
            }
        }
        assert_eq!(
            cell_owners, [1; 8],
            "等待父资源或发布期间必须保留完整旧覆盖"
        );
        if !pending {
            break;
        }
    }
    assert_eq!(host.view.len(), 1);
    assert_eq!(host.view.keys().next().unwrap().lod, 1);
    host.settle(origin, 1, 256);
    assert_eq!(host.view.len(), 8);
}

#[test]
fn allocation_failure_keeps_tickets_resources_and_capacity_unchanged() {
    let mut state = State::new(config([0; 3], [1; 3], 2));
    let mut output = Plan::with_capacity(1, 1);
    plan(
        &mut state,
        Target::new([0; 3]).unwrap(),
        Budget {
            visits: 64,
            requests: 1,
            switches: 1,
        },
        &mut output,
    )
    .unwrap();
    let build = output.requests()[0];
    let handle = state.find(build.chunk).unwrap();
    assert!(matches!(
        state.grow_capacity(usize::MAX / 2),
        Err(chunk_manager::GrowthError::Allocation(_))
    ));
    assert_eq!(state.config().capacity(), 2);
    assert_eq!(state.find(build.chunk), Some(handle));
    state.complete(build.ticket, 7).unwrap();
    assert_eq!(state.resource(handle), Some(&7));
}

#[test]
fn interrupted_window_differences_retain_reentry_events_without_a_volume_scan() {
    let mut host = Host::new(config([0; 3], [16, 4, 4], 8_192));
    let origin = Target::new([0; 3]).unwrap();
    host.settle(origin, 256, 100);
    let before = host.visits;
    for x in [1, 2, 3, 2, 1, 0] {
        host.state
            .reconfigure(config([x, 0, 0], [x + 16, 4, 4], 8_192))
            .unwrap();
        host.step(Target::new([x, 0, 0]).unwrap(), 1);
    }
    host.settle(origin, 16, 100);
    assert_eq!(host.view.len(), 256);
    assert!(
        host.view
            .keys()
            .all(|chunk| (0..16).contains(&chunk.coord[0]))
    );
    assert!(
        host.visits - before <= 768,
        "短促移动和回转不应重扫8192槽或完整重叠窗口"
    );
}

#[test]
fn window_history_overflow_reconciles_the_latest_demand_under_a_tiny_budget() {
    let mut host = Host::new(config([0; 3], [4, 2, 2], 128));
    host.settle(Target::new([0; 3]).unwrap(), 64, 100);
    for x in 1..=24 {
        host.state
            .reconfigure(config([x, 0, 0], [x + 4, 2, 2], 128))
            .unwrap();
        host.step(Target::new([x, 0, 0]).unwrap(), 1);
    }
    host.settle(Target::new([24, 0, 0]).unwrap(), 1, 512);
    assert_eq!(host.view.len(), 16);
    assert!(
        host.view
            .keys()
            .all(|chunk| (24..28).contains(&chunk.coord[0]))
    );
}
