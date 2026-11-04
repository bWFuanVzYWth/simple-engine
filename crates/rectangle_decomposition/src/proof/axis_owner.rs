//! 轴区间有效范围、像素展开及构建阶段共用关系。

use super::{AxisIntervals, Interval, IntervalBucket, LineRange, QuadLeaf64, SparseQuadError};
use vstd::prelude::*;

verus! {

pub(super) open spec fn leaf_valid(leaf: QuadLeaf64) -> bool {
    let size = crate::geometry::side(leaf.lod);
    leaf.lod <= 6 && leaf.u % size == 0 && leaf.v % size == 0
        && leaf.u as int + size <= 64 && leaf.v as int + size <= 64
}

pub(super) open spec fn leaf_checked(leaf: QuadLeaf64, result: Result<u8, SparseQuadError>) -> bool {
    let size = crate::geometry::side(leaf.lod);
    let aligned = leaf.u % size == 0 && leaf.v % size == 0;
    let inside = leaf.u as int + size <= 64 && leaf.v as int + size <= 64;
    match result {
        Ok(actual) => leaf_valid(leaf) && actual == size,
        Err(SparseQuadError::LodOutOfRange) => leaf.lod > 6,
        Err(SparseQuadError::Misaligned) => leaf.lod <= 6 && !aligned,
        Err(SparseQuadError::OutOfBounds) => leaf.lod <= 6 && aligned && !inside,
        _ => false,
    }
}

pub(super) open spec fn interval_valid(interval: Interval) -> bool {
    interval.value != 0 && interval.start < interval.end <= 64
}

pub(super) open spec fn bit(mask: u64, p: u64) -> bool {
    (mask >> p) & 1 == 1
}

impl IntervalBucket {
    pub(super) open spec fn valid(&self) -> bool {
        forall|p: u64| p < 64 && #[trigger] bit(self.starts, p) ==>
            self.items@[p as int].start == p
                && interval_valid(self.items@[p as int])
    }

    pub(super) open spec fn disjoint(&self) -> bool {
        self.valid() && forall|p: u64, q: u64| p < q < 64
            && #[trigger] bit(self.starts, p) && #[trigger] bit(self.starts, q)
            ==> self.items@[p as int].end <= q
    }
}

pub(super) open spec fn masked_value(items: Seq<Interval>, mask: u64, position: int) -> u16 {
    if exists|p: u64| p < 64 && #[trigger] bit(mask, p)
        && items[p as int].start <= position < items[p as int].end {
        let p = choose|p: u64| p < 64 && #[trigger] bit(mask, p)
            && items[p as int].start <= position < items[p as int].end;
        items[p as int].value
    } else { 0 }
}

pub(super) open spec fn bucket_value(bucket: &IntervalBucket, position: int) -> u16 {
    masked_value(bucket.items@, bucket.starts, position)
}

pub(super) proof fn masked_at(bucket: &IntervalBucket, mask: u64, p: u64, position: int)
    requires bucket.disjoint(), mask & !bucket.starts == 0, p < 64, bit(mask, p),
        bucket.items@[p as int].start <= position < bucket.items@[p as int].end,
    ensures masked_value(bucket.items@, mask, position) == bucket.items@[p as int].value,
{
    let original = bucket.starts;
    assert(bit(original, p)) by(bit_vector)
        requires mask & !original == 0, (mask >> p) & 1 == 1;
    let q = choose|q: u64| q < 64 && #[trigger] bit(mask, q)
        && bucket.items@[q as int].start <= position < bucket.items@[q as int].end;
    assert(bit(original, q)) by(bit_vector)
        requires mask & !original == 0, (mask >> q) & 1 == 1;
    if p < q { assert(bucket.items@[p as int].end <= q); }
    else if q < p { assert(bucket.items@[q as int].end <= p); }
    assert(p == q);
}

pub(super) proof fn masked_insert(bucket: &IntervalBucket, mask: u64, p: u64, position: int)
    requires bucket.disjoint(), mask & !bucket.starts == 0, p < 64, bit(bucket.starts, p),
    ensures masked_value(bucket.items@, mask | (1u64 << p), position)
        == if bucket.items@[p as int].start <= position < bucket.items@[p as int].end
            { bucket.items@[p as int].value } else { masked_value(bucket.items@, mask, position) },
{
    let next = mask | (1u64 << p);
    let original = bucket.starts;
    assert(next & !original == 0) by(bit_vector)
        requires mask & !original == 0, (original >> p) & 1 == 1, p < 64,
            next == mask | (1u64 << p);
    assert(bit(next, p)) by(bit_vector) requires p < 64, next == mask | (1u64 << p);
    if bucket.items@[p as int].start <= position < bucket.items@[p as int].end {
        masked_at(bucket, next, p, position);
    } else if exists|q: u64| q < 64 && #[trigger] bit(mask, q)
        && bucket.items@[q as int].start <= position < bucket.items@[q as int].end {
        let q = choose|q: u64| q < 64 && #[trigger] bit(mask, q)
            && bucket.items@[q as int].start <= position < bucket.items@[q as int].end;
        assert(bit(next, q)) by(bit_vector)
            requires (mask >> q) & 1 == 1, next == mask | (1u64 << p);
        masked_at(bucket, mask, q, position);
        masked_at(bucket, next, q, position);
    } else {
        assert forall|q: u64| q < 64 && #[trigger] bit(next, q) implies
            !(bucket.items@[q as int].start <= position < bucket.items@[q as int].end) by {
            assert(bit(mask, q) || q == p) by(bit_vector)
                requires (next >> q) & 1 == 1, next == mask | (1u64 << p), q < 64, p < 64;
        }
    }
}

pub(super) proof fn lowest(mask: u64)
    ensures vstd::std_specs::bits::u64_trailing_zeros(mask) <= 64,
        mask != 0 ==> vstd::std_specs::bits::u64_trailing_zeros(mask) < 64,
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(mask);
}

