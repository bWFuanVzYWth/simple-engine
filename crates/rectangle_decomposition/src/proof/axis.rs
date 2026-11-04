//! 行比较、最大非零区间及两轴逐像素对应。

use super::*;

verus! {

impl View for DenseLabels64 {
    type V = Seq<u16>;

    /// 直接借用现有嵌套数组的行主序数学视图，不创建另一份像素存储。
    open(super) spec fn view(&self) -> Seq<u16> {
        Seq::new(4096, |p: int| self.rows@[p / 64]@[p % 64])
    }
}

pub(super) proof fn label_layout(labels: &DenseLabels64)
    ensures labels@.len() == 4096,
        forall|p: int| 0 <= p < 4096 ==> #[trigger] labels@[p] == labels.rows@[p / 64]@[p % 64],
        forall|y: int, x: int| 0 <= y < 64 && 0 <= x < 64
            ==> #[trigger] labels.rows@[y]@[x] == labels@[y * 64 + x],
{
    assert forall|y: int, x: int| 0 <= y < 64 && 0 <= x < 64
        implies #[trigger] labels.rows@[y]@[x] == labels@[y * 64 + x] by {
        assert((y * 64 + x) / 64 == y && (y * 64 + x) % 64 == x) by(nonlinear_arith)
            requires 0 <= y < 64, 0 <= x < 64;
    }
}

pub(super) proof fn empty_labels(labels: &DenseLabels64)
    requires forall|p: int| 0 <= p < 4096 ==> #[trigger] labels.rows@[p / 64]@[p % 64] == 0,
    ensures labels@ == Seq::new(4096, |_: int| 0u16),
{
    assert(labels@ =~= Seq::new(4096, |_: int| 0u16));
}

pub(super) proof fn label_row(labels: &DenseLabels64, line: int)
    requires 0 <= line < 64,
    ensures labels@.len() == 4096,
        labels@.subrange(line * 64, (line + 1) * 64) == labels.rows@[line]@,
{
    assert forall|p: int| 0 <= p < 64 implies
        labels@[line * 64 + p] == labels.rows@[line]@[p] by {
        assert((line * 64 + p) / 64 == line && (line * 64 + p) % 64 == p) by(nonlinear_arith)
            requires 0 <= line < 64, 0 <= p < 64;
    }
    assert(labels@.subrange(line * 64, (line + 1) * 64) =~= labels.rows@[line]@);
}

pub(super) open spec fn block_bits(a: Seq<u16>, b: Seq<u16>, mask: u64) -> bool {
    a.len() == 32 && b.len() == 32 && mask >> 32u64 == 0
        && forall|bit: u64| bit < 64 ==> #[trigger] ((mask >> bit) & 1)
            == if bit < 32 && a[bit as int] != b[bit as int] { 1u64 } else { 0u64 }
}

pub(super) open spec fn between_rows(a: Seq<u16>, b: Seq<u16>, mask: u64) -> bool {
    a.len() == 64 && b.len() == 64
        && forall|bit: u64| bit < 64 ==> #[trigger] ((mask >> bit) & 1)
            == if a[bit as int] != b[bit as int] { 1u64 } else { 0u64 }
}

pub(super) open spec fn within_row(row: Seq<u16>, mask: u64) -> bool {
    row.len() == 64
        && forall|bit: u64| bit < 64 ==> #[trigger] ((mask >> bit) & 1)
            == if 0 < bit && row[bit as int] != row[bit as int - 1] { 1u64 } else { 0u64 }
}

pub(super) proof fn join_rows(a: Seq<u16>, b: Seq<u16>, low: u64, high: u64, mask: u64)
    requires a.len() == 64, b.len() == 64,
        block_bits(a.subrange(0, 32), b.subrange(0, 32), low),
        block_bits(a.subrange(32, 64), b.subrange(32, 64), high),
        mask == low | (high << 32),
    ensures between_rows(a, b, mask),
{
    assert forall|bit: u64| bit < 64 implies #[trigger] ((mask >> bit) & 1)
        == if a[bit as int] != b[bit as int] { 1u64 } else { 0u64 } by {
        if bit < 32 {
            assert(((low >> bit) & 1) == if a[bit as int] != b[bit as int] { 1u64 } else { 0u64 });
            assert(((mask >> bit) & 1) == ((low >> bit) & 1)) by(bit_vector)
                requires mask == low | (high << 32), bit < 32;
        } else {
            let lane = (bit - 32) as u64;
            assert(lane < 32);
            assert(((high >> lane) & 1) == if a[bit as int] != b[bit as int] { 1u64 } else { 0u64 });
            assert(((mask >> bit) & 1) == ((high >> ((bit - 32) as u64)) & 1)) by(bit_vector)
                requires mask == low | (high << 32), low >> 32u64 == 0, 32 <= bit < 64;
        }
    }
}

pub(super) proof fn join_boundaries(row: Seq<u16>, low: u64, high: u64, mask: u64)
    requires row.len() == 64,
        block_bits(row.subrange(1, 33), row.subrange(0, 32), low),
        block_bits(row.subrange(32, 64), row.subrange(31, 63), high),
        mask == (low << 1) | (high << 32),
    ensures within_row(row, mask),
{
    assert forall|bit: u64| bit < 64 implies #[trigger] ((mask >> bit) & 1)
        == if 0 < bit && row[bit as int] != row[bit as int - 1] { 1u64 } else { 0u64 } by {
        if bit == 0 {
            assert(((mask >> bit) & 1) == 0) by(bit_vector)
                requires bit == 0, mask == (low << 1) | (high << 32);
        } else if bit < 32 {
            let lane = (bit - 1) as u64;
            assert(lane < 32);
            assert(((low >> lane) & 1)
                == if row[bit as int] != row[bit as int - 1] { 1u64 } else { 0u64 });
            assert(((mask >> bit) & 1) == ((low >> ((bit - 1) as u64)) & 1)) by(bit_vector)
                requires mask == (low << 1) | (high << 32), 0 < bit < 32;
        } else if bit == 32 {
            assert(((low >> 31u64) & 1)
                == if row[32] != row[31] { 1u64 } else { 0u64 });
            assert(((high >> 0u64) & 1) == if row[32] != row[31] { 1u64 } else { 0u64 });
            assert((high >> 0u64) & 1 == high & 1) by(bit_vector);
            assert(((mask >> 32u64) & 1) == (((low >> 31u64) & 1) | (high & 1))) by(bit_vector)
                requires mask == (low << 1) | (high << 32);
            assert((((low >> 31u64) & 1) | (high & 1)) == (high & 1)) by(bit_vector)
                requires ((low >> 31u64) & 1) == (high & 1);
        } else {
            let lane = (bit - 32) as u64;
            assert(lane < 32);
            assert(((high >> lane) & 1)
                == if row[bit as int] != row[bit as int - 1] { 1u64 } else { 0u64 });
            assert(((mask >> bit) & 1) == ((high >> ((bit - 32) as u64)) & 1)) by(bit_vector)
                requires mask == (low << 1) | (high << 32), low >> 32u64 == 0, 32 < bit < 64;
        }
    }
}

