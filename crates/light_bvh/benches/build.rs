//! 包含验证、临时分配、median 分区、sibling packet 直接编码及完整输出消费的构建成本。
//! fixture 准备在计时外；没有 GPU、上传或线程调度，不代表整帧时间。
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use light_bvh::{Quad, Tree};
use std::hint::black_box;

fn fixture(count: usize, clustered: bool) -> Vec<Quad> {
    (0..count)
        .map(|index| {
            let x = if clustered {
                (index % 7) as f32 * 0.01
            } else {
                (index % 64) as f32 * 4.0
            };
            let y = if clustered {
                0.0
            } else {
                ((index / 64) % 64) as f32 * 4.0
            };
            let z = if clustered {
                0.0
            } else {
                (index / 4096) as f32 * 4.0
            };
            Quad {
                vertices: [
                    [x, y, z],
                    [x + 1.0, y, z],
                    [x + 1.0, y + 1.0, z],
                    [x, y + 1.0, z],
                ],
                power: 1.0 + (index % 17) as f32,
            }
        })
        .collect()
}

fn benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("light_bvh/build");
    for kind in ["grid", "clustered"] {
        for count in [16, 1024, 16_384] {
            let quads = fixture(count, kind == "clustered");
            group.throughput(Throughput::Elements(count as u64));
            group.bench_with_input(BenchmarkId::new(kind, count), &quads, |b, quads| {
                b.iter(|| {
                    let tree = Tree::build(black_box(quads)).expect("有效 fixture");
                    black_box(
                        tree.words()
                            .iter()
                            .fold(0_u32, |hash, &word| hash.rotate_left(1) ^ word),
                    );
                })
            });
        }
    }
    group.finish();
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
