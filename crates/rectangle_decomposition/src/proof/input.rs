//! 真实叶子列表的唯一像素覆盖；非法列表没有成功图像的语义承诺。

use super::{
    Interval, IntervalBucket, QuadLeaf64, SparseQuadError, event_proof as events,
    failure_proof as failure, proof as axis,
};
use vstd::prelude::*;

verus! {

pub(super) open spec fn covers(leaf: QuadLeaf64, p: int) -> bool {
    let side = crate::geometry::side(leaf.lod) as int;
    0 <= p < 4096 && axis::leaf_valid(leaf)
        && leaf.u <= p % 64 < leaf.u as int + side
        && leaf.v <= p / 64 < leaf.v as int + side
}

pub(super) open spec fn valid(leaves: Seq<QuadLeaf64>) -> bool {
    (forall|i: int| 0 <= i < leaves.len() ==> #[trigger] axis::leaf_valid(leaves[i]))
        && (forall|i: int, j: int, p: int| 0 <= i < j < leaves.len()
            && #[trigger] covers(leaves[i], p) ==> !#[trigger] covers(leaves[j], p))
}

pub(super) open spec fn value_at(leaves: Seq<QuadLeaf64>, p: int) -> u16 {
    if exists|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p) {
        let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p);
        leaves[i].value@
    } else { 0 }
}

pub(super) open spec fn labels(leaves: Seq<QuadLeaf64>) -> Seq<u16> {
    Seq::new(4096, |p: int| value_at(leaves, p))
}

pub(super) open spec fn conflicts(leaves: Seq<QuadLeaf64>, leaf: QuadLeaf64) -> bool {
    exists|i: int, p: int| 0 <= i < leaves.len()
        && #[trigger] covers(leaves[i], p) && #[trigger] covers(leaf, p)
}

pub(super) open spec fn checked(leaves: Seq<QuadLeaf64>, result: Result<(), SparseQuadError>) -> bool {
    match result {
        Ok(()) => valid(leaves),
        Err(SparseQuadError::LodOutOfRange) => exists|i: int| 0 <= i < leaves.len()
            && #[trigger] axis::leaf_checked(leaves[i], Err(SparseQuadError::LodOutOfRange)),
        Err(SparseQuadError::Misaligned) => exists|i: int| 0 <= i < leaves.len()
            && #[trigger] axis::leaf_checked(leaves[i], Err(SparseQuadError::Misaligned)),
        Err(SparseQuadError::OutOfBounds) => exists|i: int| 0 <= i < leaves.len()
            && #[trigger] axis::leaf_checked(leaves[i], Err(SparseQuadError::OutOfBounds)),
        Err(SparseQuadError::Overlap) => exists|i: int, j: int, p: int| 0 <= i < j < leaves.len()
            && #[trigger] covers(leaves[i], p) && #[trigger] covers(leaves[j], p),
        _ => false,
    }
}

pub(super) proof fn prefix_valid(leaves: Seq<QuadLeaf64>, count: int)
    requires valid(leaves), 0 <= count <= leaves.len(),
    ensures valid(leaves.take(count)),
{
    assert forall|i: int| 0 <= i < count implies #[trigger] axis::leaf_valid(leaves.take(count)[i]) by {
        assert(leaves.take(count)[i] == leaves[i]);
    }
    assert forall|i: int, j: int, p: int| 0 <= i < j < count && #[trigger] covers(leaves.take(count)[i], p)
        implies !#[trigger] covers(leaves.take(count)[j], p) by {
        assert(leaves.take(count)[i] == leaves[i] && leaves.take(count)[j] == leaves[j]);
    }
}

pub(super) proof fn invalid_prefix(leaves: Seq<QuadLeaf64>, count: int)
    requires 0 <= count <= leaves.len(), !valid(leaves.take(count)),
    ensures !valid(leaves),
{
    if valid(leaves) { prefix_valid(leaves, count); }
}

pub(super) proof fn unique_cover(leaves: Seq<QuadLeaf64>, p: int, i: int, j: int)
    requires valid(leaves), 0 <= i < leaves.len(), 0 <= j < leaves.len(),
        covers(leaves[i], p), covers(leaves[j], p),
    ensures i == j,
{}

pub(super) proof fn selected_value(leaves: Seq<QuadLeaf64>, p: int, i: int)
    requires valid(leaves), 0 <= i < leaves.len(), covers(leaves[i], p),
    ensures value_at(leaves, p) == leaves[i].value@,
{
    let j = choose|j: int| 0 <= j < leaves.len() && #[trigger] covers(leaves[j], p);
    unique_cover(leaves, p, i, j);
}

pub(super) proof fn nonzero_covered(leaves: Seq<QuadLeaf64>, p: int)
    requires valid(leaves),
    ensures value_at(leaves, p) != 0 <==> exists|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p),
{
    if exists|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p) {
        let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p);
        selected_value(leaves, p, i);
        assert(leaves[i].value@ != 0);
    }
}

pub(super) proof fn append_valid(leaves: Seq<QuadLeaf64>, leaf: QuadLeaf64)
    requires valid(leaves), axis::leaf_valid(leaf), !conflicts(leaves, leaf),
    ensures valid(leaves.push(leaf)),
{
    let after = leaves.push(leaf);
    assert forall|i: int| 0 <= i < after.len() implies #[trigger] axis::leaf_valid(after[i]) by {
        if i < leaves.len() { assert(after[i] == leaves[i]); }
        else { assert(after[i] == leaf); }
    }
    assert forall|i: int, j: int, p: int| 0 <= i < j < after.len() && #[trigger] covers(after[i], p)
        implies !#[trigger] covers(after[j], p) by {
        if j < leaves.len() { assert(after[i] == leaves[i] && after[j] == leaves[j]); }
        else { assert(i < leaves.len() && after[i] == leaves[i] && after[j] == leaf); }
    }
}

