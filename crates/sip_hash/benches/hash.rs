//! SipHash-1-3 / 2-4 的消息、标量坐标和八路入口基准。
//! 输入预生成；通用字节消息覆盖尾块边界，固定坐标组包含 8192 条消息。
//! seed 初始化另测，消息分配与摘要输入准备不计入 hash 时间。

#![feature(portable_simd)]

use std::{hint::black_box, simd::u32x8};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use sip_hash::SipHash;

const BATCH_COUNT: usize = 1024;
const LANES: usize = 8;
const SAMPLE_COUNT: usize = BATCH_COUNT * LANES;

fn coord(i: usize, axis: u32) -> u32 {
    let i = u32::try_from(i).unwrap_or(0);
    i.wrapping_mul(7_919 + axis * 1_009) ^ axis.wrapping_mul(0x9E37)
}

const fn make_12byte_msg(a: u32, b: u32, c: u32) -> [u8; 12] {
    let [a0, a1, a2, a3] = a.to_le_bytes();
    let [b0, b1, b2, b3] = b.to_le_bytes();
    let [c0, c1, c2, c3] = c.to_le_bytes();
    [a0, a1, a2, a3, b0, b1, b2, b3, c0, c1, c2, c3]
}

fn scalar_coords() -> Vec<(u32, u32, u32)> {
    (0..SAMPLE_COUNT)
        .map(|i| (coord(i, 0), coord(i, 1), coord(i, 2)))
        .collect()
}

fn scalar_msgs() -> Vec<[u8; 12]> {
    scalar_coords()
        .into_iter()
        .map(|(a, b, c)| make_12byte_msg(a, b, c))
        .collect()
}

fn coord_lanes(batch: usize, axis: u32) -> [u32; LANES] {
    std::array::from_fn(|lane| coord(batch * LANES + lane, axis))
}

fn simd_coords() -> Vec<([u32; LANES], [u32; LANES], [u32; LANES])> {
    (0..BATCH_COUNT)
        .map(|batch| {
            (
                coord_lanes(batch, 0),
                coord_lanes(batch, 1),
                coord_lanes(batch, 2),
            )
        })
        .collect()
}

fn bench_profile<const C: usize, const D: usize>(criterion: &mut Criterion) {
    let hasher = SipHash::<C, D>::new((0x0123_4567_89AB_CDEF, 0xFEDC_BA98_7654_3210));
    let scalar_coords = scalar_coords();
    let scalar_msgs = scalar_msgs();
    let simd_coords = simd_coords();

    criterion
        .benchmark_group(format!("sip_hash_{C}_{D}"))
        .throughput(Throughput::Elements(SAMPLE_COUNT as u64))
        .bench_function("hash_12byte_slice", |bencher| {
            bencher.iter(|| {
                let sum = scalar_msgs.iter().fold(0u64, |sum, msg| {
                    sum ^ black_box(&hasher).hash(black_box(msg))
                });
                black_box(sum);
            });
        })
        .bench_function("hash_u32x3_scalar", |bencher| {
            bencher.iter(|| {
                let sum = scalar_coords.iter().fold(0u64, |sum, &(x, y, z)| {
                    sum ^ black_box(&hasher).hash_u32x3(black_box(x), black_box(y), black_box(z))
                });
                black_box(sum);
            });
        })
        .bench_function("hash_u32x3_array_x8", |bencher| {
            bencher.iter(|| {
                let sum = simd_coords.iter().fold(0u64, |sum, &(x, y, z)| {
                    black_box(&hasher)
                        .hash_u32x3_x8(black_box(x), black_box(y), black_box(z))
                        .into_iter()
                        .fold(sum, |sum, value| sum ^ value)
                });
                black_box(sum);
            });
        })
        .bench_function("hash_u32x3_simd_x8", |bencher| {
            bencher.iter(|| {
                let sum = simd_coords.iter().fold(0u64, |sum, &(x, y, z)| {
                    let hash = black_box(&hasher).hash_u32x3_simd(
                        black_box(u32x8::from_array(x)),
                        black_box(u32x8::from_array(y)),
                        black_box(u32x8::from_array(z)),
                    );
                    hash.to_array()
                        .into_iter()
                        .fold(sum, |sum, value| sum ^ value)
                });
                black_box(sum);
            });
        });

    let mut messages = criterion.benchmark_group(format!("sip_hash_bytes_{C}_{D}"));
    for count in [0, 1, 7, 8, 9, 12, 64, 4096] {
        let message: Vec<u8> = (0..count).map(|i| (i * 79) as u8).collect();
        messages.throughput(Throughput::Bytes(count as u64));
        messages.bench_with_input(BenchmarkId::from_parameter(count), &message, |b, msg| {
            b.iter(|| black_box(hasher.hash(black_box(msg))));
        });
    }
    messages.finish();
    criterion.bench_function(&format!("sip_hash_{C}_{D}/new"), |b| {
        b.iter(|| black_box(SipHash::<C, D>::new(black_box((7, 11)))))
    });
}

fn bench_hash(criterion: &mut Criterion) {
    bench_profile::<1, 3>(criterion);
    bench_profile::<2, 4>(criterion);
}

criterion_group!(benches, bench_hash);
criterion_main!(benches);