pub(super) open spec fn first_bit(mask: u64, position: u8) -> bool {
    position <= 64 && (mask == 0 <==> position == 64)
        && (position < 64 ==> (mask >> position) & 1 == 1)
        && (forall|bit: u64| bit < position ==> #[trigger] ((mask >> bit) & 1) == 0)
        && mask << ((64 - position as u64) as u64) == 0
}

pub(super) open spec fn remaining(row: Seq<u16>, left: u8, mask: u64) -> bool {
    row.len() == 64 && left < 64
        && (left == 0 || row[left as int - 1] != row[left as int])
        && forall|bit: u64| bit < 64 ==> #[trigger] ((mask >> bit) & 1)
            == if left < bit && row[bit as int] != row[bit as int - 1] { 1u64 } else { 0u64 }
}

proof fn constant_span(row: Seq<u16>, left: int, end: int)
    requires row.len() == 64, 0 <= left < end <= 64,
        forall|p: int| left < p < end ==> #[trigger] row[p] == row[p - 1],
    ensures forall|p: int| left <= p < end ==> #[trigger] row[p] == row[left],
    decreases end - left,
{
    if left + 1 < end {
        constant_span(row, left, end - 1);
        assert(row[end - 1] == row[end - 2]);
    }
}

pub(super) proof fn next_segment(row: Seq<u16>, left: u8, mask: u64, right: u8)
    requires remaining(row, left, mask), first_bit(mask, right),
    ensures left < right <= 64,
        forall|p: int| left <= p < right ==> #[trigger] row[p] == row[left as int],
        right == 64 || row[right as int - 1] != row[right as int],
{
    if mask != 0 && right <= left {
        assert((mask >> right) & 1 == 0);
    }
    assert forall|p: int| left < p < right implies #[trigger] row[p] == row[p - 1] by {
        let bit = p as u64;
        assert((mask >> bit) & 1 == 0);
    }
    constant_span(row, left as int, right as int);
    if right < 64 {
        assert((mask >> (right as u64)) & 1 == 1);
    }
}

pub(super) proof fn advance_boundary(row: Seq<u16>, left: u8, before: u64, right: u8, after: u64)
    requires remaining(row, left, before), first_bit(before, right), before != 0,
        after == before & ((before - 1) as u64),
    ensures remaining(row, right, after),
{
    next_segment(row, left, before, right);
    assert forall|bit: u64| bit < 64 implies #[trigger] ((after >> bit) & 1)
        == if right < bit && row[bit as int] != row[bit as int - 1] { 1u64 } else { 0u64 } by {
        if bit < right {
            assert((before >> bit) & 1 == 0);
            assert((after >> bit) & 1 == 0) by(bit_vector)
                requires after == before & ((before - 1) as u64), (before >> bit) & 1 == 0;
        } else if bit == right {
            assert((after >> bit) & 1 == 0) by(bit_vector)
                requires right < 64, bit == right,
                    before << ((64 - right as u64) as u64) == 0,
                    after == before & ((before - 1) as u64);
        } else {
            assert((after >> bit) & 1 == (before >> bit) & 1) by(bit_vector)
                requires right < bit < 64, (before >> right) & 1 == 1,
                    after == before & ((before - 1) as u64);
            assert(((before >> bit) & 1)
                == if row[bit as int] != row[bit as int - 1] { 1u64 } else { 0u64 });
        }
    }
}

pub(super) open spec fn row_prefix(row: Seq<u16>, items: Seq<Interval>, end: u8) -> bool {
    row.len() == 64 && end <= 64 && items.len() <= end
        && axis_owner::line_valid(items)
        && (forall|i: int| 0 <= i < items.len() ==> #[trigger] items[i].end <= end)
        && (forall|p: int| 0 <= p < end ==> #[trigger] axis_owner::line_value(items, p) == row[p])
        && (end == 0 || end == 64 || row[end as int - 1] != row[end as int])
}

pub(super) proof fn empty_row(row: Seq<u16>)
    requires row.len() == 64,
    ensures row_prefix(row, Seq::empty(), 0),
{
    axis_owner::empty_line();
}

proof fn pushed_value(items: Seq<Interval>, item: Interval, position: int)
    requires axis_owner::line_valid(items), axis_owner::line_valid(items.push(item)),
    ensures axis_owner::line_value(items.push(item), position)
        == if item.start <= position < item.end { item.value } else { axis_owner::line_value(items, position) },
{
    let after = items.push(item);
    if item.start <= position < item.end {
        axis_owner::value_at(after, position, items.len() as int);
    } else if exists|i: int| 0 <= i < items.len() && #[trigger] items[i].start <= position < items[i].end {
        let i = choose|i: int| 0 <= i < items.len() && #[trigger] items[i].start <= position < items[i].end;
        axis_owner::value_at(items, position, i);
        axis_owner::value_at(after, position, i);
    } else {
        assert forall|i: int| 0 <= i < after.len() implies
            !(#[trigger] after[i].start <= position < after[i].end) by {
            if i < items.len() {
                assert(!(items[i].start <= position < items[i].end));
            } else {
                assert(i == items.len());
            }
        }
    }
}

pub(super) proof fn extend_row(row: Seq<u16>, items: Seq<Interval>, left: u8, right: u8, after: Seq<Interval>)
    requires row_prefix(row, items, left), left < right <= 64,
        forall|p: int| left <= p < right ==> #[trigger] row[p] == row[left as int],
        right == 64 || row[right as int - 1] != row[right as int],
        after == if row[left as int] == 0 { items } else {
            items.push(Interval { start: left, end: right, value: row[left as int] })
        },
    ensures row_prefix(row, after, right),
{
    if row[left as int] != 0 {
        let item = Interval { start: left, end: right, value: row[left as int] };
        assert forall|i: int| 0 <= i < after.len() implies #[trigger] axis_owner::interval_valid(after[i]) by {
            if i < items.len() { assert(after[i] == items[i]); }
            else { assert(after[i] == item); }
        }
        assert forall|i: int, j: int| 0 <= i < j < after.len() implies
            #[trigger] after[i].end <= #[trigger] after[j].start
            && (after[i].end == after[j].start ==> after[i].value != after[j].value) by {
            if j < items.len() {
                assert(after[i] == items[i] && after[j] == items[j]);
            } else {
                assert(j == items.len() && i < items.len());
                assert(after[j] == item && after[i] == items[i]);
                assert(axis_owner::interval_valid(items[i]));
                assert(items[i].end <= left);
                if items[i].end == left {
                    assert(left > 0 && items[i].start <= left as int - 1 < items[i].end);
                    axis_owner::value_at(items, left as int - 1, i);
                    assert(items[i].value == row[left as int - 1]);
                }
            }
        }
        assert(axis_owner::line_valid(after));
        assert forall|p: int| 0 <= p < right implies #[trigger] axis_owner::line_value(after, p) == row[p] by {
            pushed_value(items, item, p);
        }
    } else {
        assert forall|p: int| 0 <= p < right implies #[trigger] axis_owner::line_value(after, p) == row[p] by {
            if left <= p {
                assert forall|i: int| 0 <= i < items.len() implies
                    !(#[trigger] items[i].start <= p < items[i].end) by {
                    assert(items[i].end <= left);
                }
            }
        }
    }
    assert forall|i: int| 0 <= i < after.len() implies #[trigger] after[i].end <= right by {
        if i < items.len() { assert(after[i] == items[i]); }
    }
}

