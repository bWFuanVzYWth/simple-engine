//! 原点事件表的生存期、到期位和逐线非重叠关系；全部取自真实 bucket 存储。

use super::{AxisEventSweep, AxisIntervals, Interval, IntervalBucket, LineRange, proof as axis};
use vstd::prelude::*;

verus! {

pub(super) proof fn union_empty(a: u64, b: u64)
    ensures a | b == 0 ==> a == 0 && b == 0,
{
    assert(a | b == 0 ==> a == 0 && b == 0) by(bit_vector);
}

pub(super) open spec fn present(buckets: Seq<IntervalBucket>, id: int) -> bool {
    0 <= id < 4096 && axis::bit(buckets[id / 64].starts, (id % 64) as u64)
}

pub(super) open spec fn end_line(buckets: Seq<IntervalBucket>, id: int) -> int {
    id / 64 + buckets[id / 64].items@[id % 64].end - buckets[id / 64].items@[id % 64].start
}

pub(super) open spec fn domain(buckets: Seq<IntervalBucket>) -> bool {
    buckets.len() == 64 && (forall|line: int| 0 <= line < 64 ==> #[trigger] buckets[line].valid())
        && (forall|id: int| #[trigger] present(buckets, id) ==> end_line(buckets, id) <= 64)
}

pub(super) open spec fn covers(buckets: Seq<IntervalBucket>, id: int, line: int, pos: int) -> bool {
    present(buckets, id) && id / 64 <= line < end_line(buckets, id)
        && buckets[id / 64].items@[id % 64].start <= pos < buckets[id / 64].items@[id % 64].end
}

pub(super) open spec fn overlap(buckets: Seq<IntervalBucket>) -> bool {
    exists|e: int, f: int, line: int, pos: int| e != f
        && #[trigger] covers(buckets, e, line, pos) && #[trigger] covers(buckets, f, line, pos)
}

pub(super) open spec fn processed(buckets: Seq<IntervalBucket>, id: int, line: int, remaining: u64) -> bool {
    present(buckets, id) && (id / 64 < line || id / 64 == line && !axis::bit(remaining, (id % 64) as u64))
}

pub(super) open spec fn live(buckets: Seq<IntervalBucket>, id: int, line: int, remaining: u64) -> bool {
    processed(buckets, id, line, remaining) && line < end_line(buckets, id)
}

pub(super) open spec fn model(buckets: Seq<IntervalBucket>, state: &AxisEventSweep, line: int, remaining: u64) -> bool {
    domain(buckets) && -1 <= line < 64 && state.active.valid()
        && (if line < 0 { remaining == 0 } else { remaining & !buckets[line].starts == 0 })
        && (forall|slot: u64| slot < 64 ==> #[trigger] axis::bit(state.active.starts, slot)
            == exists|id: int| #[trigger] live(buckets, id, line, remaining) && id % 64 == slot)
        && (forall|id: int| #[trigger] live(buckets, id, line, remaining)
            ==> state.active.items@[id % 64] == buckets[id / 64].items@[id % 64])
        && (forall|e: int, f: int| #[trigger] live(buckets, e, line, remaining)
            && #[trigger] live(buckets, f, line, remaining) && e % 64 == f % 64 ==> e == f)
        && (forall|end: int, slot: u64| 0 <= end < 64 && slot < 64 ==> #[trigger] axis::bit(state.end_masks@[end], slot)
            == exists|id: int| #[trigger] processed(buckets, id, line, remaining)
                && end_line(buckets, id) == end && id % 64 == slot)
}

pub(super) proof fn started(buckets: Seq<IntervalBucket>, state: &AxisEventSweep)
    requires domain(buckets), state.active.valid(), state.active.starts == 0,
        forall|line: int| 0 <= line < 64 ==> #[trigger] state.end_masks@[line] == 0,
    ensures model(buckets, state, -1, 0),
{
    assert forall|slot: u64| slot < 64 implies !#[trigger] axis::bit(state.active.starts, slot) by {
        assert((0u64 >> slot) & 1 == 0) by(bit_vector) requires slot < 64;
    }
    assert forall|end: int, slot: u64| 0 <= end < 64 && slot < 64 implies !#[trigger] axis::bit(state.end_masks@[end], slot) by {
        assert(state.end_masks@[end] == 0);
        assert((0u64 >> slot) & 1 == 0) by(bit_vector) requires slot < 64;
    }
}

pub(super) proof fn event_bounds(buckets: Seq<IntervalBucket>, id: int)
    requires domain(buckets), present(buckets, id),
    ensures 0 <= id / 64 < 64, 0 <= id % 64 < 64,
        axis::interval_valid(buckets[id / 64].items@[id % 64]),
        buckets[id / 64].items@[id % 64].start == id % 64,
        id / 64 < end_line(buckets, id) <= 64,
{
    assert(buckets[id / 64].valid());
}

pub(super) proof fn expired(buckets: Seq<IntervalBucket>, before: &AxisEventSweep, after: &AxisEventSweep, line: usize)
    requires line < 64, model(buckets, before, line as int - 1, 0),
        after.active.items@ == before.active.items@, after.end_masks@ == before.end_masks@,
        after.active.starts == before.active.starts & !before.end_masks@[line as int],
    ensures model(buckets, after, line as int, buckets[line as int].starts),
{
    let pending = buckets[line as int].starts;
    let previous = before.active.starts;
    let expires = before.end_masks@[line as int];
    let next = after.active.starts;
    assert(pending & !pending == 0) by(bit_vector);
    assert forall|id: int| #[trigger] processed(buckets, id, line as int, pending)
        <==> processed(buckets, id, line as int - 1, 0) by {
        if id / 64 == line {
            assert(axis::bit(pending, (id % 64) as u64) == present(buckets, id));
        } else if present(buckets, id) {
            let slot = (id % 64) as u64;
            assert(!axis::bit(0, slot)) by(bit_vector);
        }
    }
    assert forall|slot: u64| slot < 64 implies #[trigger] axis::bit(next, slot)
        == exists|id: int| #[trigger] live(buckets, id, line as int, pending) && id % 64 == slot by {
        assert(axis::bit(next, slot) == (axis::bit(previous, slot) && !axis::bit(expires, slot)))
            by(bit_vector) requires slot < 64, next == previous & !expires;
        if axis::bit(next, slot) {
            let id = choose|id: int| #[trigger] live(buckets, id, line as int - 1, 0) && id % 64 == slot;
            if end_line(buckets, id) == line {
                assert(axis::bit(expires, slot));
            }
            assert(live(buckets, id, line as int, pending));
        }
        if exists|id: int| #[trigger] live(buckets, id, line as int, pending) && id % 64 == slot {
            let id = choose|id: int| #[trigger] live(buckets, id, line as int, pending) && id % 64 == slot;
            assert(live(buckets, id, line as int - 1, 0));
            assert(axis::bit(previous, slot));
            if axis::bit(expires, slot) {
                let prior = choose|prior: int| #[trigger] processed(buckets, prior, line as int - 1, 0)
                    && end_line(buckets, prior) == line && prior % 64 == slot;
                assert(live(buckets, prior, line as int - 1, 0));
                assert(prior == id);
            }
        }
    }
    assert forall|slot: u64| slot < 64 && #[trigger] axis::bit(next, slot)
        implies after.active.items@[slot as int].start == slot && axis::interval_valid(after.active.items@[slot as int]) by {
        assert(axis::bit(previous, slot)) by(bit_vector) requires next == previous & !expires, (next >> slot) & 1 == 1;
    }
    assert forall|id: int| #[trigger] live(buckets, id, line as int, pending)
        implies after.active.items@[id % 64] == buckets[id / 64].items@[id % 64] by {
        assert(live(buckets, id, line as int - 1, 0));
    }
    assert(after.active.valid());
    assert forall|e: int, f: int| #[trigger] live(buckets, e, line as int, pending)
        && #[trigger] live(buckets, f, line as int, pending) && e % 64 == f % 64 implies e == f by {
        assert(live(buckets, e, line as int - 1, 0));
        assert(live(buckets, f, line as int - 1, 0));
    }
    assert forall|end: int, slot: u64| 0 <= end < 64 && slot < 64 implies #[trigger] axis::bit(after.end_masks@[end], slot)
        == exists|id: int| #[trigger] processed(buckets, id, line as int, pending)
            && end_line(buckets, id) == end && id % 64 == slot by {
        if axis::bit(after.end_masks@[end], slot) {
            let id = choose|id: int| #[trigger] processed(buckets, id, line as int - 1, 0)
                && end_line(buckets, id) == end && id % 64 == slot;
            assert(processed(buckets, id, line as int, pending));
        }
        if exists|id: int| #[trigger] processed(buckets, id, line as int, pending)
            && end_line(buckets, id) == end && id % 64 == slot {
            let id = choose|id: int| #[trigger] processed(buckets, id, line as int, pending)
                && end_line(buckets, id) == end && id % 64 == slot;
            assert(processed(buckets, id, line as int - 1, 0));
        }
    }
}