pub(super) proof fn append_value(leaves: Seq<QuadLeaf64>, leaf: QuadLeaf64, p: int)
    requires valid(leaves), axis::leaf_valid(leaf), !conflicts(leaves, leaf),
    ensures value_at(leaves.push(leaf), p) == if covers(leaf, p) { leaf.value@ } else { value_at(leaves, p) },
{
    append_valid(leaves, leaf);
    let after = leaves.push(leaf);
    if covers(leaf, p) {
        selected_value(after, p, leaves.len() as int);
    } else if exists|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p) {
        let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p);
        selected_value(leaves, p, i);
        selected_value(after, p, i);
    } else {
        assert forall|i: int| 0 <= i < after.len() implies !#[trigger] covers(after[i], p) by {
            if i < leaves.len() { assert(after[i] == leaves[i]); }
            else { assert(after[i] == leaf); }
        }
    }
}

pub(super) proof fn append_labels(leaves: Seq<QuadLeaf64>, leaf: QuadLeaf64)
    requires valid(leaves), axis::leaf_valid(leaf), !conflicts(leaves, leaf),
    ensures forall|p: int| 0 <= p < 4096 ==> #[trigger] labels(leaves.push(leaf))[p]
        == if covers(leaf, p) { leaf.value@ } else { labels(leaves)[p] },
{
    assert forall|p: int| 0 <= p < 4096 implies #[trigger] labels(leaves.push(leaf))[p]
        == if covers(leaf, p) { leaf.value@ } else { labels(leaves)[p] } by {
        append_value(leaves, leaf, p);
    }
}

pub(super) proof fn empty_labels()
    ensures valid(Seq::empty()), labels(Seq::empty()) == Seq::new(4096, |_: int| 0u16),
{
    assert(labels(Seq::empty()) =~= Seq::new(4096, |_: int| 0u16));
}

pub(super) proof fn unit_bounds(leaf: QuadLeaf64)
    requires leaf.lod == 0,
    ensures axis::leaf_valid(leaf) <==> leaf.u < 64 && leaf.v < 64,
        ((leaf.u | leaf.v) < 64) <==> axis::leaf_valid(leaf),
{
    let u = leaf.u;
    let v = leaf.v;
    assert((u | v) < 64 <==> u < 64 && v < 64) by(bit_vector);
}

pub(super) open spec fn origin(leaf: QuadLeaf64) -> int {
    leaf.v as int * 64 + leaf.u
}

pub(super) proof fn unit_cover(leaf: QuadLeaf64, p: int)
    requires leaf.lod == 0, axis::leaf_valid(leaf),
    ensures 0 <= origin(leaf) < 4096, covers(leaf, p) <==> p == origin(leaf),
{
    unit_bounds(leaf);
    assert(origin(leaf) / 64 == leaf.v as int && origin(leaf) % 64 == leaf.u as int) by(nonlinear_arith)
        requires leaf.u < 64, leaf.v < 64;
    if covers(leaf, p) {
        assert(p % 64 == leaf.u as int && p / 64 == leaf.v as int);
        assert(p == p / 64 * 64 + p % 64);
    }
}

pub(super) proof fn unit_no_conflict(leaves: Seq<QuadLeaf64>, leaf: QuadLeaf64)
    requires valid(leaves), leaf.lod == 0, axis::leaf_valid(leaf), value_at(leaves, origin(leaf)) == 0,
    ensures !conflicts(leaves, leaf),
{
    if conflicts(leaves, leaf) {
        let (i, p) = choose|i: int, p: int| 0 <= i < leaves.len()
            && #[trigger] covers(leaves[i], p) && #[trigger] covers(leaf, p);
        unit_cover(leaf, p);
        selected_value(leaves, p, i);
        assert(leaves[i].value@ != 0);
    }
}

pub(super) proof fn unit_overlap(leaves: Seq<QuadLeaf64>, leaf: QuadLeaf64)
    requires valid(leaves), leaf.lod == 0, axis::leaf_valid(leaf), value_at(leaves, origin(leaf)) != 0,
    ensures checked(leaves.push(leaf), Err(SparseQuadError::Overlap)),
{
    nonzero_covered(leaves, origin(leaf));
    let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], origin(leaf));
    unit_cover(leaf, origin(leaf));
    assert(covers(leaves.push(leaf)[i], origin(leaf)));
    assert(covers(leaves.push(leaf)[leaves.len() as int], origin(leaf)));
}

pub(super) proof fn overlap_pixel(leaves: Seq<QuadLeaf64>, leaf: QuadLeaf64, p: int)
    requires valid(leaves), covers(leaf, p), value_at(leaves, p) != 0,
    ensures checked(leaves.push(leaf), Err(SparseQuadError::Overlap)),
{
    nonzero_covered(leaves, p);
    let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p);
    assert(covers(leaves.push(leaf)[i], p));
    assert(covers(leaves.push(leaf)[leaves.len() as int], p));
}

pub(super) proof fn prefix_step(leaves: Seq<QuadLeaf64>, count: int)
    requires 0 <= count < leaves.len(),
    ensures leaves.take(count + 1) == leaves.take(count).push(leaves[count]),
{
    assert(leaves.take(count + 1) =~= leaves.take(count).push(leaves[count]));
}