pub(super) open spec fn closed_before(bucket: &IntervalBucket, end: u8) -> bool {
    end <= 64 && bucket.disjoint()
        && forall|p: u64| p < 64 && #[trigger] axis_owner::bit(bucket.starts, p)
            ==> bucket.items@[p as int].end <= end
}

pub(super) proof fn free_column_start(bucket: &IntervalBucket, start: u8)
    requires closed_before(bucket, start), start < 64,
    ensures bucket.starts & (1u64 << start as u64) == 0,
{
    let mask = bucket.starts;
    if bucket.starts & (1u64 << start as u64) != 0 {
        assert(axis_owner::bit(mask, start as u64)) by(bit_vector)
            requires start < 64, mask & (1u64 << start as u64) != 0;
        assert(axis_owner::interval_valid(bucket.items@[start as int]));
        assert(bucket.items@[start as int].start == start);
        assert(bucket.items@[start as int].end <= start);
    }
}

pub(super) proof fn closed_column_insert(
    bucket: &IntervalBucket, before: &IntervalBucket, start: u8, end: u8, value: u16,
)
    requires closed_before(before, start), start < end <= 64, value != 0, bucket.valid(),
        bucket.starts == before.starts | (1u64 << start as u64),
        bucket.items@ == before.items@.update(start as int, Interval { start, end, value }),
    ensures closed_before(bucket, end),
{
    let mask = bucket.starts;
    let previous = before.starts;
    assert forall|p: u64| p < 64 && #[trigger] axis_owner::bit(bucket.starts, p)
        implies bucket.items@[p as int].end <= end by {
        assert(axis_owner::bit(mask, p) <==> axis_owner::bit(previous, p) || p == start) by(bit_vector)
            requires p < 64, start < 64, mask == previous | (1u64 << start as u64);
        if p != start {
            assert(axis_owner::bit(before.starts, p));
            assert(bucket.items@[p as int] == before.items@[p as int]);
        }
    }
    assert forall|p: u64, q: u64| p < q < 64
        && #[trigger] axis_owner::bit(bucket.starts, p) && #[trigger] axis_owner::bit(bucket.starts, q)
        implies bucket.items@[p as int].end <= q by {
        assert(axis_owner::bit(mask, p) <==> axis_owner::bit(previous, p) || p == start) by(bit_vector)
            requires p < 64, start < 64, mask == previous | (1u64 << start as u64);
        assert(axis_owner::bit(mask, q) <==> axis_owner::bit(previous, q) || q == start) by(bit_vector)
            requires q < 64, start < 64, mask == previous | (1u64 << start as u64);
        if p == start {
            assert(axis_owner::bit(before.starts, q));
            assert(axis_owner::interval_valid(before.items@[q as int]));
            assert(before.items@[q as int].start == q);
            assert(before.items@[q as int].end <= start);
        } else if q == start {
            assert(axis_owner::bit(before.starts, p));
            assert(bucket.items@[p as int] == before.items@[p as int]);
        } else {
            assert(axis_owner::bit(before.starts, p) && axis_owner::bit(before.starts, q));
            assert(bucket.items@[p as int] == before.items@[p as int]);
        }
    }
}

proof fn masked_frame(before: &IntervalBucket, after: &IntervalBucket, mask: u64, position: int)
    requires before.disjoint(), after.disjoint(), mask & !before.starts == 0, mask & !after.starts == 0,
        forall|p: u64| p < 64 && #[trigger] axis_owner::bit(mask, p)
            ==> before.items@[p as int] == after.items@[p as int],
    ensures axis_owner::masked_value(before.items@, mask, position)
        == axis_owner::masked_value(after.items@, mask, position),
{
    if exists|p: u64| p < 64 && #[trigger] axis_owner::bit(mask, p)
        && before.items@[p as int].start <= position < before.items@[p as int].end {
        let p = choose|p: u64| p < 64 && #[trigger] axis_owner::bit(mask, p)
            && before.items@[p as int].start <= position < before.items@[p as int].end;
        assert(before.items@[p as int] == after.items@[p as int]);
        axis_owner::masked_at(before, mask, p, position);
        axis_owner::masked_at(after, mask, p, position);
    } else {
        assert forall|p: u64| p < 64 && #[trigger] axis_owner::bit(mask, p) implies
            !(after.items@[p as int].start <= position < after.items@[p as int].end) by {
            assert(before.items@[p as int] == after.items@[p as int]);
        }
    }
}

