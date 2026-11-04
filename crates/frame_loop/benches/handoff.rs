//! 计入完整交接、覆盖析构和时钟准备；`take_prepared` 单独排除槽位准备。
//! 载荷直接内联，测量同步和移动，不混入业务分配、线程创建或应用调度。

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use frame_loop::{SessionClock, TickClock, latest};
use std::hint::black_box;

fn handoff<const N: usize>(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group(format!("latest/{}B", N * size_of::<u64>()));
    group.throughput(Throughput::Elements(1));
    group.bench_function("create_drop", |bench| {
        bench.iter(|| black_box(latest::<[u64; N]>()));
    });
    let mut slot = latest();
    let (mut publisher, mut consumer) = slot.split();
    group.bench_function("publish_take", |bench| {
        bench.iter(|| {
            publisher.publish(black_box([7_u64; N])).unwrap();
            black_box(consumer.take());
        });
    });
    publisher.publish([7_u64; N]).unwrap();
    group.bench_function("publish_overwrite", |bench| {
        bench.iter(|| publisher.publish(black_box([11_u64; N])).unwrap());
    });
    consumer.take();
    group.bench_function("take_empty", |bench| {
        bench.iter(|| black_box(consumer.take()));
    });
    group.bench_function("take_prepared", |bench| {
        bench.iter_batched(
            || publisher.publish([13_u64; N]).unwrap(),
            |()| black_box(consumer.take()),
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

fn clocks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("clock");
    group.bench_function("session_prepare", |bench| {
        bench.iter(|| black_box(SessionClock::new()));
    });
    let session = SessionClock::new();
    group.bench_function("session_now", |bench| {
        bench.iter(|| black_box(session.now_ns().unwrap()));
    });
    group.bench_function("session_deadline", |bench| {
        bench.iter(|| black_box(session.instant(black_box(1_000_000_000)).unwrap()));
    });
    group.bench_function("tick_prepare", |bench| {
        bench.iter(|| black_box(TickClock::new(black_box(17), black_box(30)).unwrap()));
    });
    let mut ticks = TickClock::new(17, 30).unwrap();
    group.bench_function("tick_advance", |bench| {
        bench.iter(|| {
            ticks.advance().unwrap();
            black_box(ticks.deadline_ns());
        });
    });
    group.finish();
}

fn benchmarks(criterion: &mut Criterion) {
    handoff::<4>(criterion);
    handoff::<128>(criterion);
    clocks(criterion);
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
