use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::{api, data};
use api::{Child, SubtreeState};

fn state(values: &[u32], code: u64, lod: u8) -> SubtreeState {
    let count = 1_u64 << (3 * lod);
    let start = code & !(count - 1);
    if start >= values.len() as u64 {
        return SubtreeState::Uniform { value: 0 };
    }
    let first = values[start as usize];
    if start + count > values.len() as u64 && first != 0 {
        return SubtreeState::Mixed;
    }
    let end = (start + count).min(values.len() as u64) as usize;
    if values[start as usize..end]
        .iter()
        .all(|&value| value == first)
        && (start + count <= values.len() as u64 || first == 0)
    {
        SubtreeState::Uniform { value: first }
    } else {
        SubtreeState::Mixed
    }
}

fn states_digest(values: &[SubtreeState]) -> u64 {
    values.iter().fold(0_u64, |hash, state| {
        let value = match *state {
            SubtreeState::Uniform { value } => u64::from(value) << 1,
            SubtreeState::Mixed => 1,
        };
        hash.rotate_left(7) ^ value
    })
}

pub fn benches(c: &mut Criterion) {
    points(c);
    regions(c);
    decode(c);
    prepare(c);
}

fn points(c: &mut Criterion) {
    let mut group = c.benchmark_group("read");
    for scene in ["empty", "solid", "terrain", "checker", "noise"] {
        let values = data::scene_values(scene, 6);
        let dag = data::build(&values, 6);
        let root = api::root(&dag);
        let path = data::path(7319);
        let value = api::value_at(&dag, root, path);
        assert_eq!(value, values[path.value() as usize]);
        data::emit(&format!("read/value_at/{scene}"), data::digest(&[value]));
        data::work(&format!("read/value_at/{scene}"), 1, 1);
        group.throughput(Throughput::Elements(1));
        group.bench_function(BenchmarkId::new("value_at", scene), |b| {
            b.iter(|| black_box(api::value_at(black_box(&dag), root, black_box(path))));
        });
        assert_eq!(api::value_at(&dag, root, path), value);
        data::emit_after(&format!("read/value_at/{scene}"), data::digest(&[value]));
        for (pattern, count) in [
            ("cluster", 0),
            ("cluster", 1),
            ("cluster", 16),
            ("cluster", 4096),
            ("scatter", 4096),
            ("repeated", 4096),
        ] {
            let name = format!("{scene}/{pattern}/{count}");
            let paths = data::paths(pattern, count, 6);
            let mut output = vec![0; count];
            api::values_at(&dag, root, &paths, &mut output);
            let expected: Vec<_> = paths
                .iter()
                .map(|path| values[path.value() as usize])
                .collect();
            assert_eq!(output, expected);
            data::emit(&format!("read/values_at/{name}"), data::digest(&output));
            data::work(&format!("read/values_at/{name}"), count as u64, 1);
            group.throughput(Throughput::Elements(count as u64));
            group.bench_function(BenchmarkId::new("values_at", name), |b| {
                b.iter(|| {
                    api::values_at(
                        black_box(&dag),
                        root,
                        black_box(&paths),
                        black_box(&mut output),
                    );
                    black_box(&output);
                });
            });
            assert_eq!(output, expected);
            data::emit_after(
                &format!("read/values_at/{scene}/{pattern}/{count}"),
                data::digest(&output),
            );
        }
    }
    group.finish();
}

