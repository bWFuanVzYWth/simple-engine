//! 批写覆盖：同源输入、独立值 oracle 与显式准备成本。
//!
//! `cold_pair` 从零值新池开始，计时写入和清零，包含池与写缓冲增长；实例创建与
//! 析构在计时外。`warm_pair` 从 A 开始，预热后计时 B、A 两次调用，恢复初态。
//! `snapshot_pair` 同样执行 B、A，计时外持有 A 的快照，直到该项测量结束。
//! 吞吐量统计两次调用的输入条目或 dense 体素，重复点仍按输入条目计数。
//! 场景生成、native 描述与 oracle 均在计时外；乱序输入另列准备和端到端组。
//! 验证组以批末非法项拒绝整批，衡量公共边界扫描，不代表成功写入的完整校验成本。
//! cold 输出独立预检摘要；其逐次对象由 Criterion 回收，不输出测量后的状态摘要。

use std::{collections::BTreeMap, hint::black_box};

use criterion::{
    BatchSize, BenchmarkGroup, BenchmarkId, Criterion, Throughput, measurement::WallTime,
};

use crate::{
    api::{self, Dag, DenseRegion, Path, Root, SetCell, UniformFill},
    data,
};

const ROOT_SPAN: u64 = 1_u64 << 60;

struct Oracle {
    paths: Vec<Path>,
    a: Vec<u32>,
    b: Vec<u32>,
}

impl Oracle {
    fn check(&self, dag: &Dag, root: Root, expected: &[u32]) -> u64 {
        let mut actual = vec![0; self.paths.len()];
        api::values_at(dag, root, &self.paths, &mut actual);
        assert_eq!(actual, expected);
        data::digest(&actual)
    }
}

#[allow(clippy::too_many_arguments)]
fn run_case<T: ?Sized>(
    group: &mut BenchmarkGroup<'_, WallTime>,
    kind: &str,
    name: &str,
    a: &T,
    b: &T,
    zero: &T,
    apply: fn(&mut Dag, Root, &T),
    oracle: &Oracle,
    cold: bool,
    work: u64,
) {
    let mut checked = api::new();
    let root = api::root(&checked);
    apply(&mut checked, root, a);
    oracle.check(&checked, root, &oracle.a);
    let snapshot = api::fork(&mut checked, root);
    apply(&mut checked, root, b);
    oracle.check(&checked, root, &oracle.b);
    oracle.check(&checked, snapshot, &oracle.a);
    apply(&mut checked, root, a);
    oracle.check(&checked, root, &oracle.a);
    api::release(&mut checked, snapshot);
    data::resource(&format!("{kind}/checked/{name}"), &checked);
    drop(checked);
    if cold {
        let mut fresh = api::new();
        let root = api::root(&fresh);
        apply(&mut fresh, root, b);
        oracle.check(&fresh, root, &oracle.b);
        apply(&mut fresh, root, zero);
        oracle.check(&fresh, root, &vec![0; oracle.paths.len()]);
        data::resource(&format!("{kind}/cold_pair/{name}"), &fresh);
        drop(fresh);
        let id = format!("{kind}/cold_pair/{name}");
        let zero_values = vec![0; oracle.paths.len()];
        let expected = data::digest(&zero_values);
        data::emit(&id, expected);
        data::work(&id, work, 2);
        group.bench_function(BenchmarkId::new("cold_pair", name), |bench| {
            bench.iter_batched_ref(
                api::new,
                |dag| {
                    let root = api::root(dag);
                    apply(dag, root, black_box(b));
                    apply(dag, root, black_box(zero));
                },
                BatchSize::PerIteration,
            );
        });
    }
    for snapshot in [false, true] {
        let mut dag = api::new();
        let root = api::root(&dag);
        apply(&mut dag, root, a);
        apply(&mut dag, root, b);
        apply(&mut dag, root, a);
        let held = snapshot.then(|| api::fork(&mut dag, root));
        let mode = if snapshot {
            "snapshot_pair"
        } else {
            "warm_pair"
        };
        data::resource(&format!("{kind}/{mode}/{name}"), &dag);
        let id = format!("{kind}/{mode}/{name}");
        data::emit(&id, oracle.check(&dag, root, &oracle.a));
        data::work(&id, work, 2);
        group.bench_function(BenchmarkId::new(mode, name), |bench| {
            bench.iter(|| {
                apply(&mut dag, root, black_box(b));
                apply(&mut dag, root, black_box(a));
            });
        });
        if let Some(held) = held {
            oracle.check(&dag, held, &oracle.a);
            api::release(&mut dag, held);
        }
        data::emit_after(&id, oracle.check(&dag, root, &oracle.a));
    }
}