pub(super) proof fn error_prefix(leaves: Seq<QuadLeaf64>, count: int, error: SparseQuadError)
    requires 0 <= count <= leaves.len(), checked(leaves.take(count), Err(error)),
    ensures checked(leaves, Err(error)),
{
    let prefix = leaves.take(count);
    match error {
        SparseQuadError::LodOutOfRange | SparseQuadError::Misaligned | SparseQuadError::OutOfBounds => {
            let i = choose|i: int| 0 <= i < prefix.len()
                && #[trigger] axis::leaf_checked(prefix[i], Err(error));
            assert(prefix[i] == leaves[i]);
            assert(axis::leaf_checked(leaves[i], Err(error)));
        },
        SparseQuadError::Overlap => {
            let (i, j, p) = choose|i: int, j: int, p: int| 0 <= i < j < prefix.len()
                && #[trigger] covers(prefix[i], p) && #[trigger] covers(prefix[j], p);
            assert(prefix[i] == leaves[i] && prefix[j] == leaves[j]);
            assert(covers(leaves[i], p) && covers(leaves[j], p));
        },
        _ => {},
    }
}

pub(super) proof fn checked_status(leaves: Seq<QuadLeaf64>, result: Result<(), SparseQuadError>)
    requires checked(leaves, result),
    ensures result.is_ok() == valid(leaves),
{
    match result {
        Err(SparseQuadError::LodOutOfRange) => {
            let i = choose|i: int| 0 <= i < leaves.len()
                && #[trigger] axis::leaf_checked(leaves[i], Err(SparseQuadError::LodOutOfRange));
            assert(!axis::leaf_valid(leaves[i]));
        },
        Err(SparseQuadError::Misaligned) => {
            let i = choose|i: int| 0 <= i < leaves.len()
                && #[trigger] axis::leaf_checked(leaves[i], Err(SparseQuadError::Misaligned));
            assert(!axis::leaf_valid(leaves[i]));
        },
        Err(SparseQuadError::OutOfBounds) => {
            let i = choose|i: int| 0 <= i < leaves.len()
                && #[trigger] axis::leaf_checked(leaves[i], Err(SparseQuadError::OutOfBounds));
            assert(!axis::leaf_valid(leaves[i]));
        },
        Err(SparseQuadError::Overlap) => {
            let (i, j, p) = choose|i: int, j: int, p: int| 0 <= i < j < leaves.len()
                && #[trigger] covers(leaves[i], p) && #[trigger] covers(leaves[j], p);
        },
        _ => {},
    }
}

pub(super) open spec fn event_line(leaf: QuadLeaf64, columns: bool) -> int {
    if columns { leaf.u as int } else { leaf.v as int }
}

pub(super) open spec fn event_start(leaf: QuadLeaf64, columns: bool) -> int {
    if columns { leaf.v as int } else { leaf.u as int }
}

pub(super) open spec fn event_interval(leaf: QuadLeaf64, columns: bool) -> Interval {
    let start = event_start(leaf, columns);
    Interval { start: start as u8, end: (start + crate::geometry::side(leaf.lod)) as u8, value: leaf.value@ }
}