pub(super) proof fn collision(buckets: Seq<IntervalBucket>, state: &AxisEventSweep, line: usize, remaining: u64, slot: u64)
    requires line < 64, model(buckets, state, line as int, remaining), slot < 64,
        axis::bit(remaining, slot), axis::bit(state.active.starts, slot),
    ensures overlap(buckets),
{
    let prior = choose|id: int| #[trigger] live(buckets, id, line as int, remaining) && id % 64 == slot;
    let id = line as int * 64 + slot;
    assert(id / 64 == line && id % 64 == slot) by(nonlinear_arith)
        requires line < 64, slot < 64, id == line as int * 64 + slot;
    let mask = buckets[line as int].starts;
    assert(axis::bit(mask, slot)) by(bit_vector) requires slot < 64, (remaining >> slot) & 1 == 1, remaining & !mask == 0;
    event_bounds(buckets, id);
    event_bounds(buckets, prior);
    assert(id != prior);
    assert(covers(buckets, id, line as int, slot as int));
    assert(covers(buckets, prior, line as int, slot as int));
}

pub(super) proof fn selected(buckets: Seq<IntervalBucket>, state: &AxisEventSweep, line: usize, remaining: u64, slot: u64)
    requires line < 64, model(buckets, state, line as int, remaining), remaining != 0,
        slot == vstd::std_specs::bits::u64_trailing_zeros(remaining),
    ensures slot < 64, axis::bit(remaining, slot),
        present(buckets, line as int * 64 + slot),
        buckets[line as int].items@[slot as int].start == slot,
        axis::interval_valid(buckets[line as int].items@[slot as int]),
        line < end_line(buckets, line as int * 64 + slot) <= 64,
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(remaining);
    assert(axis::bit(remaining, slot));
    let mask = buckets[line as int].starts;
    assert(axis::bit(mask, slot)) by(bit_vector)
        requires slot < 64, (remaining >> slot) & 1 == 1, remaining & !mask == 0;
    let id = line as int * 64 + slot;
    assert(id / 64 == line && id % 64 == slot) by(nonlinear_arith)
        requires line < 64, slot < 64, id == line as int * 64 + slot;
    event_bounds(buckets, id);
}