pub(super) proof fn cleared_lowest(mask: u64, slot: u64, next: u64)
    requires mask != 0, slot == vstd::std_specs::bits::u64_trailing_zeros(mask),
        next == mask & ((mask - 1) as u64),
    ensures next < mask, slot < 64, bit(mask, slot),
        forall|p: u64| p < 64 ==> (#[trigger] bit(next, p) <==> bit(mask, p) && p != slot),
        forall|p: u64| p < 64 && #[trigger] bit(next, p) ==> slot < p,
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(mask);
    assert(next < mask) by(bit_vector) requires mask != 0, next == mask & ((mask - 1) as u64);
    assert forall|p: u64| p < 64 implies #[trigger] bit(next, p) <==> bit(mask, p) && p != slot by {
        if p < slot {
            assert(!bit(mask, p));
            assert(!bit(next, p)) by(bit_vector)
                requires next == mask & ((mask - 1) as u64), (mask >> p) & 1 == 0;
        } else {
            assert(bit(next, p) <==> bit(mask, p) && p != slot) by(bit_vector)
                requires next == mask & ((mask - 1) as u64), slot <= p < 64,
                    (mask >> slot) & 1 == 1,
                    mask << ((64 - slot) as u64) == 0;
        }
    }
    assert forall|p: u64| p < 64 && #[trigger] bit(next, p) implies slot < p by {
        if p < slot { assert(!bit(mask, p)); }
    }
}

pub(super) open spec fn bucket_progress(bucket: &IntervalBucket, line: Seq<Interval>, remaining: u64) -> bool {
    bucket.disjoint() && remaining & !bucket.starts == 0 && line_valid(line)
        && line.len() <= vstd::std_specs::bits::u64_trailing_zeros(remaining)
        && (forall|q: u64, i: int| q < 64 && #[trigger] bit(remaining, q) && 0 <= i < line.len()
            ==> #[trigger] line[i].end <= q)
        && (forall|p: int| 0 <= p < 64 ==> #[trigger] line_value(line, p)
            == masked_value(bucket.items@, bucket.starts & !remaining, p))
}

pub(super) proof fn bucket_started(bucket: &IntervalBucket)
    requires bucket.disjoint(),
    ensures bucket_progress(bucket, Seq::empty(), bucket.starts),
{
    lowest(bucket.starts);
    let starts = bucket.starts;
    assert(starts & !starts == 0) by(bit_vector);
    assert forall|p: int| 0 <= p < 64 implies #[trigger] line_value(Seq::empty(), p)
        == masked_value(bucket.items@, 0, p) by {
        assert forall|q: u64| q < 64 implies !#[trigger] bit(0, q) by {
            assert((0u64 >> q) & 1 == 0) by(bit_vector);
        }
    }
}

