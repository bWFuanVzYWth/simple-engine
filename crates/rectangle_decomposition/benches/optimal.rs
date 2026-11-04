//! 同一像素区域上的最少分解及 AoS/SoA 输出完整消费。
//!
//! 连续输入与 sparse leaves 均在计时外生成；输入准备另列独立组。热路径复用同一
//! scratch 与输出，包含调用和面积/标签消费。内存日志列固定 scratch、输入和输出，
//! 不含 Criterion、allocator、线程栈临时值或应用并发；所有方案均消费完整几何与标签。
//! worker 内对象与 4096 个 leaf 槽按固定字段序存放，整体经过 black_box 后再分借用。
//! leaves Vec 在计时外生成，复制有效前缀后释放；日志区分固定保留字节与有效输入字节。

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rectangle_decomposition::AREA;
use rectangle_decomposition::optimal::{
    DenseLabels64, PackedRectangles64, QuadLeaf64, Rectangle, SparseOptimalScratch64,
};

#[repr(C)]
struct State {
    scratch: SparseOptimalScratch64,
    packed: PackedRectangles64,
    labels: DenseLabels64,
    leaves: [QuadLeaf64; AREA],
}

fn labels(kind: &str) -> DenseLabels64 {
    let mut labels = DenseLabels64::new();
    let mut random = 42u32;
    for (y, row) in labels.rows_mut().iter_mut().enumerate() {
        for (x, value) in row.iter_mut().enumerate() {
            random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *value = match kind {
                "uniform" | "coarse_uniform" => 7,
                "stripes" => (x / 8 + 1) as u16,
                "t_shapes" if y % 4 == 0 && x % 4 == 1 || y % 4 == 1 && x % 4 < 3 => 7,
                "holes" if x % 2 != 0 || y % 2 != 0 => 1,
                "random_holes" => u16::from((random >> 16) & 7 != 0),
                "binary_noise" => ((random >> 16) & 1) as u16,
                "checker" => (1 + (x + y) % 2) as u16,
                "noise" => ((random >> 16) % 5) as u16,
                "dyadic" => (1 + x / 16 + 4 * (y / 16)) as u16,
                _ => 0,
            };
        }
    }
    labels
}

fn leaves(labels: &DenseLabels64, side: u8) -> Vec<QuadLeaf64> {
    let mut leaves = Vec::new();
    for v in (0..64u8).step_by(usize::from(side)) {
        for u in (0..64u8).step_by(usize::from(side)) {
            if let Some(value) =
                std::num::NonZeroU16::new(labels.rows()[usize::from(v)][usize::from(u)])
            {
                leaves.push(QuadLeaf64 {
                    u,
                    v,
                    lod: side.ilog2() as u8,
                    value,
                });
            }
        }
    }
    leaves
}

fn consume(rectangles: impl Iterator<Item = Rectangle>) -> u64 {
    rectangles
        .map(|rectangle| {
            u64::from(rectangle.x.end - rectangle.x.start)
                * u64::from(rectangle.y.end - rectangle.y.start)
                * u64::from(rectangle.value)
        })
        .sum()
}

fn consume_packed(rectangles: &PackedRectangles64) -> u64 {
    rectangles
        .bounds()
        .iter()
        .zip(rectangles.labels())
        .map(|(&bounds, &value)| {
            let [x0, x1, y0, y1] = bounds.to_le_bytes();
            u64::from(x1 - x0) * u64::from(y1 - y0) * u64::from(value)
        })
        .sum()
}

fn verify(rectangles: &[Rectangle], labels: &DenseLabels64) {
    let mut covered = [[0; 64]; 64];
    for rectangle in rectangles {
        assert_ne!(rectangle.value, 0);
        assert!(rectangle.x.start < rectangle.x.end && rectangle.x.end <= 64);
        assert!(rectangle.y.start < rectangle.y.end && rectangle.y.end <= 64);
        for row in &mut covered[usize::from(rectangle.y.start)..usize::from(rectangle.y.end)] {
            for pixel in &mut row[usize::from(rectangle.x.start)..usize::from(rectangle.x.end)] {
                assert_eq!(*pixel, 0);
                *pixel = rectangle.value;
            }
        }
    }
    assert_eq!(covered, *labels.rows());
}

fn benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("rectangle/optimal");
    group.throughput(Throughput::Elements(AREA as u64));
    let mut state = State {
        scratch: SparseOptimalScratch64::new(),
        packed: PackedRectangles64::new(),
        labels: DenseLabels64::new(),
        leaves: [QuadLeaf64 {
            u: 0,
            v: 0,
            lod: 0,
            value: std::num::NonZeroU16::MIN,
        }; AREA],
    };
    for kind in [
        "uniform",
        "coarse_uniform",
        "stripes",
        "t_shapes",
        "holes",
        "random_holes",
        "binary_noise",
        "checker",
        "noise",
        "dyadic",
    ] {
        state.labels = labels(kind);
        let input = leaves(
            &state.labels,
            match kind {
                "coarse_uniform" => 64,
                "dyadic" => 16,
                _ => 1,
            },
        );
        let input_len = input.len();
        state.leaves[..input_len].copy_from_slice(&input);
        drop(input);
        let State {
            scratch,
            packed,
            labels,
            leaves,
        } = black_box(&mut state);
        let input = &leaves[..input_len];
        let expected = labels
            .rows()
            .as_flattened()
            .iter()
            .copied()
            .map(u64::from)
            .sum::<u64>();
        let aos = scratch.decompose_labels_borrowed(labels).unwrap();
        verify(aos, labels);
        assert_eq!(consume(aos.iter().copied()), expected);
        let minimum = aos.len();
        let borrowed_active_bytes = size_of_val(aos);
        assert_eq!(
            scratch.decompose_labels_packed(labels, packed).unwrap(),
            minimum
        );
        assert_eq!(consume_packed(packed), expected);
        assert_eq!(scratch.decompose_packed(input, packed).unwrap(), minimum);
        assert_eq!(consume_packed(packed), expected);
        eprintln!(
            "optimal/{kind}: input_dense_bytes={}, input_sparse_bytes={}, input_sparse_active_bytes={}, scratch_bytes={}, packed_bytes={}, borrowed_active_bytes={}, minimum_rectangles={minimum}",
            size_of::<DenseLabels64>(),
            size_of::<[QuadLeaf64; AREA]>(),
            size_of_val(input),
            scratch.retained_bytes(),
            packed.retained_bytes(),
            borrowed_active_bytes,
        );
        group.bench_function(BenchmarkId::new("labels/packed", kind), |b| {
            b.iter(|| {
                scratch
                    .decompose_labels_packed(black_box(&*labels), packed)
                    .unwrap();
                black_box(consume_packed(black_box(&*packed)));
            })
        });
        group.bench_function(BenchmarkId::new("labels/borrowed", kind), |b| {
            b.iter(|| {
                let output = scratch
                    .decompose_labels_borrowed(black_box(&*labels))
                    .unwrap();
                black_box(consume(black_box(output).iter().copied()));
            })
        });
        group.bench_function(BenchmarkId::new("leaves/packed", kind), |b| {
            b.iter(|| {
                scratch.decompose_packed(black_box(input), packed).unwrap();
                black_box(consume_packed(black_box(&*packed)));
            })
        });
    }
    group.finish();
    let mut preparation = c.benchmark_group("rectangle/prepare");
    let labels = labels("noise");
    preparation.throughput(Throughput::Elements(AREA as u64));
    preparation.bench_function("unit_leaves", |b| {
        b.iter(|| black_box(leaves(black_box(&labels), 1)))
    });
    preparation.finish();
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