/// 事件表只在叶子原点占槽；不在这一阶段假定不同原点的叶子没有覆盖重叠。
pub(super) open spec fn distinct_origins(leaves: Seq<QuadLeaf64>) -> bool {
    (forall|i: int| 0 <= i < leaves.len() ==> #[trigger] axis::leaf_valid(leaves[i]))
        && (forall|i: int, j: int| 0 <= i < j < leaves.len()
            ==> leaves[i].u != leaves[j].u || leaves[i].v != leaves[j].v)
}

pub(super) open spec fn seeded(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, columns: bool) -> bool {
    distinct_origins(leaves) && buckets.len() == 64
        && (forall|line: int| 0 <= line < 64 ==> #[trigger] buckets[line].valid())
        && (forall|line: int, slot: u64| 0 <= line < 64 && slot < 64
            ==> #[trigger] axis::bit(buckets[line].starts, slot)
                == exists|i: int| 0 <= i < leaves.len()
                    && event_line(leaves[i], columns) == line && event_start(leaves[i], columns) == slot)
        && (forall|i: int| 0 <= i < leaves.len() ==> #[trigger]
            buckets[event_line(leaves[i], columns)].items@[event_start(leaves[i], columns)]
                == event_interval(leaves[i], columns))
}

pub(super) proof fn seeded_empty(buckets: Seq<IntervalBucket>, columns: bool)
    requires buckets.len() == 64,
        forall|line: int| 0 <= line < 64 ==> #[trigger] buckets[line].starts == 0 && buckets[line].valid(),
    ensures seeded(Seq::empty(), buckets, columns),
{
    assert forall|line: int, slot: u64| 0 <= line < 64 && slot < 64
        implies !#[trigger] axis::bit(buckets[line].starts, slot) by {
        assert(buckets[line].starts == 0);
        assert((0u64 >> slot) & 1 == 0) by(bit_vector) requires slot < 64;
    }
}

pub(super) open spec fn bucket_insert(before: Seq<IntervalBucket>, after: Seq<IntervalBucket>, leaf: QuadLeaf64, columns: bool) -> bool {
    before.len() == 64 && after.len() == 64 && axis::leaf_valid(leaf)
        && after[event_line(leaf, columns)].starts == before[event_line(leaf, columns)].starts
            | (1u64 << event_start(leaf, columns) as u64)
        && after[event_line(leaf, columns)].items@ == before[event_line(leaf, columns)].items@
            .update(event_start(leaf, columns), event_interval(leaf, columns))
        && (forall|line: int| 0 <= line < 64 && line != event_line(leaf, columns)
            ==> #[trigger] after[line] == before[line])
}

pub(super) open spec fn seed_write(
    row_before: Seq<IntervalBucket>, row_after: Seq<IntervalBucket>,
    col_before: Seq<IntervalBucket>, col_after: Seq<IntervalBucket>, leaf: QuadLeaf64, result: Result<(), SparseQuadError>,
) -> bool {
    match result {
        Ok(()) => axis::leaf_valid(leaf)
            && !axis::bit(row_before[leaf.v as int].starts, leaf.u as u64)
            && !axis::bit(col_before[leaf.u as int].starts, leaf.v as u64)
            && bucket_insert(row_before, row_after, leaf, false)
            && bucket_insert(col_before, col_after, leaf, true),
        Err(SparseQuadError::Overlap) => axis::leaf_valid(leaf)
            && (axis::bit(row_before[leaf.v as int].starts, leaf.u as u64)
                || axis::bit(col_before[leaf.u as int].starts, leaf.v as u64)),
        Err(error) => axis::leaf_checked(leaf, Err(error)) && row_after == row_before && col_after == col_before,
    }
}

pub(super) open spec fn origin_write(before: Seq<IntervalBucket>, after: Seq<IntervalBucket>, leaf: QuadLeaf64, columns: bool, result: Result<(), SparseQuadError>) -> bool {
    match result {
        Ok(()) => !axis::bit(before[event_line(leaf, columns)].starts, event_start(leaf, columns) as u64)
            && bucket_insert(before, after, leaf, columns),
        Err(SparseQuadError::Overlap) => axis::bit(before[event_line(leaf, columns)].starts, event_start(leaf, columns) as u64)
            ,
        _ => false,
    }
}

pub(super) proof fn origin_inserted(before: Seq<IntervalBucket>, after: Seq<IntervalBucket>, leaf: QuadLeaf64, columns: bool, result: Result<(), SparseQuadError>)
    requires before.len() == 64, after.len() == 64, axis::leaf_valid(leaf),
        forall|line: int| 0 <= line < 64 ==> #[trigger] before[line].valid(),
        after[event_line(leaf, columns)].valid(),
        forall|line: int| 0 <= line < 64 && line != event_line(leaf, columns) ==> #[trigger] after[line] == before[line],
        match result {
            Ok(()) => !axis::bit(before[event_line(leaf, columns)].starts, event_start(leaf, columns) as u64)
                && after[event_line(leaf, columns)].starts == before[event_line(leaf, columns)].starts | (1u64 << event_start(leaf, columns) as u64)
                && after[event_line(leaf, columns)].items@ == before[event_line(leaf, columns)].items@.update(event_start(leaf, columns), event_interval(leaf, columns)),
            Err(SparseQuadError::Overlap) => axis::bit(before[event_line(leaf, columns)].starts, event_start(leaf, columns) as u64)
                ,
            _ => false,
        },
    ensures origin_write(before, after, leaf, columns, result),
        forall|line: int| 0 <= line < 64 ==> #[trigger] after[line].valid(),
{
    assert forall|line: int| 0 <= line < 64 implies #[trigger] after[line].valid() by {
        if line != event_line(leaf, columns) { assert(after[line] == before[line]); }
    }
}

pub(super) proof fn bit_test(mask: u64, slot: u8)
    requires slot < 64,
    ensures axis::bit(mask, slot as u64) == (mask & (1u64 << slot as u64) != 0),
{
    assert(((mask >> slot as u64) & 1 == 1) == (mask & (1u64 << slot as u64) != 0))
        by(bit_vector) requires slot < 64;
}

proof fn insert_bit(before: u64, after: u64, at: u64, slot: u64)
    requires at < 64, slot < 64, after == before | (1u64 << at),
    ensures axis::bit(after, slot) == (axis::bit(before, slot) || slot == at),
{
    assert(((after >> slot) & 1 == 1) == (((before >> slot) & 1 == 1) || slot == at))
        by(bit_vector) requires at < 64, slot < 64, after == before | (1u64 << at);
}

proof fn origin_free(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, leaf: QuadLeaf64, columns: bool)
    requires seeded(leaves, buckets, columns), axis::leaf_valid(leaf),
        !axis::bit(buckets[event_line(leaf, columns)].starts, event_start(leaf, columns) as u64),
    ensures distinct_origins(leaves.push(leaf)),
{
    assert forall|i: int| 0 <= i < leaves.len() implies leaves[i].u != leaf.u || leaves[i].v != leaf.v by {
        if leaves[i].u == leaf.u && leaves[i].v == leaf.v {
            assert(event_line(leaves[i], columns) == event_line(leaf, columns));
            assert(event_start(leaves[i], columns) == event_start(leaf, columns));
            assert(exists|i: int| 0 <= i < leaves.len()
                && event_line(leaves[i], columns) == event_line(leaf, columns)
                && event_start(leaves[i], columns) == event_start(leaf, columns));
        }
    }
}

proof fn seeded_insert(leaves: Seq<QuadLeaf64>, before: Seq<IntervalBucket>, after: Seq<IntervalBucket>, leaf: QuadLeaf64, columns: bool)
    requires seeded(leaves, before, columns), bucket_insert(before, after, leaf, columns),
        !axis::bit(before[event_line(leaf, columns)].starts, event_start(leaf, columns) as u64),
        forall|line: int| 0 <= line < 64 ==> #[trigger] after[line].valid(),
    ensures seeded(leaves.push(leaf), after, columns),
{
    origin_free(leaves, before, leaf, columns);
    assert forall|line: int, slot: u64| 0 <= line < 64 && slot < 64
        implies #[trigger] axis::bit(after[line].starts, slot)
            == exists|i: int| 0 <= i < leaves.push(leaf).len()
                && event_line(leaves.push(leaf)[i], columns) == line && event_start(leaves.push(leaf)[i], columns) == slot by {
        if line == event_line(leaf, columns) {
            insert_bit(before[line].starts, after[line].starts, event_start(leaf, columns) as u64, slot);
        }
        if exists|i: int| 0 <= i < leaves.len()
            && event_line(leaves[i], columns) == line && event_start(leaves[i], columns) == slot {
            let i = choose|i: int| 0 <= i < leaves.len()
                && event_line(leaves[i], columns) == line && event_start(leaves[i], columns) == slot;
            assert(leaves.push(leaf)[i] == leaves[i]);
        }
        if line == event_line(leaf, columns) && slot == event_start(leaf, columns) {
            assert(leaves.push(leaf)[leaves.len() as int] == leaf);
        }
        if exists|i: int| 0 <= i < leaves.push(leaf).len()
            && event_line(leaves.push(leaf)[i], columns) == line && event_start(leaves.push(leaf)[i], columns) == slot {
            let i = choose|i: int| 0 <= i < leaves.push(leaf).len()
                && event_line(leaves.push(leaf)[i], columns) == line && event_start(leaves.push(leaf)[i], columns) == slot;
            if i < leaves.len() {
                assert(leaves.push(leaf)[i] == leaves[i]);
                assert(axis::bit(before[line].starts, slot));
            }
        }
    }
    assert forall|i: int| 0 <= i < leaves.push(leaf).len() implies #[trigger]
        after[event_line(leaves.push(leaf)[i], columns)].items@[event_start(leaves.push(leaf)[i], columns)]
            == event_interval(leaves.push(leaf)[i], columns) by {
        if i < leaves.len() {
            let old_leaf = leaves[i];
            assert(axis::leaf_valid(old_leaf));
            assert(before[event_line(old_leaf, columns)].items@[event_start(old_leaf, columns)]
                == event_interval(old_leaf, columns));
            assert(event_line(old_leaf, columns) != event_line(leaf, columns)
                || event_start(old_leaf, columns) != event_start(leaf, columns));
            if event_line(old_leaf, columns) != event_line(leaf, columns) {
                assert(after[event_line(old_leaf, columns)] == before[event_line(old_leaf, columns)]);
            } else {
                assert(after[event_line(leaf, columns)].items@[event_start(old_leaf, columns)]
                    == before[event_line(leaf, columns)].items@[event_start(old_leaf, columns)]);
            }
        } else {
            assert(leaves.push(leaf)[i] == leaf);
            assert(after[event_line(leaf, columns)].items@[event_start(leaf, columns)] == event_interval(leaf, columns));
        }
    }
}

proof fn origin_overlap(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, leaf: QuadLeaf64, columns: bool)
    requires seeded(leaves, buckets, columns), axis::leaf_valid(leaf),
        axis::bit(buckets[event_line(leaf, columns)].starts, event_start(leaf, columns) as u64),
    ensures checked(leaves.push(leaf), Err(SparseQuadError::Overlap)),
        failure::seed_leaf(leaves, leaf, SparseQuadError::Overlap),
{
    let i = choose|i: int| 0 <= i < leaves.len()
        && event_line(leaves[i], columns) == event_line(leaf, columns)
        && event_start(leaves[i], columns) == event_start(leaf, columns);
    let p = origin(leaf);
    assert(covers(leaves.push(leaf)[i], p));
    assert(covers(leaves.push(leaf)[leaves.len() as int], p));
    assert(leaves[i].u == leaf.u && leaves[i].v == leaf.v);
}

pub(super) open spec fn seed_context(
    row_before: Seq<IntervalBucket>, row_after: Seq<IntervalBucket>,
    col_before: Seq<IntervalBucket>, col_after: Seq<IntervalBucket>, leaf: QuadLeaf64, result: Result<(), SparseQuadError>,
) -> bool {
    forall|prefix: Seq<QuadLeaf64>| #![trigger seeded(prefix, row_before, false), seeded(prefix, col_before, true)]
        seeded(prefix, row_before, false) && seeded(prefix, col_before, true)
        ==> match result {
            Ok(()) => seeded(prefix.push(leaf), row_after, false) && seeded(prefix.push(leaf), col_after, true),
            Err(error) => checked(prefix.push(leaf), Err(error)) && failure::seed_leaf(prefix, leaf, error),
        }
}

pub(super) proof fn seed_frontiers(
    row_before: Seq<IntervalBucket>, row_after: Seq<IntervalBucket>,
    col_before: Seq<IntervalBucket>, col_after: Seq<IntervalBucket>, leaf: QuadLeaf64, result: Result<(), SparseQuadError>,
)
    requires seed_write(row_before, row_after, col_before, col_after, leaf, result),
        row_after.len() == 64, col_after.len() == 64,
        forall|line: int| 0 <= line < 64 ==> #[trigger] row_after[line].valid(),
        forall|line: int| 0 <= line < 64 ==> #[trigger] col_after[line].valid(),
    ensures seed_context(row_before, row_after, col_before, col_after, leaf, result),
{
    assert forall|prefix: Seq<QuadLeaf64>| #![trigger seeded(prefix, row_before, false), seeded(prefix, col_before, true)]
        seeded(prefix, row_before, false) && seeded(prefix, col_before, true)
        implies match result {
            Ok(()) => seeded(prefix.push(leaf), row_after, false) && seeded(prefix.push(leaf), col_after, true),
            Err(error) => checked(prefix.push(leaf), Err(error)) && failure::seed_leaf(prefix, leaf, error),
        } by {
        match result {
            Ok(()) => {
                seeded_insert(prefix, row_before, row_after, leaf, false);
                seeded_insert(prefix, col_before, col_after, leaf, true);
            },
            Err(SparseQuadError::Overlap) => {
                if axis::bit(row_before[leaf.v as int].starts, leaf.u as u64) {
                    origin_overlap(prefix, row_before, leaf, false);
                } else { origin_overlap(prefix, col_before, leaf, true); }
            },
            Err(error) => {
                assert(axis::leaf_checked(prefix.push(leaf)[prefix.len() as int], Err(error)));
            },
        }
    }
}

pub(super) proof fn unit_origins_valid(leaves: Seq<QuadLeaf64>)
    requires distinct_origins(leaves), forall|i: int| 0 <= i < leaves.len() ==> #[trigger] leaves[i].lod == 0,
    ensures valid(leaves),
{
    assert forall|i: int, j: int, p: int| 0 <= i < j < leaves.len() && #[trigger] covers(leaves[i], p)
        implies !#[trigger] covers(leaves[j], p) by {
        if covers(leaves[j], p) {
            assert(p % 64 == leaves[i].u && p / 64 == leaves[i].v);
            assert(p % 64 == leaves[j].u && p / 64 == leaves[j].v);
        }
    }
}