fn cell_oracle(a: &[SetCell], b: &[SetCell]) -> Oracle {
    let a: BTreeMap<_, _> = a
        .iter()
        .map(|cell| (cell.path.value(), cell.data))
        .collect();
    let b: BTreeMap<_, _> = b
        .iter()
        .map(|cell| (cell.path.value(), cell.data))
        .collect();
    let mut codes: Vec<_> = a.keys().chain(b.keys()).copied().collect();
    codes.extend([ROOT_SPAN - 1, ROOT_SPAN - 2]);
    codes.sort_unstable();
    codes.dedup();
    Oracle {
        paths: codes.iter().map(|&code| data::path(code)).collect(),
        a: codes
            .iter()
            .map(|code| a.get(code).copied().unwrap_or(0))
            .collect(),
        b: codes
            .iter()
            .map(|code| b.get(code).copied().unwrap_or(0))
            .collect(),
    }
}

fn cell_case(count: usize, scatter: bool, duplicate: bool) -> Vec<SetCell> {
    let mut cells = Vec::with_capacity(count * if duplicate { 2 } else { 1 });
    for index in 0..count {
        let code = if scatter {
            index as u64 * (ROOT_SPAN / count.max(1) as u64)
        } else {
            index as u64
        };
        let cell = SetCell {
            path: data::path(code),
            data: (index as u32 * 17) % 31 + 1,
        };
        if duplicate {
            cells.push(SetCell {
                data: cell.data ^ 128,
                ..cell
            });
        }
        cells.push(cell);
    }
    cells
}

fn cells(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_cells");
    for (name, count, scatter, duplicate, noop, reverted) in [
        ("empty", 0, false, false, false, false),
        ("one", 1, false, false, false, false),
        ("cluster_128", 128, false, false, false, false),
        ("scatter_128", 128, true, false, false, false),
        ("cluster_4096", 4096, false, false, false, false),
        ("scatter_4096", 4096, true, false, false, false),
        ("duplicates_4096", 2048, false, true, false, false),
        ("scatter_duplicates_4096", 2048, true, true, false, false),
        ("noop_4096", 4096, false, false, true, false),
        ("reverted_input_4096", 2048, false, true, false, true),
    ] {
        let a = cell_case(count, scatter, duplicate);
        let mut b = a.clone();
        if !noop && !reverted {
            for cell in &mut b {
                cell.data ^= 32;
            }
        } else if reverted {
            for cell in b.iter_mut().step_by(2) {
                cell.data ^= 32;
            }
        }
        let zero: Vec<_> = a.iter().map(|cell| SetCell { data: 0, ..*cell }).collect();
        let oracle = cell_oracle(&a, &b);
        group.throughput(Throughput::Elements((a.len() * 2) as u64));
        run_case(
            &mut group,
            "write_cells",
            name,
            a.as_slice(),
            b.as_slice(),
            zero.as_slice(),
            api::set_cells,
            &oracle,
            !noop && !reverted,
            (a.len() * 2) as u64,
        );
    }
    group.finish();
}

fn fill_case(count: usize, lod: u8, scatter: bool, gap: bool) -> Vec<UniformFill> {
    let span = 1_u64 << (3 * lod);
    let stride = if scatter {
        (ROOT_SPAN / count.max(1) as u64 / span) * span
    } else {
        span * if gap { 2 } else { 1 }
    };
    (0..count)
        .map(|index| UniformFill {
            path: data::path(index as u64 * stride),
            lod,
            data: index as u32 % 7 + 1,
        })
        .collect()
}

