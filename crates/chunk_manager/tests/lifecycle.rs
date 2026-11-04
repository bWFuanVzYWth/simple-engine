//! 通过公共接口验证预算规划、异步完成与覆盖发布。

use std::collections::{HashSet, VecDeque};

use chunk_manager::{
    Bounds, Budget, Build, Change, Chunk, Config, CursorStep, Plan, RootCursor, State, StateError,
    Stats, Target, plan,
};

#[derive(Debug)]
struct Resource(Chunk);

struct Host {
    state: State<Resource>,
    output: Plan,
    builds: VecDeque<Build>,
    retired: Vec<Chunk>,
    budget: Budget,
    completed_per_frame: usize,
    frame: u64,
}

impl Host {
    fn new(config: Config) -> Self {
        Self {
            state: State::new(config),
            output: Plan::with_capacity(8, 4),
            builds: VecDeque::new(),
            retired: Vec::new(),
            budget: Budget {
                visits: 192,
                requests: 8,
                switches: 4,
            },
            completed_per_frame: 8,
            frame: 0,
        }
    }

    fn tick(&mut self, target: Target) -> Stats {
        self.frame += 1;
        let stats = plan(&mut self.state, target, self.budget, &mut self.output).unwrap();
        assert!(stats.visits <= self.budget.visits);
        assert!(stats.requests <= self.budget.requests);
        assert!(stats.switches <= self.budget.switches);
        self.builds.extend(self.output.requests().iter().copied());
        for _ in 0..self.completed_per_frame {
            let Some(build) = self.builds.pop_front() else {
                break;
            };
            if let Err(rejected) = self.state.complete(build.ticket, Resource(build.chunk)) {
                assert_eq!(rejected.error, StateError::StaleTicket);
            }
        }
        for patch in self.output.switches() {
            self.state.publish(patch.token, self.frame).unwrap();
        }
        self.output.clear();
        for _ in 0..16 {
            let Some(resource) = self.state.take_retired(self.frame) else {
                break;
            };
            self.retired.push(resource.0);
        }
        stats
    }

    fn settle(&mut self, target: Target) {
        for _ in 0..30_000 {
            let stats = self.tick(target);
            if self.frame.is_multiple_of(64) {
                assert_no_overlap(&self.state);
            }
            if !stats.pending && self.builds.is_empty() {
                assert_eq!(visible(&self.state), desired(*self.state.config(), target));
                assert_no_overlap(&self.state);
                return;
            }
        }
        panic!(
            "目标未收敛: slots={}, requests={}, patches={}",
            self.state.len(),
            self.state.pending_requests(),
            self.state.pending_patches()
        );
    }
}

fn visible(state: &State<Resource>) -> HashSet<Chunk> {
    state
        .visible()
        .map(|(_, chunk, resource)| {
            assert_eq!(chunk, resource.0);
            chunk
        })
        .collect()
}

fn assert_no_overlap(state: &State<Resource>) {
    let chunks = visible(state);
    for chunk in &chunks {
        let mut parent = *chunk;
        while parent.lod < state.config().max_lod() {
            parent.lod += 1;
            parent.coord = parent.coord.map(|value| value.div_euclid(2));
            assert!(
                !chunks.contains(&parent),
                "父子覆盖重叠: {parent:?} / {chunk:?}"
            );
        }
    }
}

fn desired(config: Config, target: Target) -> HashSet<Chunk> {
    let mut roots = RootCursor::new(config, target);
    let mut stack = Vec::new();
    loop {
        match roots.advance() {
            CursorStep::Item(chunk) => stack.push(chunk),
            CursorStep::Progress => {}
            CursorStep::Done => break,
        }
    }
    let mut leaves = HashSet::new();
    while let Some(chunk) = stack.pop() {
        if config.should_split(chunk, target) {
            for octant in 0..8 {
                stack.push(Chunk::new(
                    std::array::from_fn(|axis| {
                        chunk.coord[axis] * 2 + ((octant >> axis) & 1) as i64
                    }),
                    chunk.lod - 1,
                ));
            }
        } else {
            leaves.insert(chunk);
        }
    }
    leaves
}

#[test]
fn fixed_budget_reaches_a_complete_nonoverlapping_cut() {
    let mut host = Host::new(Config::new(2, 4, 12_000).unwrap());
    host.completed_per_frame = 3;
    host.settle(Target::new([-3, 2, 5]).unwrap());
}

