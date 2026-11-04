//! CPU DAG 的构建、快照编辑和有序查询；输入准备在计时外，构建包含分配与析构。
use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use hash_dag::{DenseRegion, HashDag, SetCell, ZOrder20x3};

fn path(code: u64) -> ZOrder20x3 {
    ZOrder20x3::from_value(code).unwrap()
}

fn values(scene: &str) -> Vec<u32> {
    (0..1_u32 << 18)
        .map(|code| {
            let (x, y, z) = path(u64::from(code)).decode();
            match scene {
                "solid" => 7,
                "terrain" => u32::from(y < 24 + (x * 3 + z * 5) % 17) * 7,
                "repeated" => (code & 63) * 7 + ((code >> 6) & 1) * 101 + 1,
                "noise" => code.wrapping_mul(0x9e37_79b9).rotate_left(13) ^ 0xd1b5_4a32,
                _ => unreachable!(),
            }
        })
        .collect()
}

fn build(values: &[u32]) -> HashDag {
    let mut dag = HashDag::new().unwrap();
    dag.set_dense_regions(
        dag.root(),
        &[DenseRegion {
            path: ZOrder20x3::ZERO,
            lod: 6,
            values,
        }],
    )
    .unwrap();
    dag
}

fn benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("dag");
    for scene in ["solid", "terrain", "repeated", "noise"] {
        let input = values(scene);
        group.bench_with_input(BenchmarkId::new("build", scene), &input, |b, input| {
            b.iter(|| black_box(build(black_box(input))));
        });
    }
    let mut dag = build(&values("terrain"));
    for count in [1, 128, 4096] {
        let paths: Vec<_> = (0..count).map(|i| path(i * 31)).collect();
        let mut output = vec![0; paths.len()];
        group.bench_function(BenchmarkId::new("read", count), |b| {
            b.iter(|| {
                dag.values_at(dag.root(), black_box(&paths), black_box(&mut output))
                    .unwrap();
                black_box(&output);
            });
        });
    }
    let mut edits: Vec<_> = (0..128)
        .map(|i| SetCell {
            path: path(i * 31),
            data: 3,
        })
        .collect();
    group.bench_function("edit/128_snapshot", |b| {
        b.iter(|| {
            let snapshot = dag.fork_root(dag.root()).unwrap();
            for edit in &mut edits {
                edit.data ^= 1;
            }
            dag.set_cells(dag.root(), black_box(&edits)).unwrap();
            dag.release_root(snapshot).unwrap();
        });
    });
    group.finish();
    let mut group = c.benchmark_group("dag/export");
    for scene in ["solid", "terrain", "noise"] {
        let dag = build(&values(scene));
        let bytes = (0..HashDag::DEPTH)
            .map(|level| {
                let (values, masks) = dag.node_arrays(level).unwrap();
                size_of_val(values) + size_of_val(masks)
            })
            .sum::<usize>();
        let mut output = Vec::<u8>::with_capacity(bytes);
        group.throughput(criterion::Throughput::Elements(u64::from(HashDag::DEPTH)));
        group.bench_function(BenchmarkId::new("borrow", scene), |b| {
            b.iter(|| {
                for level in 0..HashDag::DEPTH {
                    black_box(dag.node_arrays(black_box(level)).unwrap());
                }
                black_box(dag.root_child(dag.root()).unwrap());
            });
        });
        group.throughput(criterion::Throughput::Bytes(bytes as u64));
        group.bench_function(BenchmarkId::new("copy", scene), |b| {
            b.iter(|| {
                output.clear();
                for level in 0..HashDag::DEPTH {
                    let (values, masks) = dag.node_arrays(level).unwrap();
                    output.extend_from_slice(bytemuck::cast_slice(values));
                    output.extend_from_slice(bytemuck::cast_slice(masks));
                }
                black_box(&output);
            });
        });
    }
    group.finish();
}

criterion_group! {
    name = dag;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_millis(500));
    targets = benches
}
criterion_main!(dag);