fn fill_oracle(a: &[UniformFill], b: &[UniformFill]) -> Oracle {
    let mut codes = vec![0, ROOT_SPAN - 1];
    for fill in a {
        let span = 1_u64 << (3 * fill.lod);
        let base = fill.path.value() & !(span - 1);
        codes.extend([base, base + span / 2, base + span - 1]);
        if base > 0 {
            codes.push(base - 1);
        }
        if base + span < ROOT_SPAN {
            codes.push(base + span);
        }
    }
    codes.sort_unstable();
    codes.dedup();
    let at = |fills: &[UniformFill], code| {
        let end = fills.partition_point(|fill| fill.path.value() <= code);
        end.checked_sub(1)
            .and_then(|index| {
                let fill = &fills[index];
                (code < fill.path.value() + (1_u64 << (3 * fill.lod))).then_some(fill.data)
            })
            .unwrap_or(0)
    };
    Oracle {
        paths: codes.iter().map(|&code| data::path(code)).collect(),
        a: codes.iter().map(|&code| at(a, code)).collect(),
        b: codes.iter().map(|&code| at(b, code)).collect(),
    }
}

fn fills(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_fill");
    for (name, count, lod, scatter, gap, noop) in [
        ("empty", 0, 0, false, false, false),
        ("one_lod0", 1, 0, false, false, false),
        ("one_lod1", 1, 1, false, false, false),
        ("adjacent_128_lod3", 128, 3, false, false, false),
        ("gapped_128_lod4", 128, 4, false, true, false),
        ("scatter_128_lod3", 128, 3, true, false, false),
        ("cluster_4096_lod2", 4096, 2, false, false, false),
        ("scatter_4096_lod2", 4096, 2, true, false, false),
        ("one_lod19", 1, 19, false, false, false),
        ("full_root_lod20", 1, 20, false, false, false),
        ("noop_128_lod3", 128, 3, false, false, true),
    ] {
        let a = fill_case(count, lod, scatter, gap);
        let b: Vec<_> = a
            .iter()
            .map(|fill| UniformFill {
                data: if noop { fill.data } else { fill.data ^ 32 },
                ..*fill
            })
            .collect();
        let zero: Vec<_> = a
            .iter()
            .map(|fill| UniformFill { data: 0, ..*fill })
            .collect();
        let oracle = fill_oracle(&a, &b);
        group.throughput(Throughput::Elements((a.len() * 2) as u64));
        run_case(
            &mut group,
            "write_fill",
            name,
            a.as_slice(),
            b.as_slice(),
            zero.as_slice(),
            api::fill_uniform,
            &oracle,
            !noop,
            (a.len() * 2) as u64,
        );
    }
    let a = [
        UniformFill {
            path: data::path(0),
            lod: 1,
            data: 3,
        },
        UniformFill {
            path: data::path(64),
            lod: 2,
            data: 7,
        },
        UniformFill {
            path: data::path(512),
            lod: 3,
            data: 11,
        },
        UniformFill {
            path: data::path(8192),
            lod: 4,
            data: 13,
        },
    ];
    let b = a.map(|fill| UniformFill {
        data: fill.data ^ 32,
        ..fill
    });
    let zero = a.map(|fill| UniformFill { data: 0, ..fill });
    group.throughput(Throughput::Elements(8));
    run_case(
        &mut group,
        "write_fill",
        "mixed_lod_gapped",
        a.as_slice(),
        b.as_slice(),
        zero.as_slice(),
        api::fill_uniform,
        &fill_oracle(&a, &b),
        true,
        8,
    );
    group.finish();
}

fn dense_regions<'a>(specs: &[(u64, u8)], values: &'a [Vec<u32>]) -> Vec<DenseRegion<'a>> {
    specs
        .iter()
        .zip(values)
        .map(|(&(code, lod), values)| DenseRegion {
            path: data::path(code),
            lod,
            values: values.as_slice(),
        })
        .collect()
}