#[test]
fn interrupted_movement_eventually_matches_the_latest_target() {
    let mut host = Host::new(Config::new(3, 4, 20_000).unwrap());
    host.completed_per_frame = 2;
    for step in 0..120_i64 {
        host.tick(Target::new([step, step / 3, -step / 5]).unwrap());
    }
    host.completed_per_frame = 8;
    host.settle(Target::new([120, 40, -24]).unwrap());
}

#[test]
fn teleport_reclaims_old_visible_resources_without_rebuilding_them() {
    let mut host = Host::new(Config::new(2, 4, 16_000).unwrap());
    host.settle(Target::new([0; 3]).unwrap());
    let old = visible(&host.state);
    host.retired.clear();
    host.settle(Target::new([512, -512, 256]).unwrap());
    assert!(old.iter().all(|chunk| host.state.find(*chunk).is_none()));
    let retired: HashSet<_> = host.retired.into_iter().collect();
    assert!(old.is_subset(&retired));
}

#[test]
fn rejecting_a_patch_keeps_its_resources_for_retry() {
    let mut host = Host::new(Config::new(1, 4, 4_000).unwrap());
    host.budget.switches = 1;
    let target = Target::new([0; 3]).unwrap();
    let mut rejected = false;
    for _ in 0..1_000 {
        plan(&mut host.state, target, host.budget, &mut host.output).unwrap();
        for build in host.output.requests() {
            host.state
                .complete(build.ticket, Resource(build.chunk))
                .unwrap();
        }
        if let Some(patch) = host.output.switches().first() {
            let Change::Activate(chunk) = patch.change else {
                panic!("初始化应先启用根")
            };
            let before = visible(&host.state);
            host.state.reject_patch(patch.token).unwrap();
            assert_eq!(visible(&host.state), before);
            assert!(
                host.state
                    .resource(host.state.find(chunk).unwrap())
                    .is_some()
            );
            rejected = true;
        }
        host.output.clear();
        if rejected {
            break;
        }
    }
    assert!(rejected);
    host.settle(target);
}

#[test]
fn retirement_capacity_blocks_removal_until_the_last_use_finishes() {
    let mut state = State::new(Config::new(1, 4, 1).unwrap());
    let mut output = Plan::with_capacity(1, 1);
    let budget = Budget {
        visits: 64,
        requests: 1,
        switches: 1,
    };
    let mut target = Target::new([0; 3]).unwrap();
    let mut activations = 0;
    for _ in 0..30_000 {
        plan(&mut state, target, budget, &mut output).unwrap();
        for build in output.requests() {
            state.complete(build.ticket, Resource(build.chunk)).unwrap();
        }
        for patch in output.switches() {
            state.publish(patch.token, 100).unwrap();
            if matches!(patch.change, Change::Activate(_)) {
                activations += 1;
            }
        }
        output.clear();
        if activations == 1 {
            target = Target::new([1_000; 3]).unwrap();
        }
        if activations == 2 {
            break;
        }
    }
    assert_eq!(activations, 2);
    plan(
        &mut state,
        Target::new([2_000; 3]).unwrap(),
        Budget::default(),
        &mut output,
    )
    .unwrap();
    let (handle, _, _) = state.visible().next().unwrap();
    assert_eq!(
        state.prepare_remove(handle),
        Err(StateError::RetirementFull)
    );
    assert!(state.is_visible(handle));
    assert!(state.take_retired(99).is_none());
    assert!(state.take_retired(100).is_some());
    let patch = state.prepare_remove(handle).unwrap();
    state.publish(patch.token, 100).unwrap();
    assert!(!state.is_visible(handle));
}

