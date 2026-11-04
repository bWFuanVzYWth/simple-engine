//! 调用方连续输入与独占输出上的批转换，覆盖传递函数、原色矩阵及显示量化。
//!
//! 输入、类型包装与输出容量在计时外准备；计时包含逐项转换、写输出和一次整批
//! black_box 消费，不在每个颜色后累加摘要或分配。缓冲复用，冷分配与线程调度不在本核内。
//! `unit` 覆盖标称范围，`extended` 包含负值和 HDR；异常量化另测，不把所有输入混为一组。

use std::hint::black_box;

use color::{LinearRec2020Rgba, LinearSrgbRgba, Srgb8Rgba, SrgbRgba};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

fn input_bytes(count: usize) -> Vec<Srgb8Rgba> {
    (0..count)
        .map(|i| {
            Srgb8Rgba::new(
                (i * 17) as u8,
                (i * 79 + 37) as u8,
                (i * 149 + 251) as u8,
                (i * 43 + 127) as u8,
            )
        })
        .collect()
}

fn input_float(count: usize, extended: bool) -> Vec<[f32; 4]> {
    (0..count)
        .map(|i| {
            std::array::from_fn(|axis| {
                let value = ((i * (79 + axis * 38) + axis * 337) % 4096) as f32 / 4095.0;
                if extended { value * 4.25 - 0.25 } else { value }
            })
        })
        .collect()
}

fn convert<I: Copy, O>(input: &[I], output: &mut [O], transform: impl Fn(I) -> O) {
    for (&value, slot) in input.iter().zip(output.iter_mut()) {
        *slot = transform(value);
    }
    black_box(output);
}

fn batch<I: Copy, O: Copy>(
    c: &mut Criterion,
    name: &str,
    prepare: impl Fn(usize) -> Vec<I>,
    initial: O,
    transform: impl Fn(I) -> O + Copy,
) {
    let mut group = c.benchmark_group(format!("color/{name}"));
    for count in [1, 64, 4096] {
        let input = prepare(count);
        let mut output = vec![initial; count];
        convert(&input, &mut output, transform);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(BenchmarkId::from_parameter(count), |b| {
            b.iter(|| convert(black_box(&input), black_box(&mut output), transform));
        });
    }
    group.finish();
}

fn srgb_input(count: usize, extended: bool) -> Vec<SrgbRgba> {
    input_float(count, extended)
        .into_iter()
        .map(|[r, g, b, a]| SrgbRgba::new(r, g, b, a))
        .collect()
}

fn linear_input(count: usize, extended: bool) -> Vec<LinearSrgbRgba> {
    input_float(count, extended)
        .into_iter()
        .map(|[r, g, b, a]| LinearSrgbRgba::new(r, g, b, a))
        .collect()
}

fn rec2020_input(count: usize, extended: bool) -> Vec<LinearRec2020Rgba> {
    input_float(count, extended)
        .into_iter()
        .map(|[r, g, b, a]| LinearRec2020Rgba::new(r, g, b, a))
        .collect()
}

fn exceptional_input(count: usize) -> Vec<SrgbRgba> {
    (0..count)
        .map(|i| {
            let values = [
                f32::NAN,
                f32::NEG_INFINITY,
                -0.5,
                0.25,
                0.5,
                1.5,
                f32::INFINITY,
            ];
            SrgbRgba::new(
                values[i % values.len()],
                values[(i + 1) % values.len()],
                values[(i + 2) % values.len()],
                values[(i + 3) % values.len()],
            )
        })
        .collect()
}

fn conversions(c: &mut Criterion) {
    batch(
        c,
        "byte_to_linear_srgb",
        input_bytes,
        LinearSrgbRgba::new(0.0, 0.0, 0.0, 0.0),
        Srgb8Rgba::to_linear_srgb,
    );
    batch(
        c,
        "byte_to_rec2020",
        input_bytes,
        LinearRec2020Rgba::new(0.0, 0.0, 0.0, 0.0),
        Srgb8Rgba::to_linear_rec2020,
    );
    for (kind, extended) in [("unit", false), ("extended", true)] {
        batch(
            c,
            &format!("{kind}/decode_srgb"),
            |count| srgb_input(count, extended),
            LinearSrgbRgba::new(0.0, 0.0, 0.0, 0.0),
            SrgbRgba::to_linear_srgb,
        );
        batch(
            c,
            &format!("{kind}/encode_srgb"),
            |count| linear_input(count, extended),
            SrgbRgba::new(0.0, 0.0, 0.0, 0.0),
            LinearSrgbRgba::to_srgb,
        );
        batch(
            c,
            &format!("{kind}/linear_srgb_to_rec2020"),
            |count| linear_input(count, extended),
            LinearRec2020Rgba::new(0.0, 0.0, 0.0, 0.0),
            LinearSrgbRgba::to_linear_rec2020,
        );
        batch(
            c,
            &format!("{kind}/rec2020_to_linear_srgb"),
            |count| rec2020_input(count, extended),
            LinearSrgbRgba::new(0.0, 0.0, 0.0, 0.0),
            LinearRec2020Rgba::to_linear_srgb,
        );
        batch(
            c,
            &format!("{kind}/quantize_srgb"),
            |count| srgb_input(count, extended),
            Srgb8Rgba::new(0, 0, 0, 0),
            SrgbRgba::to_srgb8,
        );
        batch(
            c,
            &format!("{kind}/display_rec2020"),
            |count| rec2020_input(count, extended),
            Srgb8Rgba::new(0, 0, 0, 0),
            LinearRec2020Rgba::to_srgb8,
        );
    }
    batch(
        c,
        "exceptional/quantize_srgb",
        exceptional_input,
        Srgb8Rgba::new(0, 0, 0, 0),
        SrgbRgba::to_srgb8,
    );
}

criterion_group!(benches, conversions);
criterion_main!(benches);