pub(super) proof fn column_insert_values(
    bucket: &IntervalBucket, before: &IntervalBucket, start: u8, end: u8, value: u16,
)
    requires closed_before(before, start), start < end <= 64, value != 0, bucket.disjoint(),
        bucket.starts == before.starts | (1u64 << start as u64),
        bucket.items@ == before.items@.update(start as int, Interval { start, end, value }),
    ensures forall|p: int| 0 <= p < 64 ==> #[trigger] axis_owner::bucket_value(bucket, p)
        == if start <= p < end { value } else { axis_owner::bucket_value(before, p) },
{
    let previous = before.starts;
    let next = bucket.starts;
    assert(previous & !previous == 0) by(bit_vector);
    assert(previous & !next == 0) by(bit_vector)
        requires next == previous | (1u64 << start as u64);
    assert(axis_owner::bit(next, start as u64)) by(bit_vector)
        requires start < 64, next == previous | (1u64 << start as u64);
    assert forall|p: u64| p < 64 && #[trigger] axis_owner::bit(previous, p)
        implies before.items@[p as int] == bucket.items@[p as int] by {
        if p == start {
            assert(axis_owner::interval_valid(before.items@[p as int]));
            assert(before.items@[p as int].start == start);
            assert(before.items@[p as int].end <= start);
        }
    }
    assert forall|p: int| 0 <= p < 64 implies #[trigger] axis_owner::bucket_value(bucket, p)
        == if start <= p < end { value } else { axis_owner::bucket_value(before, p) } by {
        masked_frame(before, bucket, previous, p);
        axis_owner::masked_insert(bucket, previous, start as u64, p);
    }
}

/// 已关闭 bucket 覆盖 start 之前的列；start..end 是尚未提交的同值纵段。
pub(super) open spec fn column_frontier(
    labels: Seq<u16>, x: int, end: u8, bucket: &IntervalBucket, start: u8, value: u16,
) -> bool {
    labels.len() == 4096 && 0 <= x < 64 && 0 < end <= 64 && start < end
        && closed_before(bucket, start)
        && (forall|y: int| 0 <= y < start ==> #[trigger] axis_owner::bucket_value(bucket, y) == labels[y * 64 + x])
        && (forall|y: int| start <= y < end ==> #[trigger] labels[y * 64 + x] == value)
}

proof fn closed_background(bucket: &IntervalBucket, end: u8, position: int)
    requires closed_before(bucket, end), end <= position,
    ensures axis_owner::bucket_value(bucket, position) == 0,
{
    assert forall|p: u64| p < 64 && #[trigger] axis_owner::bit(bucket.starts, p)
        implies !(bucket.items@[p as int].start <= position < bucket.items@[p as int].end) by {
        assert(bucket.items@[p as int].end <= end);
    }
}

pub(super) proof fn initial_column(labels: Seq<u16>, x: int, bucket: &IntervalBucket, value: u16)
    requires labels.len() == 4096, 0 <= x < 64, bucket.starts == 0, value == labels[x],
    ensures column_frontier(labels, x, 1, bucket, 0, value),
{
    axis_owner::bucket_empty(bucket);
    assert forall|p: u64| p < 64 implies !#[trigger] axis_owner::bit(bucket.starts, p) by {
        assert((0u64 >> p) & 1 == 0) by(bit_vector) requires p < 64;
    }
}

pub(super) proof fn continue_column(
    labels: Seq<u16>, x: int, end: u8, bucket: &IntervalBucket, start: u8, value: u16,
)
    requires column_frontier(labels, x, end, bucket, start, value), end < 64,
        labels[end as int * 64 + x] == value,
    ensures column_frontier(labels, x, (end + 1) as u8, bucket, start, value),
{
    assert forall|y: int| start <= y < end + 1 implies #[trigger] labels[y * 64 + x] == value by {
        if y < end { assert(labels[y * 64 + x] == value); }
    }
}

proof fn commit_column(
    labels: Seq<u16>, x: int, end: u8, before: &IntervalBucket, start: u8, value: u16, after: &IntervalBucket,
)
    requires column_frontier(labels, x, end, before, start, value), closed_before(after, end),
        forall|p: int| 0 <= p < 64 ==> #[trigger] axis_owner::bucket_value(after, p)
            == if value != 0 && start <= p < end { value } else { axis_owner::bucket_value(before, p) },
    ensures forall|p: int| 0 <= p < end ==> #[trigger] axis_owner::bucket_value(after, p) == labels[p * 64 + x],
{
    assert forall|p: int| 0 <= p < end implies #[trigger] axis_owner::bucket_value(after, p) == labels[p * 64 + x] by {
        if start <= p {
            if value == 0 { closed_background(before, start, p); }
        }
    }
}

pub(super) open spec fn restart_context(
    before: &IntervalBucket, start: u8, value: u16, after: &IntervalBucket, end: u8, next: u16,
) -> bool {
    forall|labels: Seq<u16>, x: int| column_frontier(labels, x, end, before, start, value)
        && labels[end as int * 64 + x] == next
        ==> #[trigger] column_frontier(labels, x, (end + 1) as u8, after, end, next)
}

pub(super) proof fn restart_frontiers(
    before: &IntervalBucket, start: u8, value: u16, after: &IntervalBucket, end: u8, next: u16,
)
    requires start < end < 64, closed_before(after, end),
        forall|p: int| 0 <= p < 64 ==> #[trigger] axis_owner::bucket_value(after, p)
            == if value != 0 && start <= p < end { value } else { axis_owner::bucket_value(before, p) },
    ensures restart_context(before, start, value, after, end, next),
{
    assert forall|labels: Seq<u16>, x: int| column_frontier(labels, x, end, before, start, value)
        && labels[end as int * 64 + x] == next
        implies #[trigger] column_frontier(labels, x, (end + 1) as u8, after, end, next) by {
        commit_column(labels, x, end, before, start, value, after);
    }
}

pub(super) open spec fn columns_frontier(
    labels: Seq<u16>, end: u8, buckets: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>,
) -> bool {
    buckets.len() == 64 && starts.len() == 64 && values.len() == 64
        && forall|x: int| 0 <= x < 64 ==> #[trigger]
            column_frontier(labels, x, end, &buckets[x], starts[x], values[x])
}

pub(super) open spec fn column_height(y: u8, changes: u64, x: int) -> u8 {
    if axis_owner::bit(changes, x as u64) { y } else { (y + 1) as u8 }
}