#[test]
fn an_old_removal_cannot_punch_a_hole_after_the_target_returns() {
    let mut host = Host::new(Config::new(1, 4, 4_000).unwrap());
    let original = Target::new([0; 3]).unwrap();
    host.settle(original);
    let previous = visible(&host.state);
    host.budget.switches = 1;
    let mut held = None;
    for _ in 0..2_000 {
        plan(
            &mut host.state,
            Target::new([1_000; 3]).unwrap(),
            host.budget,
            &mut host.output,
        )
        .unwrap();
        for build in host.output.requests() {
            host.state
                .complete(build.ticket, Resource(build.chunk))
                .unwrap();
        }
        for patch in host.output.switches() {
            if matches!(patch.change, Change::Remove(_)) {
                held = Some(*patch);
            } else {
                host.state.publish(patch.token, host.frame).unwrap();
            }
        }
        host.output.clear();
        if held.is_some() {
            break;
        }
    }
    let patch = held.expect("应发现旧根移除变化");
    plan(
        &mut host.state,
        original,
        Budget::default(),
        &mut host.output,
    )
    .unwrap();
    assert_eq!(
        host.state.publish(patch.token, host.frame),
        Err(StateError::Transition)
    );
    assert!(previous.is_subset(&visible(&host.state)));
    assert_eq!(host.state.pending_patches(), 1);
    host.state.reject_patch(patch.token).unwrap();
    host.settle(original);
}

#[test]
fn radius_and_bounds_reconfigure_without_replacing_the_state() {
    let config = Config::new(2, 4, 30_000).unwrap();
    let mut host = Host::new(config);
    let target = Target::new([0; 3]).unwrap();
    host.settle(target);
    let old = visible(&host.state);
    let bytes = host.state.reserved_bytes();
    host.state.set_radius(5).unwrap();
    host.settle(target);
    assert_ne!(visible(&host.state), old);
    let config = host
        .state
        .config()
        .with_bounds(Some(Bounds::new([0; 3], [2; 3]).unwrap()));
    host.state.reconfigure(config).unwrap();
    host.settle(target);
    assert_eq!(host.state.reserved_bytes(), bytes);
}

#[test]
fn adding_finite_bounds_replaces_an_unfinished_unbounded_scan() {
    let config = Config::new(1, u32::MAX, 8).unwrap();
    let mut host = Host::new(config);
    let target = Target::new([0; 3]).unwrap();
    host.budget.visits = 1;
    assert!(host.tick(target).pending);
    host.state
        .reconfigure(config.with_bounds(Some(Bounds::new([-1; 3], [0; 3]).unwrap())))
        .unwrap();
    host.budget.visits = 64;
    let mut converged = false;
    for _ in 0..10 {
        if !host.tick(target).pending {
            converged = true;
            break;
        }
    }
    assert!(converged, "有限边界不应等待旧的巨大球扫描结束");
    assert_eq!(
        visible(&host.state),
        HashSet::from([Chunk::new([-1; 3], 0)])
    );
}

#[test]
fn repeated_edit_cancels_the_old_refresh_within_the_output_budget() {
    let config = Config::new(1, 4, 8)
        .unwrap()
        .with_bounds(Some(Bounds::new([0; 3], [1; 3]).unwrap()));
    let mut state = State::new(config);
    let mut output = Plan::with_capacity(1, 1);
    let target = Target::new([0; 3]).unwrap();
    let budget = Budget {
        visits: 64,
        requests: 1,
        switches: 1,
    };
    for _ in 0..1_000 {
        let stats = plan(&mut state, target, budget, &mut output).unwrap();
        for build in output.requests() {
            state.complete(build.ticket, 1).unwrap();
        }
        for patch in output.switches() {
            state.publish(patch.token, 0).unwrap();
        }
        output.clear();
        if !stats.pending {
            break;
        }
    }
    let handle = state.find(Chunk::new([0; 3], 0)).unwrap();
    state.invalidate(handle).unwrap();
    plan(&mut state, target, budget, &mut output).unwrap();
    let old = output.requests()[0];
    output.clear();
    state.invalidate(handle).unwrap();
    let stats = plan(&mut state, target, budget, &mut output).unwrap();
    assert_eq!(stats.cancellations, 1);
    assert_eq!(output.cancellations(), &[old.ticket]);
    let next = output.requests()[0];
    output.clear();
    assert_ne!(old.ticket, next.ticket);
    assert_eq!(state.complete(old.ticket, 9).unwrap_err().resource, 9);
    assert_eq!(state.resource(handle), Some(&1));
    state.complete(next.ticket, 10).unwrap();
    plan(&mut state, target, budget, &mut output).unwrap();
    let patch = output.switches()[0];
    assert_eq!(patch.change, Change::Replace(Chunk::new([0; 3], 0)));
    state.publish(patch.token, 5).unwrap();
    assert_eq!(state.resource(handle), Some(&10));
    assert_eq!(state.take_retired(4), None);
    assert_eq!(state.take_retired(5), Some(1));
}
