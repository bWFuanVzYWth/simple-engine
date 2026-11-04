use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};

use crate::{api, data};
use api::SetCell;

pub fn benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("lifecycle");
    group.throughput(Throughput::Elements(1));
    let dag = api::new();
    assert_eq!(api::value_at(&dag, api::root(&dag), data::path(0)), 0);
    data::emit("lifecycle/new_drop", 0);
    data::work("lifecycle/new_drop", 1, 1);
    data::resource("lifecycle/new", &dag);
    group.bench_function("new_drop", |b| b.iter(|| drop(black_box(api::new()))));
    data::emit_after("lifecycle/new_drop", 0);
    for scene in ["empty", "solid", "terrain", "noise"] {
        let values = data::scene_values(scene, 6);
        let mut dag = data::build(&values, 6);
        let root = api::root(&dag);
        let snapshot = api::fork(&mut dag, root);
        assert_eq!(
            api::value_at(&dag, snapshot, data::path(7319)),
            values[7319]
        );
        api::release(&mut dag, snapshot);
        data::emit(
            &format!("lifecycle/fork_release/{scene}"),
            data::digest(&[values[7319]]),
        );
        data::resource(&format!("lifecycle/fork_release/{scene}"), &dag);
        data::work(&format!("lifecycle/fork_release/{scene}"), 1, 2);
        group.bench_function(BenchmarkId::new("fork_release", scene), |b| {
            b.iter(|| {
                let snapshot = api::fork(black_box(&mut dag), root);
                api::release(black_box(&mut dag), black_box(snapshot));
            });
        });
        assert_eq!(api::value_at(&dag, root, data::path(7319)), values[7319]);
        data::emit_after(
            &format!("lifecycle/fork_release/{scene}"),
            data::digest(&[values[7319]]),
        );
        // 构造放在 setup，计时只包含实际所有者析构。
        data::work(&format!("lifecycle/drop_populated/{scene}"), 1, 1);
        group.bench_function(BenchmarkId::new("drop_populated", scene), |b| {
            b.iter_batched(
                || data::build(black_box(&values), 6),
                |dag| drop(black_box(dag)),
                BatchSize::LargeInput,
            );
        });
    }
    let values = data::scene_values("terrain", 6);
    data::work("lifecycle/fork_release_first_growth", 1, 2);
    group.bench_function("fork_release_first_growth", |b| {
        b.iter_batched(
            || data::build(&values, 6),
            |mut dag| {
                let root = api::root(&dag);
                let snapshot = api::fork(&mut dag, root);
                api::release(&mut dag, snapshot);
                black_box(dag)
            },
            BatchSize::LargeInput,
        );
    });
    // 默认根修改后旧快照成为独占分支，release 才触发实际级联回收。
    let edits: Vec<_> = data::paths("scatter", 4096, 6)
        .into_iter()
        .map(|path| SetCell { path, data: 13 })
        .collect();
    data::work("lifecycle/release_unique/4096", 1, 1);
    group.bench_function("release_unique/4096", |b| {
        b.iter_batched(
            || {
                let mut dag = data::build(&values, 6);
                let root = api::root(&dag);
                let snapshot = api::fork(&mut dag, root);
                api::set_cells(&mut dag, root, &edits);
                (dag, snapshot)
            },
            |(mut dag, snapshot)| {
                api::release(&mut dag, snapshot);
                black_box(dag)
            },
            BatchSize::LargeInput,
        );
    });
    for snapshots in [1, 8, 64] {
        let mut dag = data::build(&values, 6);
        let root = api::root(&dag);
        let mut held = Vec::with_capacity(snapshots);
        for version in 0..snapshots {
            held.push(api::fork(&mut dag, root));
            api::set_cells(
                &mut dag,
                root,
                &[SetCell {
                    path: data::path(version as u64 * 127),
                    data: version as u32 + 17,
                }],
            );
        }
        let edit_paths = data::paths("scatter", 128, 6);
        let a: Vec<_> = edit_paths
            .iter()
            .copied()
            .map(|path| SetCell { path, data: 29 })
            .collect();
        let b: Vec<_> = edit_paths
            .iter()
            .copied()
            .map(|path| SetCell { path, data: 31 })
            .collect();
        api::set_cells(&mut dag, root, &a);
        api::set_cells(&mut dag, root, &b);
        for &path in &edit_paths {
            assert_eq!(api::value_at(&dag, root, path), 31);
        }
        assert_eq!(api::value_at(&dag, held[0], data::path(0)), values[0]);
        data::emit(
            &format!("lifecycle/update_held/128_pair/{snapshots}"),
            data::digest(&[31, values[0]]),
        );
        data::resource(&format!("lifecycle/update_held/{snapshots}"), &dag);
        data::work(
            &format!("lifecycle/update_held/128_pair/{snapshots}"),
            256,
            2,
        );
        group.throughput(Throughput::Elements(256));
        group.bench_function(
            BenchmarkId::new("update_held/128_pair", snapshots),
            |bench| {
                bench.iter(|| {
                    api::set_cells(black_box(&mut dag), root, black_box(&a));
                    api::set_cells(black_box(&mut dag), root, black_box(&b));
                });
            },
        );
        for &path in &edit_paths {
            assert_eq!(api::value_at(&dag, root, path), 31);
        }
        assert_eq!(api::value_at(&dag, held[0], data::path(0)), values[0]);
        data::emit_after(
            &format!("lifecycle/update_held/128_pair/{snapshots}"),
            data::digest(&[31, values[0]]),
        );
        data::resource(&format!("lifecycle/update_held_after/{snapshots}"), &dag);
        for snapshot in held {
            api::release(&mut dag, snapshot);
        }
    }
    group.finish();
}