fn dense(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_dense");
    for (name, scene, specs, noop) in [
        ("empty", "empty", vec![], false),
        ("one_lod0", "solid", vec![(0, 0)], false),
        (
            "adjacent_4096_lod0",
            "noise",
            (0..4096).map(|i| (i, 0)).collect(),
            false,
        ),
        (
            "adjacent_4096_lod0_varied",
            "global_noise",
            (0..4096).map(|i| (i, 0)).collect(),
            false,
        ),
        (
            "sparse_boundary_128_lod0",
            "global_noise",
            (0..128).map(|i| (61 + i * 3, 0)).collect(),
            false,
        ),
        (
            "mixed_lod0_regions",
            "global_noise",
            vec![
                (0, 0),
                (7, 0),
                (8, 1),
                (16, 0),
                (63, 0),
                (64, 2),
                (128, 0),
                (129, 0),
                (512, 3),
                (1024, 0),
            ],
            false,
        ),
        (
            "scatter_4096_lod0",
            "noise",
            (0..4096).map(|i| (i * (ROOT_SPAN / 4096), 0)).collect(),
            false,
        ),
        (
            "adjacent_8_lod2",
            "terrain",
            (0..8).map(|i| (i * 64, 2)).collect(),
            false,
        ),
        (
            "gapped_64_lod2",
            "noise",
            (0..64).map(|i| (i * 128, 2)).collect(),
            false,
        ),
        (
            "scatter_64_lod2",
            "noise",
            (0..64).map(|i| (i * (ROOT_SPAN / 64), 2)).collect(),
            false,
        ),
        (
            "mixed_lod_gapped",
            "repeated",
            vec![(0, 1), (64, 2), (512, 3), (8192, 4)],
            false,
        ),
        ("odd_lod5_terrain", "terrain", vec![(0, 5)], false),
        ("odd_lod5_noise", "noise", vec![(0, 5)], false),
        ("lod6_empty", "empty", vec![(0, 6)], false),
        ("lod6_solid", "solid", vec![(0, 6)], false),
        ("lod6_terrain", "terrain", vec![(0, 6)], false),
        ("lod6_checker", "checker", vec![(0, 6)], false),
        ("lod6_repeated", "repeated", vec![(0, 6)], false),
        ("lod6_noise", "noise", vec![(0, 6)], false),
        ("noop_lod6_terrain", "terrain", vec![(0, 6)], true),
    ] {
        let a_values: Vec<_> = specs
            .iter()
            .map(|&(base, lod)| {
                if scene == "global_noise" {
                    (0..1_u64 << (3 * lod))
                        .map(|offset| data::scene_value("noise", base + offset, lod))
                        .collect()
                } else {
                    data::scene_values(scene, lod)
                }
            })
            .collect();
        let b_values: Vec<Vec<_>> = a_values
            .iter()
            .map(|values| {
                values
                    .iter()
                    .map(|value| if noop { *value } else { value ^ 32 })
                    .collect()
            })
            .collect();
        let zeros: Vec<_> = a_values
            .iter()
            .map(|values| vec![0; values.len()])
            .collect();
        let a = dense_regions(&specs, &a_values);
        let b = dense_regions(&specs, &b_values);
        let zero = dense_regions(&specs, &zeros);
        let mut oracle = Oracle {
            paths: Vec::new(),
            a: Vec::new(),
            b: Vec::new(),
        };
        let mut previous_end = 0;
        for (index, &(base, _)) in specs.iter().enumerate() {
            if base > previous_end {
                oracle.paths.push(data::path(base - 1));
                oracle.a.push(0);
                oracle.b.push(0);
            }
            for (offset, (&av, &bv)) in a_values[index].iter().zip(&b_values[index]).enumerate() {
                oracle.paths.push(data::path(base + offset as u64));
                oracle.a.push(av);
                oracle.b.push(bv);
            }
            previous_end = base + a_values[index].len() as u64;
        }
        oracle.paths.push(data::path(ROOT_SPAN - 1));
        oracle.a.push(0);
        oracle.b.push(0);
        let work = (a_values.iter().map(Vec::len).sum::<usize>() * 2) as u64;
        group.throughput(Throughput::Elements(work));
        run_case(
            &mut group,
            "write_dense",
            name,
            a.as_slice(),
            b.as_slice(),
            zero.as_slice(),
            api::set_dense_regions,
            &oracle,
            !noop,
            work,
        );
    }
    group.finish();
}