pub(super) proof fn bucket_next(bucket: &IntervalBucket, line: Seq<Interval>, remaining: u64,
    slot: u64, next: u64)
    requires bucket_progress(bucket, line, remaining), remaining != 0,
        slot == vstd::std_specs::bits::u64_trailing_zeros(remaining),
        next == remaining & ((remaining - 1) as u64),
    ensures appendable(line, bucket.items@[slot as int]),
        bucket_progress(bucket, merged(line, bucket.items@[slot as int]), next),
        merged(line, bucket.items@[slot as int]).len() <= line.len() + 1, next < remaining,
{
    cleared_lowest(remaining, slot, next);
    lowest(next);
    let original = bucket.starts;
    assert(bit(original, slot)) by(bit_vector)
        requires remaining & !original == 0, (remaining >> slot) & 1 == 1;
    assert(appendable(line, bucket.items@[slot as int]));
    let interval = bucket.items@[slot as int];
    merge_properties(line, interval);
    let result = merged(line, interval);
    if next != 0 {
        vstd::std_specs::bits::axiom_u64_trailing_zeros(next);
        let q = vstd::std_specs::bits::u64_trailing_zeros(next) as u64;
        assert(bit(next, q));
        assert(slot < q);
    }
    assert(result.len() <= vstd::std_specs::bits::u64_trailing_zeros(next));
    assert(next & !original == 0) by(bit_vector)
        requires remaining & !original == 0, next == remaining & ((remaining - 1) as u64);
    assert forall|q: u64, i: int| q < 64 && #[trigger] bit(next, q) && 0 <= i < result.len()
        implies #[trigger] result[i].end <= q by {
        assert(slot < q);
        assert(bit(original, q)) by(bit_vector)
            requires next & !original == 0, (next >> q) & 1 == 1;
        assert(interval.end <= q);
    }
    let seen = original & !remaining;
    assert(seen & !original == 0) by(bit_vector) requires seen == original & !remaining;
    assert(original & !next == seen | (1u64 << slot)) by(bit_vector)
        requires remaining & !original == 0, next == remaining & ((remaining - 1) as u64),
            slot < 64, (remaining >> slot) & 1 == 1,
            remaining << ((64 - slot) as u64) == 0, seen == original & !remaining;
    assert forall|p: int| 0 <= p < 64 implies #[trigger] line_value(result, p)
        == masked_value(bucket.items@, original & !next, p) by {
        masked_insert(bucket, seen, slot, p);
    }
}

pub(super) proof fn bucket_empty(bucket: &IntervalBucket)
    requires bucket.starts == 0,
    ensures bucket.valid(),
{
    assert forall|p: u64| p < 64 implies !#[trigger] bit(0, p) by {
        assert((0u64 >> p) & 1 == 0) by(bit_vector);
    }
}

pub(super) proof fn bucket_items_frame(before: Seq<IntervalBucket>, after: Seq<IntervalBucket>, index: usize)
    requires before.len() == 64, after.len() == 64, index < 64,
        after[index as int].items@ == before[index as int].items@,
        forall|i: int| 0 <= i < 64 && i != index ==> #[trigger] after[i] == before[i],
    ensures forall|i: int| 0 <= i < 64 ==> #[trigger] after[i].items@ == before[i].items@,
{
    assert forall|i: int| 0 <= i < 64 implies #[trigger] after[i].items@ == before[i].items@ by {
        if i != index { assert(after[i] == before[i]); }
    }
}

pub(super) proof fn bucket_insert(
    items: Seq<Interval>, starts: u64, interval: Interval, after: &IntervalBucket,
)
    requires items.len() == 64, interval_valid(interval),
        forall|p: u64| p < 64 && #[trigger] bit(starts, p) ==>
            items[p as int].start == p && interval_valid(items[p as int]),
        after.items@ == items.update(interval.start as int, interval),
        after.starts == starts | (1u64 << interval.start as u64),
    ensures after.valid(),
{
    let slot = interval.start as u64;
    let next = after.starts;
    assert forall|p: u64| p < 64 && #[trigger] bit(after.starts, p) implies
        after.items@[p as int].start == p
            && interval_valid(after.items@[p as int]) by {
        assert(bit(next, p) <==> bit(starts, p) || p == slot) by(bit_vector)
            requires p < 64, slot < 64, next == starts | (1u64 << slot);
        if p != slot {
            assert(bit(starts, p));
            assert(after.items@[p as int] == items[p as int]);
        }
    }
}