proof fn unit_event(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, columns: bool, line: int, slot: u64)
    requires seeded(leaves, buckets, columns), 0 <= line < 64, slot < 64, axis::bit(buckets[line].starts, slot),
        forall|i: int| 0 <= i < leaves.len() ==> #[trigger] leaves[i].lod == 0,
    ensures buckets[line].items@[slot as int].start == slot, buckets[line].items@[slot as int].end == slot + 1,
{
    let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] event_line(leaves[i], columns) == line
        && event_start(leaves[i], columns) == slot;
    assert(buckets[line].items@[slot as int] == event_interval(leaves[i], columns));
}

pub(super) proof fn seeded_units(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, columns: bool)
    requires seeded(leaves, buckets, columns),
        forall|i: int| 0 <= i < leaves.len() ==> #[trigger] leaves[i].lod == 0,
    ensures valid(leaves), forall|line: int| 0 <= line < 64 ==> #[trigger] buckets[line].disjoint(),
        forall|line: int, pos: int| 0 <= line < 64 && 0 <= pos < 64 ==> #[trigger] axis::bucket_value(&buckets[line], pos)
            == value_at(leaves, if columns { pos * 64 + line } else { line * 64 + pos }),
{
    unit_origins_valid(leaves);
    assert forall|line: int| 0 <= line < 64 implies #[trigger] buckets[line].disjoint() by {
        assert forall|p: u64, q: u64| p < q < 64 && #[trigger] axis::bit(buckets[line].starts, p)
            && #[trigger] axis::bit(buckets[line].starts, q) implies buckets[line].items@[p as int].end <= q by {
            unit_event(leaves, buckets, columns, line, p);
        }
    }
    assert forall|line: int, pos: int| 0 <= line < 64 && 0 <= pos < 64 implies #[trigger] axis::bucket_value(&buckets[line], pos)
        == value_at(leaves, if columns { pos * 64 + line } else { line * 64 + pos }) by {
        let p = if columns { pos * 64 + line } else { line * 64 + pos };
        assert(p / 64 == if columns { pos } else { line }) by(nonlinear_arith)
            requires 0 <= line < 64, 0 <= pos < 64, p == if columns { pos * 64 + line } else { line * 64 + pos };
        assert(p % 64 == if columns { line } else { pos }) by(nonlinear_arith)
            requires 0 <= line < 64, 0 <= pos < 64, p == if columns { pos * 64 + line } else { line * 64 + pos };
        let mask = buckets[line].starts;
        assert(mask & !mask == 0) by(bit_vector);
        if exists|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p) {
            let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p);
            selected_value(leaves, p, i);
            assert(event_line(leaves[i], columns) == line && event_start(leaves[i], columns) == pos);
            assert(axis::bit(buckets[line].starts, pos as u64));
            axis::masked_at(&buckets[line], buckets[line].starts, pos as u64, pos);
        } else {
            assert forall|slot: u64| slot < 64 && #[trigger] axis::bit(buckets[line].starts, slot)
                implies !(buckets[line].items@[slot as int].start <= pos < buckets[line].items@[slot as int].end) by {
                unit_event(leaves, buckets, columns, line, slot);
                if buckets[line].items@[slot as int].start <= pos < buckets[line].items@[slot as int].end {
                    let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] event_line(leaves[i], columns) == line
                        && event_start(leaves[i], columns) == slot;
                    assert(covers(leaves[i], p));
                }
            }
        }
    }
}

