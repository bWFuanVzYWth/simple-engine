//! 根差分的独立成本与快照编辑全链路；输入准备、初始 DAG 和输出 oracle 在计时外。
//! warm 复用输出和访问集合，cold_buffer 包含首次容量增长；edit_diff_release_pair
//! 每迭代编辑两次并恢复相同终态，计入 fork、写入、差分和释放快照，不含应用调度或 GPU。
use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use hash_dag::{DenseRegion, HashDag, NodeDiff, RootId, SetCell, ZOrder20x3};

fn path(code: u64) -> ZOrder20x3 {
    ZOrder20x3::from_value(code).unwrap()
}

fn build(scene: &str) -> HashDag {
    let values: Vec<_> = (0..1_u32 << 18)
        .map(|code| match scene {
            "repeated" => (code & 63) + 1,
            "noise" => code.wrapping_mul(0x9e37_79b9).rotate_left(13) ^ 0xd1b5_4a32,
            _ => unreachable!(),
        })
        .collect();
    let mut dag = HashDag::new().unwrap();
    dag.set_dense_regions(
        dag.root(),
        &[DenseRegion {
            path: path(0),
            lod: 6,
            values: &values,
        }],
    )
    .unwrap();
    dag
}

fn edits(count: usize, value: u32) -> Vec<SetCell> {
    let mut codes: Vec<_> = (0..count as u64)
        .map(|index| index.wrapping_mul(0x9e37_79b9) & ((1 << 18) - 1))
        .collect();
    codes.sort_unstable();
    codes
        .into_iter()
        .map(|code| SetCell {
            path: path(code),
            data: value,
        })
        .collect()
}

fn digest(diff: &NodeDiff) -> u64 {
    diff.nodes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, node| {
            (hash ^ (u64::from(node.level) << 32 | node.index.get())).wrapping_mul(0x100_0000_01b3)
        })
}

fn pair(dag: &mut HashDag, root: RootId, diff: &mut NodeDiff, a: &[SetCell], b: &[SetCell]) {
    for edits in [a, b] {
        let snapshot = dag.fork_root(root).unwrap();
        dag.set_cells(root, edits).unwrap();
        black_box(diff.between(dag, snapshot, root).unwrap());
        dag.release_root(snapshot).unwrap();
    }
}

fn benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("root_diff");
    let mut shared = build("repeated");
    let root = shared.root();
    let empty = shared.fork_root(root).unwrap();
    shared
        .fill_uniform(
            empty,
            &[hash_dag::UniformFill {
                path: path(0),
                lod: HashDag::AXIS_BITS,
                data: 0,
            }],
        )
        .unwrap();
    let mut diff = NodeDiff::new();
    assert_eq!(
        diff.between(&shared, empty, root).unwrap().len(),
        usize::from(HashDag::DEPTH)
    );
    println!("SVDAG_ORACLE\troot_diff/shared\t{:016x}", digest(&diff));
    group.bench_function("shared", |b| {
        b.iter(|| {
            black_box(diff.between(black_box(&shared), empty, root).unwrap());
        })
    });
    group.bench_function("unchanged", |b| {
        b.iter(|| {
            black_box(diff.between(black_box(&shared), root, root).unwrap());
        })
    });
    shared.release_root(empty).unwrap();

    for count in [1, 128, 4096] {
        let mut dag = build("noise");
        let root = dag.root();
        let before = dag.fork_root(root).unwrap();
        let a = edits(count, 29);
        let b = edits(count, 31);
        dag.set_cells(root, &a).unwrap();
        let mut diff = NodeDiff::new();
        let changed = diff.between(&dag, before, root).unwrap().len();
        let expected = digest(&diff);
        println!("SVDAG_ORACLE\troot_diff/warm/{count}\t{expected:016x}");
        println!(
            "SVDAG_RESOURCE\troot_diff/warm/{count}\tdiff_retained_bytes\t{}",
            diff.retained_bytes()
        );
        println!("SVDAG_WORK\troot_diff/warm/{count}\t{count}\t1");
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(BenchmarkId::new("warm", count), |bench| {
            bench.iter(|| {
                black_box(diff.between(black_box(&dag), before, root).unwrap());
            })
        });
        assert_eq!(diff.nodes().len(), changed);
        assert_eq!(digest(&diff), expected);
        group.bench_function(BenchmarkId::new("cold_buffer", count), |bench| {
            bench.iter_batched(
                NodeDiff::new,
                |mut diff| {
                    black_box(diff.between(black_box(&dag), before, root).unwrap());
                    black_box(diff)
                },
                BatchSize::LargeInput,
            );
        });
        dag.release_root(before).unwrap();
        dag.set_cells(root, &b).unwrap();
        pair(&mut dag, root, &mut diff, &a, &b);
        println!(
            "SVDAG_WORK\troot_diff/edit_diff_release_pair/{count}\t{}\t8",
            count * 2
        );
        println!(
            "SVDAG_RESOURCE\troot_diff/edit_diff_release_pair/{count}\tdag_retained_bytes\t{}",
            dag.retained_bytes()
        );
        group.throughput(Throughput::Elements(count as u64 * 2));
        group.bench_function(BenchmarkId::new("edit_diff_release_pair", count), |bench| {
            bench.iter(|| {
                pair(
                    black_box(&mut dag),
                    root,
                    black_box(&mut diff),
                    black_box(&a),
                    black_box(&b),
                );
            })
        });
        for cell in &b {
            assert_eq!(dag.value_at(root, cell.path).unwrap(), cell.data);
        }
    }
    group.finish();
}

criterion_group! {
    name = diff;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_millis(500));
    targets = benches
}
criterion_main!(diff);
