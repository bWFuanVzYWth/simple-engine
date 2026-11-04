use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::{api, data};
use api::{Dag, HitBehavior, Ray, RaySample, RaycastResult, SetCell, UniformFill};

use super::{results_digest, sample_digest};

const fn target_hit(sample: RaySample) -> HitBehavior {
    if sample.value == 99 {
        HitBehavior::Hit
    } else {
        HitBehavior::Pass
    }
}

fn fixture(name: &str) -> (Dag, Ray) {
    let mut dag = api::new();
    let root = api::root(&dag);
    let mut input = Ray {
        origin: [0.5; 3],
        dir: [1.0, 0.0, 0.0],
        max_t: 2048.0,
    };
    match name {
        "invalid" => input.dir = [0.0; 3],
        "initial_hit" => api::fill_uniform(
            &mut dag,
            root,
            &[UniformFill {
                path: data::path(0),
                lod: 20,
                data: 99,
            }],
        ),
        "empty_miss" => {}
        "dense_line_target" => {
            let cells: Vec<_> = (0..512)
                .map(|x| SetCell {
                    path: api::Path::encode(x, 4, 4).unwrap(),
                    data: if x == 511 { 99 } else { 7 },
                })
                .collect();
            api::set_cells(&mut dag, root, &cells);
            input.origin = [-1.5, 4.5, 4.5];
        }
        _ => unreachable!(),
    }
    (dag, input)
}

fn validate(name: &str, result: RaycastResult) {
    match name {
        "invalid" => assert!(matches!(result, RaycastResult::InvalidInput { .. })),
        "initial_hit" => {
            let hit = result.as_hit().expect("初始命中");
            assert!(hit.is_initial());
            assert_eq!((hit.voxel, hit.value, hit.t), ((0, 0, 0), 99, 0.0));
        }
        "empty_miss" => assert!(matches!(result, RaycastResult::Miss { .. })),
        "dense_line_target" => {
            let hit = result.as_hit().expect("扫描到末端 target");
            assert_eq!(
                (hit.voxel, hit.prev_empty, hit.value, hit.lod, hit.t),
                ((511, 4, 4), (510, 4, 4), 99, 0, 512.5)
            );
        }
        _ => unreachable!(),
    }
}

fn digest(output: &[RaycastResult], samples: u64, callbacks: u64) -> u64 {
    results_digest(output) ^ samples.rotate_left(11) ^ callbacks
}

// 同一射线与静态 predicate，单条和各批大小只改变入口及重复次数。
// 这里省去主场景组的字符串策略分派与 callback black_box，分离固定开销和逐叶成本。
pub(super) fn benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("rays_control");
    for name in ["invalid", "initial_hit", "empty_miss", "dense_line_target"] {
        let (dag, input) = fixture(name);
        let root = api::root(&dag);
        let mut sample_hash = 0xcbf2_9ce4_8422_2325;
        let mut callbacks = 0_u64;
        let expected = api::raycast(&dag, root, input, |sample| {
            sample_digest(&mut sample_hash, 0, sample);
            callbacks += 1;
            target_hit(sample)
        });
        validate(name, expected);
        assert_eq!(
            callbacks,
            match name {
                "invalid" => 0,
                "initial_hit" | "empty_miss" => 1,
                "dense_line_target" => 512,
                _ => unreachable!(),
            }
        );
        let single_name = format!("rays_control/single/{name}");
        let before = digest(&[expected], sample_hash, callbacks);
        data::emit(&single_name, before);
        data::work(&single_name, 1, 1);
        println!("SVDAG_RAY_CALLBACKS\t{single_name}\t{callbacks}");
        group.throughput(Throughput::Elements(1));
        group.bench_function(BenchmarkId::new("single", name), |b| {
            b.iter(|| {
                black_box(api::raycast(
                    black_box(&dag),
                    root,
                    black_box(input),
                    target_hit,
                ))
            });
        });
        let mut after_hash = 0xcbf2_9ce4_8422_2325;
        let mut after_callbacks = 0;
        let after = api::raycast(&dag, root, input, |sample| {
            sample_digest(&mut after_hash, 0, sample);
            after_callbacks += 1;
            target_hit(sample)
        });
        assert_eq!(after, expected);
        assert_eq!((after_hash, after_callbacks), (sample_hash, callbacks));
        data::emit_after(&single_name, digest(&[after], after_hash, after_callbacks));
        let sentinel = api::raycast(
            &dag,
            root,
            Ray {
                origin: [0.5; 3],
                dir: [1.0, 0.0, 0.0],
                max_t: 0.0,
            },
            |_| HitBehavior::Block,
        );
        assert_ne!(sentinel, expected);
        for count in [1, 16, 4096] {
            let rays = vec![input; count];
            let mut output = vec![sentinel; count];
            let mut batch_hash = 0xcbf2_9ce4_8422_2325;
            let mut batch_callbacks = 0;
            api::raycast_batch(&dag, root, &rays, &mut output, |index, sample| {
                sample_digest(&mut batch_hash, index, sample);
                batch_callbacks += 1;
                target_hit(sample)
            });
            assert!(output.iter().all(|&result| result == expected));
            assert_eq!(batch_callbacks, callbacks * count as u64);
            let workload = format!("rays_control/batch/{name}/{count}");
            let before = digest(&output, batch_hash, batch_callbacks);
            data::emit(&workload, before);
            data::work(&workload, count as u64, 1);
            println!("SVDAG_RAY_CALLBACKS\t{workload}\t{batch_callbacks}");
            group.throughput(Throughput::Elements(count as u64));
            group.bench_function(BenchmarkId::new("batch", format!("{name}/{count}")), |b| {
                b.iter(|| {
                    api::raycast_batch(
                        black_box(&dag),
                        root,
                        black_box(&rays),
                        black_box(&mut output),
                        |_, sample| target_hit(sample),
                    );
                    black_box(&output);
                });
            });
            let mut after_hash = 0xcbf2_9ce4_8422_2325;
            let mut after_callbacks = 0;
            api::raycast_batch(&dag, root, &rays, &mut output, |index, sample| {
                sample_digest(&mut after_hash, index, sample);
                after_callbacks += 1;
                target_hit(sample)
            });
            assert!(output.iter().all(|&result| result == expected));
            assert_eq!((after_hash, after_callbacks), (batch_hash, batch_callbacks));
            data::emit_after(&workload, digest(&output, after_hash, after_callbacks));
        }
    }
    group.finish();
}