pub(super) open spec fn line_valid(items: Seq<Interval>) -> bool {
    (forall|i: int| 0 <= i < items.len() ==> #[trigger] interval_valid(items[i]))
        && (forall|i: int, j: int| 0 <= i < j < items.len() ==>
            #[trigger] items[i].end <= #[trigger] items[j].start
            && (items[i].end == items[j].start ==> items[i].value != items[j].value))
}

pub(super) open spec fn appendable(items: Seq<Interval>, interval: Interval) -> bool {
    line_valid(items) && interval_valid(interval)
        && forall|i: int| 0 <= i < items.len() ==> #[trigger] items[i].end <= interval.start
}

pub(super) open spec fn merged(items: Seq<Interval>, interval: Interval) -> Seq<Interval> {
    if items.len() > 0 && items.last().value == interval.value
        && items.last().end == interval.start {
        items.drop_last().push(Interval {
            start: items.last().start, end: interval.end, value: interval.value,
        })
    } else { items.push(interval) }
}

pub(super) proof fn merge_shape(items: Seq<Interval>, interval: Interval)
    requires appendable(items, interval),
    ensures line_valid(merged(items, interval)),
        items.len() <= merged(items, interval).len() <= items.len() + 1,
        forall|i: int| 0 <= i < merged(items, interval).len()
            ==> #[trigger] merged(items, interval)[i].end <= interval.end,
{
    let result = merged(items, interval);
    let join = items.len() > 0 && items.last().value == interval.value
        && items.last().end == interval.start;
    if items.len() > 0 { assert(items.last() == items[items.len() - 1]); assert(interval_valid(items[items.len() - 1])); }
    let prefix = if join { items.drop_last() } else { items };
    let tail = if join { Interval { start: items.last().start, end: interval.end, value: interval.value } }
        else { interval };
    assert(result == prefix.push(tail));
    assert forall|i: int| 0 <= i < result.len() implies #[trigger] interval_valid(result[i]) by {
        if i < prefix.len() { assert(result[i] == items[i]); }
    }
    assert forall|i: int, j: int| 0 <= i < j < result.len() implies
        #[trigger] result[i].end <= #[trigger] result[j].start
        && (result[i].end == result[j].start ==> result[i].value != result[j].value) by {
        if j < prefix.len() { assert(result[j] == items[j]); }
        else if join { assert(j == items.len() - 1); }
        else if items[i].end == interval.start && items[i].value == interval.value {
            if i < items.len() - 1 {
                assert(items[i].end <= items[items.len() - 1].start);
                assert(items[i].end <= items.last().start < items.last().end <= interval.start);
            }
        }
    }
    assert(line_valid(result));
    assert forall|i: int| 0 <= i < result.len() implies #[trigger] result[i].end <= interval.end by {
        if i < prefix.len() { assert(result[i] == items[i]); }
    }
}

pub(super) proof fn merge_value(items: Seq<Interval>, interval: Interval, p: int)
    requires appendable(items, interval), 0 <= p < 64,
    ensures line_value(merged(items, interval), p)
        == if interval.start <= p < interval.end { interval.value } else { line_value(items, p) },
{
    merge_shape(items, interval);
    let result = merged(items, interval);
    let join = items.len() > 0 && items.last().value == interval.value
        && items.last().end == interval.start;
    let prefix = if join { items.drop_last() } else { items };
    let tail = if join { Interval { start: items.last().start, end: interval.end, value: interval.value } }
        else { interval };
    assert(result == prefix.push(tail));
    if join { assert(interval_valid(items[items.len() - 1])); }
    assert(tail.start <= interval.start);

        if tail.start <= p < tail.end {
            value_at(result, p, result.len() - 1);
            if !(interval.start <= p < interval.end) {
                assert(join);
                value_at(items, p, items.len() - 1);
            }
            assert(line_value(result, p) == if interval.start <= p < interval.end
                { interval.value } else { line_value(items, p) });
        } else if exists|i: int| 0 <= i < items.len()
            && #[trigger] items[i].start <= p < items[i].end {
            let i = choose|i: int| 0 <= i < items.len()
                && #[trigger] items[i].start <= p < items[i].end;
            assert(i < prefix.len());
            assert(result[i] == items[i]);
            value_at(items, p, i);
            value_at(result, p, i);
            assert(p < interval.start);
            assert(line_value(result, p) == line_value(items, p));
        } else {
            assert forall|i: int| 0 <= i < result.len() implies
                !(#[trigger] result[i].start <= p < result[i].end) by {
                if i < prefix.len() { assert(result[i] == items[i]); }
            }
            assert(!(interval.start <= p < interval.end));
            assert(line_value(items, p) == 0);
            assert(line_value(result, p) == 0);
        }
}