pub(super) open spec fn mixed_frontiers(
    labels: Seq<u16>, y: u8, buckets: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>, changes: u64,
) -> bool {
    0 < y < 64 && buckets.len() == 64 && starts.len() == 64 && values.len() == 64
        && forall|x: int| 0 <= x < 64 ==> #[trigger]
            column_frontier(labels, x, column_height(y, changes, x),
                &buckets[x], starts[x], values[x])
}

pub(super) open spec fn pending_columns(buckets: Seq<IntervalBucket>, starts: Seq<u8>, y: u8, changes: u64) -> bool {
    0 < y < 64 && buckets.len() == 64 && starts.len() == 64
        && forall|x: u64| x < 64 && #[trigger] axis_owner::bit(changes, x)
            ==> closed_before(&buckets[x as int], starts[x as int]) && starts[x as int] < y
}

pub(super) proof fn pending_from_mixed(
    labels: Seq<u16>, y: u8, buckets: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>, changes: u64,
)
    requires mixed_frontiers(labels, y, buckets, starts, values, changes),
    ensures pending_columns(buckets, starts, y, changes),
{
    assert forall|x: u64| x < 64 && #[trigger] axis_owner::bit(changes, x)
        implies closed_before(&buckets[x as int], starts[x as int]) && starts[x as int] < y by {
        assert(column_frontier(labels, x as int, column_height(y, changes, x as int),
            &buckets[x as int], starts[x as int], values[x as int]));
        assert(column_frontier(labels, x as int, y, &buckets[x as int], starts[x as int], values[x as int]));
    }
}

pub(super) proof fn selected_column(buckets: Seq<IntervalBucket>, starts: Seq<u8>, y: u8, changes: u64, x: usize)
    requires pending_columns(buckets, starts, y, changes), changes != 0,
        x == vstd::std_specs::bits::u64_trailing_zeros(changes),
    ensures x < 64, closed_before(&buckets[x as int], starts[x as int]), starts[x as int] < y,
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(changes);
    assert(axis_owner::bit(changes, x as u64));
}

pub(super) open spec fn row_matches(labels: Seq<u16>, y: u8, row: Seq<u16>) -> bool {
    labels.len() == 4096 && y < 64 && row.len() == 64
        && forall|x: int| 0 <= x < 64 ==> #[trigger] row[x] == labels[y as int * 64 + x]
}

pub(super) open spec fn column_step_frames(
    before: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>, changes: u64,
    after: Seq<IntervalBucket>, next_starts: Seq<u8>, next_values: Seq<u16>, next_changes: u64,
    current: Seq<u16>, y: u8,
) -> bool {
    let slot = vstd::std_specs::bits::u64_trailing_zeros(changes) as int;
    before.len() == 64 && starts.len() == 64 && values.len() == 64 && current.len() == 64
        && after.len() == 64 && 0 < y < 64 && changes != 0 && 0 <= slot < 64
        && next_starts == starts.update(slot, y) && next_values == values.update(slot, current[slot])
        && next_changes == changes & ((changes - 1) as u64)
        && restart_context(&before[slot], starts[slot], values[slot], &after[slot], y, current[slot])
        && (forall|x: int| 0 <= x < 64 && x != slot ==> #[trigger] after[x] == before[x])
}

pub(super) open spec fn column_step_context(
    before: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>, changes: u64,
    after: Seq<IntervalBucket>, next_starts: Seq<u8>, next_values: Seq<u16>, next_changes: u64,
    current: Seq<u16>, y: u8,
) -> bool {
    forall|labels: Seq<u16>| mixed_frontiers(labels, y, before, starts, values, changes)
        && row_matches(labels, y, current)
        ==> #[trigger] mixed_frontiers(labels, y, after, next_starts, next_values, next_changes)
}

/// 待关闭列借用上一行，已处理列借用当前行；不需要执行期标签缓存。
pub(super) open spec fn frontier_values(current: Seq<u16>, previous: Seq<u16>, changes: u64) -> Seq<u16> {
    Seq::new(64, |x: int| if axis_owner::bit(changes, x as u64) { previous[x] } else { current[x] })
}

pub(super) proof fn initial_values(current: Seq<u16>, previous: Seq<u16>, changes: u64)
    requires current.len() == 64, previous.len() == 64, between_rows(current, previous, changes),
    ensures frontier_values(current, previous, changes) == previous,
{
    assert forall|x: int| 0 <= x < 64 implies frontier_values(current, previous, changes)[x] == previous[x] by {
        if !axis_owner::bit(changes, x as u64) {
            assert(((changes >> x as u64) & 1) == if current[x] != previous[x] { 1u64 } else { 0u64 });
        }
    }
    assert(frontier_values(current, previous, changes) =~= previous);
}

pub(super) proof fn frontier_values_step(current: Seq<u16>, previous: Seq<u16>, changes: u64, next: u64, slot: usize)
    requires current.len() == 64, previous.len() == 64, changes != 0,
        slot == vstd::std_specs::bits::u64_trailing_zeros(changes), next == changes & ((changes - 1) as u64),
    ensures slot < 64, frontier_values(current, previous, changes)[slot as int] == previous[slot as int],
        frontier_values(current, previous, next) == frontier_values(current, previous, changes).update(slot as int, current[slot as int]),
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(changes);
    axis_owner::cleared_lowest(changes, slot as u64, next);
    assert(axis_owner::bit(changes, slot as u64));
    assert forall|x: int| 0 <= x < 64 implies frontier_values(current, previous, next)[x]
        == frontier_values(current, previous, changes).update(slot as int, current[slot as int])[x] by {
        if x == slot { assert(!axis_owner::bit(next, slot as u64)); }
        else { assert(axis_owner::bit(changes, x as u64) == axis_owner::bit(next, x as u64)); }
    }
    assert(frontier_values(current, previous, next) =~= frontier_values(current, previous, changes).update(slot as int, current[slot as int]));
}

pub(super) proof fn finished_values(current: Seq<u16>, previous: Seq<u16>)
    requires current.len() == 64,
    ensures frontier_values(current, previous, 0) == current,
{
    assert forall|x: int| 0 <= x < 64 implies !#[trigger] axis_owner::bit(0, x as u64) by {
        let bit = x as u64;
        assert(((0u64 >> bit) & 1) == 0) by(bit_vector);
    }
    assert(frontier_values(current, previous, 0) =~= current);
}

