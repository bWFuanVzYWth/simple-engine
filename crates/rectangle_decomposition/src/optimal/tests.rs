use std::{error::Error, num::NonZeroU16};

use super::{
    DenseLabels64, PackedRectangles64, QuadLeaf64, Rectangle, SparseOptimalScratch64,
    SparseQuadError,
};

// 独立像素分区 DP：首个像素必为覆盖它的矩形左上角，所有后继状态严格变小。
fn minimum_counts(side: usize) -> Vec<u8> {
    let mut counts = vec![0; 1 << (side * side)];
    for mask in 1usize..counts.len() {
        let first = mask.trailing_zeros() as usize;
        let x = first % side;
        let y = first / side;
        let mut best = u8::MAX;
        for height in 1..=side - y {
            for width in 1..=side - x {
                let mut rectangle = 0;
                for row in y..y + height {
                    rectangle |= ((1 << width) - 1) << (row * side + x);
                }
                if rectangle & mask == rectangle {
                    best = best.min(1 + counts[mask ^ rectangle]);
                }
            }
        }
        counts[mask] = best;
    }
    counts
}

fn small_coverage(rectangles: &[Rectangle], side: usize, labels: &[u16]) {
    let mut actual = vec![0; side * side];
    for rectangle in rectangles {
        assert!(rectangle.x.start < rectangle.x.end && usize::from(rectangle.x.end) <= side);
        assert!(rectangle.y.start < rectangle.y.end && usize::from(rectangle.y.end) <= side);
        assert_ne!(rectangle.value, 0);
        for y in rectangle.y.start..rectangle.y.end {
            for x in rectangle.x.start..rectangle.x.end {
                let value = &mut actual[usize::from(y) * side + usize::from(x)];
                assert_eq!(*value, 0, "矩形面积不能重叠");
                *value = rectangle.value;
            }
        }
    }
    assert_eq!(actual, labels);
}

fn leaves(labels: &DenseLabels64) -> Vec<QuadLeaf64> {
    let mut result = Vec::new();
    for (v, row) in labels.rows().iter().enumerate() {
        for (u, &label) in row.iter().enumerate() {
            if let Some(value) = NonZeroU16::new(label) {
                result.push(QuadLeaf64 {
                    u: u as u8,
                    v: v as u8,
                    lod: 0,
                    value,
                });
            }
        }
    }
    result
}

#[test]
fn every_binary_four_by_four_partition_has_minimum_count() -> Result<(), SparseQuadError> {
    let oracle = minimum_counts(4);
    let mut labels = DenseLabels64::new();
    let mut scratch = SparseOptimalScratch64::new();
    for (mask, &minimum) in oracle.iter().enumerate() {
        for y in 0..4 {
            for x in 0..4 {
                labels.rows_mut()[y][x] = ((mask >> (y * 4 + x)) & 1) as u16;
            }
        }
        let rectangles = scratch.decompose_labels_borrowed(&labels)?;
        assert_eq!(rectangles.len(), usize::from(minimum), "像素图 {mask:04x}");
        let mut covered = 0usize;
        for rectangle in rectangles {
            assert_eq!(rectangle.value, 1);
            assert!(rectangle.x.start < rectangle.x.end && rectangle.x.end <= 4);
            assert!(rectangle.y.start < rectangle.y.end && rectangle.y.end <= 4);
            for y in rectangle.y.start..rectangle.y.end {
                for x in rectangle.x.start..rectangle.x.end {
                    let bit = 1 << (usize::from(y) * 4 + usize::from(x));
                    assert_eq!(covered & bit, 0);
                    covered |= bit;
                }
            }
        }
        assert_eq!(covered, mask);
    }
    Ok(())
}

#[test]
fn every_two_label_three_by_three_partition_matches_independent_oracle()
-> Result<(), SparseQuadError> {
    let oracle = minimum_counts(3);
    let mut labels = DenseLabels64::new();
    let mut scratch = SparseOptimalScratch64::new();
    for code in 0..3usize.pow(9) {
        let mut remaining = code;
        let mut masks = [0usize; 2];
        let mut pixels = [0; 9];
        for (index, pixel) in pixels.iter_mut().enumerate() {
            *pixel = (remaining % 3) as u16;
            remaining /= 3;
            labels.rows_mut()[index / 3][index % 3] = *pixel;
            if *pixel != 0 {
                masks[usize::from(*pixel - 1)] |= 1 << index;
            }
        }
        let rectangles = scratch.decompose_labels_borrowed(&labels)?;
        let minimum = masks
            .into_iter()
            .map(|mask| usize::from(oracle[mask]))
            .sum::<usize>();
        assert_eq!(rectangles.len(), minimum, "标签图 {code}");
        small_coverage(rectangles, 3, &pixels);
    }
    Ok(())
}

