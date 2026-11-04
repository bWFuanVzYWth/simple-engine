//! 局部构建／合并成本及调用方显式调度；输入准备和读取核验在计时外。
//!
//! merge_prebuilt 包含目标池分配／去重／祖先发布／析构；local_reuse 包含局部去重和槽重置。
//! scheduled_4 包含 std::thread 创建、调度、join、局部构建、合并与清理，不代表持久线程池。
//! 各路径的工作集同时统计结果槽和输入，不能只据局部构建时间推断应用载入收益。
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use hash_dag::{BuiltRegion, DenseRegion, HashDag, LocalBuilder, LocalTree, ZOrder20x3};
use std::{hint::black_box, time::Duration};

const CHUNKS: usize = 8;
const LOD: u8 = 6;
fn path(code: u64) -> ZOrder20x3 {
    ZOrder20x3::from_value(code).unwrap()
}

fn inputs(scene: &str) -> Vec<Vec<u32>> {
    (0..CHUNKS)
        .map(|chunk| {
            (0..1_u32 << 18)
                .map(|code| {
                    let (x, y, z) = path(u64::from(code)).decode();
                    match scene {
                        "solid" => 7,
                        "terrain" => u32::from(y < 20 + (x * 3 + z * 5 + chunk as u32) % 23) * 7,
                        "repeated" => (code & 63) + 1,
                        "noise" => code.wrapping_mul(0x9e3779b9).rotate_left(13) ^ chunk as u32,
                        _ => unreachable!(),
                    }
                })
                .collect()
        })
        .collect()
}

fn merge(trees: &[LocalTree]) -> HashDag {
    let regions: Vec<_> = trees
        .iter()
        .enumerate()
        .map(|(index, tree)| BuiltRegion {
            path: path((index as u64) << 18),
            tree,
        })
        .collect();
    let mut dag = HashDag::new().unwrap();
    dag.set_built_regions(dag.root(), &regions).unwrap();
    dag
}

fn dense(input: &[Vec<u32>]) -> HashDag {
    let regions: Vec<_> = input
        .iter()
        .enumerate()
        .map(|(index, values)| DenseRegion {
            path: path((index as u64) << 18),
            lod: LOD,
            values,
        })
        .collect();
    let mut dag = HashDag::new().unwrap();
    dag.set_dense_regions(dag.root(), &regions).unwrap();
    dag
}

fn benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("dag_built");
    group.throughput(Throughput::Elements((CHUNKS as u64) << 18));
    for scene in ["solid", "terrain", "repeated", "noise"] {
        let input = inputs(scene);
        let mut trees: Vec<_> = (0..CHUNKS).map(|_| LocalTree::new()).collect();
        let mut builder = LocalBuilder::new();
        for (tree, values) in trees.iter_mut().zip(&input) {
            builder.build(tree, LOD, values).unwrap();
        }
        let imported = merge(&trees);
        let expected = dense(&input);
        for (index, values) in input.iter().enumerate() {
            for local in [0, 1, 63, 64, 4095, 262143] {
                let p = path(((index as u64) << 18) + local as u64);
                assert_eq!(
                    imported.value_at(imported.root(), p).unwrap(),
                    values[local]
                );
                assert_eq!(
                    expected.value_at(expected.root(), p).unwrap(),
                    values[local]
                );
            }
        }
        eprintln!(
            "{scene}: dense_input={} local_slots={} builder_scratch={} target_imported={} target_dense={}B (仅heap容量，不含Self/allocator/线程栈)",
            input
                .iter()
                .map(|x| x.capacity() * size_of::<u32>())
                .sum::<usize>(),
            trees.iter().map(LocalTree::retained_bytes).sum::<usize>(),
            builder.retained_bytes(),
            imported.retained_bytes(),
            expected.retained_bytes()
        );
        drop(imported);
        drop(expected);
        group.bench_function(BenchmarkId::new("dense_serial", scene), |b| {
            b.iter(|| black_box(dense(black_box(&input))))
        });
        group.bench_function(BenchmarkId::new("local_reuse", scene), |b| {
            b.iter(|| {
                for (tree, values) in trees.iter_mut().zip(&input) {
                    builder.build(tree, LOD, black_box(values)).unwrap();
                }
                black_box(&trees);
            })
        });
        group.bench_function(BenchmarkId::new("merge_prebuilt", scene), |b| {
            b.iter(|| black_box(merge(black_box(&trees))))
        });
        group.bench_function(BenchmarkId::new("scheduled_4", scene), |b| {
            b.iter(|| {
                std::thread::scope(|scope| {
                    let mut jobs = Vec::new();
                    for (outputs, inputs) in trees.chunks_mut(2).zip(input.chunks(2)) {
                        jobs.push(scope.spawn(move || {
                            let mut worker = LocalBuilder::new();
                            for (tree, values) in outputs.iter_mut().zip(inputs) {
                                worker.build(tree, LOD, values).unwrap();
                            }
                        }));
                    }
                    for job in jobs {
                        job.join().expect("bench线程panic");
                    }
                });
                black_box(merge(&trees));
            })
        });
    }
    group.finish();
}
criterion_group! {name=benches_group;config=Criterion::default().sample_size(20).measurement_time(Duration::from_secs(1));targets=benches}
criterion_main!(benches_group);