pub(super) proof fn inserted(
    buckets: Seq<IntervalBucket>, before: &AxisEventSweep, after: &AxisEventSweep,
    line: usize, remaining: u64, next: u64, slot: u64,
)
    requires line < 64, model(buckets, before, line as int, remaining), remaining != 0,
        slot == vstd::std_specs::bits::u64_trailing_zeros(remaining), next == remaining & ((remaining - 1) as u64),
        !axis::bit(before.active.starts, slot), after.active.valid(),
        after.active.starts == before.active.starts | (1u64 << slot),
        after.active.items@ == before.active.items@.update(slot as int, buckets[line as int].items@[slot as int]),
        after.end_masks@ == if end_line(buckets, line as int * 64 + slot) < 64 {
            before.end_masks@.update(end_line(buckets, line as int * 64 + slot),
                before.end_masks@[end_line(buckets, line as int * 64 + slot)] | (1u64 << slot))
        } else { before.end_masks@ },
    ensures model(buckets, after, line as int, next), next < remaining,
{
    selected(buckets, before, line, remaining, slot);
    axis::cleared_lowest(remaining, slot, next);
    let id = line as int * 64 + slot;
    assert(id / 64 == line && id % 64 == slot) by(nonlinear_arith)
        requires line < 64, slot < 64, id == line as int * 64 + slot;
    let raw = buckets[line as int].starts;
    assert(next & !raw == 0) by(bit_vector)
        requires remaining & !raw == 0, next == remaining & ((remaining - 1) as u64);
    assert forall|e: int| #[trigger] processed(buckets, e, line as int, next)
        <==> processed(buckets, e, line as int, remaining) || e == id by {
        if present(buckets, e) && e / 64 == line {
            assert(e == e / 64 * 64 + e % 64);
            assert(axis::bit(next, (e % 64) as u64) <==> axis::bit(remaining, (e % 64) as u64) && e % 64 != slot);
        }
    }
    assert forall|e: int| #[trigger] live(buckets, e, line as int, next)
        <==> live(buckets, e, line as int, remaining) || e == id by {
        assert(processed(buckets, e, line as int, next) <==> processed(buckets, e, line as int, remaining) || e == id);
    }
    let mask = before.active.starts;
    let updated = after.active.starts;
    assert forall|p: u64| p < 64 implies #[trigger] axis::bit(updated, p)
        == exists|e: int| #[trigger] live(buckets, e, line as int, next) && e % 64 == p by {
        assert(axis::bit(updated, p) == (axis::bit(mask, p) || p == slot)) by(bit_vector)
            requires p < 64, slot < 64, updated == mask | (1u64 << slot);
        if axis::bit(mask, p) {
            let e = choose|e: int| #[trigger] live(buckets, e, line as int, remaining) && e % 64 == p;
            assert(live(buckets, e, line as int, next));
        }
        if p == slot { assert(live(buckets, id, line as int, next)); }
        if exists|e: int| #[trigger] live(buckets, e, line as int, next) && e % 64 == p {
            let e = choose|e: int| #[trigger] live(buckets, e, line as int, next) && e % 64 == p;
            if e != id { assert(live(buckets, e, line as int, remaining)); assert(axis::bit(mask, p)); }
        }
    }
    assert forall|e: int| #[trigger] live(buckets, e, line as int, next)
        implies after.active.items@[e % 64] == buckets[e / 64].items@[e % 64] by {
        if e != id {
            assert(live(buckets, e, line as int, remaining));
            assert(axis::bit(before.active.starts, (e % 64) as u64));
            assert(e % 64 != slot);
        }
    }
    assert forall|e: int, f: int| #[trigger] live(buckets, e, line as int, next)
        && #[trigger] live(buckets, f, line as int, next) && e % 64 == f % 64 implies e == f by {
        if e != id && f != id {
            assert(live(buckets, e, line as int, remaining)); assert(live(buckets, f, line as int, remaining));
        } else if e != f {
            let old = if e == id { f } else { e };
            assert(live(buckets, old, line as int, remaining));
            assert(axis::bit(before.active.starts, slot));
        }
    }
    let end_at = end_line(buckets, id);
    assert forall|end: int, p: u64| 0 <= end < 64 && p < 64 implies #[trigger] axis::bit(after.end_masks@[end], p)
        == exists|e: int| #[trigger] processed(buckets, e, line as int, next) && end_line(buckets, e) == end && e % 64 == p by {
        if end == end_at {
            let old = before.end_masks@[end];
            let updated = after.end_masks@[end];
            assert(axis::bit(updated, p) == (axis::bit(old, p) || p == slot)) by(bit_vector)
                requires p < 64, slot < 64, updated == old | (1u64 << slot);
        }
        if axis::bit(before.end_masks@[end], p) {
            let e = choose|e: int| #[trigger] processed(buckets, e, line as int, remaining) && end_line(buckets, e) == end && e % 64 == p;
            assert(processed(buckets, e, line as int, next));
        }
        if end == end_at && p == slot { assert(processed(buckets, id, line as int, next)); }
        if exists|e: int| #[trigger] processed(buckets, e, line as int, next) && end_line(buckets, e) == end && e % 64 == p {
            let e = choose|e: int| #[trigger] processed(buckets, e, line as int, next) && end_line(buckets, e) == end && e % 64 == p;
            if e != id {
                assert(processed(buckets, e, line as int, remaining));
                assert(axis::bit(before.end_masks@[end], p));
            }
        }
    }
}

