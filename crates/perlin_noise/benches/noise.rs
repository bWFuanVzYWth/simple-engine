//! 纯噪声与梯度分解核基准；输入预生成，输出复用。
//! batch 包含完整校验与尾部处理，已给梯度采样不含梯度准备，该成本由 gradient 项单列。
//! 坐标覆盖负侧地形网格、整数邻域和散布样本；结果摘要在计时外打印。

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use perlin_noise::{CornerGradients, MAX_COORD, PerlinNoise};

const SEED: (u64, u64) = (0x7d2f_c719_0f0a_4135, 0x9e37_79b9_7f4a_7c15);

fn coords(scene: &str, count: usize) -> [Vec<f32>; 3] {
    std::array::from_fn(|axis| {
        (0..count)
            .map(|index| match scene {
                "terrain" => {
                    let position = (index >> (axis * 6)) & 63;
                    if axis == 0 {
                        let base = (position & !7) as f32 - 64.0;
                        base * (1.0 / 24.0) + (position & 7) as f32 * (1.0 / 24.0)
                    } else {
                        (position as f32 - 64.0) * (1.0 / 24.0)
                    }
                }
                "integer_near" => {
                    let value = ((index * 79 + axis * 37) % 256) as f32 - 128.0;
                    match index % 3 {
                        0 => value,
                        1 => value.next_up(),
                        _ => value.next_down(),
                    }
                }
                _ => {
                    let bits = (index as u32).wrapping_mul(7_919 + axis as u32 * 1_009)
                        ^ (axis as u32).wrapping_mul(0x9e37);
                    (bits & 0xffff) as f32 * (1.0 / 128.0) - 256.0
                }
            })
            .collect()
    })
}

fn fixed_coords(scene: &str) -> [[f32; 8]; 3] {
    coords(scene, 8).map(|values| std::array::from_fn(|lane| values[lane]))
}

fn given_gradients(noise: &PerlinNoise, [x, y, z]: [[f32; 8]; 3]) -> CornerGradients {
    let bx = x.map(|v| v.floor() as i32);
    let by = y.map(|v| v.floor() as i32);
    let bz = z.map(|v| v.floor() as i32);
    std::array::from_fn(|corner| {
        noise.gradient_x8(
            bx.map(|v| v + (corner & 1) as i32),
            by.map(|v| v + ((corner >> 1) & 1) as i32),
            bz.map(|v| v + ((corner >> 2) & 1) as i32),
        )
    })
}

fn digest(values: &[f32]) -> u64 {
    values.iter().fold(0xcbf2_9ce4_8422_2325, |hash, value| {
        (hash ^ u64::from(value.to_bits())).wrapping_mul(0x100_0000_01b3)
    })
}

fn bench_noise(c: &mut Criterion) {
    let noise = PerlinNoise::new(SEED);
    c.bench_function("perlin/new", |b| {
        b.iter(|| black_box(PerlinNoise::new(black_box(SEED))))
    });

    let mut fixed = c.benchmark_group("perlin/x8");
    fixed.throughput(Throughput::Elements(8));
    for scene in ["random", "terrain", "integer_near"] {
        let [x, y, z] = fixed_coords(scene);
        let expected = noise.noise_3d_x8(x, y, z);
        println!("NOISE_ORACLE\tx8/{scene}\t{}", digest(&expected));
        fixed.bench_with_input(BenchmarkId::new("direct", scene), &(x, y, z), |b, input| {
            b.iter(|| {
                let (x, y, z) = black_box(*input);
                black_box(noise.noise_3d_x8(x, y, z))
            });
        });

        let gradients = given_gradients(&noise, [x, y, z]);
        let relative = [x, y, z].map(|axis| axis.map(|v| v - v.floor()));
        let [rx, ry, rz] = relative;
        assert_eq!(
            PerlinNoise::sample_gradients_x8(rx, ry, rz, &gradients).map(f32::to_bits),
            expected.map(f32::to_bits)
        );
        fixed.bench_with_input(
            BenchmarkId::new("supplied_gradients", scene),
            &(relative, gradients),
            |b, input| {
                b.iter(|| {
                    let (relative, gradients) = black_box(input);
                    let [x, y, z] = *relative;
                    black_box(PerlinNoise::sample_gradients_x8(x, y, z, gradients))
                });
            },
        );
    }
    let gx = [i32::MIN, -76, -2, -1, 0, 1, 79, i32::MAX];
    fixed.bench_function("gradient", |b| {
        b.iter(|| black_box(noise.gradient_x8(black_box(gx), black_box(gx), black_box(gx))))
    });
    fixed.finish();

    let mut batch = c.benchmark_group("perlin/batch");
    for scene in ["random", "terrain", "integer_near"] {
        for count in [0, 1, 7, 8, 9, 64, 4096] {
            let [x, y, z] = coords(scene, count);
            let mut output = vec![0.0; count];
            noise.sample_3d_batch(&x, &y, &z, &mut output).unwrap();
            println!("NOISE_ORACLE\tbatch/{scene}/{count}\t{}", digest(&output));
            batch.throughput(Throughput::Elements(count as u64));
            batch.bench_function(BenchmarkId::new(scene, count), |b| {
                b.iter(|| {
                    noise
                        .sample_3d_batch(
                            black_box(&x),
                            black_box(&y),
                            black_box(&z),
                            black_box(&mut output),
                        )
                        .unwrap();
                    black_box(&output);
                });
            });
        }
    }
    batch.finish();

    let [x, mut y, z] = coords("terrain", 4096);
    y[4095] = MAX_COORD.next_up();
    let mut output = vec![0.0; 4096];
    c.bench_function("perlin/reject/invalid_tail", |b| {
        b.iter(|| {
            black_box(noise.sample_3d_batch(
                black_box(&x),
                black_box(&y),
                black_box(&z),
                black_box(&mut output),
            ))
        });
    });
    c.bench_function("perlin/reject/length", |b| {
        b.iter(|| {
            black_box(noise.sample_3d_batch(
                black_box(&x),
                black_box(&y[..1]),
                black_box(&z),
                black_box(&mut output),
            ))
        });
    });
}

criterion_group!(benches, bench_noise);
criterion_main!(benches);