pub(super) proof fn unit_axis_completed(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, output: &super::AxisIntervals, columns: bool)
    requires seeded(leaves, buckets, columns), axis::complete_progress(buckets, output, 64),
        forall|i: int| 0 <= i < leaves.len() ==> #[trigger] leaves[i].lod == 0,
    ensures valid(leaves), output.represents(labels(leaves), columns), output.boundaries_complete(),
{
    seeded_units(leaves, buckets, columns);
    assert forall|line: int, pos: int| 0 <= line < 64 && 0 <= pos < 64 implies
        #[trigger] axis::line_value(output.line_spec(line), pos)
            == labels(leaves)[if columns { pos * 64 + line } else { line * 64 + pos }] by {
        assert(axis::bucket_value(&buckets[line], pos)
            == value_at(leaves, if columns { pos * 64 + line } else { line * 64 + pos }));
    }
}

pub(super) open spec fn event_id(leaf: QuadLeaf64, columns: bool) -> int {
    event_line(leaf, columns) * 64 + event_start(leaf, columns)
}

proof fn seeded_event(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, columns: bool, i: int)
    requires seeded(leaves, buckets, columns), 0 <= i < leaves.len(),
    ensures events::present(buckets, event_id(leaves[i], columns)),
        event_id(leaves[i], columns) / 64 == event_line(leaves[i], columns),
        event_id(leaves[i], columns) % 64 == event_start(leaves[i], columns),
        buckets[event_line(leaves[i], columns)].items@[event_start(leaves[i], columns)] == event_interval(leaves[i], columns),
        events::end_line(buckets, event_id(leaves[i], columns)) == event_line(leaves[i], columns) + crate::geometry::side(leaves[i].lod),
{
    let leaf = leaves[i];
    assert(axis::leaf_valid(leaf));
    let line = event_line(leaf, columns);
    let slot = event_start(leaf, columns);
    assert(event_id(leaf, columns) / 64 == line && event_id(leaf, columns) % 64 == slot) by(nonlinear_arith)
        requires 0 <= line < 64, 0 <= slot < 64, event_id(leaf, columns) == line * 64 + slot;
    assert(axis::bit(buckets[line].starts, slot as u64));
}

