use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::{api, data};
use api::{HitBehavior, Ray, RaySample, RaycastResult, UniformFill};

#[path = "ray_controls.rs"]
mod controls;

fn behavior(policy: &str, sample: RaySample) -> HitBehavior {
    match policy {
        "hit" => {
            if sample.value == 0 {
                HitBehavior::Pass
            } else {
                HitBehavior::Hit
            }
        }
        "pass" => HitBehavior::Pass,
        "block" => HitBehavior::Block,
        _ => unreachable!(),
    }
}

fn rays(origin: &str, direction: &str, count: usize) -> Vec<Ray> {
    (0..count)
        .map(|index| {
            let offset = (index % 32) as f64;
            let origin = match (origin, direction) {
                ("inside", "tie") => [8.0 + offset; 3],
                ("outside", "tie") => [-2.0; 3],
                ("inside", _) => [8.5, 7.5 + offset, 9.5 + offset / 2.0],
                ("outside", _) => [-2.0, 7.5 + offset, 9.5 + offset / 2.0],
                _ => unreachable!(),
            };
            let dir = match direction {
                "axis" => [1.0, 0.0, 0.0],
                "diag" => [1.0, 0.41, 0.73],
                "tie" => [1.0; 3],
                "thin" => [1.0, 1e-10, -1e-12],
                "away" => [-1.0, 0.0, 0.0],
                _ => unreachable!(),
            };
            Ray {
                origin,
                dir,
                max_t: 128.0,
            }
        })
        .collect()
}

fn sample_digest(hash: &mut u64, index: usize, sample: RaySample) {
    for value in [
        index as u32,
        sample.voxel.0,
        sample.voxel.1,
        sample.voxel.2,
        sample.value,
        u32::from(sample.lod),
        sample.step_count,
    ] {
        *hash = (*hash ^ u64::from(value)).wrapping_mul(0x100_0000_01b3);
    }
}

// Debug 包含 t、进入面和终止原因；只在 preflight 序列化，计时循环不格式化。
fn results_digest(output: &[RaycastResult]) -> u64 {
    output
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |mut hash, result| {
            for byte in format!("{result:?}").bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
            }
            hash
        })
}

pub fn benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("rays");
    for scene in ["empty", "uniform", "terrain", "checker", "noise"] {
        let mut dag = if scene == "uniform" {
            api::new()
        } else {
            data::build_scene(scene, 6)
        };
        let root = api::root(&dag);
        if scene == "uniform" {
            api::fill_uniform(
                &mut dag,
                root,
                &[UniformFill {
                    path: data::path(0),
                    lod: 20,
                    data: 7,
                }],
            );
        }
        // 每个维度有代表性覆盖，避免把所有场景、方向、策略和规模做笛卡尔积。
        for (origin, direction, policy, count) in [
            ("inside", "axis", "hit", 1),
            ("outside", "axis", "hit", 1),
            ("outside", "diag", "hit", 16),
            ("inside", "tie", "hit", 16),
            ("outside", "tie", "hit", 16),
            ("outside", "thin", "hit", 16),
            ("outside", "away", "hit", 16),
            ("inside", "axis", "pass", 16),
            ("outside", "diag", "block", 16),
            ("outside", "axis", "hit", 4096),
            ("inside", "diag", "pass", 4096),
        ] {
            let rays = rays(origin, direction, count);
            let seed = api::raycast(&dag, root, rays[0], |_| HitBehavior::Pass);
            let mut output = vec![seed; count];
            let mut batch_hash = 0xcbf2_9ce4_8422_2325;
            let mut callback_count = 0_u64;
            let mut previous_index = 0;
            api::raycast_batch(&dag, root, &rays, &mut output, |index, sample| {
                assert!(index >= previous_index);
                previous_index = index;
                sample_digest(&mut batch_hash, index, sample);
                callback_count += 1;
                behavior(policy, sample)
            });
            let mut single_hash = 0xcbf2_9ce4_8422_2325;
            for (index, (&ray, expected)) in rays.iter().zip(&output).enumerate() {
                let actual = api::raycast(&dag, root, ray, |sample| {
                    sample_digest(&mut single_hash, index, sample);
                    behavior(policy, sample)
                });
                assert_eq!(actual, *expected);
                assert!(!actual.is_anomalous());
                if scene == "uniform" && policy == "hit" && direction != "away" {
                    assert!(actual.is_hit());
                }
                if scene == "empty" && policy == "hit" {
                    assert!(!actual.is_hit());
                }
                if policy == "pass" {
                    assert!(!actual.is_hit());
                }
            }
            assert_eq!(single_hash, batch_hash);
            let name = format!("{scene}/{origin}/{direction}/{policy}/{count}");
            data::emit(
                &format!("rays/batch/{name}"),
                results_digest(&output) ^ batch_hash.rotate_left(11) ^ callback_count,
            );
            println!("SVDAG_RAY_CALLBACKS\t{name}\t{callback_count}");
            data::work(&format!("rays/batch/{name}"), count as u64, 1);
            group.throughput(Throughput::Elements(count as u64));
            group.bench_function(BenchmarkId::new("batch", &name), |b| {
                b.iter(|| {
                    api::raycast_batch(
                        black_box(&dag),
                        root,
                        black_box(&rays),
                        black_box(&mut output),
                        |index, sample| {
                            black_box(index);
                            behavior(policy, sample)
                        },
                    );
                    black_box(&output);
                });
            });
            let mut after_hash = 0xcbf2_9ce4_8422_2325;
            let mut after_count = 0;
            api::raycast_batch(&dag, root, &rays, &mut output, |index, sample| {
                sample_digest(&mut after_hash, index, sample);
                after_count += 1;
                behavior(policy, sample)
            });
            assert_eq!((after_hash, after_count), (batch_hash, callback_count));
            data::emit_after(
                &format!("rays/batch/{name}"),
                results_digest(&output) ^ after_hash.rotate_left(11) ^ after_count,
            );
            if count == 1 {
                data::emit(&format!("rays/single/{name}"), results_digest(&output));
                data::work(&format!("rays/single/{name}"), 1, 1);
                group.throughput(Throughput::Elements(1));
                group.bench_function(BenchmarkId::new("single", &name), |b| {
                    b.iter(|| {
                        black_box(api::raycast(
                            black_box(&dag),
                            root,
                            black_box(rays[0]),
                            |sample| behavior(policy, sample),
                        ))
                    });
                });
                assert_eq!(
                    api::raycast(&dag, root, rays[0], |sample| behavior(policy, sample)),
                    output[0]
                );
                data::emit_after(&format!("rays/single/{name}"), results_digest(&output));
            }
        }
        let mut empty = [];
        api::raycast_batch(&dag, root, &[], &mut empty, |_, _| panic!("空批不能回调"));
        data::emit(&format!("rays/empty/{scene}"), 0);
        data::work(&format!("rays/empty/{scene}"), 0, 1);
        group.throughput(Throughput::Elements(0));
        group.bench_function(BenchmarkId::new("empty", scene), |b| {
            b.iter(|| {
                api::raycast_batch(
                    black_box(&dag),
                    root,
                    black_box(&[]),
                    black_box(&mut empty),
                    |_, _| HitBehavior::Pass,
                )
            });
        });
        data::emit_after(&format!("rays/empty/{scene}"), 0);
    }
    group.finish();
    controls::benches(c);
}