pub(super) open spec fn masked(bucket: &IntervalBucket, mask: u64) -> IntervalBucket {
    IntervalBucket { items: bucket.items, starts: mask }
}

pub(super) open spec fn ordered_prefix(bucket: &IntervalBucket, output: Seq<Interval>, remaining: u64, previous_end: u8) -> bool {
    bucket.valid() && remaining & !bucket.starts == 0 && axis::line_valid(output)
        && output.len() <= vstd::std_specs::bits::u64_trailing_zeros(remaining)
        && masked(bucket, bucket.starts & !remaining).disjoint()
        && (forall|p: u64, q: u64| p < 64 && q < 64 && #[trigger] axis::bit(bucket.starts & !remaining, p)
            && #[trigger] axis::bit(remaining, q) ==> p < q)
        && (forall|p: u64| p < 64 && #[trigger] axis::bit(bucket.starts & !remaining, p)
            ==> bucket.items@[p as int].end <= previous_end)
        && (previous_end == 0 || exists|p: u64| p < 64 && #[trigger] axis::bit(bucket.starts & !remaining, p)
            && bucket.items@[p as int].end == previous_end)
        && (forall|pos: int| 0 <= pos < 64 ==> #[trigger] axis::line_value(output, pos)
            == axis::masked_value(bucket.items@, bucket.starts & !remaining, pos))
}

pub(super) proof fn ordered_started(bucket: &IntervalBucket)
    requires bucket.valid(),
    ensures ordered_prefix(bucket, Seq::empty(), bucket.starts, 0),
{
    axis::lowest(bucket.starts);
    let raw = bucket.starts;
    assert(raw & !raw == 0) by(bit_vector);
    assert forall|p: u64| p < 64 implies !#[trigger] axis::bit(0, p) by {
        assert((0u64 >> p) & 1 == 0) by(bit_vector);
    }
}

