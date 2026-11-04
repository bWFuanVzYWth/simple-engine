//! raster 单步写入与实际叶子前缀的对应；整图旧态仅使用函数借用的 old 视图。

use super::*;

verus! {

pub(super) open spec fn unit_write(
    before: Seq<u16>, after: Seq<u16>, leaf: QuadLeaf64, result: Result<(), SparseQuadError>,
) -> bool {
    before.len() == 4096 && after.len() == 4096 && leaf.lod == 0
        && match result {
            Ok(()) => axis_owner::leaf_valid(leaf) && before[input::origin(leaf)] == 0
                && after == before.update(input::origin(leaf), leaf.value@),
            Err(SparseQuadError::OutOfBounds) => !axis_owner::leaf_valid(leaf) && after == before,
            Err(SparseQuadError::Overlap) => axis_owner::leaf_valid(leaf)
                && before[input::origin(leaf)] != 0 && after == before,
            _ => false,
        }
}

pub(super) open spec fn leaf_context(
    before: Seq<u16>, after: Seq<u16>, leaf: QuadLeaf64, result: Result<(), SparseQuadError>,
) -> bool {
    forall|prefix: Seq<QuadLeaf64>| #![trigger input::labels(prefix)] input::valid(prefix) && before == input::labels(prefix)
        ==> match result {
            Ok(()) => input::valid(prefix.push(leaf)) && after == input::labels(prefix.push(leaf)),
            Err(error) => input::checked(prefix.push(leaf), Err(error)),
        }
}

pub(super) proof fn unit_frontiers(
    before: Seq<u16>, after: Seq<u16>, leaf: QuadLeaf64, result: Result<(), SparseQuadError>,
)
    requires unit_write(before, after, leaf, result),
    ensures leaf_context(before, after, leaf, result),
{
    assert forall|prefix: Seq<QuadLeaf64>| #![trigger input::labels(prefix)] input::valid(prefix) && before == input::labels(prefix)
        implies match result {
            Ok(()) => input::valid(prefix.push(leaf)) && after == input::labels(prefix.push(leaf)),
            Err(error) => input::checked(prefix.push(leaf), Err(error)),
        } by {
        match result {
            Ok(()) => {
                input::unit_no_conflict(prefix, leaf);
                input::append_valid(prefix, leaf);
                assert forall|p: int| 0 <= p < 4096 implies #[trigger] after[p] == input::labels(prefix.push(leaf))[p] by {
                    input::append_value(prefix, leaf, p);
                    input::unit_cover(leaf, p);
                }
                assert(after =~= input::labels(prefix.push(leaf)));
            },
            Err(SparseQuadError::OutOfBounds) => {
                assert(axis_owner::leaf_checked(leaf, Err(SparseQuadError::OutOfBounds)));
                assert(axis_owner::leaf_checked(prefix.push(leaf)[prefix.len() as int], Err(SparseQuadError::OutOfBounds)));
            },
            Err(SparseQuadError::Overlap) => { input::unit_overlap(prefix, leaf); },
            _ => {},
        }
    }
}

pub(super) open spec fn row_occupancy(row: Seq<u16>, mask: u64) -> bool {
    row.len() == 64 && forall|x: u64| x < 64 ==> #[trigger] axis_owner::bit(mask, x) == (row[x as int] != 0)
}

pub(super) open spec fn replaced(row: Seq<u16>, start: u8, end: u8, value: u16) -> Seq<u16> {
    Seq::new(64, |x: int| if start <= x < end { value } else { row[x] })
}