#[test]
fn sparse_dyadic_leaves_keep_coverage_across_holes_and_labels() -> Result<(), SparseQuadError> {
    let input = [
        QuadLeaf64 {
            u: 0,
            v: 0,
            lod: 2,
            value: NonZeroU16::MIN,
        },
        QuadLeaf64 {
            u: 4,
            v: 0,
            lod: 1,
            value: NonZeroU16::MIN,
        },
        QuadLeaf64 {
            u: 4,
            v: 4,
            lod: 1,
            value: NonZeroU16::MAX,
        },
        QuadLeaf64 {
            u: 6,
            v: 6,
            lod: 1,
            value: NonZeroU16::MAX,
        },
    ];
    let mut labels = DenseLabels64::new();
    for leaf in input {
        let side = 1 << leaf.lod;
        for row in &mut labels.rows_mut()[usize::from(leaf.v)..usize::from(leaf.v + side)] {
            row[usize::from(leaf.u)..usize::from(leaf.u + side)].fill(leaf.value.get());
        }
    }
    let expected: Vec<_> = labels.rows()[..8]
        .iter()
        .flat_map(|row| row[..8].iter().copied())
        .collect();
    let mut scratch = SparseOptimalScratch64::new();
    let result = scratch.decompose_borrowed(&input)?;
    small_coverage(result, 8, &expected);
    let count = result.len();
    assert_eq!(scratch.decompose_labels_borrowed(&labels)?.len(), count);
    let mut packed = PackedRectangles64::new();
    scratch.decompose_packed(&input, &mut packed)?;
    small_coverage(&packed.iter().collect::<Vec<_>>(), 8, &expected);
    assert_eq!(packed.len(), count);
    Ok(())
}

#[test]
fn full_plane_capacity_and_empty_output_survive_scratch_reuse() -> Result<(), SparseQuadError> {
    let mut scratch = SparseOptimalScratch64::new();
    let mut packed = PackedRectangles64::new();
    let mut labels = DenseLabels64::new();
    for pattern in [0, 1, 2, 3, 0] {
        for (y, row) in labels.rows_mut().iter_mut().enumerate() {
            for (x, label) in row.iter_mut().enumerate() {
                *label = match pattern {
                    0 if (x + y) % 2 == 0 => 1,
                    0 | 2 => u16::MAX,
                    1 if x % 2 == 1 || y % 2 == 1 => 1,
                    _ => 0,
                };
            }
        }
        let count = [4096, 1025, 1, 0][pattern];
        let input = leaves(&labels);
        let expected = labels.rows().as_flattened();
        assert_eq!(scratch.decompose_packed(&input, &mut packed)?, count);
        small_coverage(&packed.iter().collect::<Vec<_>>(), 64, expected);
        assert_eq!(
            scratch.decompose_labels_packed(&labels, &mut packed)?,
            count
        );
        small_coverage(&packed.iter().collect::<Vec<_>>(), 64, expected);
    }
    Ok(())
}

#[test]
fn invalid_sparse_input_never_reaches_sink() {
    let base = QuadLeaf64 {
        u: 0,
        v: 0,
        lod: 1,
        value: NonZeroU16::MIN,
    };
    let mut scratch = SparseOptimalScratch64::new();
    for (input, expected) in [
        (vec![QuadLeaf64 { lod: 7, ..base }], "lod"),
        (vec![QuadLeaf64 { u: 1, ..base }], "alignment"),
        (vec![QuadLeaf64 { u: 64, ..base }], "bounds"),
        (vec![base, base], "overlap"),
    ] {
        let result = scratch.decompose_into(&input, |_| panic!("非法输入不得交付输出"));
        assert!(matches!(
            (expected, result),
            ("lod", Err(SparseQuadError::LodOutOfRange))
                | ("alignment", Err(SparseQuadError::Misaligned))
                | ("bounds", Err(SparseQuadError::OutOfBounds))
                | ("overlap", Err(SparseQuadError::Overlap))
        ));
    }
    assert_eq!(
        scratch
            .decompose_borrowed(&[base])
            .expect("失败后复用")
            .len(),
        1
    );
}

#[test]
fn sink_failure_preserves_prefix_and_allocation_source() -> Result<(), SparseQuadError> {
    let input = [
        QuadLeaf64 {
            u: 0,
            v: 0,
            lod: 0,
            value: NonZeroU16::MIN,
        },
        QuadLeaf64 {
            u: 2,
            v: 0,
            lod: 0,
            value: NonZeroU16::MAX,
        },
    ];
    let mut scratch = SparseOptimalScratch64::new();
    let expected = scratch.decompose_borrowed(&input)?.to_vec();
    let mut output = Vec::new();
    let result = scratch.decompose_into(&input, |rectangle| {
        if !output.is_empty() {
            Vec::<u8>::new().try_reserve(usize::MAX)?;
        }
        output.push(rectangle);
        Ok(())
    });
    let error = result.expect_err("第二个矩形触发容量错误");
    assert!(matches!(error, SparseQuadError::Allocation(_)));
    assert!(error.source().is_some());
    assert_eq!(output, expected[..1]);
    assert_eq!(scratch.decompose_borrowed(&input)?.len(), 2);
    Ok(())
}

#[test]
fn independent_workers_reuse_fixed_scratch_on_two_mib_stacks() {
    let workers = [(); 2].map(|()| {
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                let mut scratch = SparseOptimalScratch64::new();
                let mut labels = DenseLabels64::new();
                let mut output = PackedRectangles64::new();
                for value in [1, u16::MAX, 0, 7] {
                    for row in labels.rows_mut() {
                        row.fill(value);
                    }
                    let count = scratch
                        .decompose_labels_packed(&labels, &mut output)
                        .expect("合法切面");
                    assert_eq!(count, usize::from(value != 0));
                }
                (scratch.retained_bytes(), output.retained_bytes())
            })
            .expect("创建 worker")
    });
    for worker in workers {
        let (scratch, packed) = worker.join().expect("worker 完成");
        eprintln!("optimal 固定存储：scratch={scratch} B，packed={packed} B；worker 栈为 2 MiB");
    }
}