pub(super) proof fn ordered_selected(bucket: &IntervalBucket, output: Seq<Interval>, remaining: u64, previous_end: u8, slot: u64)
    requires ordered_prefix(bucket, output, remaining, previous_end), remaining != 0,
        slot == vstd::std_specs::bits::u64_trailing_zeros(remaining),
    ensures slot < 64, axis::bit(bucket.starts, slot), bucket.items@[slot as int].start == slot,
        axis::interval_valid(bucket.items@[slot as int]),
        previous_end > bucket.items@[slot as int].start ==> !bucket.disjoint(),
        bucket.disjoint() ==> previous_end <= bucket.items@[slot as int].start,
        previous_end <= bucket.items@[slot as int].start ==> axis::appendable(output, bucket.items@[slot as int]),
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(remaining);
    assert(axis::bit(remaining, slot));
    let raw = bucket.starts;
    assert(axis::bit(raw, slot)) by(bit_vector)
        requires slot < 64, (remaining >> slot) & 1 == 1, remaining & !raw == 0;
    if previous_end > bucket.items@[slot as int].start {
        let p = choose|p: u64| p < 64 && #[trigger] axis::bit(raw & !remaining, p)
            && bucket.items@[p as int].end == previous_end;
        assert(p < slot);
        assert(axis::bit(raw, p)) by(bit_vector) requires (raw & !remaining) >> p & 1 == 1;
    }
    if previous_end <= bucket.items@[slot as int].start {
        assert forall|i: int| 0 <= i < output.len() implies #[trigger] output[i].end <= bucket.items@[slot as int].start by {
            assert(axis::interval_valid(output[i]));
            let pos = output[i].end as int - 1;
            axis::value_at(output, pos, i);
            assert(axis::masked_value(bucket.items@, raw & !remaining, pos) != 0);
            let p = choose|p: u64| p < 64 && #[trigger] axis::bit(raw & !remaining, p)
                && bucket.items@[p as int].start <= pos < bucket.items@[p as int].end;
            assert(output[i].end <= previous_end);
        }
    }
}

pub(super) proof fn ordered_advanced(
    bucket: &IntervalBucket, before: Seq<Interval>, after: Seq<Interval>, remaining: u64, next: u64,
    previous_end: u8, slot: u64,
)
    requires ordered_prefix(bucket, before, remaining, previous_end), remaining != 0,
        slot == vstd::std_specs::bits::u64_trailing_zeros(remaining), next == remaining & ((remaining - 1) as u64),
        previous_end <= bucket.items@[slot as int].start,
        after == axis::merged(before, bucket.items@[slot as int]),
    ensures ordered_prefix(bucket, after, next, bucket.items@[slot as int].end), next < remaining,
{
    ordered_selected(bucket, before, remaining, previous_end, slot);
    axis::cleared_lowest(remaining, slot, next);
    let raw = bucket.starts;
    let old_done = raw & !remaining;
    let done = raw & !next;
    assert(raw & (1u64 << slot) != 0) by(bit_vector)
        requires slot < 64, (raw >> slot) & 1 == 1;
    assert(done == old_done | (1u64 << slot)) by(bit_vector)
        requires slot < 64, (remaining >> slot) & 1 == 1, raw & (1u64 << slot) != 0,
            done == raw & !next, old_done == raw & !remaining,
            next == remaining & ((remaining - 1) as u64),
            remaining << ((64 - slot) as u64) == 0;
    assert(next & !raw == 0) by(bit_vector)
        requires remaining & !raw == 0, next == remaining & ((remaining - 1) as u64);
    assert forall|p: u64| p < 64 implies #[trigger] axis::bit(done, p) == (axis::bit(old_done, p) || p == slot) by {
        assert(((done >> p) & 1 == 1) == (((old_done >> p) & 1 == 1) || p == slot)) by(bit_vector)
            requires p < 64, slot < 64, done == old_done | (1u64 << slot);
    }
    assert forall|p: u64, q: u64| p < q < 64 && #[trigger] axis::bit(done, p) && #[trigger] axis::bit(done, q)
        implies bucket.items@[p as int].end <= q by {
        if p != slot && q != slot {
            assert(axis::bit(old_done, p)); assert(axis::bit(old_done, q));
            assert(masked(bucket, old_done).disjoint());
            disjoint_at(&masked(bucket, old_done), p, q);
        }
        else if q == slot { assert(axis::bit(old_done, p)); assert(bucket.items@[p as int].end <= previous_end); }
        else { assert(axis::bit(old_done, q)); assert(axis::bit(remaining, slot)); assert(q < slot); }
    }
    assert forall|p: u64| p < 64 && #[trigger] axis::bit(done, p)
        implies bucket.items@[p as int].start == p && axis::interval_valid(bucket.items@[p as int]) by {
        assert(axis::bit(raw, p)) by(bit_vector) requires done == raw & !next, (done >> p) & 1 == 1;
    }
    assert(masked(bucket, done).disjoint());
    assert forall|p: u64, q: u64| p < 64 && q < 64 && #[trigger] axis::bit(done, p) && #[trigger] axis::bit(next, q)
        implies p < q by {
        assert(axis::bit(remaining, q));
        if p == slot { assert(slot < q); }
        else { assert(axis::bit(old_done, p)); }
    }
    axis::merge_properties(before, bucket.items@[slot as int]);
    assert(after.len() <= slot + 1);
    vstd::std_specs::bits::axiom_u64_trailing_zeros(next);
    if next != 0 { assert(axis::bit(next, vstd::std_specs::bits::u64_trailing_zeros(next) as u64)); }
    assert(slot < vstd::std_specs::bits::u64_trailing_zeros(next));
    assert forall|p: u64| p < 64 && #[trigger] axis::bit(done, p)
        implies bucket.items@[p as int].end <= bucket.items@[slot as int].end by {
        if p != slot { assert(bucket.items@[p as int].end <= previous_end); }
    }
    assert(axis::bit(done, slot));
    assert forall|pos: int| 0 <= pos < 64 implies #[trigger] axis::line_value(after, pos)
        == axis::masked_value(bucket.items@, done, pos) by {
        assert(old_done & !done == 0) by(bit_vector) requires done == old_done | (1u64 << slot);
        axis::masked_insert(&masked(bucket, done), old_done, slot, pos);
    }
}