proof fn event_origin(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, columns: bool, id: int)
    requires seeded(leaves, buckets, columns), events::present(buckets, id),
    ensures exists|i: int| 0 <= i < leaves.len() && #[trigger] event_id(leaves[i], columns) == id,
{
    let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] event_line(leaves[i], columns) == id / 64
        && event_start(leaves[i], columns) == id % 64;
    assert(event_id(leaves[i], columns) == id);
}

pub(super) proof fn seeded_domain(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, columns: bool)
    requires seeded(leaves, buckets, columns),
    ensures events::domain(buckets),
{
    assert forall|id: int| #[trigger] events::present(buckets, id) implies events::end_line(buckets, id) <= 64 by {
        event_origin(leaves, buckets, columns, id);
        let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] event_id(leaves[i], columns) == id;
        seeded_event(leaves, buckets, columns, i);
        assert(axis::leaf_valid(leaves[i]));
    }
}

proof fn grid_coordinates(columns: bool, line: int, pos: int)
    requires 0 <= line < 64, 0 <= pos < 64,
    ensures ({let p = if columns { pos * 64 + line } else { line * 64 + pos };
        0 <= p < 4096 && p / 64 == if columns { pos } else { line }
            && p % 64 == if columns { line } else { pos }}),
{
    let p = if columns { pos * 64 + line } else { line * 64 + pos };
    assert(p / 64 == if columns { pos } else { line }) by(nonlinear_arith)
        requires 0 <= line < 64, 0 <= pos < 64, p == if columns { pos * 64 + line } else { line * 64 + pos };
    assert(p % 64 == if columns { line } else { pos }) by(nonlinear_arith)
        requires 0 <= line < 64, 0 <= pos < 64, p == if columns { pos * 64 + line } else { line * 64 + pos };
}

proof fn event_cover(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, columns: bool, i: int, line: int, pos: int)
    requires seeded(leaves, buckets, columns), 0 <= i < leaves.len(), 0 <= line < 64, 0 <= pos < 64,
    ensures events::covers(buckets, event_id(leaves[i], columns), line, pos)
        <==> covers(leaves[i], if columns { pos * 64 + line } else { line * 64 + pos }),
{
    seeded_event(leaves, buckets, columns, i);
    grid_coordinates(columns, line, pos);
}

pub(super) proof fn seeded_validity(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, columns: bool)
    requires seeded(leaves, buckets, columns),
    ensures !events::overlap(buckets) <==> valid(leaves),
        events::overlap(buckets) ==> checked(leaves, Err(SparseQuadError::Overlap)),
{
    seeded_domain(leaves, buckets, columns);
    if events::overlap(buckets) {
        let (e, f, line, pos) = choose|e: int, f: int, line: int, pos: int| e != f
            && #[trigger] events::covers(buckets, e, line, pos) && #[trigger] events::covers(buckets, f, line, pos);
        events::event_bounds(buckets, e); events::event_bounds(buckets, f);
        assert(0 <= line < 64 && 0 <= pos < 64);
        event_origin(leaves, buckets, columns, e); event_origin(leaves, buckets, columns, f);
        let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] event_id(leaves[i], columns) == e;
        let j = choose|j: int| 0 <= j < leaves.len() && #[trigger] event_id(leaves[j], columns) == f;
        event_cover(leaves, buckets, columns, i, line, pos); event_cover(leaves, buckets, columns, j, line, pos);
        assert(i != j);
        let p = if columns { pos * 64 + line } else { line * 64 + pos };
        if i < j { assert(covers(leaves[i], p) && covers(leaves[j], p)); }
        else { assert(covers(leaves[j], p) && covers(leaves[i], p)); }
        assert(!valid(leaves));
        assert(checked(leaves, Err(SparseQuadError::Overlap)));
    }
    if !valid(leaves) {
        let (i, j, p) = choose|i: int, j: int, p: int| 0 <= i < j < leaves.len()
            && #[trigger] covers(leaves[i], p) && #[trigger] covers(leaves[j], p);
        let line = if columns { p % 64 } else { p / 64 };
        let pos = if columns { p / 64 } else { p % 64 };
        event_cover(leaves, buckets, columns, i, line, pos); event_cover(leaves, buckets, columns, j, line, pos);
        assert(event_id(leaves[i], columns) != event_id(leaves[j], columns));
        assert(events::overlap(buckets));
    }
}