#[derive(Default)]
struct PrepareScratch {
    indexed: Vec<(usize, SetCell)>,
    ordered: Vec<SetCell>,
}

impl PrepareScratch {
    fn prepare(&mut self, raw: &[SetCell]) {
        self.indexed.clear();
        self.indexed.extend(raw.iter().copied().enumerate());
        self.indexed
            .sort_unstable_by_key(|&(index, cell)| (cell.path.value(), index));
        self.ordered.clear();
        for group in self.indexed.chunk_by(|a, b| a.1.path == b.1.path) {
            self.ordered.push(group.last().unwrap().1);
        }
    }
}

fn prepared_digest(scratch: &PrepareScratch, raw: &[SetCell]) -> u64 {
    let expected: BTreeMap<_, _> = raw
        .iter()
        .map(|cell| (cell.path.value(), cell.data))
        .collect();
    assert_eq!(scratch.ordered.len(), expected.len());
    let mut encoded = Vec::with_capacity(expected.len() * 3);
    for (cell, (&code, &value)) in scratch.ordered.iter().zip(&expected) {
        assert_eq!((cell.path.value(), cell.data), (code, value));
        encoded.extend([code as u32, (code >> 32) as u32, value]);
    }
    data::digest(&encoded)
}

fn prepare(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_prepare");
    for scatter in [false, true] {
        let name = if scatter {
            "scatter_duplicates_4096"
        } else {
            "cluster_duplicates_4096"
        };
        let mut raw_a = cell_case(2048, scatter, true);
        // 固定置换，保留乱序输入原序号；同坐标最后一项由 BTreeMap 独立确认。
        for index in (1..raw_a.len()).rev() {
            let other = (index.wrapping_mul(0x9e37_79b9) ^ (index >> 3)) % (index + 1);
            raw_a.swap(index, other);
        }
        let raw_b: Vec<_> = raw_a
            .iter()
            .map(|cell| SetCell {
                data: cell.data ^ 32,
                ..*cell
            })
            .collect();
        let oracle = cell_oracle(&raw_a, &raw_b);
        let mut scratch = PrepareScratch::default();
        scratch.prepare(&raw_a);
        assert_eq!(scratch.ordered.len(), 2048);
        let mut dag = api::new();
        let root = api::root(&dag);
        api::set_cells(&mut dag, root, &scratch.ordered);
        let digest = oracle.check(&dag, root, &oracle.a);
        assert_eq!(digest, data::digest(&oracle.a));
        drop(dag);
        group.throughput(Throughput::Elements(raw_a.len() as u64));
        let cold_id = format!("write_prepare/cold_one/{name}");
        data::emit(&cold_id, prepared_digest(&scratch, &raw_a));
        data::work(&cold_id, raw_a.len() as u64, 0);
        group.bench_function(BenchmarkId::new("cold_one", name), |bench| {
            bench.iter_batched_ref(
                PrepareScratch::default,
                |scratch| {
                    scratch.prepare(black_box(&raw_a));
                    black_box(&scratch.ordered);
                },
                BatchSize::PerIteration,
            );
        });
        let warm_id = format!("write_prepare/warm_one/{name}");
        data::emit(&warm_id, prepared_digest(&scratch, &raw_a));
        data::work(&warm_id, raw_a.len() as u64, 0);
        group.bench_function(BenchmarkId::new("warm_one", name), |bench| {
            bench.iter(|| {
                scratch.prepare(black_box(&raw_a));
                black_box(&scratch.ordered);
            });
        });
        data::emit_after(&warm_id, prepared_digest(&scratch, &raw_a));
        for snapshot in [false, true] {
            let mut dag = api::new();
            let root = api::root(&dag);
            scratch.prepare(&raw_a);
            api::set_cells(&mut dag, root, &scratch.ordered);
            scratch.prepare(&raw_b);
            api::set_cells(&mut dag, root, &scratch.ordered);
            oracle.check(&dag, root, &oracle.b);
            scratch.prepare(&raw_a);
            api::set_cells(&mut dag, root, &scratch.ordered);
            let held = snapshot.then(|| api::fork(&mut dag, root));
            let mode = if snapshot {
                "prepare_snapshot_pair"
            } else {
                "prepare_warm_pair"
            };
            let id = format!("write_prepare/{mode}/{name}");
            data::emit(&id, oracle.check(&dag, root, &oracle.a));
            data::work(&id, (raw_a.len() * 2) as u64, 2);
            group.throughput(Throughput::Elements((raw_a.len() * 2) as u64));
            group.bench_function(BenchmarkId::new(mode, name), |bench| {
                bench.iter(|| {
                    scratch.prepare(black_box(&raw_b));
                    api::set_cells(&mut dag, root, &scratch.ordered);
                    scratch.prepare(black_box(&raw_a));
                    api::set_cells(&mut dag, root, &scratch.ordered);
                });
            });
            data::emit_after(&id, oracle.check(&dag, root, &oracle.a));
            if let Some(held) = held {
                oracle.check(&dag, held, &oracle.a);
                api::release(&mut dag, held);
            }
        }
    }
    group.finish();
}

