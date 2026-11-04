//! `initial_step` 在计时外预分配状态，只计固定预算的首次规划，不合并输出。
//! `budget_scale` 保持半径与输出额度，只改变访问预算；各组状态容量均为 30_000。
//! `stable` 预先收敛，只计稳定目标规划；`move` 与 `teleport` 包含规划、即时构建
//! 完成、覆盖发布和有界回收，不包含真实建网、GPU 工作或状态初始化与析构。
//! 所有 case 每次只推进一帧，默认预算为 128 次访问、16 个请求、16 次替换；吞吐量按帧记。

use chunk_manager::{Bounds, Budget, Chunk, Config, Plan, State, StateError, Stats, Target, plan};
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

const BUDGET: Budget = Budget {
    visits: 128,
    requests: 16,
    switches: 16,
};
const RETIRE_BUDGET: usize = 128;
const ORIGIN: [i64; 3] = [0; 3];

struct Host {
    state: State<u64>,
    batch: Plan,
    frame: u64,
}

impl Host {
    fn new(radius: u32) -> Self {
        Self::configured(Config::new(3, radius, 30_000).unwrap())
    }

    fn configured(config: Config) -> Self {
        Self {
            state: State::new(config),
            batch: Plan::with_capacity(BUDGET.requests, BUDGET.switches),
            frame: 0,
        }
    }

    fn step(&mut self, coord: [i64; 3]) -> Stats {
        self.step_budget(coord, BUDGET)
    }

    fn step_budget(&mut self, coord: [i64; 3], budget: Budget) -> Stats {
        let stats = plan(
            &mut self.state,
            Target::new(coord).unwrap(),
            budget,
            &mut self.batch,
        )
        .unwrap();
        self.frame += 1;
        for build in self.batch.requests() {
            self.state.complete(build.ticket, self.frame).unwrap();
        }
        for patch in self.batch.switches() {
            match self.state.publish(patch.token, self.frame) {
                Ok(()) => {}
                Err(StateError::Transition) => self.state.reject_patch(patch.token).unwrap(),
                Err(error) => panic!("覆盖发布失败：{error}"),
            }
        }
        for _ in 0..RETIRE_BUDGET {
            let Some(resource) = self.state.take_retired(self.frame) else {
                break;
            };
            black_box(resource);
        }
        self.batch.clear();
        stats
    }

    fn settled() -> Self {
        let mut host = Self::new(4);
        for _ in 0..100_000 {
            let stats = host.step(ORIGIN);
            if !stats.pending && host.state.retired_len() == 0 {
                return host;
            }
        }
        panic!("区块状态未在预热上限内收敛");
    }

    fn settle(&mut self, budget: Budget) -> (usize, usize, usize) {
        let mut totals = (0, 0, 0);
        for _ in 0..100_000 {
            let stats = self.step_budget(ORIGIN, budget);
            totals.0 += stats.visits;
            totals.1 += stats.requests;
            totals.2 += stats.switches;
            if !stats.pending {
                return totals;
            }
        }
        panic!("真实调用链未在基准上限内收敛");
    }
}

fn bounded() -> Config {
    Config::new(1, 1_000, 8_192)
        .unwrap()
        .with_bounds(Some(Bounds::new([0; 3], [8; 3]).unwrap()))
}

fn initial_step(host: &mut Host, budget: Budget) -> Stats {
    plan(
        &mut host.state,
        Target::new(black_box(ORIGIN)).unwrap(),
        budget,
        &mut host.batch,
    )
    .unwrap()
}

fn print_initial_stats(name: &str, radius: u32, budget: Budget) {
    let mut host = Host::new(radius);
    let stats = initial_step(&mut host, budget);
    eprintln!(
        "{name}: radius={radius} visits={} requests={} switches={} blocked={} nodes={} state_bytes={} plan_bytes={}",
        stats.visits,
        stats.requests,
        stats.switches,
        stats.blocked,
        host.state.len(),
        host.state.reserved_bytes(),
        host.batch.reserved_bytes()
    );
}

fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("chunk/planning");
    group.throughput(Throughput::Elements(1));

    for radius in [4, 6, 64] {
        print_initial_stats("initial_step", radius, BUDGET);
        group.bench_with_input(
            BenchmarkId::new("initial_step", radius),
            &radius,
            |bench, &radius| {
                bench.iter_batched_ref(
                    || Host::new(radius),
                    |host| {
                        black_box(initial_step(host, BUDGET));
                    },
                    BatchSize::PerIteration,
                );
            },
        );
    }

    for visits in [32, 128, 512] {
        let budget = Budget { visits, ..BUDGET };
        print_initial_stats("budget_scale", 4, budget);
        group.bench_with_input(
            BenchmarkId::new("budget_scale", visits),
            &budget,
            |bench, &budget| {
                bench.iter_batched_ref(
                    || Host::new(4),
                    |host| {
                        black_box(initial_step(host, budget));
                    },
                    BatchSize::PerIteration,
                );
            },
        );
    }

    let mut stable = Host::settled();
    group.bench_function("stable", |bench| {
        bench.iter(|| {
            let stats = plan(
                &mut stable.state,
                Target::new(black_box(ORIGIN)).unwrap(),
                BUDGET,
                &mut stable.batch,
            )
            .unwrap();
            black_box(stats);
            stable.batch.clear();
        });
    });

    let mut moving = Host::settled();
    let mut phase = 0_i64;
    group.bench_function("move", |bench| {
        bench.iter(|| {
            // 往返单格移动；相同有限路径避免样本间的世界坐标持续漂移。
            phase = (phase + 1) % 32;
            let x = if phase <= 16 { phase } else { 32 - phase };
            black_box(moving.step(black_box([x, 0, 0])));
        });
    });

    let mut teleporting = Host::settled();
    let mut side = 1_i64;
    group.bench_function("teleport", |bench| {
        bench.iter(|| {
            side = -side;
            black_box(teleporting.step(black_box([side * 32_768, 0, 0])));
        });
    });
    group.finish();

    // 全程仅使用原有公开入口，可把同一基准文件放入冻结源码对照。
    // 状态初始化在计时外；以下包含规划、完成、发布与回收，不包含真实资源构建。
    let mut throughput = criterion.benchmark_group("chunk/throughput");
    for visits in [1, 16, 128] {
        let budget = Budget { visits, ..BUDGET };
        let mut sample = Host::configured(bounded());
        let totals = sample.settle(budget);
        assert_eq!(totals.1, 512);
        assert_eq!(totals.2, 512);
        eprintln!(
            "bounded_initial visits={visits}: frames={} work={} requests={} switches={} effective_ratio={:.6}",
            sample.frame,
            totals.0,
            totals.1,
            totals.2,
            (totals.1 + totals.2) as f64 / totals.0 as f64
        );
        throughput.throughput(Throughput::Elements(512));
        throughput.bench_with_input(
            BenchmarkId::new("initial_to_cut", visits),
            &budget,
            |bench, &budget| {
                bench.iter_batched_ref(
                    || Host::configured(bounded()),
                    |host| {
                        black_box(host.settle(budget));
                    },
                    BatchSize::PerIteration,
                );
            },
        );
    }

    let mut sparse = Host::configured(bounded());
    sparse.settle(BUDGET);
    let edited = sparse.state.find(Chunk::new([0; 3], 0)).unwrap();
    let before = sparse.frame;
    sparse.state.invalidate(edited).unwrap();
    let totals = sparse.settle(BUDGET);
    eprintln!(
        "sparse_edit: frames={} work={} requests={} switches={}",
        sparse.frame - before,
        totals.0,
        totals.1,
        totals.2
    );
    throughput.throughput(Throughput::Elements(1));
    throughput.bench_function("single_edit_to_cut", |bench| {
        bench.iter(|| {
            sparse.state.invalidate(edited).unwrap();
            black_box(sparse.settle(BUDGET));
        });
    });

    let mut paused = Host::configured(bounded());
    let blocked_budget = Budget {
        requests: 0,
        switches: 0,
        ..BUDGET
    };
    // 先发现全部512个节点，排除根发现成本，保留真实额度背压状态。
    for _ in 0..256 {
        paused.step_budget(ORIGIN, blocked_budget);
    }
    let stats = paused.step_budget(ORIGIN, blocked_budget);
    eprintln!(
        "quota_blocked: work={} requests={} switches={} blocked={}",
        stats.visits, stats.requests, stats.switches, stats.blocked
    );
    throughput.bench_function("quota_blocked_frame", |bench| {
        bench.iter(|| {
            black_box(paused.step_budget(ORIGIN, blocked_budget));
        });
    });

    let mut equivalent = Host::configured(bounded());
    equivalent.settle(BUDGET);
    let mut flip = false;
    equivalent.state.set_radius(1_001).unwrap();
    let stats = equivalent.step([1, 0, 0]);
    eprintln!(
        "unchanged_window: work={} requests={} switches={} blocked={}",
        stats.visits, stats.requests, stats.switches, stats.blocked
    );
    throughput.bench_function("unchanged_window_config_and_target", |bench| {
        bench.iter(|| {
            flip = !flip;
            equivalent
                .state
                .set_radius(1_000 + u32::from(flip))
                .unwrap();
            black_box(equivalent.step([i64::from(flip), 0, 0]));
        });
    });
    throughput.finish();
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