fn regions(c: &mut Criterion) {
    let mut group = c.benchmark_group("structure");
    for scene in ["solid", "terrain", "checker", "noise"] {
        let values = data::scene_values(scene, 6);
        let dag = data::build(&values, 6);
        let root = api::root(&dag);
        for lod in [0, 1, 2, 5, 6, 19, 20] {
            let paths = data::paths("scatter", 128, 7);
            let mut output = vec![SubtreeState::Mixed; paths.len()];
            let expected: Vec<_> = paths
                .iter()
                .map(|path| state(&values, path.value(), lod))
                .collect();
            api::subtrees_at(&dag, root, &paths, lod, &mut output);
            assert_eq!(output, expected);
            let name = format!("{scene}/lod{lod}/128");
            data::emit(
                &format!("structure/subtrees_at/{name}"),
                states_digest(&output),
            );
            data::work(&format!("structure/subtrees_at/{name}"), 128, 1);
            group.throughput(Throughput::Elements(128));
            group.bench_function(BenchmarkId::new("subtrees_at", &name), |b| {
                b.iter(|| {
                    api::subtrees_at(
                        black_box(&dag),
                        root,
                        black_box(&paths),
                        lod,
                        black_box(&mut output),
                    );
                    black_box(&output);
                });
            });
            assert_eq!(output, expected);
            data::emit_after(
                &format!("structure/subtrees_at/{name}"),
                states_digest(&output),
            );
            let path = data::path(7319);
            let actual = api::subtree_at(&dag, root, path, lod);
            assert_eq!(actual, state(&values, path.value(), lod));
            let name = format!("{scene}/lod{lod}");
            data::emit(
                &format!("structure/subtree_at/{name}"),
                states_digest(&[actual]),
            );
            data::work(&format!("structure/subtree_at/{name}"), 1, 1);
            group.throughput(Throughput::Elements(1));
            group.bench_function(BenchmarkId::new("subtree_at", &name), |b| {
                b.iter(|| black_box(api::subtree_at(black_box(&dag), root, black_box(path), lod)));
            });
            assert_eq!(api::subtree_at(&dag, root, path, lod), actual);
            data::emit_after(
                &format!("structure/subtree_at/{name}"),
                states_digest(&[actual]),
            );
        }
        for lod in [0, 2, 6, 20] {
            let path = data::path(7319);
            let child = api::subtree_child(&dag, root, path, lod);
            let actual = match child {
                Child::Leaf(value) => SubtreeState::Uniform { value },
                Child::Ptr(_) => SubtreeState::Mixed,
            };
            assert_eq!(actual, state(&values, path.value(), lod));
            let name = format!("{scene}/lod{lod}");
            data::emit(
                &format!("structure/subtree_child/{name}"),
                states_digest(&[actual]),
            );
            data::work(&format!("structure/subtree_child/{name}"), 1, 1);
            group.throughput(Throughput::Elements(1));
            group.bench_function(BenchmarkId::new("subtree_child", &name), |b| {
                b.iter(|| {
                    black_box(api::subtree_child(
                        black_box(&dag),
                        root,
                        black_box(path),
                        lod,
                    ))
                });
            });
            assert_eq!(api::subtree_child(&dag, root, path, lod), child);
            data::emit_after(
                &format!("structure/subtree_child/{name}"),
                states_digest(&[actual]),
            );
        }
    }
    // 空批和单项批分开，避免把固定边界成本归因于前缀复用。
    let dag = data::build_scene("terrain", 6);
    let root = api::root(&dag);
    for count in [0, 1, 16, 4096] {
        let paths = data::paths("cluster", count, 6);
        let mut output = vec![SubtreeState::Mixed; count];
        api::subtrees_at(&dag, root, &paths, 3, &mut output);
        data::emit(
            &format!("structure/subtrees_size/lod3/{count}"),
            states_digest(&output),
        );
        let before = states_digest(&output);
        data::work(
            &format!("structure/subtrees_size/lod3/{count}"),
            count as u64,
            1,
        );
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(BenchmarkId::new("subtrees_size/lod3", count), |b| {
            b.iter(|| {
                api::subtrees_at(
                    black_box(&dag),
                    root,
                    black_box(&paths),
                    3,
                    black_box(&mut output),
                );
                black_box(&output);
            });
        });
        assert_eq!(states_digest(&output), before);
        data::emit_after(
            &format!("structure/subtrees_size/lod3/{count}"),
            states_digest(&output),
        );
    }
    group.finish();
}