pub(super) proof fn column_step_frontiers(
    before: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>, changes: u64,
    after: Seq<IntervalBucket>, next_starts: Seq<u8>, next_values: Seq<u16>, next_changes: u64,
    current: Seq<u16>, y: u8,
)
    requires pending_columns(before, starts, y, changes),
        column_step_frames(before, starts, values, changes, after, next_starts, next_values, next_changes, current, y),
        closed_before(&after[vstd::std_specs::bits::u64_trailing_zeros(changes) as int], y),
    ensures pending_columns(after, next_starts, y, next_changes),
        column_step_context(before, starts, values, changes, after, next_starts, next_values, next_changes, current, y),
{
    let slot = vstd::std_specs::bits::u64_trailing_zeros(changes) as u64;
    axis_owner::cleared_lowest(changes, slot, next_changes);
    assert forall|x: u64| x < 64 && #[trigger] axis_owner::bit(next_changes, x)
        implies closed_before(&after[x as int], next_starts[x as int]) && next_starts[x as int] < y by {
        assert(x != slot && axis_owner::bit(changes, x));
        assert(after[x as int] == before[x as int]);
    }
    assert forall|labels: Seq<u16>| mixed_frontiers(labels, y, before, starts, values, changes)
        && row_matches(labels, y, current)
        implies #[trigger] mixed_frontiers(labels, y, after, next_starts, next_values, next_changes) by {
        assert forall|x: int| 0 <= x < 64 implies #[trigger]
            column_frontier(labels, x, column_height(y, next_changes, x),
                &after[x], next_starts[x], next_values[x]) by {
            assert(column_frontier(labels, x, column_height(y, changes, x), &before[x], starts[x], values[x]));
            if x == slot {
                assert(axis_owner::bit(changes, slot) && !axis_owner::bit(next_changes, slot));
                assert(column_frontier(labels, x, y, &before[x], starts[x], values[x]));
                assert(current[x] == labels[y as int * 64 + x]);
                assert(column_frontier(labels, x, (y + 1) as u8, &after[x], y, current[x]));
            } else {
                assert(after[x] == before[x]);
                assert(axis_owner::bit(next_changes, x as u64) == axis_owner::bit(changes, x as u64));
            }
        }
    }
}

pub(super) open spec fn rows_prefix(labels: Seq<u16>, rows: &AxisIntervals, end: u8) -> bool {
    labels.len() == 4096 && rows.valid() && rows.lines@.len() == 64 && 0 < end <= 64
        && rows.intervals@.len() <= end as int * 64
        && (forall|line: int, p: int| 0 <= line < end && 0 <= p < 64
            ==> #[trigger] axis_owner::line_value(rows.line_spec(line), p) == labels[line * 64 + p])
        && (forall|line: int| 0 < line < end && !axis_owner::bit(rows.boundaries, line as u64)
            ==> #[trigger] rows.line_spec(line) == rows.line_spec(line - 1))
}

pub(super) open spec fn row_step_context(
    before: &AxisIntervals, after: &AxisIntervals, y: u8, current: Seq<u16>, changed: bool,
) -> bool {
    forall|labels: Seq<u16>| rows_prefix(labels, before, y) && row_matches(labels, y, current)
        && (!changed ==> forall|p: int| 0 <= p < 64 ==> #[trigger] labels[y as int * 64 + p] == labels[(y as int - 1) * 64 + p])
        ==> #[trigger] rows_prefix(labels, after, (y + 1) as u8)
}

pub(super) proof fn previous_lines(before: &AxisIntervals, after: &AxisIntervals, end: int)
    requires before.valid(), after.valid(), before.lines@.len() == 64, after.lines@.len() == 64,
        0 <= end <= 64,
        before.intervals@.len() <= after.intervals@.len(),
        after.intervals@.take(before.intervals@.len() as int) == before.intervals@,
        forall|line: int| 0 <= line < end ==> #[trigger] after.lines@[line] == before.lines@[line],
    ensures forall|line: int| 0 <= line < end ==> #[trigger] after.line_spec(line) == before.line_spec(line),
{
    assert forall|line: int| 0 <= line < end implies #[trigger] after.line_spec(line) == before.line_spec(line) by {
        let range = before.lines@[line];
        assert(before.range_valid(range));
        assert(after.intervals@.subrange(range.start as int, range.end as int)
            =~= before.intervals@.subrange(range.start as int, range.end as int));
    }
}

pub(super) proof fn row_step_frontiers(before: &AxisIntervals, after: &AxisIntervals, y: u8, current: Seq<u16>, changed: bool)
    requires before.valid(), after.valid(), before.lines@.len() == 64, after.lines@.len() == 64,
        0 < y < 64, current.len() == 64, after.intervals@.len() <= (y as int + 1) * 64,
        before.intervals@.len() <= after.intervals@.len(),
        after.intervals@.take(before.intervals@.len() as int) == before.intervals@,
        forall|line: int| 0 <= line < y ==> #[trigger] after.lines@[line] == before.lines@[line],
        after.boundaries == if changed { before.boundaries | (1u64 << y) } else { before.boundaries },
        changed ==> forall|p: int| 0 <= p < 64 ==> #[trigger] axis_owner::line_value(after.line_spec(y as int), p) == current[p],
        !changed ==> after.lines@[y as int] == before.lines@[y as int - 1],
    ensures row_step_context(before, after, y, current, changed),
{
    previous_lines(before, after, y as int);
    let old_mask = before.boundaries;
    let new_mask = after.boundaries;
    assert forall|labels: Seq<u16>| rows_prefix(labels, before, y) && row_matches(labels, y, current)
        && (!changed ==> forall|p: int| 0 <= p < 64 ==> #[trigger] labels[y as int * 64 + p] == labels[(y as int - 1) * 64 + p])
        implies #[trigger] rows_prefix(labels, after, (y + 1) as u8) by {
        assert forall|line: int, p: int| 0 <= line < y + 1 && 0 <= p < 64 implies
            #[trigger] axis_owner::line_value(after.line_spec(line), p) == labels[line * 64 + p] by {
            if line < y { assert(after.line_spec(line) == before.line_spec(line)); }
            else if !changed {
                assert(after.line_spec(line) == before.line_spec(line - 1));
            }
        }
        assert forall|line: int| 0 < line < y + 1 && !#[trigger] axis_owner::bit(after.boundaries, line as u64)
            implies after.line_spec(line) == after.line_spec(line - 1) by {
            if line < y {
                let bit = line as u64;
                assert(axis_owner::bit(old_mask, bit) == axis_owner::bit(new_mask, bit)) by(bit_vector)
                    requires 0 < bit < y, y < 64,
                        new_mask == if changed { old_mask | (1u64 << y) } else { old_mask };
                assert(before.line_spec(line) == before.line_spec(line - 1));
            } else {
                if changed {
                    assert(axis_owner::bit(new_mask, y as u64)) by(bit_vector)
                        requires y < 64, new_mask == old_mask | (1u64 << y);
                }
                assert(after.line_spec(line) == before.line_spec(line - 1));
            }
        }
    }
}

pub(super) proof fn matched_label_row(labels: &DenseLabels64, y: u8, row: Seq<u16>)
    requires y < 64, row == labels.rows@[y as int]@,
    ensures row_matches(labels@, y, row),
{
    label_layout(labels);
}

pub(super) proof fn initial_frontiers(
    labels: Seq<u16>, buckets: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>,
)
    requires row_matches(labels, 0, values), buckets.len() == 64, starts.len() == 64,
        forall|x: int| 0 <= x < 64 ==> #[trigger] buckets[x].valid() && buckets[x].starts == 0,
        forall|x: int| 0 <= x < 64 ==> #[trigger] starts[x] == 0,
    ensures columns_frontier(labels, 1, buckets, starts, values),
{
    assert forall|x: int| 0 <= x < 64 implies #[trigger]
        column_frontier(labels, x, 1, &buckets[x], starts[x], values[x]) by {
        assert(buckets[x].valid());
        initial_column(labels, x, &buckets[x], values[x]);
    }
}

pub(super) proof fn initial_rows(labels: Seq<u16>, rows: &AxisIntervals, row: Seq<u16>, range: LineRange)
    requires row_matches(labels, 0, row), rows.valid(), rows.lines@.len() == 64, rows.intervals@.len() <= 64,
        rows.lines@[0] == range,
        forall|p: int| 0 <= p < 64 ==> #[trigger] axis_owner::line_value(
            rows.intervals@.subrange(range.start as int, range.end as int), p) == row[p],
    ensures rows_prefix(labels, rows, 1),
{
    assert forall|line: int, p: int| 0 <= line < 1 && 0 <= p < 64 implies
        #[trigger] axis_owner::line_value(rows.line_spec(line), p) == labels[line * 64 + p] by {
        assert(line == 0);
    }
}