pub(super) proof fn event_values(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, columns: bool)
    requires seeded(leaves, buckets, columns), valid(leaves),
    ensures forall|line: int, pos: int| 0 <= line < 64 && 0 <= pos < 64 ==> #[trigger] events::value_at(buckets, line, pos)
        == value_at(leaves, if columns { pos * 64 + line } else { line * 64 + pos }),
{
    seeded_validity(leaves, buckets, columns);
    assert forall|line: int, pos: int| 0 <= line < 64 && 0 <= pos < 64 implies #[trigger] events::value_at(buckets, line, pos)
        == value_at(leaves, if columns { pos * 64 + line } else { line * 64 + pos }) by {
        let p = if columns { pos * 64 + line } else { line * 64 + pos };
        grid_coordinates(columns, line, pos);
        if exists|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p) {
            let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] covers(leaves[i], p);
            event_cover(leaves, buckets, columns, i, line, pos);
            seeded_event(leaves, buckets, columns, i);
            selected_value(leaves, p, i);
            events::selected_value(buckets, event_id(leaves[i], columns), line, pos);
        } else {
            assert forall|id: int| !#[trigger] events::covers(buckets, id, line, pos) by {
                if events::covers(buckets, id, line, pos) {
                    event_origin(leaves, buckets, columns, id);
                    let i = choose|i: int| 0 <= i < leaves.len() && #[trigger] event_id(leaves[i], columns) == id;
                    event_cover(leaves, buckets, columns, i, line, pos);
                }
            }
        }
    }
}

pub(super) proof fn event_axis_completed(leaves: Seq<QuadLeaf64>, buckets: Seq<IntervalBucket>, output: &super::AxisIntervals, columns: bool)
    requires seeded(leaves, buckets, columns), !events::overlap(buckets), output.valid(), output.lines@.len() == 64,
        forall|line: int, pos: int| 0 <= line < 64 && 0 <= pos < 64 ==> #[trigger]
            axis::line_value(output.line_spec(line), pos) == events::value_at(buckets, line, pos),
    ensures valid(leaves), output.represents(labels(leaves), columns),
{
    seeded_validity(leaves, buckets, columns);
    event_values(leaves, buckets, columns);
    assert forall|line: int, pos: int| 0 <= line < 64 && 0 <= pos < 64 implies #[trigger]
        axis::line_value(output.line_spec(line), pos) == labels(leaves)[if columns { pos * 64 + line } else { line * 64 + pos }] by {
        assert(events::value_at(buckets, line, pos) == value_at(leaves, if columns { pos * 64 + line } else { line * 64 + pos }));
    }
}

#[cfg(reject_input_valid_failure)]
fn reject_input_valid_failure(
    leaves: &[QuadLeaf64], labels: &mut super::DenseLabels64, rows: &mut super::AxisIntervals,
    columns: &mut super::AxisIntervals, row_buckets: &mut [IntervalBucket; 64], column_buckets: &mut [IntervalBucket; 64],
)
    requires valid(leaves@), old(rows).valid(), old(columns).valid(),
{
    let result = super::build_axis_intervals_from_leaves(leaves, labels, rows, columns, row_buckets, column_buckets);
    assert(result is Err);
}

#[cfg(reject_input_overlap_success)]
fn reject_input_overlap_success(
    leaves: &[QuadLeaf64], labels: &mut super::DenseLabels64, rows: &mut super::AxisIntervals,
    columns: &mut super::AxisIntervals, row_buckets: &mut [IntervalBucket; 64], column_buckets: &mut [IntervalBucket; 64],
)
    requires checked(leaves@, Err(SparseQuadError::Overlap)), old(rows).valid(), old(columns).valid(),
{
    let result = super::build_axis_intervals_from_leaves(leaves, labels, rows, columns, row_buckets, column_buckets);
    assert(result is Ok);
}

#[cfg(reject_input_raster_pixel)]
fn reject_input_raster_pixel(leaves: &[QuadLeaf64], labels: &mut super::DenseLabels64, occupied: &mut [IntervalBucket; 64], p: usize)
    requires valid(leaves@), p < 4096, value_at(leaves@, p as int) != 0,
{
    let result = super::dense::rasterize_leaves(leaves, labels, occupied);
    assert(result is Ok);
    assert(labels@[p as int] == 0);
}

#[cfg(reject_input_error_history)]
fn reject_input_error_history(
    leaves: &[QuadLeaf64], labels: &mut super::DenseLabels64, rows: &mut super::AxisIntervals,
    columns: &mut super::AxisIntervals, row_buckets: &mut [IntervalBucket; 64], column_buckets: &mut [IntervalBucket; 64],
    error: SparseQuadError,
)
    requires failure::failure_for(leaves@) == Some(error), old(rows).valid(), old(columns).valid(),
{
    let result = super::build_axis_intervals_from_leaves(leaves, labels, rows, columns, row_buckets, column_buckets);
    assert(result is Err);
    assert(result->Err_0 != error);
}

}