pub(super) open spec fn row_step(
    before: Seq<u16>, after: Seq<u16>, old_mask: u64, new_mask: u64,
    start: u8, end: u8, value: u16, result: Result<(), SparseQuadError>,
) -> bool {
    start < end <= 64 && value != 0 && row_occupancy(before, old_mask) && row_occupancy(after, new_mask)
        && match result {
            Ok(()) => after == replaced(before, start, end, value)
                && (forall|x: int| start <= x < end ==> #[trigger] before[x] == 0),
            Err(SparseQuadError::Overlap) => after == before && new_mask == old_mask
                && exists|x: int| start <= x < end && #[trigger] before[x] != 0,
            _ => false,
        }
}

proof fn interval_bit(start: u8, end: u8, mask: u64, x: u64)
    requires start <= 64, end <= 64, x < 64, crate::geometry::cell_bits(start, end, mask),
    ensures axis_owner::bit(mask, x) == (start <= x < end),
{
    let local = x as u8;
    assert(((mask >> local) & 1) == if start <= local < end { 1u64 } else { 0u64 });
    assert(((mask >> x) & 1) == ((mask >> local) & 1)) by(bit_vector)
        requires x < 64, local == x as u8;
}

pub(super) proof fn row_effect(
    before: Seq<u16>, after: Seq<u16>, old_mask: u64, new_mask: u64,
    start: u8, end: u8, mask: u64, value: u16, result: Result<(), SparseQuadError>,
)
    requires start < end <= 64, value != 0, row_occupancy(before, old_mask),
        crate::geometry::cell_bits(start, end, mask),
        match result {
            Ok(()) => old_mask & mask == 0 && new_mask == old_mask | mask
                && after == replaced(before, start, end, value),
            Err(SparseQuadError::Overlap) => old_mask & mask != 0 && new_mask == old_mask && after == before,
            _ => false,
        },
    ensures row_step(before, after, old_mask, new_mask, start, end, value, result),
{
    match result {
        Ok(()) => {
            assert forall|x: u64| x < 64 implies #[trigger] axis_owner::bit(new_mask, x) == (after[x as int] != 0) by {
                interval_bit(start, end, mask, x);
                assert(axis_owner::bit(new_mask, x) == (axis_owner::bit(old_mask, x) || axis_owner::bit(mask, x)))
                    by(bit_vector) requires x < 64, new_mask == old_mask | mask;
            }
            assert forall|x: int| start <= x < end implies #[trigger] before[x] == 0 by {
                let bit = x as u64;
                interval_bit(start, end, mask, bit);
                assert(!axis_owner::bit(old_mask, bit)) by(bit_vector)
                    requires bit < 64, old_mask & mask == 0, (mask >> bit) & 1 == 1;
            }
        },
        Err(SparseQuadError::Overlap) => {
            let common = old_mask & mask;
            vstd::std_specs::bits::axiom_u64_trailing_zeros(common);
            let bit = vstd::std_specs::bits::u64_trailing_zeros(common) as u64;
            assert(axis_owner::bit(old_mask, bit) && axis_owner::bit(mask, bit)) by(bit_vector)
                requires bit < 64, (common >> bit) & 1 == 1, common == old_mask & mask;
            interval_bit(start, end, mask, bit);
            assert(before[bit as int] != 0);
        },
        _ => {},
    }
}

pub(super) open spec fn grid_occupancy(labels: Seq<u16>, buckets: Seq<IntervalBucket>) -> bool {
    labels.len() == 4096 && buckets.len() == 64
        && forall|y: int| 0 <= y < 64 ==> #[trigger]
            row_occupancy(labels.subrange(y * 64, (y + 1) * 64), buckets[y].starts)
}

pub(super) proof fn empty_occupancy(labels: Seq<u16>, buckets: Seq<IntervalBucket>)
    requires labels == Seq::new(4096, |_: int| 0u16), buckets.len() == 64,
        forall|y: int| 0 <= y < 64 ==> #[trigger] buckets[y].valid() && buckets[y].starts == 0,
    ensures grid_occupancy(labels, buckets),
{
    assert forall|y: int| 0 <= y < 64 implies #[trigger]
        row_occupancy(labels.subrange(y * 64, (y + 1) * 64), buckets[y].starts) by {
        assert(buckets[y].valid());
        assert forall|x: u64| x < 64 implies #[trigger] axis_owner::bit(0, x)
            == (labels.subrange(y * 64, (y + 1) * 64)[x as int] != 0) by {
            assert((0u64 >> x) & 1 == 0) by(bit_vector) requires x < 64;
        }
    }
}