pub(super) proof fn merge_properties(items: Seq<Interval>, interval: Interval)
    requires appendable(items, interval),
    ensures line_valid(merged(items, interval)),
        items.len() <= merged(items, interval).len() <= items.len() + 1,
        forall|p: int| 0 <= p < 64 ==> #[trigger] line_value(merged(items, interval), p)
            == if interval.start <= p < interval.end { interval.value } else { line_value(items, p) },
        forall|i: int| 0 <= i < merged(items, interval).len()
            ==> #[trigger] merged(items, interval)[i].end <= interval.end,
{
    merge_shape(items, interval);
    assert forall|p: int| 0 <= p < 64 implies #[trigger] line_value(merged(items, interval), p)
        == if interval.start <= p < interval.end { interval.value } else { line_value(items, p) } by {
        merge_value(items, interval, p);
    }
}

pub(super) open spec fn line_value(items: Seq<Interval>, position: int) -> u16 {
    if exists|i: int| 0 <= i < items.len()
        && #[trigger] items[i].start <= position < items[i].end {
        let i = choose|i: int| 0 <= i < items.len()
            && #[trigger] items[i].start <= position < items[i].end;
        items[i].value
    } else { 0 }
}

impl AxisIntervals {
    #[verifier::type_invariant]
    pub(super) open spec fn type_invariant(&self) -> bool { self.valid() }

    pub(super) proof fn establish(tracked value: &Self)
        ensures value.valid(),
    {
        use_type_invariant(value);
    }

    pub(super) open spec fn valid(&self) -> bool {
        self.lines.valid() && self.intervals.valid() && self.lines@.len() <= 64
            && (forall|i: int| 0 <= i < self.lines@.len() ==>
                self.range_valid(#[trigger] self.lines@[i]))
    }

    pub(super) open spec fn range_valid(&self, range: LineRange) -> bool {
        range.start <= range.end <= self.intervals@.len()
            && line_valid(self.intervals@.subrange(range.start as int, range.end as int))
    }

    pub(super) open spec fn line_spec(&self, line: int) -> Seq<Interval> {
        self.intervals@.subrange(self.lines@[line].start as int, self.lines@[line].end as int)
    }

    pub(super) open spec fn boundaries_complete(&self) -> bool {
        forall|line: u64| 0 < line < 64 && !#[trigger] bit(self.boundaries, line) ==>
            self.line_spec(line as int) == self.line_spec(line as int - 1)
    }

    pub(super) open spec fn labels(&self) -> Seq<u16> {
        Seq::new(4096, |p: int| line_value(self.line_spec(p / 64), p % 64))
    }

    /// 所有线引用最大非零 runs，且展开等于同一行主序像素视图。
    pub(super) open spec fn represents(&self, labels: Seq<u16>, columns: bool) -> bool {
        self.valid() && labels.len() == 4096 && self.lines@.len() == 64
            && (forall|line: int, p: int| 0 <= line < 64 && 0 <= p < 64 ==>
                #[trigger] line_value(self.line_spec(line), p)
                    == labels[if columns { p * 64 + line } else { line * 64 + p }])
    }
}

pub(super) proof fn prefix_ranges(lines: Seq<LineRange>, before: Seq<Interval>, after: Seq<Interval>)
    requires before.len() <= after.len(), after.take(before.len() as int) == before,
        forall|i: int| 0 <= i < lines.len() ==> #[trigger] lines[i].start <= lines[i].end <= before.len()
            && line_valid(before.subrange(lines[i].start as int, lines[i].end as int)),
    ensures forall|i: int| 0 <= i < lines.len() ==>
        #[trigger] line_valid(after.subrange(lines[i].start as int, lines[i].end as int)),
        forall|i: int| 0 <= i < lines.len() ==> #[trigger] after.subrange(lines[i].start as int, lines[i].end as int)
            == before.subrange(lines[i].start as int, lines[i].end as int),
{
    assert forall|i: int| 0 <= i < lines.len() implies
        #[trigger] line_valid(after.subrange(lines[i].start as int, lines[i].end as int)) by {
        assert(after.subrange(lines[i].start as int, lines[i].end as int)
            =~= before.subrange(lines[i].start as int, lines[i].end as int));
    }
}

pub(super) proof fn keep_published(axis: &AxisIntervals, before: &AxisIntervals)
    requires before.valid(), axis.lines.valid(), axis.intervals.valid(),
        axis.lines@ == before.lines@,
        before.intervals@.len() <= axis.intervals@.len(),
        axis.intervals@.take(before.intervals@.len() as int) == before.intervals@,
    ensures axis.valid(),
{
    prefix_ranges(before.lines@, before.intervals@, axis.intervals@);
    assert forall|i: int| 0 <= i < axis.lines@.len() implies
        axis.range_valid(#[trigger] axis.lines@[i]) by {
        let range = axis.lines@[i];
        assert(before.range_valid(range));
    }
}

pub(super) proof fn keep_prefix(axis: &AxisIntervals, before: &AxisIntervals, prefix: usize)
    requires before.valid(), axis.lines.valid(), axis.intervals.valid(), axis.lines@ == before.lines@,
        prefix <= before.intervals@.len(), prefix <= axis.intervals@.len(),
        axis.intervals@.take(prefix as int) == before.intervals@.take(prefix as int),
        forall|i: int| 0 <= i < before.lines@.len() ==> #[trigger] before.lines@[i].end <= prefix,
    ensures axis.valid(),
{
    assert forall|i: int| 0 <= i < axis.lines@.len() implies axis.range_valid(#[trigger] axis.lines@[i]) by {
        let range = axis.lines@[i];
        assert(before.range_valid(range));
        assert(before.lines@[i].end <= prefix);
        assert forall|p: int| range.start <= p < range.end implies #[trigger] axis.intervals@[p] == before.intervals@[p] by {
            assert(axis.intervals@.take(prefix as int)[p] == axis.intervals@[p]);
            assert(before.intervals@.take(prefix as int)[p] == before.intervals@[p]);
        }
        assert(axis.intervals@.subrange(range.start as int, range.end as int)
            =~= before.intervals@.subrange(range.start as int, range.end as int));
    }
}