pub(super) proof fn begin_changed_row(
    labels: Seq<u16>, y: u8, buckets: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>,
    current: Seq<u16>, previous: Seq<u16>, changes: u64,
)
    requires columns_frontier(labels, y, buckets, starts, values), 0 < y < 64,
        row_matches(labels, y, current), row_matches(labels, (y - 1) as u8, previous),
        between_rows(current, previous, changes),
    ensures mixed_frontiers(labels, y, buckets, starts, values, changes),
        changes == 0 ==> forall|p: int| 0 <= p < 64 ==> #[trigger] labels[y as int * 64 + p] == labels[(y as int - 1) * 64 + p],
{
    assert forall|x: int| 0 <= x < 64 implies #[trigger]
        column_frontier(labels, x, column_height(y, changes, x), &buckets[x], starts[x], values[x]) by {
        assert(column_frontier(labels, x, y, &buckets[x], starts[x], values[x]));
        assert(values[x] == labels[(y as int - 1) * 64 + x]);
        let bit = x as u64;
        assert(((changes >> bit) & 1) == if current[x] != previous[x] { 1u64 } else { 0u64 });
        if !axis_owner::bit(changes, bit) {
            assert(current[x] == previous[x]);
            continue_column(labels, x, y, &buckets[x], starts[x], values[x]);
        }
    }
    if changes == 0 {
        assert forall|p: int| 0 <= p < 64 implies #[trigger] labels[y as int * 64 + p] == labels[(y as int - 1) * 64 + p] by {
            let bit = p as u64;
            assert((changes >> bit) & 1 == 0) by(bit_vector) requires changes == 0, bit < 64;
            assert(((changes >> bit) & 1) == if current[p] != previous[p] { 1u64 } else { 0u64 });
        }
    }
}

pub(super) proof fn changed_row_finished(
    labels: Seq<u16>, y: u8, buckets: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>,
)
    requires mixed_frontiers(labels, y, buckets, starts, values, 0),
    ensures columns_frontier(labels, (y + 1) as u8, buckets, starts, values),
{
    assert forall|x: int| 0 <= x < 64 implies #[trigger]
        column_frontier(labels, x, (y + 1) as u8, &buckets[x], starts[x], values[x]) by {
        let bit = x as u64;
        assert(!axis_owner::bit(0, bit)) by(bit_vector) requires bit < 64;
        assert(column_frontier(labels, x, column_height(y, 0, x), &buckets[x], starts[x], values[x]));
    }
}

pub(super) open spec fn complete_column(labels: Seq<u16>, x: int, bucket: &IntervalBucket) -> bool {
    labels.len() == 4096 && 0 <= x < 64 && bucket.disjoint()
        && forall|y: int| 0 <= y < 64 ==> #[trigger] axis_owner::bucket_value(bucket, y) == labels[y * 64 + x]
}