fn validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_validation");
    for count in [128, 4096] {
        let mut cells = cell_case(count, false, false);
        cells.last_mut().unwrap().path = data::path(0);
        let mut fills = fill_case(count, 2, false, false);
        fills.last_mut().unwrap().path = fills[count - 2].path;
        let values = data::scene_values("noise", 2);
        let mut dense: Vec<_> = fills
            .iter()
            .enumerate()
            .map(|(index, _)| DenseRegion {
                path: data::path(index as u64 * 64),
                lod: 2,
                values: values.as_slice(),
            })
            .collect();
        dense.last_mut().unwrap().values = &values[..63];
        let mut dag = api::new();
        let root = api::root(&dag);
        api::reject_cells(&mut dag, root, &cells);
        api::reject_fills(&mut dag, root, &fills);
        api::reject_dense(&mut dag, root, &dense);
        assert_eq!(api::value_at(&dag, root, data::path(0)), 0);
        group.throughput(Throughput::Elements(count as u64));
        let cells_id = format!("write_validation/cells_invalid_tail/{count}");
        data::emit(&cells_id, data::digest(&[0]));
        data::work(&cells_id, count as u64, 1);
        group.bench_function(BenchmarkId::new("cells_invalid_tail", count), |bench| {
            bench.iter(|| api::reject_cells(&mut dag, root, black_box(&cells)));
        });
        assert_eq!(api::value_at(&dag, root, data::path(0)), 0);
        data::emit_after(&cells_id, data::digest(&[0]));
        let fill_id = format!("write_validation/fill_invalid_tail/{count}");
        data::emit(&fill_id, data::digest(&[0]));
        data::work(&fill_id, count as u64, 1);
        group.bench_function(BenchmarkId::new("fill_invalid_tail", count), |bench| {
            bench.iter(|| api::reject_fills(&mut dag, root, black_box(&fills)));
        });
        assert_eq!(api::value_at(&dag, root, data::path(0)), 0);
        data::emit_after(&fill_id, data::digest(&[0]));
        let dense_id = format!("write_validation/dense_invalid_tail/{count}");
        data::emit(&dense_id, data::digest(&[0]));
        data::work(&dense_id, count as u64, 1);
        group.bench_function(BenchmarkId::new("dense_invalid_tail", count), |bench| {
            bench.iter(|| api::reject_dense(&mut dag, root, black_box(&dense)));
        });
        assert_eq!(api::value_at(&dag, root, data::path(0)), 0);
        data::emit_after(&dense_id, data::digest(&[0]));
    }
    group.finish();
}

pub fn benches(c: &mut Criterion) {
    cells(c);
    fills(c);
    dense(c);
    prepare(c);
    validation(c);
}