pub(super) proof fn prefix_preserves_owner(before: &AxisIntervals, prefix: usize)
    requires before.valid(), prefix <= before.intervals@.len(),
        forall|i: int| 0 <= i < before.lines@.len() ==> #[trigger] before.lines@[i].end <= prefix,
    ensures forall|after: AxisIntervals| #![trigger after.valid()]
        after.lines == before.lines && after.intervals.valid() && prefix <= after.intervals@.len()
            && after.intervals@.take(prefix as int) == before.intervals@.take(prefix as int) ==> after.valid(),
        forall|after: Seq<Interval>, i: int| #![trigger line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int))]
            prefix <= after.len() && after.take(prefix as int) == before.intervals@.take(prefix as int)
                && 0 <= i < before.lines@.len()
                ==> line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int)),
{
    assert forall|after: AxisIntervals| #![trigger after.valid()]
        after.lines == before.lines && after.intervals.valid() && prefix <= after.intervals@.len()
            && after.intervals@.take(prefix as int) == before.intervals@.take(prefix as int) implies after.valid() by {
        keep_prefix(&after, before, prefix);
    }
    assert forall|after: Seq<Interval>, i: int| #![trigger line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int))]
        prefix <= after.len() && after.take(prefix as int) == before.intervals@.take(prefix as int)
            && 0 <= i < before.lines@.len()
            implies line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int)) by {
        let range = before.lines@[i];
        assert(before.range_valid(range));
        assert(before.lines@[i].end <= prefix);
        assert forall|p: int| range.start <= p < range.end implies #[trigger] after[p] == before.intervals@[p] by {
            assert(after.take(prefix as int)[p] == after[p]);
            assert(before.intervals@.take(prefix as int)[p] == before.intervals@[p]);
        }
        assert(after.subrange(range.start as int, range.end as int)
            =~= before.intervals@.subrange(range.start as int, range.end as int));
    }
}

pub(super) proof fn publication_preserves_owner(before: &AxisIntervals, index: usize, range: LineRange)
    requires before.valid(), index < before.lines@.len(), before.range_valid(range),
    ensures forall|after: AxisIntervals| #![trigger after.valid()]
        after.lines.valid() && after.lines@ == before.lines@.update(index as int, range)
            && after.intervals == before.intervals ==> after.valid(),
        forall|lines: Seq<LineRange>, i: int| #![trigger line_valid(before.intervals@.subrange(lines[i].start as int, lines[i].end as int))]
            lines == before.lines@.update(index as int, range) && 0 <= i < lines.len()
                ==> line_valid(before.intervals@.subrange(lines[i].start as int, lines[i].end as int)),
{
    assert forall|after: AxisIntervals| #![trigger after.valid()]
        after.lines.valid() && after.lines@ == before.lines@.update(index as int, range)
            && after.intervals == before.intervals implies after.valid() by {
        assert forall|i: int| 0 <= i < after.lines@.len() implies after.range_valid(#[trigger] after.lines@[i]) by {
            if i != index { assert(after.lines@[i] == before.lines@[i]); assert(before.range_valid(before.lines@[i])); }
        }
    }
    assert forall|lines: Seq<LineRange>, i: int| #![trigger line_valid(before.intervals@.subrange(lines[i].start as int, lines[i].end as int))]
        lines == before.lines@.update(index as int, range) && 0 <= i < lines.len()
            implies line_valid(before.intervals@.subrange(lines[i].start as int, lines[i].end as int)) by {
        if i == index { assert(lines[i] == range); }
        else { assert(lines[i] == before.lines@[i]); assert(before.range_valid(before.lines@[i])); }
    }
}