proof fn disjoint_at(bucket: &IntervalBucket, p: u64, q: u64)
    requires bucket.disjoint(), p < q < 64, axis::bit(bucket.starts, p), axis::bit(bucket.starts, q),
    ensures bucket.items@[p as int].end <= q,
{}

pub(super) proof fn ordered_complete(bucket: &IntervalBucket, output: Seq<Interval>, previous_end: u8)
    requires ordered_prefix(bucket, output, 0, previous_end),
    ensures bucket.disjoint(), axis::line_valid(output), output.len() <= 64,
        forall|pos: int| 0 <= pos < 64 ==> #[trigger] axis::line_value(output, pos) == axis::bucket_value(bucket, pos),
{
    let raw = bucket.starts;
    assert(raw & !0u64 == raw) by(bit_vector);
    assert(masked(bucket, raw) == *bucket);
}

pub(super) open spec fn line_disjoint(buckets: Seq<IntervalBucket>, line: int) -> bool {
    forall|e: int, f: int, pos: int| #[trigger] covers(buckets, e, line, pos)
        && #[trigger] covers(buckets, f, line, pos) ==> e == f
}

pub(super) open spec fn value_at(buckets: Seq<IntervalBucket>, line: int, pos: int) -> u16 {
    if exists|id: int| #[trigger] covers(buckets, id, line, pos) {
        let id = choose|id: int| #[trigger] covers(buckets, id, line, pos);
        buckets[id / 64].items@[id % 64].value
    } else { 0 }
}

pub(super) proof fn selected_value(buckets: Seq<IntervalBucket>, id: int, line: int, pos: int)
    requires !overlap(buckets), covers(buckets, id, line, pos),
    ensures value_at(buckets, line, pos) == buckets[id / 64].items@[id % 64].value,
{
    let other = choose|other: int| #[trigger] covers(buckets, other, line, pos);
    assert(other == id);
}

proof fn covered_is_live(buckets: Seq<IntervalBucket>, id: int, line: int, pos: int)
    requires covers(buckets, id, line, pos),
    ensures live(buckets, id, line, 0),
{
    let slot = (id % 64) as u64;
    assert((0u64 >> slot) & 1 == 0) by(bit_vector);
}

pub(super) proof fn active_overlap(buckets: Seq<IntervalBucket>, state: &AxisEventSweep, line: int)
    requires model(buckets, state, line, 0), 0 <= line < 64, !state.active.disjoint(),
    ensures overlap(buckets),
{
    let (p, q) = choose|p: u64, q: u64| p < q < 64 && #[trigger] axis::bit(state.active.starts, p)
        && #[trigger] axis::bit(state.active.starts, q) && state.active.items@[p as int].end > q;
    let e = choose|e: int| #[trigger] live(buckets, e, line, 0) && e % 64 == p;
    let f = choose|f: int| #[trigger] live(buckets, f, line, 0) && f % 64 == q;
    assert(state.active.items@[p as int].start == p);
    assert(state.active.items@[q as int].start == q);
    assert(covers(buckets, e, line, q as int));
    assert(covers(buckets, f, line, q as int));
}

