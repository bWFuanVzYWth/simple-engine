//! 选中内部切线的端点 XOR、逐行覆盖和边界标记。

use super::scan_proof::{bit64, bit128};
use super::{HorizontalCut, VerticalCut};
use vstd::prelude::*;

verus! {

pub(super) open spec fn horizontal_valid(cut: HorizontalCut) -> bool {
    1 <= cut.y <= 63 && 1 <= cut.start < cut.end <= 63
}

pub(super) open spec fn vertical_valid(cut: VerticalCut) -> bool {
    1 <= cut.x <= 63 && 1 <= cut.start < cut.end <= 63
}

pub(super) open spec fn horizontal_input(cuts: Seq<HorizontalCut>) -> bool {
    forall|i: int| 0 <= i < cuts.len() ==> #[trigger] horizontal_valid(cuts[i])
}

pub(super) open spec fn vertical_input(cuts: Seq<VerticalCut>) -> bool {
    (forall|i: int| 0 <= i < cuts.len() ==> #[trigger] vertical_valid(cuts[i]))
        && (forall|i: int, j: int| 0 <= i < j < cuts.len() && cuts[i].x == cuts[j].x
            ==> #[trigger] cuts[i].end < #[trigger] cuts[j].start || cuts[j].end < cuts[i].start)
}

pub(super) open spec fn cut_bit(cut: VerticalCut) -> u128 { 1u128 << cut.x as u32 }

pub(super) open spec fn event(cut: VerticalCut, row: int) -> u128 {
    if row == cut.start || row == cut.end { cut_bit(cut) } else { 0 }
}

pub(super) open spec fn cover(cut: VerticalCut, row: int) -> u128 {
    if cut.start <= row < cut.end { cut_bit(cut) } else { 0 }
}

pub(super) open spec fn events(cuts: Seq<VerticalCut>, count: int, row: int) -> u128
    decreases count,
{
    if count <= 0 { 0 } else { events(cuts, count - 1, row) ^ event(cuts[count - 1], row) }
}

pub(super) open spec fn coverage(cuts: Seq<VerticalCut>, count: int, row: int) -> u128
    decreases count,
{
    if count <= 0 { 0 } else { coverage(cuts, count - 1, row) ^ cover(cuts[count - 1], row) }
}

pub(super) open spec fn active(cuts: Seq<VerticalCut>, count: int, row: int, x: u32) -> bool {
    exists|i: int| 0 <= i < count && #[trigger] cuts[i].x == x && cuts[i].start <= row < cuts[i].end
}

pub(super) proof fn cut_transition(cut: VerticalCut, row: int)
    requires vertical_valid(cut), 0 <= row < 64,
    ensures cover(cut, row) == cover(cut, row - 1) ^ event(cut, row),
{
    let bit = cut_bit(cut);
    assert(bit ^ bit == 0 && bit ^ 0 == bit && 0u128 ^ bit == bit && 0u128 ^ 0u128 == 0) by(bit_vector);
    if row < cut.start {} else if row == cut.start {} else if row < cut.end {}
    else if row == cut.end {} else {}
}

pub(super) proof fn coverage_transition(cuts: Seq<VerticalCut>, count: int, row: int)
    requires 0 <= count <= cuts.len(), 0 <= row < 64,
        forall|i: int| 0 <= i < count ==> #[trigger] vertical_valid(cuts[i]),
    ensures coverage(cuts, count, row) == coverage(cuts, count, row - 1) ^ events(cuts, count, row),
    decreases count,
{
    assert(0u128 ^ 0u128 == 0) by(bit_vector);
    if count > 0 {
        coverage_transition(cuts, count - 1, row);
        cut_transition(cuts[count - 1], row);
        let a = coverage(cuts, count - 1, row - 1);
        let b = events(cuts, count - 1, row);
        let c = cover(cuts[count - 1], row - 1);
        let d = event(cuts[count - 1], row);
        assert((a ^ b) ^ (c ^ d) == (a ^ c) ^ (b ^ d)) by(bit_vector);
    }
}

pub(super) proof fn before_zero(cuts: Seq<VerticalCut>, count: int)
    requires 0 <= count <= cuts.len(),
    ensures coverage(cuts, count, -1) == 0,
    decreases count,
{
    assert(0u128 ^ 0u128 == 0) by(bit_vector);
    if count > 0 { before_zero(cuts, count - 1); }
}

pub(super) proof fn cover_bit(cut: VerticalCut, row: int, x: u32)
    requires vertical_valid(cut), x < 128,
    ensures bit128(cover(cut, row), x) <==> cut.x == x && cut.start <= row < cut.end,
{
    let p = cut.x as u32;
    let enabled = cut.start <= row < cut.end;
    let mask = if enabled { 1u128 << p } else { 0u128 };
    assert(bit128(mask, x) <==> enabled && p == x) by(bit_vector)
        requires p < 128, x < 128, mask == if enabled { 1u128 << p } else { 0u128 };
}

pub(super) proof fn bit_zero(mask: u128, x: u32)
    ensures !bit128(mask, x) <==> (mask >> x) & 1 == 0,
{
    assert(!bit128(mask, x) <==> (mask >> x) & 1 == 0) by(bit_vector);
}

pub(super) proof fn coverage_bit(cuts: Seq<VerticalCut>, count: int, row: int, x: u32)
    requires vertical_input(cuts), 0 <= count <= cuts.len(), x < 128,
    ensures bit128(coverage(cuts, count, row), x) <==> active(cuts, count, row, x),
    decreases count,
{
    if count == 0 {
        assert(!bit128(0, x)) by(bit_vector);
    } else {
        coverage_bit(cuts, count - 1, row, x);
        let cut = cuts[count - 1];
        cover_bit(cut, row, x);
        let previous = coverage(cuts, count - 1, row);
        let addition = cover(cut, row);
        bit_zero(previous, x); bit_zero(addition, x);
        if cut.x == x && cut.start <= row < cut.end {
            if active(cuts, count - 1, row, x) {
                let i = choose|i: int| 0 <= i < count - 1 && #[trigger] cuts[i].x == x
                    && cuts[i].start <= row < cuts[i].end;
                assert(cuts[i].end < cut.start || cut.end < cuts[i].start);
            }
            assert(!bit128(previous, x));
            assert(bit128(previous ^ addition, x)) by(bit_vector)
                requires (previous >> x) & 1 == 0, (addition >> x) & 1 == 1;
            assert(active(cuts, count, row, x));
        } else {
            assert(bit128(previous ^ addition, x) == bit128(previous, x)) by(bit_vector)
                requires (addition >> x) & 1 == 0;
            if active(cuts, count, row, x) {
                let i = choose|i: int| 0 <= i < count && #[trigger] cuts[i].x == x
                    && cuts[i].start <= row < cuts[i].end;
                assert(i < count - 1);
                assert(active(cuts, count - 1, row, x));
            }
        }
    }
}

pub(super) open spec fn vertical_marked(cuts: Seq<VerticalCut>, count: int, boundaries: u64) -> bool {
    forall|i: int| 0 <= i < count ==> #[trigger] bit64(boundaries, cuts[i].start) && bit64(boundaries, cuts[i].end)
}

pub(super) open spec fn vertical_events(cuts: Seq<VerticalCut>, count: int, masks: Seq<u128>, boundaries: u64) -> bool {
    vertical_input(cuts) && 0 <= count <= cuts.len() && masks.len() == 64
        && vertical_marked(cuts, count, boundaries)
        && forall|row: int| 0 <= row < 64 ==> #[trigger] masks[row] == events(cuts, count, row)
}

pub(super) open spec fn inserted(masks: Seq<u128>, cut: VerticalCut) -> Seq<u128> {
    masks.update(cut.start as int, masks[cut.start as int] ^ cut_bit(cut))
        .update(cut.end as int, masks[cut.end as int] ^ cut_bit(cut))
}

pub(super) proof fn marked(before: u64, after: u64, a: u8, b: u8)
    requires a < 64, b < 64, after == before | (1u64 << a) | (1u64 << b),
    ensures bit64(after, a), bit64(after, b),
        forall|p: u8| bit64(before, p) ==> #[trigger] bit64(after, p),
{
    assert(bit64(after, a) && bit64(after, b)) by(bit_vector)
        requires a < 64, b < 64, after == before | (1u64 << a) | (1u64 << b);
    assert forall|p: u8| bit64(before, p) implies #[trigger] bit64(after, p) by {
        assert(bit64(after, p)) by(bit_vector)
            requires (before >> p) & 1 == 1, after == before | (1u64 << a) | (1u64 << b);
    }
}

pub(super) proof fn vertical_started(cuts: Seq<VerticalCut>)
    requires vertical_input(cuts),
    ensures vertical_events(cuts, 0, Seq::new(64, |_: int| 0u128), 0),
{}

pub(super) proof fn vertical_inserted(cuts: Seq<VerticalCut>, count: int,
    before: Seq<u128>, after: Seq<u128>, old_boundaries: u64, boundaries: u64)
    requires vertical_events(cuts, count, before, old_boundaries), count < cuts.len(),
        after == inserted(before, cuts[count]),
        boundaries == old_boundaries | (1u64 << cuts[count].start) | (1u64 << cuts[count].end),
    ensures vertical_events(cuts, count + 1, after, boundaries),
{
    let cut = cuts[count];
    assert(vertical_valid(cut));
    marked(old_boundaries, boundaries, cut.start, cut.end);
    assert forall|row: int| 0 <= row < 64 implies #[trigger] after[row] == events(cuts, count + 1, row) by {
        let previous = before[row];
        assert(previous ^ 0 == previous) by(bit_vector);
        if row == cut.start {} else if row == cut.end {} else {}
    }
}

pub(super) open spec fn vertical_prefix(cuts: Seq<VerticalCut>, masks: Seq<u128>, row: int, active: u128) -> bool {
    vertical_input(cuts) && masks.len() == 64 && 0 <= row <= 64
        && active == if row == 0 { 0 } else { coverage(cuts, cuts.len() as int, row - 1) }
        && (forall|i: int| 0 <= i < row ==> #[trigger] masks[i] == coverage(cuts, cuts.len() as int, i))
        && (forall|i: int| row <= i < 64 ==> #[trigger] masks[i] == events(cuts, cuts.len() as int, i))
}

pub(super) proof fn prefix_started(cuts: Seq<VerticalCut>, masks: Seq<u128>, boundaries: u64)
    requires vertical_events(cuts, cuts.len() as int, masks, boundaries),
    ensures vertical_prefix(cuts, masks, 0, 0),
{}

pub(super) proof fn prefix_step(cuts: Seq<VerticalCut>, before: Seq<u128>, after: Seq<u128>, row: int,
    previous: u128, active: u128)
    requires vertical_prefix(cuts, before, row, previous), row < 64,
        active == previous ^ before[row], after == before.update(row, active),
    ensures vertical_prefix(cuts, after, row + 1, active),
{
    coverage_transition(cuts, cuts.len() as int, row);
    if row == 0 { before_zero(cuts, cuts.len() as int); }
    assert(active == coverage(cuts, cuts.len() as int, row));
}

pub(super) open spec fn horizontal_active(cuts: Seq<HorizontalCut>, count: int, row: int, x: u8) -> bool {
    exists|i: int| 0 <= i < count && #[trigger] cuts[i].y == row && cuts[i].start <= x < cuts[i].end
}

pub(super) open spec fn horizontal_marked(cuts: Seq<HorizontalCut>, count: int, boundaries: u64) -> bool {
    forall|i: int| 0 <= i < count ==> #[trigger] bit64(boundaries, cuts[i].y)
}

pub(super) open spec fn horizontal_progress(cuts: Seq<HorizontalCut>, count: int, masks: Seq<u64>, boundaries: u64) -> bool {
    horizontal_input(cuts) && 0 <= count <= cuts.len() && masks.len() == 64
        && horizontal_marked(cuts, count, boundaries)
        && forall|row: int, x: u8| 0 <= row < 64 && x < 64 ==>
            (#[trigger] bit64(masks[row], x) <==> horizontal_active(cuts, count, row, x))
}

pub(super) open spec fn built(horizontal: Seq<HorizontalCut>, vertical: Seq<VerticalCut>,
    horizontal_masks: Seq<u64>, vertical_masks: Seq<u128>, boundaries: u64) -> bool {
    horizontal_progress(horizontal, horizontal.len() as int, horizontal_masks, boundaries)
        && vertical_input(vertical) && vertical_masks.len() == 64
        && vertical_marked(vertical, vertical.len() as int, boundaries)
        && forall|row: int| 0 <= row < 64 ==> #[trigger] vertical_masks[row]
            == coverage(vertical, vertical.len() as int, row)
}

pub(super) proof fn horizontal_started(cuts: Seq<HorizontalCut>, boundaries: u64)
    requires horizontal_input(cuts),
    ensures horizontal_progress(cuts, 0, Seq::new(64, |_: int| 0u64), boundaries),
{
    assert forall|x: u8| x < 64 implies
        (!#[trigger] bit64(0, x)) by { assert((0u64 >> x) & 1 == 0) by(bit_vector); }
}

pub(super) proof fn horizontal_step(cuts: Seq<HorizontalCut>, count: int,
    before: Seq<u64>, after: Seq<u64>, old_boundaries: u64, boundaries: u64, range: u64)
    requires horizontal_progress(cuts, count, before, old_boundaries), count < cuts.len(),
        horizontal_valid(cuts[count]),
        crate::geometry::cell_bits(cuts[count].start, cuts[count].end, range),
        after == before.update(cuts[count].y as int, before[cuts[count].y as int] | range),
        boundaries == old_boundaries | (1u64 << cuts[count].y),
    ensures horizontal_progress(cuts, count + 1, after, boundaries),
        forall|p: u8| bit64(old_boundaries, p) ==> #[trigger] bit64(boundaries, p),
{
    let cut = cuts[count];
    assert(horizontal_valid(cut));
    let y = cut.y;
    assert(boundaries == old_boundaries | (1u64 << y) | (1u64 << y)) by(bit_vector)
        requires boundaries == old_boundaries | (1u64 << y);
    marked(old_boundaries, boundaries, cut.y, cut.y);
    assert forall|row: int, x: u8| 0 <= row < 64 && x < 64 implies
        (#[trigger] bit64(after[row], x) <==> horizontal_active(cuts, count + 1, row, x)) by {
        let previous = before[row];
        if row == cut.y {
            assert(bit64(previous | range, x) <==> bit64(previous, x) || bit64(range, x)) by(bit_vector);
        }
        if horizontal_active(cuts, count + 1, row, x) {
            let i = choose|i: int| 0 <= i < count + 1 && #[trigger] cuts[i].y == row
                && cuts[i].start <= x < cuts[i].end;
            if i < count { assert(horizontal_active(cuts, count, row, x)); }
        }
        if horizontal_active(cuts, count, row, x) {
            let i = choose|i: int| 0 <= i < count && #[trigger] cuts[i].y == row
                && cuts[i].start <= x < cuts[i].end;
            assert(horizontal_active(cuts, count + 1, row, x));
        }
        if row == cut.y && cut.start <= x < cut.end {
            assert(horizontal_active(cuts, count + 1, row, x));
        }
    }
}

pub(super) proof fn vertical_marks_preserved(cuts: Seq<VerticalCut>, count: int, before: u64, after: u64)
    requires 0 <= count <= cuts.len(), vertical_marked(cuts, count, before),
        forall|p: u8| bit64(before, p) ==> #[trigger] bit64(after, p),
    ensures vertical_marked(cuts, count, after),
{
    assert forall|i: int| 0 <= i < count implies #[trigger] bit64(after, cuts[i].start) && bit64(after, cuts[i].end) by {
        assert(bit64(before, cuts[i].start));
        assert(bit64(before, cuts[i].end));
    }
}

}