pub(super) proof fn append_preserves_owner(before: &AxisIntervals, interval: Interval)
    requires before.valid(),
    ensures forall|after: AxisIntervals| #![trigger after.valid()]
        after.lines == before.lines && after.intervals.valid()
            && after.intervals@ == before.intervals@.push(interval) ==> after.valid(),
        forall|after: Seq<Interval>, i: int| #![trigger line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int))]
            after == before.intervals@.push(interval) && 0 <= i < before.lines@.len()
                ==> line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int)),
{
    assert forall|after: AxisIntervals| #![trigger after.valid()]
        after.lines == before.lines && after.intervals.valid()
            && after.intervals@ == before.intervals@.push(interval) implies after.valid() by {
        assert(after.intervals@.take(before.intervals@.len() as int) =~= before.intervals@);
        keep_published(&after, before);
    }
    assert forall|after: Seq<Interval>, i: int| #![trigger line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int))]
        after == before.intervals@.push(interval) && 0 <= i < before.lines@.len()
            implies line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int)) by {
        let range = before.lines@[i];
        assert(before.range_valid(range));
        assert(after.subrange(range.start as int, range.end as int)
            =~= before.intervals@.subrange(range.start as int, range.end as int));
    }
}

pub(super) proof fn slot_preserves_owner(before: &AxisIntervals, index: usize)
    requires before.valid(), index < before.intervals@.len(),
        forall|i: int| 0 <= i < before.lines@.len() ==> #[trigger] before.lines@[i].end <= index,
    ensures forall|after: AxisIntervals| #![trigger after.valid()]
        after.lines == before.lines && after.intervals.valid() && index < after.intervals@.len()
            && after.intervals@ == before.intervals@.update(index as int, after.intervals@[index as int]) ==> after.valid(),
        forall|after: Seq<Interval>, i: int| #![trigger line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int))]
            index < after.len() && after == before.intervals@.update(index as int, after[index as int])
                && 0 <= i < before.lines@.len()
                ==> line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int)),
{
    assert forall|after: AxisIntervals| #![trigger after.valid()]
        after.lines == before.lines && after.intervals.valid() && index < after.intervals@.len()
            && after.intervals@ == before.intervals@.update(index as int, after.intervals@[index as int]) implies after.valid() by {
        assert(after.intervals@.take(index as int) =~= before.intervals@.take(index as int));
        keep_prefix(&after, before, index);
    }
    assert forall|after: Seq<Interval>, i: int| #![trigger line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int))]
        index < after.len() && after == before.intervals@.update(index as int, after[index as int])
            && 0 <= i < before.lines@.len()
            implies line_valid(after.subrange(before.lines@[i].start as int, before.lines@[i].end as int)) by {
        let range = before.lines@[i];
        assert(before.range_valid(range));
        assert(before.lines@[i].end <= index);
        assert(after.subrange(range.start as int, range.end as int)
            =~= before.intervals@.subrange(range.start as int, range.end as int));
    }
}