fn decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode");
    for scene in ["empty", "solid", "terrain", "checker", "noise"] {
        let dag = data::build_scene(scene, 6);
        let root = api::root(&dag);
        for lod in [5, 6] {
            let side = 1_usize << lod;
            let base = if lod == 5 { 32_u32 } else { 0 };
            let path = api::Path::encode(base, base, base).unwrap();
            let mut output = vec![0; side.pow(3)];
            api::decode_region(&dag, root, path, lod, &mut output);
            let expected: Vec<_> = (0..output.len())
                .map(|index| {
                    let path = api::Path::encode(
                        base + (index % side) as u32,
                        base + ((index / side) % side) as u32,
                        base + (index / side.pow(2)) as u32,
                    )
                    .unwrap();
                    data::scene_value(scene, path.value(), 6)
                })
                .collect();
            assert_eq!(output, expected);
            let name = format!("{scene}/lod{lod}");
            data::emit(&format!("decode/region/{name}"), data::digest(&output));
            data::work(&format!("decode/region/{name}"), output.len() as u64, 1);
            group.throughput(Throughput::Bytes(output.len() as u64 * 4));
            group.bench_function(BenchmarkId::new("region", &name), |b| {
                b.iter(|| {
                    api::decode_region(black_box(&dag), root, path, lod, black_box(&mut output));
                    black_box(&output);
                });
            });
            assert_eq!(output, expected);
            data::emit_after(&format!("decode/region/{name}"), data::digest(&output));
            for axis in 0..3_u8 {
                for plane in [0, side - 1] {
                    let mut output = vec![0; side.pow(2)];
                    api::decode_plane(&dag, root, path, lod, axis, plane, &mut output);
                    let expected: Vec<_> = (0..output.len())
                        .map(|index| {
                            let mut xyz = [base; 3];
                            xyz[usize::from(axis)] += plane as u32;
                            xyz[(usize::from(axis) + 1) % 3] += (index % side) as u32;
                            xyz[(usize::from(axis) + 2) % 3] += (index / side) as u32;
                            data::scene_value(
                                scene,
                                api::Path::encode(xyz[0], xyz[1], xyz[2]).unwrap().value(),
                                6,
                            )
                        })
                        .collect();
                    assert_eq!(output, expected);
                    let name = format!("{scene}/lod{lod}/axis{axis}/plane{plane}");
                    data::emit(&format!("decode/plane/{name}"), data::digest(&output));
                    data::work(&format!("decode/plane/{name}"), output.len() as u64, 1);
                    group.throughput(Throughput::Bytes(output.len() as u64 * 4));
                    group.bench_function(BenchmarkId::new("plane", &name), |b| {
                        b.iter(|| {
                            api::decode_plane(
                                black_box(&dag),
                                root,
                                path,
                                lod,
                                axis,
                                plane,
                                black_box(&mut output),
                            );
                            black_box(&output);
                        });
                    });
                    assert_eq!(output, expected);
                    data::emit_after(&format!("decode/plane/{name}"), data::digest(&output));
                }
            }
        }
    }
    group.finish();
}

fn prepare(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_prepare");
    let values = data::scene_values("terrain", 6);
    let dag = data::build(&values, 6);
    let root = api::root(&dag);
    for count in [128, 4096] {
        let raw: Vec<_> = (0..count)
            .map(|index| data::path((index as u64 * 0x9e37_79b9) & ((1 << 18) - 1)))
            .collect();
        let expected: Vec<_> = raw
            .iter()
            .map(|path| values[path.value() as usize])
            .collect();
        let mut indexed = Vec::with_capacity(count);
        let mut paths = Vec::with_capacity(count);
        let mut sorted_output = vec![0; count];
        let mut output = vec![0; count];
        data::emit(
            &format!("read_prepare/sort_read_scatter/{count}"),
            data::digest(&expected),
        );
        data::work(
            &format!("read_prepare/sort_read_scatter/{count}"),
            count as u64,
            1,
        );
        group.throughput(Throughput::Elements(count as u64));
        let mut run = || {
            indexed.clear();
            indexed.extend(
                black_box(&raw)
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, path)| (path.value(), index)),
            );
            indexed.sort_unstable();
            paths.clear();
            paths.extend(indexed.iter().map(|&(code, _)| data::path(code)));
            api::values_at(black_box(&dag), root, &paths, &mut sorted_output);
            for ((_, index), value) in indexed.iter().zip(&sorted_output) {
                output[*index] = *value;
            }
            black_box(&output);
        };
        run();
        group.bench_function(BenchmarkId::new("sort_read_scatter", count), |b| {
            b.iter(&mut run)
        });
        drop(run);
        assert_eq!(output, expected);
        data::emit_after(
            &format!("read_prepare/sort_read_scatter/{count}"),
            data::digest(&output),
        );
    }
    group.finish();
}
