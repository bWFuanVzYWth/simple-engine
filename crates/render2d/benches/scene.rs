//! CPU 场景构建、校验、分箱及图表几何；无字体、shader 或 GPU。
//!
//! 复用组包含 clear 和重建，cold 组另含分配/析构；validate 只检查预建场景。
//! 图表输入在计时外生成，输出 Vec 复用；Elements 表示控件或输入样本数。
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use render2d::{Color, DrawList, Rect, plot};

const BOUNDS: Rect = Rect::new(0.0, 0.0, 1024.0, 768.0);

fn build(list: &mut DrawList, count: usize) {
    list.clear();
    let mut canvas = list.canvas(BOUNDS);
    for i in 0..count {
        let rect = Rect::new(
            (i % 32) as f32 * 32.0,
            (i / 32 % 32) as f32 * 24.0,
            30.0,
            22.0,
        );
        let mut clipped = canvas.clipped(rect);
        clipped.rounded_rect(rect, 3.0, Color::linear(0.2, 0.4, 0.8, 0.5));
        clipped.border(rect.inset(1.0), 2.0, 1.0, Color::WHITE);
    }
}

fn scenes(c: &mut Criterion) {
    let mut group = c.benchmark_group("scene");
    for count in [64_usize, 1024, 8192] {
        let mut list = DrawList::new();
        build(&mut list, count);
        list.validate().unwrap();
        println!(
            "SCENE\t{count}\tretained_bytes={}\tops={}",
            list.retained_bytes(),
            list.ops().len()
        );
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(BenchmarkId::new("build_reused", count), |b| {
            b.iter(|| {
                build(&mut list, black_box(count));
                black_box(&list);
            });
        });
        group.bench_function(BenchmarkId::new("validate", count), |b| {
            b.iter(|| black_box(&list).validate().unwrap());
        });
        group.bench_function(BenchmarkId::new("build_drop_cold", count), |b| {
            b.iter(|| {
                let mut list = DrawList::new();
                build(&mut list, black_box(count));
                black_box(&list);
            });
        });
    }
    group.finish();
}

fn plots(c: &mut Criterion) {
    let mut group = c.benchmark_group("plot");
    for count in [512_usize, 8192] {
        let samples: Vec<_> = (0..count)
            .map(|i| ((i * 79) % 1800) as f32 * 0.01 - 1.0)
            .collect();
        let mut histogram = plot::Histogram::new(32, 0.0, 16.0).unwrap();
        histogram.replace(&samples);
        assert_eq!(
            histogram.bins().iter().sum::<usize>(),
            samples
                .iter()
                .filter(|&&v| (0.0..=16.0).contains(&v))
                .count()
        );
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(BenchmarkId::new("histogram_replace", count), |b| {
            b.iter(|| {
                histogram.replace(black_box(&samples));
                black_box(histogram.bins());
            });
        });
    }
    let mut list = DrawList::new();
    let bins: Vec<_> = (0..64).map(|i| (i * 17) % 101).collect();
    group.throughput(Throughput::Elements(bins.len() as u64));
    group.bench_function("histogram_geometry", |b| {
        b.iter(|| {
            list.clear();
            plot::histogram(
                &mut list.canvas(BOUNDS),
                BOUNDS,
                black_box(&bins),
                Color::WHITE,
            );
            black_box(&list);
        });
    });
    for count in [64_usize, 512] {
        let samples: Vec<_> = (0..count)
            .map(|i| {
                std::array::from_fn::<_, 4, _>(|stage| ((i * 79 + stage * 37) % 100) as f32 * 0.01)
            })
            .collect();
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(BenchmarkId::new("stacked_geometry", count), |b| {
            b.iter(|| {
                list.clear();
                plot::stacked_bars(
                    &mut list.canvas(BOUNDS),
                    BOUNDS,
                    black_box(&samples),
                    &[Color::WHITE; 4],
                    4.0,
                );
                black_box(&list);
            });
        });
    }
    list.validate().unwrap();
    group.finish();
}

criterion_group!(benches, scenes, plots);
criterion_main!(benches);