pub(super) proof fn active_disjoint(buckets: Seq<IntervalBucket>, state: &AxisEventSweep, line: int)
    requires model(buckets, state, line, 0), 0 <= line < 64, !overlap(buckets),
    ensures state.active.disjoint(),
{
    if !state.active.disjoint() { active_overlap(buckets, state, line); }
}

pub(super) proof fn active_values(buckets: Seq<IntervalBucket>, state: &AxisEventSweep, line: int)
    requires model(buckets, state, line, 0), 0 <= line < 64, state.active.disjoint(),
    ensures line_disjoint(buckets, line),
        forall|pos: int| 0 <= pos < 64 ==> #[trigger] axis::bucket_value(&state.active, pos) == value_at(buckets, line, pos),
{
    let mask = state.active.starts;
    assert(mask & !mask == 0) by(bit_vector);
    assert forall|e: int, f: int, pos: int| #[trigger] covers(buckets, e, line, pos)
        && #[trigger] covers(buckets, f, line, pos) implies e == f by {
        covered_is_live(buckets, e, line, pos); covered_is_live(buckets, f, line, pos);
        let p = (e % 64) as u64;
        let q = (f % 64) as u64;
        assert(axis::bit(mask, p)); assert(axis::bit(mask, q));
        if p < q { disjoint_at(&state.active, p, q); }
        else if q < p { disjoint_at(&state.active, q, p); }
        assert(p == q);
    }
    assert forall|pos: int| 0 <= pos < 64 implies #[trigger] axis::bucket_value(&state.active, pos) == value_at(buckets, line, pos) by {
        if exists|id: int| #[trigger] covers(buckets, id, line, pos) {
            let id = choose|id: int| #[trigger] covers(buckets, id, line, pos);
            covered_is_live(buckets, id, line, pos);
            let slot = (id % 64) as u64;
            assert(axis::bit(mask, slot));
            axis::masked_at(&state.active, mask, slot, pos);
        } else {
            assert forall|p: u64| p < 64 && #[trigger] axis::bit(mask, p)
                implies !(state.active.items@[p as int].start <= pos < state.active.items@[p as int].end) by {
                if state.active.items@[p as int].start <= pos < state.active.items@[p as int].end {
                    let id = choose|id: int| #[trigger] live(buckets, id, line, 0) && id % 64 == p;
                    assert(covers(buckets, id, line, pos));
                }
            }
        }
    }
}

pub(super) open spec fn progress(buckets: Seq<IntervalBucket>, state: &AxisEventSweep, output: &AxisIntervals, line: int, previous: LineRange) -> bool {
    domain(buckets) && 0 <= line <= 64 && model(buckets, state, line - 1, 0) && state.active.disjoint()
        && output.valid() && output.lines@.len() == 64 && output.intervals@.len() <= line * 64
        && output.range_valid(previous)
        && (if line == 0 { previous.start == 0 && previous.end == 0 } else { previous == output.lines@[line - 1] })
        && (forall|pos: int| 0 <= pos < 64 ==> #[trigger]
            axis::line_value(output.intervals@.subrange(previous.start as int, previous.end as int), pos)
                == axis::bucket_value(&state.active, pos))
        && (forall|i: int| 0 <= i < line ==> #[trigger] line_disjoint(buckets, i))
        && (forall|i: int, pos: int| 0 <= i < line && 0 <= pos < 64 ==> #[trigger]
            axis::line_value(output.line_spec(i), pos) == value_at(buckets, i, pos))
        && (forall|i: u64| 0 < i < line && !#[trigger] axis::bit(output.boundaries, i)
            ==> output.line_spec(i as int) == output.line_spec(i as int - 1))
}

pub(super) proof fn progress_started(buckets: Seq<IntervalBucket>, state: &AxisEventSweep, output: &AxisIntervals)
    requires domain(buckets), model(buckets, state, -1, 0), state.active.starts == 0,
        output.valid(), output.lines@.len() == 64, output.intervals@.len() == 0, output.boundaries == 0,
    ensures progress(buckets, state, output, 0, LineRange { start: 0, end: 0 }),
{
    assert forall|slot: u64| slot < 64 implies !#[trigger] axis::bit(state.active.starts, slot) by {
        assert((0u64 >> slot) & 1 == 0) by(bit_vector);
    }
    assert(output.intervals@.subrange(0, 0) =~= Seq::<Interval>::empty());
}