pub(super) proof fn selected_occupancy(labels: &DenseLabels64, buckets: Seq<IntervalBucket>, y: usize)
    requires grid_occupancy(labels@, buckets), y < 64,
    ensures row_occupancy(labels.rows@[y as int]@, buckets[y as int].starts),
{
    proof::label_row(labels, y as int);
    assert(row_occupancy(labels@.subrange(y as int * 64, (y as int + 1) * 64), buckets[y as int].starts));
}

pub(super) open spec fn in_row(p: int, y: usize, start: u8, end: u8) -> bool {
    0 <= p < 4096 && p / 64 == y && start <= p % 64 < end
}

pub(super) open spec fn grid_replaced(before: Seq<u16>, y: usize, start: u8, end: u8, value: u16) -> Seq<u16> {
    Seq::new(4096, |p: int| if in_row(p, y, start, end) { value } else { before[p] })
}

pub(super) open spec fn row_at_step(
    before: Seq<u16>, after: Seq<u16>, y: usize, start: u8, end: u8, value: u16, result: Result<(), SparseQuadError>,
) -> bool {
    before.len() == 4096 && after.len() == 4096 && y < 64 && start < end <= 64 && value != 0
        && match result {
            Ok(()) => after == grid_replaced(before, y, start, end, value)
                && (forall|p: int| #[trigger] in_row(p, y, start, end) ==> before[p] == 0),
            Err(SparseQuadError::Overlap) => after == before
                && exists|p: int| #[trigger] in_row(p, y, start, end) && before[p] != 0,
            _ => false,
        }
}

pub(super) proof fn row_at_effect(
    before: &DenseLabels64, after: &DenseLabels64, old_buckets: Seq<IntervalBucket>, new_buckets: Seq<IntervalBucket>,
    y: usize, start: u8, end: u8, value: u16, result: Result<(), SparseQuadError>,
)
    requires grid_occupancy(before@, old_buckets), new_buckets.len() == 64, y < 64,
        row_step(before.rows@[y as int]@, after.rows@[y as int]@,
            old_buckets[y as int].starts, new_buckets[y as int].starts, start, end, value, result),
        forall|line: int| 0 <= line < 64 && line != y ==> #[trigger] after.rows@[line] == before.rows@[line],
        forall|line: int| 0 <= line < 64 && line != y ==> #[trigger] new_buckets[line] == old_buckets[line],
        new_buckets[y as int].items@ == old_buckets[y as int].items@,
    ensures row_at_step(before@, after@, y, start, end, value, result), grid_occupancy(after@, new_buckets),
        forall|line: int| 0 <= line < 64 ==> #[trigger] new_buckets[line].items@ == old_buckets[line].items@,
{
    proof::label_layout(before);
    proof::label_layout(after);
    axis_owner::bucket_items_frame(old_buckets, new_buckets, y);
    assert forall|line: int| 0 <= line < 64 implies #[trigger]
        row_occupancy(after@.subrange(line * 64, (line + 1) * 64), new_buckets[line].starts) by {
        proof::label_row(before, line);
        proof::label_row(after, line);
        if line != y {
            assert(new_buckets[line] == old_buckets[line]);
            assert(row_occupancy(before@.subrange(line * 64, (line + 1) * 64), old_buckets[line].starts));
        }
    }
    match result {
        Ok(()) => {
            assert forall|p: int| 0 <= p < 4096 implies #[trigger] after@[p]
                == if in_row(p, y, start, end) { value } else { before@[p] } by {
                if p / 64 != y { assert(after.rows@[p / 64] == before.rows@[p / 64]); }
            }
            assert(after@ =~= grid_replaced(before@, y, start, end, value));
            assert forall|p: int| #[trigger] in_row(p, y, start, end) implies before@[p] == 0 by {
                assert(before.rows@[y as int]@[p % 64] == 0);
            }
            assert(row_at_step(before@, after@, y, start, end, value, result));
        },
        Err(SparseQuadError::Overlap) => {
            assert(after@ =~= before@);
            let x = choose|x: int| start <= x < end && #[trigger] before.rows@[y as int]@[x] != 0;
            let p = y as int * 64 + x;
            assert(p / 64 == y && p % 64 == x) by(nonlinear_arith)
                requires 0 <= y < 64, 0 <= x < 64, p == y as int * 64 + x;
            assert(in_row(p, y, start, end) && before@[p] != 0);
            assert(exists|p: int| #[trigger] in_row(p, y, start, end) && before@[p] != 0);
            assert(row_at_step(before@, after@, y, start, end, value, result));
        },
        _ => {},
    }
}

pub(super) open spec fn pixel_conflict(before: Seq<u16>, leaf: QuadLeaf64) -> bool {
    exists|p: int| #[trigger] input::covers(leaf, p) && before[p] != 0
}

pub(super) open spec fn painted(before: Seq<u16>, leaf: QuadLeaf64) -> Seq<u16> {
    Seq::new(4096, |p: int| if input::covers(leaf, p) { leaf.value@ } else { before[p] })
}

pub(super) open spec fn leaf_write(
    before: Seq<u16>, after: Seq<u16>, leaf: QuadLeaf64, result: Result<(), SparseQuadError>,
) -> bool {
    before.len() == 4096 && after.len() == 4096 && match result {
        Ok(()) => axis_owner::leaf_valid(leaf) && !pixel_conflict(before, leaf) && after == painted(before, leaf),
        Err(SparseQuadError::Overlap) => axis_owner::leaf_valid(leaf) && pixel_conflict(before, leaf),
        Err(error) => axis_owner::leaf_checked(leaf, Err(error)) && after == before,
    }
}

pub(super) proof fn leaf_frontiers(
    before: Seq<u16>, after: Seq<u16>, leaf: QuadLeaf64, result: Result<(), SparseQuadError>,
)
    requires leaf_write(before, after, leaf, result),
    ensures leaf_context(before, after, leaf, result),
{
    assert forall|prefix: Seq<QuadLeaf64>| #![trigger input::labels(prefix)] input::valid(prefix) && before == input::labels(prefix)
        implies match result {
            Ok(()) => input::valid(prefix.push(leaf)) && after == input::labels(prefix.push(leaf)),
            Err(error) => input::checked(prefix.push(leaf), Err(error)),
        } by {
        match result {
            Ok(()) => {
                if input::conflicts(prefix, leaf) {
                    let (i, p) = choose|i: int, p: int| 0 <= i < prefix.len()
                        && #[trigger] input::covers(prefix[i], p) && #[trigger] input::covers(leaf, p);
                    input::selected_value(prefix, p, i);
                    assert(prefix[i].value@ != 0);
                    assert(pixel_conflict(before, leaf));
                }
                input::append_valid(prefix, leaf);
                assert forall|p: int| 0 <= p < 4096 implies #[trigger] after[p] == input::labels(prefix.push(leaf))[p] by {
                    input::append_value(prefix, leaf, p);
                }
                assert(after =~= input::labels(prefix.push(leaf)));
            },
            Err(SparseQuadError::Overlap) => {
                let p = choose|p: int| #[trigger] input::covers(leaf, p) && before[p] != 0;
                input::overlap_pixel(prefix, leaf, p);
            },
            Err(error) => {
                assert(axis_owner::leaf_checked(prefix.push(leaf)[prefix.len() as int], Err(error)));
            },
        }
    }
}