pub(super) open spec fn finishing_columns(
    labels: Seq<u16>, buckets: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>, end: usize,
) -> bool {
    buckets.len() == 64 && starts.len() == 64 && values.len() == 64 && end <= 64
        && (forall|x: int| 0 <= x < end ==> #[trigger] complete_column(labels, x, &buckets[x]))
        && (forall|x: int| end <= x < 64 ==> #[trigger] column_frontier(labels, x, 64, &buckets[x], starts[x], values[x]))
}

pub(super) proof fn finishing_started(labels: Seq<u16>, buckets: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>)
    requires columns_frontier(labels, 64, buckets, starts, values),
    ensures finishing_columns(labels, buckets, starts, values, 0),
{}

pub(super) open spec fn finish_context(
    before: Seq<IntervalBucket>, after: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>, x: usize,
) -> bool {
    forall|labels: Seq<u16>| finishing_columns(labels, before, starts, values, x)
        ==> #[trigger] finishing_columns(labels, after, starts, values, (x + 1) as usize)
}

pub(super) proof fn finish_frontiers(
    before: Seq<IntervalBucket>, after: Seq<IntervalBucket>, starts: Seq<u8>, values: Seq<u16>, x: usize,
)
    requires before.len() == 64, after.len() == 64, starts.len() == 64, values.len() == 64, x < 64,
        closed_before(&after[x as int], 64),
        forall|i: int| 0 <= i < 64 && i != x ==> #[trigger] after[i] == before[i],
        forall|p: int| 0 <= p < 64 ==> #[trigger] axis_owner::bucket_value(&after[x as int], p)
            == if values[x as int] != 0 && starts[x as int] <= p { values[x as int] }
                else { axis_owner::bucket_value(&before[x as int], p) },
    ensures finish_context(before, after, starts, values, x),
{
    assert forall|labels: Seq<u16>| finishing_columns(labels, before, starts, values, x)
        implies #[trigger] finishing_columns(labels, after, starts, values, (x + 1) as usize) by {
        assert(column_frontier(labels, x as int, 64, &before[x as int], starts[x as int], values[x as int]));
        commit_column(labels, x as int, 64, &before[x as int], starts[x as int], values[x as int], &after[x as int]);
        assert(complete_column(labels, x as int, &after[x as int]));
        assert forall|i: int| 0 <= i < x + 1 implies #[trigger] complete_column(labels, i, &after[i]) by {
            if i < x { assert(after[i] == before[i]); }
        }
        assert forall|i: int| x + 1 <= i < 64 implies #[trigger]
            column_frontier(labels, i, 64, &after[i], starts[i], values[i]) by {
            assert(after[i] == before[i]);
        }
    }
}

pub(super) proof fn dense_axes_finished(labels: Seq<u16>, rows: &AxisIntervals, columns: &AxisIntervals, buckets: Seq<IntervalBucket>)
    requires rows_prefix(labels, rows, 64), buckets.len() == 64,
        forall|x: int| 0 <= x < 64 ==> #[trigger] complete_column(labels, x, &buckets[x]),
        axis_owner::complete_progress(buckets, columns, 64),
    ensures rows.represents(labels, false), columns.represents(labels, true),
        rows.boundaries_complete(), columns.boundaries_complete(),
{
    assert forall|x: int, y: int| 0 <= x < 64 && 0 <= y < 64 implies
        #[trigger] axis_owner::line_value(columns.line_spec(x), y) == labels[y * 64 + x] by {
        assert(complete_column(labels, x, &buckets[x]));
        assert(axis_owner::bucket_value(&buckets[x], y) == labels[y * 64 + x]);
    }
    assert forall|line: u64| 0 < line < 64 && !#[trigger] axis_owner::bit(rows.boundaries, line)
        implies rows.line_spec(line as int) == rows.line_spec(line as int - 1) by {
        assert(0 < (line as int) < 64);
    }
}

pub(super) proof fn completed_buckets(labels: Seq<u16>, buckets: Seq<IntervalBucket>)
    requires buckets.len() == 64,
        forall|x: int| 0 <= x < 64 ==> #[trigger] complete_column(labels, x, &buckets[x]),
    ensures forall|x: int| 0 <= x < 64 ==> #[trigger] buckets[x].disjoint(),
{
    assert forall|x: int| 0 <= x < 64 implies #[trigger] buckets[x].disjoint() by {
        assert(complete_column(labels, x, &buckets[x]));
    }
}

fn row_edge_bits(current: &[u16; 64], previous: &[u16; 64]) {
    let mask = row_changes(current, previous);
    assert(((mask >> 0u64) & 1) == if current@[0] != previous@[0] { 1u64 } else { 0u64 });
    assert((mask >> 0u64) & 1 == mask & 1) by(bit_vector);
    assert((mask & 1) == if current@[0] != previous@[0] { 1u64 } else { 0u64 });
    assert(((mask >> 63u64) & 1) == if current@[63] != previous@[63] { 1u64 } else { 0u64 });
}

fn overlapping_boundary(row: &[u16; 64]) {
    let mask = row_boundaries(row);
    assert((mask >> 0u64) & 1 == 0);
    assert((mask >> 0u64) & 1 == mask & 1) by(bit_vector);
    assert(mask & 1 == 0);
    assert(((mask >> 32u64) & 1) == if row@[32] != row@[31] { 1u64 } else { 0u64 });
}

fn borrowed_label_update(y: usize, x: usize, value: u16)
    requires y < 64, x < 64,
{
    let mut labels = DenseLabels64::new();
    {
        let rows = labels.rows_mut();
        assert forall|line: int, p: int| 0 <= line < 64 && 0 <= p < 64
            implies #[trigger] rows@[line]@[p] == 0 by {}
        rows[y][x] = value;
    }
    assert(labels@[(y * 64 + x) as int] == value);
    assert forall|p: int| 0 <= p < 4096 && p != y * 64 + x implies #[trigger] labels@[p] == 0 by {
        assert(p / 64 != y || p % 64 != x) by(nonlinear_arith)
            requires 0 <= p < 4096, p != y * 64 + x;
    }
}

#[cfg(reject_axis_row_bit)]
fn reject_axis_row_bit(current: &[u16; 64], previous: &[u16; 64])
    requires current@[63] != previous@[63],
{
    let mask = row_changes(current, previous);
    assert((mask >> 63u64) & 1 == 0);
}

#[cfg(reject_axis_zero_boundary)]
fn reject_axis_zero_boundary(row: &[u16; 64]) {
    let mask = row_boundaries(row);
    assert(mask & 1 == 1);
}

#[cfg(reject_axis_overlap_boundary)]
fn reject_axis_overlap_boundary(row: &[u16; 64])
    requires row@[32] != row@[31],
{
    let mask = row_boundaries(row);
    assert((mask >> 32u64) & 1 == 0);
}

#[cfg(reject_axis_row_label)]
fn reject_axis_row_label(row: &[u16; 64], output: &mut AxisIntervals)
    requires old(output).valid(), old(output).intervals@.len() <= 4032, row@[63] != 0,
{
    let result = append_row(row, output);
    match result {
        Ok(range) => {
            assert(axis_owner::line_value(output.intervals@.subrange(range.start as int, range.end as int), 63) == 0);
        },
        Err(_) => {},
    }
}

#[cfg(reject_axis_column_overlap)]
fn reject_axis_column_overlap(bucket: &mut IntervalBucket, start: u8, end: u8, value: u16)
    requires closed_before(old(bucket), start), start < end <= 64, value != 0,
{
    let result = close_column_run(bucket, start, end, value);
    assert(result is Err);
}

}