pub(super) proof fn progress_completed(buckets: Seq<IntervalBucket>, state: &AxisEventSweep, output: &AxisIntervals, previous: LineRange)
    requires progress(buckets, state, output, 64, previous),
    ensures !overlap(buckets), output.valid(), output.lines@.len() == 64, output.boundaries_complete(),
        forall|line: int, pos: int| 0 <= line < 64 && 0 <= pos < 64 ==> #[trigger]
            axis::line_value(output.line_spec(line), pos) == value_at(buckets, line, pos),
{
    if overlap(buckets) {
        let (e, f, line, pos) = choose|e: int, f: int, line: int, pos: int| e != f
            && #[trigger] covers(buckets, e, line, pos) && #[trigger] covers(buckets, f, line, pos);
        event_bounds(buckets, e); event_bounds(buckets, f);
        assert(0 <= line < 64);
        assert(line_disjoint(buckets, line));
    }
}

pub(super) proof fn step_finished(
    buckets: Seq<IntervalBucket>, before: &AxisEventSweep, after: &AxisEventSweep,
    old_output: &AxisIntervals, output: &AxisIntervals, line: usize, previous: LineRange, range: LineRange,
)
    requires progress(buckets, before, old_output, line as int, previous), line < 64,
        model(buckets, after, line as int, 0), after.active.disjoint(),
        output.valid(), output.lines@.len() == 64,
        old_output.intervals@.len() <= output.intervals@.len() <= old_output.intervals@.len() + 64,
        output.intervals@.take(old_output.intervals@.len() as int) == old_output.intervals@,
        output.lines@ == old_output.lines@.update(line as int, range),
        forall|pos: int| 0 <= pos < 64 ==> #[trigger] axis::line_value(output.line_spec(line as int), pos)
            == axis::bucket_value(&after.active, pos),
        output.boundaries == old_output.boundaries || output.boundaries == old_output.boundaries | (1u64 << line),
        !axis::bit(output.boundaries, line as u64) ==> range == previous,
    ensures progress(buckets, after, output, line as int + 1, range),
{
    active_values(buckets, after, line as int);
    axis::prefix_ranges(old_output.lines@, old_output.intervals@, output.intervals@);
    axis::boundary_kept(old_output.boundaries, output.boundaries, line as u64);
    assert(output.lines@[line as int] == range);
    assert(output.range_valid(output.lines@[line as int]));
    assert(output.range_valid(range));
    assert(output.line_spec(line as int) == output.intervals@.subrange(range.start as int, range.end as int));
    assert forall|pos: int| 0 <= pos < 64 implies #[trigger]
        axis::line_value(output.intervals@.subrange(range.start as int, range.end as int), pos)
            == axis::bucket_value(&after.active, pos) by {
        assert(axis::line_value(output.line_spec(line as int), pos) == axis::bucket_value(&after.active, pos));
    }
    assert forall|i: int, pos: int| 0 <= i <= line && 0 <= pos < 64 implies #[trigger]
        axis::line_value(output.line_spec(i), pos) == value_at(buckets, i, pos) by {
        if i == line {
            assert(axis::bucket_value(&after.active, pos) == value_at(buckets, line as int, pos));
        } else {
            assert(output.lines@[i] == old_output.lines@[i]);
            assert(output.line_spec(i) == old_output.line_spec(i));
        }
    }
    assert forall|i: int| 0 <= i <= line implies #[trigger] line_disjoint(buckets, i) by {
        if i < line { assert(line_disjoint(buckets, i)); }
    }
    assert forall|i: u64| 0 < i <= line && !#[trigger] axis::bit(output.boundaries, i)
        implies output.line_spec(i as int) == output.line_spec(i as int - 1) by {
        if i == line {
            assert(output.lines@[line as int] == previous);
            assert(output.lines@[line as int - 1] == previous);
        } else {
            assert(!axis::bit(old_output.boundaries, i));
            assert(output.line_spec(i as int) == old_output.line_spec(i as int));
            assert(output.line_spec(i as int - 1) == old_output.line_spec(i as int - 1));
        }
    }
}

#[cfg(reject_events_expiry)]
fn reject_events_expiry(buckets: &[IntervalBucket; 64], state: &mut AxisEventSweep, line: usize, slot: u64)
    requires line < 64, slot < 64, model(buckets@, old(state), line as int - 1, 0),
        axis::bit(old(state).end_masks@[line as int], slot),
{
    let _ = state.expire_line(buckets, line);
    assert(axis::bit(state.active.starts, slot));
}

}