pub(super) open spec fn leaf_frontier(before: Seq<u16>, current: Seq<u16>, leaf: QuadLeaf64, y: u8) -> bool {
    before.len() == 4096 && current.len() == 4096 && axis_owner::leaf_valid(leaf)
        && leaf.v <= y <= leaf.v as int + crate::geometry::side(leaf.lod)
        && (forall|p: int| 0 <= p < 4096 ==> #[trigger] current[p]
            == if input::covers(leaf, p) && p / 64 < y { leaf.value@ } else { before[p] })
        && (forall|p: int| #[trigger] input::covers(leaf, p) && p / 64 < y ==> before[p] == 0)
}

pub(super) proof fn leaf_started(before: Seq<u16>, leaf: QuadLeaf64)
    requires before.len() == 4096, axis_owner::leaf_valid(leaf),
    ensures leaf_frontier(before, before, leaf, leaf.v),
{}

proof fn row_of_leaf(leaf: QuadLeaf64, y: u8, end_x: u8, p: int)
    requires axis_owner::leaf_valid(leaf), leaf.v <= y < leaf.v as int + crate::geometry::side(leaf.lod),
        end_x == leaf.u as int + crate::geometry::side(leaf.lod),
    ensures in_row(p, y as usize, leaf.u, end_x) <==> input::covers(leaf, p) && p / 64 == y,
{}

pub(super) open spec fn leaf_row_context(
    before: Seq<u16>, after: Seq<u16>, leaf: QuadLeaf64, y: u8, result: Result<(), SparseQuadError>,
) -> bool {
    forall|origin: Seq<u16>| #![trigger leaf_frontier(origin, before, leaf, y)] leaf_frontier(origin, before, leaf, y)
        ==> match result {
            Ok(()) => leaf_frontier(origin, after, leaf, (y + 1) as u8),
            Err(SparseQuadError::Overlap) => pixel_conflict(origin, leaf),
            _ => false,
        }
}

pub(super) proof fn leaf_row_effect(
    before: Seq<u16>, after: Seq<u16>, leaf: QuadLeaf64, y: u8, end_x: u8, result: Result<(), SparseQuadError>,
)
    requires axis_owner::leaf_valid(leaf), leaf.v <= y < leaf.v as int + crate::geometry::side(leaf.lod),
        end_x == leaf.u as int + crate::geometry::side(leaf.lod),
        row_at_step(before, after, y as usize, leaf.u, end_x, leaf.value@, result),
    ensures leaf_row_context(before, after, leaf, y, result),
{
    assert forall|origin: Seq<u16>| #![trigger leaf_frontier(origin, before, leaf, y)] leaf_frontier(origin, before, leaf, y)
        implies match result {
            Ok(()) => leaf_frontier(origin, after, leaf, (y + 1) as u8),
            Err(SparseQuadError::Overlap) => pixel_conflict(origin, leaf),
            _ => false,
        } by {
        match result {
            Ok(()) => {
                assert forall|p: int| 0 <= p < 4096 implies #[trigger] after[p]
                    == if input::covers(leaf, p) && p / 64 < y + 1 { leaf.value@ } else { origin[p] } by {
                    row_of_leaf(leaf, y, end_x, p);
                }
                assert forall|p: int| #[trigger] input::covers(leaf, p) && p / 64 < y + 1 implies origin[p] == 0 by {
                    if p / 64 == y {
                        row_of_leaf(leaf, y, end_x, p);
                        assert(before[p] == 0);
                    }
                }
            },
            Err(SparseQuadError::Overlap) => {
                let p = choose|p: int| #[trigger] in_row(p, y as usize, leaf.u, end_x) && before[p] != 0;
                row_of_leaf(leaf, y, end_x, p);
                assert(before[p] == origin[p]);
                assert(pixel_conflict(origin, leaf));
            },
            _ => {},
        }
    }
}

pub(super) proof fn leaf_completed(before: Seq<u16>, after: Seq<u16>, leaf: QuadLeaf64, end: u8)
    requires leaf_frontier(before, after, leaf, end), end == leaf.v as int + crate::geometry::side(leaf.lod),
    ensures leaf_write(before, after, leaf, Ok(())),
{
    assert(after =~= painted(before, leaf));
}

}