pub(super) proof fn complete_step_finished(
    buckets: Seq<IntervalBucket>, before: &AxisIntervals, after: &AxisIntervals,
    line: usize, range: LineRange, previous: LineRange, new_line: Seq<Interval>,
)
    requires complete_progress(buckets, before, line as int), line < 64,
        after.valid(), after.lines@.len() == 64,
        before.intervals@.len() <= after.intervals@.len() <= before.intervals@.len() + 64,
        after.intervals@.take(before.intervals@.len() as int) == before.intervals@,
        after.lines@ == before.lines@.update(line as int, range),
        after.line_spec(line as int) == new_line,
        forall|p: int| 0 <= p < 64 ==> #[trigger] line_value(new_line, p) == bucket_value(&buckets[line as int], p),
        after.boundaries == before.boundaries || after.boundaries == before.boundaries | (1u64 << line),
        !bit(after.boundaries, line as u64) ==> range == previous,
        line > 0 ==> previous == before.lines@[line as int - 1],
    ensures complete_progress(buckets, after, line as int + 1),
{
    prefix_ranges(before.lines@, before.intervals@, after.intervals@);
    assert forall|i: int, p: int| 0 <= i <= line && 0 <= p < 64 implies
        #[trigger] line_value(after.line_spec(i), p) == bucket_value(&buckets[i], p) by {
        if i == line { assert(after.line_spec(i) == new_line); }
        else {
            let old_range = before.lines@[i];
            assert(after.lines@[i] == old_range);
            assert(after.line_spec(i) == before.line_spec(i));
        }
    }
    boundary_kept(before.boundaries, after.boundaries, line as u64);
    assert forall|i: u64| 0 < i <= line && !#[trigger] bit(after.boundaries, i)
        implies after.line_spec(i as int) == after.line_spec(i as int - 1) by {
        if i == line {
            assert(range == previous);
            assert(after.line_spec(i as int - 1) == before.line_spec(i as int - 1));
        } else {
            assert(!bit(before.boundaries, i));
            assert(after.line_spec(i as int) == before.line_spec(i as int));
            assert(after.line_spec(i as int - 1) == before.line_spec(i as int - 1));
        }
    }
}

pub(super) open spec fn complete_progress(buckets: Seq<IntervalBucket>, axis: &AxisIntervals, line: int) -> bool {
    buckets.len() == 64 && 0 <= line <= 64 && axis.valid() && axis.lines@.len() == 64
        && axis.intervals@.len() <= line * 64
        && (forall|i: int, p: int| 0 <= i < line && 0 <= p < 64 ==>
            #[trigger] line_value(axis.line_spec(i), p) == bucket_value(&buckets[i], p))
        && (forall|i: u64| 0 < i < line && !#[trigger] bit(axis.boundaries, i) ==>
            axis.line_spec(i as int) == axis.line_spec(i as int - 1))
}

pub(super) proof fn complete_started(buckets: Seq<IntervalBucket>, axis: &AxisIntervals)
    requires buckets.len() == 64, axis.valid(), axis.lines@.len() == 64, axis.intervals@.len() == 0,
    ensures complete_progress(buckets, axis, 0),
{}

pub(super) proof fn boundary_kept(before: u64, after: u64, line: u64)
    requires line < 64, after == before || after == before | (1u64 << line),
    ensures forall|i: u64| i < line ==> (#[trigger] bit(before, i) <==> bit(after, i)),
        after == before | (1u64 << line) ==> bit(after, line),
{
    assert(after == before | (1u64 << line) ==> bit(after, line)) by(bit_vector)
        requires line < 64;
    assert forall|i: u64| i < line implies (#[trigger] bit(before, i) <==> bit(after, i)) by {
        assert(bit(before, i) <==> bit(after, i)) by(bit_vector)
            requires i < line < 64, after == before || after == before | (1u64 << line);
    }
}

pub(super) proof fn empty_line()
    ensures line_valid(Seq::<Interval>::empty()),
{}

pub(super) proof fn empty_axis(axis: &AxisIntervals)
    requires axis.lines.valid(), axis.intervals.valid(), axis.lines@.len() == 0,
    ensures axis.valid(),
{}

pub(super) proof fn cleared_axis(axis: &AxisIntervals)
    requires axis.lines.valid(), axis.intervals.valid(), axis.intervals@.len() == 0,
        axis.lines@ == Seq::new(64, |_: int| LineRange { start: 0, end: 0 }),
    ensures axis.valid(), axis.lines@.len() == 64,
{
    empty_line();
    assert forall|i: int| 0 <= i < axis.lines@.len() implies
        axis.range_valid(#[trigger] axis.lines@[i]) by {
        assert(axis.intervals@.subrange(0, 0) =~= Seq::<Interval>::empty());
    }
}

pub(super) proof fn unique_interval(items: Seq<Interval>, position: int, i: int, j: int)
    requires line_valid(items), 0 <= i < items.len(), 0 <= j < items.len(),
        items[i].start <= position < items[i].end,
        items[j].start <= position < items[j].end,
    ensures i == j,
{}

pub(super) proof fn value_at(items: Seq<Interval>, position: int, i: int)
    requires line_valid(items), 0 <= i < items.len(),
        items[i].start <= position < items[i].end,
    ensures line_value(items, position) == items[i].value,
{
    let j = choose|j: int| 0 <= j < items.len()
        && #[trigger] items[j].start <= position < items[j].end;
    unique_interval(items, position, i, j);
}

}
