//! 实际最大轴区间、相邻区间交集与像素 good chord 的桥。

use super::proof::{interval_valid, line_valid, line_value, value_at};
use super::{AxisIntervals, EffectiveChord, Interval, Orientation, ValuedChord};
use crate::partition::{corners, good};
use vstd::prelude::*;

verus! {

pub(super) open spec fn native_good(labels: Seq<u16>, vertical: bool, chord: ValuedChord) -> bool {
    chord.well_formed(vertical) && good::chord(labels, vertical, chord.line(), chord.start(), chord.end(), chord.value)
}

pub(super) open spec fn ids(chords: Seq<ValuedChord>) -> Seq<int> {
    chords.map_values(|chord: ValuedChord| chord.id())
}

pub(super) open spec fn all_good(labels: Seq<u16>, vertical: bool, chords: Seq<ValuedChord>) -> bool {
    forall|i: int| 0 <= i < chords.len() ==> #[trigger] native_good(labels, vertical, chords[i])
}

pub(super) open spec fn ordered(chords: Seq<ValuedChord>) -> bool {
    forall|i: int, j: int| 0 <= i < j < chords.len() ==> (#[trigger] chords[i]).id() < (#[trigger] chords[j]).id()
}

proof fn same_ordered_prefix(first: Seq<ValuedChord>, second: Seq<ValuedChord>, count: int)
    requires ordered(first), ordered(second), ids(first).to_set() == ids(second).to_set(),
        0 <= count <= first.len(), count <= second.len(),
    ensures forall|i: int| 0 <= i < count ==> (#[trigger] first[i]).id() == (#[trigger] second[i]).id(),
    decreases count,
{
    broadcast use vstd::seq_lib::group_seq_properties;
    if count > 0 {
        same_ordered_prefix(first, second, count - 1);
        let i = count - 1;
        assert(ids(first)[i] == first[i].id());
        assert(ids(second)[i] == second[i].id());
        assert(ids(first).contains(first[i].id()));
        assert(ids(second).contains(second[i].id()));
        assert(ids(first).to_set().contains(first[i].id()));
        assert(ids(second).to_set().contains(second[i].id()));
        assert(ids(second).contains(first[i].id()));
        assert(ids(first).contains(second[i].id()));
        let a = choose|a: int| 0 <= a < first.len() && ids(first)[a] == second[i].id();
        let b = choose|b: int| 0 <= b < second.len() && ids(second)[b] == first[i].id();
        if a < i { assert(first[a].id() == second[a].id()); }
        if b < i { assert(first[b].id() == second[b].id()); }
        assert(a >= i && b >= i);
        assert(first[i].id() == second[i].id());
    }
}

/// 完整 good 集加实际枚举顺序确定整个 chord 序列，不受区间底层存储历史影响。
pub(super) proof fn extraction_unique(labels: Seq<u16>, vertical: bool,
    first: Seq<ValuedChord>, second: Seq<ValuedChord>)
    requires axis_collected(labels, vertical, first, 64), axis_collected(labels, vertical, second, 64),
    ensures first == second,
{
    broadcast use vstd::seq_lib::group_seq_properties;
    extraction_complete(labels, vertical, first);
    extraction_complete(labels, vertical, second);
    ids(first).unique_seq_to_set();
    ids(second).unique_seq_to_set();
    assert(first.len() == second.len());
    same_ordered_prefix(first, second, first.len() as int);
    assert forall|i: int| 0 <= i < first.len() implies #[trigger] first[i] == second[i] by {
        assert(native_good(labels, vertical, first[i]));
        assert(native_good(labels, vertical, second[i]));
        native_id(labels, vertical, first[i]);
        native_id(labels, vertical, second[i]);
        assert(first[i].well_formed(vertical) && second[i].well_formed(vertical));
        assert(first[i].line() == second[i].line() && first[i].start() == second[i].start()
            && first[i].end() == second[i].end() && first[i].value == second[i].value);
        if vertical {
            assert(first[i].chord.orientation is Vertical && second[i].chord.orientation is Vertical);
            assert(first[i].chord.x2 == second[i].chord.x2);
        } else {
            assert(first[i].chord.orientation is Horizontal && second[i].chord.orientation is Horizontal);
            assert(first[i].chord.y2 == second[i].chord.y2);
        }
        assert(first[i].chord == second[i].chord);
    }
    assert(first =~= second);
}

pub(super) proof fn interval_start_bound(items: Seq<Interval>, i: int)
    requires line_valid(items), 0 <= i < items.len(),
    ensures i <= items[i].start,
    decreases i,
{
    if i > 0 {
        interval_start_bound(items, i - 1);
        assert(interval_valid(items[i - 1]));
        assert(items[i - 1].end <= items[i].start);
    }
}

pub(super) proof fn line_count(items: Seq<Interval>)
    requires line_valid(items),
    ensures items.len() <= 64,
{
    if items.len() > 0 {
        interval_start_bound(items, items.len() - 1);
        assert(interval_valid(items.last()));
    }
}

pub(super) open spec fn suffix_on_line(items: Seq<ValuedChord>, start: int, coordinate: u8) -> bool {
    forall|p: int| start <= p < items.len() ==> #[trigger] items[p].line() == coordinate
}

pub(super) proof fn suffix_preserved(before: Seq<ValuedChord>, after: Seq<ValuedChord>, coordinate: u8)
    requires before.len() <= after.len(), after.take(before.len() as int) == before,
        suffix_on_line(after, before.len() as int, coordinate),
    ensures forall|start: int| 0 <= start <= before.len() && suffix_on_line(before, start, coordinate)
        ==> #[trigger] suffix_on_line(after, start, coordinate),
{
    assert forall|start: int| 0 <= start <= before.len() && suffix_on_line(before, start, coordinate)
        implies #[trigger] suffix_on_line(after, start, coordinate) by {
        assert forall|p: int| start <= p < after.len() implies #[trigger] after[p].line() == coordinate by {
            if p < before.len() { assert(after[p] == before[p]); }
        }
    }
}

pub(super) proof fn prefix_ids(before: Seq<ValuedChord>, after: Seq<ValuedChord>)
    requires before.len() <= after.len(), after.take(before.len() as int) == before,
    ensures forall|id: int| ids(before).contains(id) ==> #[trigger] ids(after).contains(id),
{
    broadcast use vstd::seq_lib::group_seq_properties;
    assert forall|id: int| ids(before).contains(id) implies #[trigger] ids(after).contains(id) by {
        let i = choose|i: int| 0 <= i < before.len() && ids(before)[i] == id;
        assert(after[i] == before[i]);
        assert(ids(after)[i] == id);
    }
}

pub(super) proof fn native_id(labels: Seq<u16>, vertical: bool, chord: ValuedChord)
    requires native_good(labels, vertical, chord),
    ensures good::good(labels, chord.id()), good::vertical(chord.id()) == vertical,
        good::line(chord.id()) == chord.line(), good::start(chord.id()) == chord.start(),
        good::end(chord.id()) == chord.end(), good::value(labels, chord.id()) == chord.value,
{
    good::encoded_chord(labels, vertical, chord.line(), chord.start(), chord.end(), chord.value);
}

pub(super) proof fn append_capacity(labels: Seq<u16>, vertical: bool, before: Seq<ValuedChord>, chord: ValuedChord)
    requires all_good(labels, vertical, before), ids(before).no_duplicates(),
        native_good(labels, vertical, chord), !ids(before).contains(chord.id()),
    ensures before.len() < 1953, all_good(labels, vertical, before.push(chord)),
        ids(before.push(chord)).no_duplicates(), ids(before.push(chord)) == ids(before).push(chord.id()),
{
    broadcast use vstd::seq_lib::group_seq_properties;
    broadcast use vstd::set_lib::group_set_properties;
    let after = before.push(chord);
    assert(ids(after) =~= ids(before).push(chord.id()));
    assert forall|i: int| 0 <= i < after.len() implies #[trigger] native_good(labels, vertical, after[i]) by {
        if i < before.len() { assert(after[i] == before[i]); }
    }
    assert forall|i: int, j: int| 0 <= i < j < ids(after).len() implies #[trigger] ids(after)[i] != #[trigger] ids(after)[j] by {
        if j < before.len() { assert(ids(after)[j] == ids(before)[j]); }
        else { assert(!ids(before).contains(chord.id())); }
    }
    assert(ids(after).no_duplicates());
    let produced = ids(after).to_set();
    ids(after).unique_seq_to_set();
    vstd::set_lib::lemma_int_range(0, 524288);
    assert(produced.subset_of(good::chords(labels, vertical))) by {
        assert forall|id: int| #[trigger] produced.contains(id) implies good::chords(labels, vertical).contains(id) by {
            let i = choose|i: int| 0 <= i < after.len() && ids(after)[i] == id;
            native_id(labels, vertical, after[i]);
        }
    }
    good::chord_capacity(labels, vertical);
    vstd::set_lib::lemma_len_subset(produced, good::chords(labels, vertical));
}

pub(super) open spec fn cell(labels: Seq<u16>, columns: bool, line: int, p: int) -> u16 {
    if columns { corners::at(labels, line, p) } else { corners::at(labels, p, line) }
}

pub(super) open spec fn line_matches(labels: Seq<u16>, columns: bool, line: int, items: Seq<Interval>) -> bool {
    labels.len() == 4096 && 0 <= line < 64 && line_valid(items)
        && forall|p: int| 0 <= p < 64 ==> #[trigger] line_value(items, p) == cell(labels, columns, line, p)
}

pub(super) proof fn axis_line(axis: &AxisIntervals, labels: Seq<u16>, columns: bool, line: int)
    requires axis.represents(labels, columns), 0 <= line < 64,
    ensures line_matches(labels, columns, line, axis.line_spec(line)),
{
    assert forall|p: int| 0 <= p < 64 implies #[trigger] line_value(axis.line_spec(line), p)
        == cell(labels, columns, line, p) by {
        if columns { corners::cell_coordinates(line, p); }
        else { corners::cell_coordinates(p, line); }
    }
}

pub(super) proof fn adjacent_distinct(items: Seq<Interval>, i: int, p: int)
    requires line_valid(items), 0 <= i < items.len(), p == items[i].start - 1 || p == items[i].end,
    ensures line_value(items, p) != items[i].value,
{
    assert(interval_valid(items[i]));
    if exists|j: int| 0 <= j < items.len() && #[trigger] items[j].start <= p < items[j].end {
        let j = choose|j: int| 0 <= j < items.len() && #[trigger] items[j].start <= p < items[j].end;
        value_at(items, p, j);
        assert(interval_valid(items[j]));
        if p == items[i].end {
            if j <= i { if j < i { assert(items[j].end <= items[i].start); } }
            assert(i < j);
            assert(items[i].end <= items[j].start);
        } else {
            if i <= j { if i < j { assert(items[i].end <= items[j].start); } }
            assert(j < i);
            assert(items[j].end <= items[i].start);
        }
    }
}

pub(super) open spec fn maximal(labels: Seq<u16>, columns: bool, line: int, interval: Interval) -> bool {
    interval_valid(interval) && cell(labels, columns, line, interval.start - 1) != interval.value
        && cell(labels, columns, line, interval.end as int) != interval.value
        && forall|p: int| interval.start <= p < interval.end ==>
            #[trigger] cell(labels, columns, line, p) == interval.value
}

pub(super) proof fn interval_maximal(labels: Seq<u16>, columns: bool, line: int, items: Seq<Interval>, i: int)
    requires line_matches(labels, columns, line, items), 0 <= i < items.len(),
    ensures maximal(labels, columns, line, items[i]),
{
    let interval = items[i];
    assert(interval_valid(interval));
    adjacent_distinct(items, i, interval.start - 1);
    adjacent_distinct(items, i, interval.end as int);
    assert forall|p: int| interval.start <= p < interval.end implies
        #[trigger] cell(labels, columns, line, p) == interval.value by {
        value_at(items, p, i);
    }
}

pub(super) open spec fn eligible(a: Interval, b: Interval) -> bool {
    a.value == b.value && a.start != b.start && a.end != b.end
        && vstd::math::max(a.start as int, b.start as int) < vstd::math::min(a.end as int, b.end as int)
}

pub(super) proof fn pair_good(labels: Seq<u16>, columns: bool, coordinate: int,
    first: Seq<Interval>, second: Seq<Interval>, i: int, j: int)
    requires 1 <= coordinate <= 63, line_matches(labels, columns, coordinate - 1, first),
        line_matches(labels, columns, coordinate, second),
        0 <= i < first.len(), 0 <= j < second.len(), eligible(first[i], second[j]),
    ensures good::chord(labels, columns, coordinate,
        vstd::math::max(first[i].start as int, second[j].start as int),
        vstd::math::min(first[i].end as int, second[j].end as int), first[i].value),
{
    interval_maximal(labels, columns, coordinate - 1, first, i);
    interval_maximal(labels, columns, coordinate, second, j);
    let a = first[i]; let b = second[j];
    let start = vstd::math::max(a.start as int, b.start as int);
    let end = vstd::math::min(a.end as int, b.end as int);
    let value = a.value;
    assert(1 <= start < end <= 63);
    if a.start < b.start {
        assert(cell(labels, columns, coordinate - 1, start - 1) == value);
        assert(cell(labels, columns, coordinate, start - 1) != value);
    } else {
        assert(cell(labels, columns, coordinate - 1, start - 1) != value);
        assert(cell(labels, columns, coordinate, start - 1) == value);
    }
    assert(cell(labels, columns, coordinate - 1, start) == value);
    assert(cell(labels, columns, coordinate, start) == value);
    if a.end < b.end {
        assert(cell(labels, columns, coordinate - 1, end) != value);
        assert(cell(labels, columns, coordinate, end) == value);
    } else {
        assert(cell(labels, columns, coordinate - 1, end) == value);
        assert(cell(labels, columns, coordinate, end) != value);
    }
    assert(cell(labels, columns, coordinate - 1, end - 1) == value);
    assert(cell(labels, columns, coordinate, end - 1) == value);
    if columns {} else {}
    assert(good::reflex(labels, columns, coordinate, start, value));
    assert(good::reflex(labels, columns, coordinate, end, value));
    assert forall|p: int| start <= p < end implies #[trigger] good::unit_inside(labels, columns, coordinate, p, value) by {
        assert(cell(labels, columns, coordinate - 1, p) == value);
        assert(cell(labels, columns, coordinate, p) == value);
    }
}

pub(super) proof fn find_interval(labels: Seq<u16>, columns: bool, line: int,
    items: Seq<Interval>, p: int, value: u16)
    requires line_matches(labels, columns, line, items), 0 <= p < 64,
        cell(labels, columns, line, p) == value, value != 0,
    ensures exists|i: int| 0 <= i < items.len() && #[trigger] items[i].value == value
        && items[i].start <= p < items[i].end,
{
    assert(line_value(items, p) == value);
    assert(exists|i: int| 0 <= i < items.len() && #[trigger] items[i].start <= p < items[i].end);
    let i = choose|i: int| 0 <= i < items.len() && #[trigger] items[i].start <= p < items[i].end;
    value_at(items, p, i);
}

pub(super) proof fn contained_prefix(labels: Seq<u16>, columns: bool, line: int,
    items: Seq<Interval>, i: int, start: int, end: int, value: u16)
    requires line_matches(labels, columns, line, items), 0 <= i < items.len(),
        0 <= start < end <= 64, items[i].start <= start < items[i].end, items[i].value == value,
        forall|p: int| start <= p < end ==> #[trigger] cell(labels, columns, line, p) == value,
    ensures end <= items[i].end,
{
    interval_maximal(labels, columns, line, items, i);
    if items[i].end < end {
        assert(cell(labels, columns, line, items[i].end as int) == value);
    }
}

/// 任意像素 good chord 在两条真实最大轴区间中有且仅有对应的正交集对。
pub(super) proof fn find_pair(labels: Seq<u16>, columns: bool, coordinate: int,
    first: Seq<Interval>, second: Seq<Interval>, start: int, end: int, value: u16)
    requires good::chord(labels, columns, coordinate, start, end, value),
        line_matches(labels, columns, coordinate - 1, first),
        line_matches(labels, columns, coordinate, second),
    ensures exists|i: int, j: int| 0 <= i < first.len() && 0 <= j < second.len()
        && #[trigger] eligible(first[i], second[j]) && first[i].value == value
        && vstd::math::max(first[i].start as int, second[j].start as int) == start
        && vstd::math::min(first[i].end as int, second[j].end as int) == end,
{
    assert(good::unit_inside(labels, columns, coordinate, start, value));
    assert forall|p: int| start <= p < end implies
        #[trigger] cell(labels, columns, coordinate - 1, p) == value by {
        assert(good::unit_inside(labels, columns, coordinate, p, value));
    }
    assert forall|p: int| start <= p < end implies
        #[trigger] cell(labels, columns, coordinate, p) == value by {
        assert(good::unit_inside(labels, columns, coordinate, p, value));
    }
    find_interval(labels, columns, coordinate - 1, first, start, value);
    find_interval(labels, columns, coordinate, second, start, value);
    let i = choose|i: int| 0 <= i < first.len() && #[trigger] first[i].value == value
        && first[i].start <= start < first[i].end;
    let j = choose|j: int| 0 <= j < second.len() && #[trigger] second[j].value == value
        && second[j].start <= start < second[j].end;
    contained_prefix(labels, columns, coordinate - 1, first, i, start, end, value);
    contained_prefix(labels, columns, coordinate, second, j, start, end, value);
    interval_maximal(labels, columns, coordinate - 1, first, i);
    interval_maximal(labels, columns, coordinate, second, j);
    let a = first[i]; let b = second[j];
    good::endpoint_blocks(labels, columns, coordinate, start, end, value);
    if a.start < start && b.start < start {
        assert(cell(labels, columns, coordinate - 1, start - 1) == value);
        assert(cell(labels, columns, coordinate, start - 1) == value);
    }
    if end < a.end && end < b.end {
        assert(cell(labels, columns, coordinate - 1, end) == value);
        assert(cell(labels, columns, coordinate, end) == value);
    }
    assert(vstd::math::max(a.start as int, b.start as int) == start);
    assert(vstd::math::min(a.end as int, b.end as int) == end);
    if a.start == b.start {
        assert(cell(labels, columns, coordinate - 1, start - 1) != value);
        assert(cell(labels, columns, coordinate, start - 1) != value);
        assert(good::quadrants(labels, if columns {coordinate} else {start}, if columns {start} else {coordinate}, value) == 2);
    }
    if a.end == b.end {
        assert(cell(labels, columns, coordinate - 1, end) != value);
        assert(cell(labels, columns, coordinate, end) != value);
        assert(good::unit_inside(labels, columns, coordinate, end - 1, value));
        assert(good::quadrants(labels, if columns {coordinate} else {end}, if columns {end} else {coordinate}, value) == 2);
    }
    assert(eligible(a, b));
}

pub(super) open spec fn converted(vertical: bool, coordinate: u8, a: Interval, b: Interval) -> ValuedChord {
    let start = vstd::math::max(a.start as int, b.start as int) as u8;
    let end = vstd::math::min(a.end as int, b.end as int) as u8;
    ValuedChord { value: a.value, chord: if vertical {
        EffectiveChord { orientation: Orientation::Vertical, x1: coordinate, y1: start, x2: coordinate, y2: end }
    } else {
        EffectiveChord { orientation: Orientation::Horizontal, x1: start, y1: coordinate, x2: end, y2: coordinate }
    } }
}

pub(super) proof fn pair_native(labels: Seq<u16>, vertical: bool, coordinate: u8,
    first: Seq<Interval>, second: Seq<Interval>, i: int, j: int)
    requires 1 <= coordinate <= 63, line_matches(labels, vertical, coordinate - 1, first),
        line_matches(labels, vertical, coordinate as int, second),
        0 <= i < first.len(), 0 <= j < second.len(), eligible(first[i], second[j]),
    ensures native_good(labels, vertical, converted(vertical, coordinate, first[i], second[j])),
{
    pair_good(labels, vertical, coordinate as int, first, second, i, j);
}

pub(super) proof fn pair_unique(labels: Seq<u16>, vertical: bool, coordinate: u8,
    first: Seq<Interval>, second: Seq<Interval>, i: int, j: int, a: int, b: int)
    requires 1 <= coordinate <= 63, line_matches(labels, vertical, coordinate - 1, first),
        line_matches(labels, vertical, coordinate as int, second),
        0 <= i < first.len(), 0 <= j < second.len(), eligible(first[i], second[j]),
        0 <= a < first.len(), 0 <= b < second.len(), eligible(first[a], second[b]),
        converted(vertical, coordinate, first[i], second[j]).id() == converted(vertical, coordinate, first[a], second[b]).id(),
    ensures i == a, j == b,
{
    let left = converted(vertical, coordinate, first[i], second[j]);
    let right = converted(vertical, coordinate, first[a], second[b]);
    pair_native(labels, vertical, coordinate, first, second, i, j);
    pair_native(labels, vertical, coordinate, first, second, a, b);
    native_id(labels, vertical, left);
    native_id(labels, vertical, right);
    assert(left.start() == right.start());
    super::proof::unique_interval(first, left.start(), i, a);
    super::proof::unique_interval(second, left.start(), j, b);
}

pub(super) open spec fn processed(vertical: bool, coordinate: u8, first: Seq<Interval>, second: Seq<Interval>,
    i: int, j: int, id: int) -> bool {
    exists|a: int, b: int| 0 <= a < first.len() && 0 <= b < second.len() && (a < i || b < j)
        && #[trigger] eligible(first[a], second[b]) && converted(vertical, coordinate, first[a], second[b]).id() == id
}

pub(super) proof fn pair_advance(vertical: bool, coordinate: u8, first: Seq<Interval>, second: Seq<Interval>,
    i: int, j: int, next_i: int, next_j: int, id: int)
    requires line_valid(first), line_valid(second), 0 <= i < first.len(), 0 <= j < second.len(),
        next_i == i + if first[i].end <= second[j].end { 1int } else { 0int },
        next_j == j + if second[j].end <= first[i].end { 1int } else { 0int },
    ensures processed(vertical, coordinate, first, second, next_i, next_j, id)
        <==> processed(vertical, coordinate, first, second, i, j, id)
            || (eligible(first[i], second[j]) && converted(vertical, coordinate, first[i], second[j]).id() == id),
{
    if processed(vertical, coordinate, first, second, next_i, next_j, id) {
        let (a, b) = choose|a: int, b: int| 0 <= a < first.len() && 0 <= b < second.len()
            && (a < next_i || b < next_j) && #[trigger] eligible(first[a], second[b])
            && converted(vertical, coordinate, first[a], second[b]).id() == id;
        if a < i || b < j {
            assert(processed(vertical, coordinate, first, second, i, j, id));
        } else {
            if a == i && j < b {
                assert(first[i].end <= second[j].end <= second[b].start);
            }
            if b == j && i < a {
                assert(second[j].end <= first[i].end <= first[a].start);
            }
            assert(a == i && b == j);
        }
    }
    if processed(vertical, coordinate, first, second, i, j, id) {
        let (a, b) = choose|a: int, b: int| 0 <= a < first.len() && 0 <= b < second.len()
            && (a < i || b < j) && #[trigger] eligible(first[a], second[b])
            && converted(vertical, coordinate, first[a], second[b]).id() == id;
        assert(a < next_i || b < next_j);
        assert(processed(vertical, coordinate, first, second, next_i, next_j, id));
    }
    if eligible(first[i], second[j]) && converted(vertical, coordinate, first[i], second[j]).id() == id {
        assert(i < next_i || j < next_j);
        assert(processed(vertical, coordinate, first, second, next_i, next_j, id));
    }
}

pub(super) open spec fn collector_progress(labels: Seq<u16>, vertical: bool, coordinate: u8,
    first: Seq<Interval>, second: Seq<Interval>, output: Seq<ValuedChord>, i: int, j: int) -> bool {
    1 <= coordinate <= 63 && line_matches(labels, vertical, coordinate - 1, first)
        && line_matches(labels, vertical, coordinate as int, second)
        && 0 <= i <= first.len() && 0 <= j <= second.len()
        && all_good(labels, vertical, output) && ids(output).no_duplicates() && ordered(output)
        && (forall|p: int| 0 <= p < output.len() ==> #[trigger] output[p].line() <= coordinate)
        && (forall|id: int| good::line(id) == coordinate ==> (#[trigger] ids(output).contains(id)
            <==> processed(vertical, coordinate, first, second, i, j, id)))
}

pub(super) proof fn collector_started(labels: Seq<u16>, vertical: bool, coordinate: u8,
    first: Seq<Interval>, second: Seq<Interval>, output: Seq<ValuedChord>)
    requires 1 <= coordinate <= 63, line_matches(labels, vertical, coordinate - 1, first),
        line_matches(labels, vertical, coordinate as int, second),
        all_good(labels, vertical, output), ids(output).no_duplicates(), ordered(output),
        forall|p: int| 0 <= p < output.len() ==> #[trigger] output[p].line() < coordinate,
    ensures collector_progress(labels, vertical, coordinate, first, second, output, 0, 0),
{
    broadcast use vstd::seq_lib::group_seq_properties;
    assert forall|id: int| good::line(id) == coordinate implies (!#[trigger] ids(output).contains(id)) by {
        if ids(output).contains(id) {
            let p = choose|p: int| 0 <= p < output.len() && ids(output)[p] == id;
            native_id(labels, vertical, output[p]);
        }
    }
}

pub(super) proof fn candidate_ready(labels: Seq<u16>, vertical: bool, coordinate: u8,
    first: Seq<Interval>, second: Seq<Interval>, output: Seq<ValuedChord>, i: int, j: int)
    requires collector_progress(labels, vertical, coordinate, first, second, output, i, j),
        i < first.len(), j < second.len(), eligible(first[i], second[j]),
    ensures output.len() < 1953,
        native_good(labels, vertical, converted(vertical, coordinate, first[i], second[j])),
        !ids(output).contains(converted(vertical, coordinate, first[i], second[j]).id()),
{
    let chord = converted(vertical, coordinate, first[i], second[j]);
    pair_native(labels, vertical, coordinate, first, second, i, j);
    native_id(labels, vertical, chord);
    if ids(output).contains(chord.id()) {
        assert(processed(vertical, coordinate, first, second, i, j, chord.id()));
        let (a, b) = choose|a: int, b: int| 0 <= a < first.len() && 0 <= b < second.len()
            && (a < i || b < j) && #[trigger] eligible(first[a], second[b])
            && converted(vertical, coordinate, first[a], second[b]).id() == chord.id();
        pair_unique(labels, vertical, coordinate, first, second, i, j, a, b);
    }
    append_capacity(labels, vertical, output, chord);
}

pub(super) proof fn candidate_last(labels: Seq<u16>, vertical: bool, coordinate: u8,
    first: Seq<Interval>, second: Seq<Interval>, output: Seq<ValuedChord>, i: int, j: int)
    requires collector_progress(labels, vertical, coordinate, first, second, output, i, j),
        i < first.len(), j < second.len(), eligible(first[i], second[j]),
    ensures ordered(output.push(converted(vertical, coordinate, first[i], second[j]))),
{
    broadcast use vstd::seq_lib::group_seq_properties;
    let chord = converted(vertical, coordinate, first[i], second[j]);
    pair_native(labels, vertical, coordinate, first, second, i, j);
    native_id(labels, vertical, chord);
    assert forall|p: int| 0 <= p < output.len() implies (#[trigger] output[p]).id() < chord.id() by {
        let previous = output[p];
        native_id(labels, vertical, previous);
        assert(ids(output)[p] == previous.id());
        if previous.line() == coordinate {
            assert(ids(output).contains(previous.id()));
            assert(processed(vertical, coordinate, first, second, i, j, previous.id()));
            let (a, b) = choose|a: int, b: int| 0 <= a < first.len() && 0 <= b < second.len()
                && (a < i || b < j) && #[trigger] eligible(first[a], second[b])
                && converted(vertical, coordinate, first[a], second[b]).id() == previous.id();
            let earlier = converted(vertical, coordinate, first[a], second[b]);
            pair_native(labels, vertical, coordinate, first, second, a, b);
            native_id(labels, vertical, earlier);
            assert(previous.start() == earlier.start() && previous.end() == earlier.end());
            if a < i { assert(first[a].end <= first[i].start); }
            else { assert(second[b].end <= second[j].start); }
            assert(previous.end() <= chord.start());
        }
        assert(previous.line() < chord.line() || previous.line() == chord.line() && previous.start() < chord.start());
    }
    assert forall|a: int, b: int| 0 <= a < b < output.push(chord).len() implies
        (#[trigger] output.push(chord)[a]).id() < (#[trigger] output.push(chord)[b]).id() by {
        if b < output.len() { assert(output.push(chord)[a] == output[a]); assert(output.push(chord)[b] == output[b]); }
    }
}

pub(super) proof fn collector_step(labels: Seq<u16>, vertical: bool, coordinate: u8,
    first: Seq<Interval>, second: Seq<Interval>, before: Seq<ValuedChord>, after: Seq<ValuedChord>,
    i: int, j: int, next_i: int, next_j: int)
    requires collector_progress(labels, vertical, coordinate, first, second, before, i, j),
        i < first.len(), j < second.len(),
        next_i == i + if first[i].end <= second[j].end { 1int } else { 0int },
        next_j == j + if second[j].end <= first[i].end { 1int } else { 0int },
        after == if eligible(first[i], second[j]) { before.push(converted(vertical, coordinate, first[i], second[j])) } else { before },
    ensures collector_progress(labels, vertical, coordinate, first, second, after, next_i, next_j),
        before.len() <= after.len(), after.take(before.len() as int) == before,
        forall|p: int| before.len() <= p < after.len() ==> #[trigger] after[p].line() == coordinate,
        forall|start: int| 0 <= start <= before.len() && suffix_on_line(before, start, coordinate)
            ==> #[trigger] suffix_on_line(after, start, coordinate),
{
    broadcast use vstd::seq_lib::group_seq_properties;
    let emit = eligible(first[i], second[j]);
    let chord = converted(vertical, coordinate, first[i], second[j]);
    if emit {
        candidate_ready(labels, vertical, coordinate, first, second, before, i, j);
        candidate_last(labels, vertical, coordinate, first, second, before, i, j);
        append_capacity(labels, vertical, before, chord);
        assert(ids(after) =~= ids(before).push(chord.id()));
    }
    assert(after.take(before.len() as int) =~= before);
    assert(suffix_on_line(after, before.len() as int, coordinate));
    suffix_preserved(before, after, coordinate);
    assert forall|p: int| 0 <= p < after.len() implies #[trigger] after[p].line() <= coordinate by {
        if p < before.len() { assert(after[p] == before[p]); }
    }
    assert forall|id: int| good::line(id) == coordinate implies (#[trigger] ids(after).contains(id)
        <==> processed(vertical, coordinate, first, second, next_i, next_j, id)) by {
        pair_advance(vertical, coordinate, first, second, i, j, next_i, next_j, id);
    }
}

pub(super) proof fn collector_complete(labels: Seq<u16>, vertical: bool, coordinate: u8,
    first: Seq<Interval>, second: Seq<Interval>, output: Seq<ValuedChord>, i: int, j: int)
    requires collector_progress(labels, vertical, coordinate, first, second, output, i, j),
        i == first.len() || j == second.len(),
    ensures forall|id: int| good::good(labels, id) && good::vertical(id) == vertical && good::line(id) == coordinate
        ==> #[trigger] ids(output).contains(id),
{
    assert forall|id: int| good::good(labels, id) && good::vertical(id) == vertical && good::line(id) == coordinate
        implies #[trigger] ids(output).contains(id) by {
        good::decoded_id(id);
        find_pair(labels, vertical, coordinate as int, first, second, good::start(id), good::end(id), good::value(labels, id));
        let (a, b) = choose|a: int, b: int| 0 <= a < first.len() && 0 <= b < second.len()
            && #[trigger] eligible(first[a], second[b]) && first[a].value == good::value(labels, id)
            && vstd::math::max(first[a].start as int, second[b].start as int) == good::start(id)
            && vstd::math::min(first[a].end as int, second[b].end as int) == good::end(id);
        assert(converted(vertical, coordinate, first[a], second[b]).id() == id);
        assert(processed(vertical, coordinate, first, second, i, j, id));
    }
}

pub(super) proof fn rows_self(axis: &AxisIntervals)
    requires axis.valid(), axis.lines@.len() == 64,
    ensures axis.represents(axis.labels(), false),
{
    assert forall|line: int, p: int| 0 <= line < 64 && 0 <= p < 64 implies
        #[trigger] line_value(axis.line_spec(line), p) == axis.labels()[line * 64 + p] by {
        assert((line * 64 + p) / 64 == line && (line * 64 + p) % 64 == p) by(nonlinear_arith)
            requires 0 <= p < 64;
    }
}

pub(super) proof fn boundary_bit(mask: u64, coordinate: u8)
    requires coordinate < 64,
    ensures (mask & (1u64 << coordinate)) == 0 <==> !super::proof::bit(mask, coordinate as u64),
{
    assert((mask & (1u64 << coordinate)) == 0 <==> !super::proof::bit(mask, coordinate as u64)) by(bit_vector)
        requires coordinate < 64;
}

pub(super) proof fn same_lines_empty(labels: Seq<u16>, vertical: bool, coordinate: u8, items: Seq<Interval>)
    requires 1 <= coordinate <= 63, line_matches(labels, vertical, coordinate - 1, items),
        line_matches(labels, vertical, coordinate as int, items),
    ensures forall|id: int| good::good(labels, id) && good::vertical(id) == vertical
        ==> good::line(id) != coordinate,
{
    assert forall|id: int| good::good(labels, id) && good::vertical(id) == vertical
        implies good::line(id) != coordinate by {
        if good::line(id) == coordinate {
            find_pair(labels, vertical, coordinate as int, items, items, good::start(id), good::end(id), good::value(labels, id));
            let (i, j) = choose|i: int, j: int| 0 <= i < items.len() && 0 <= j < items.len()
                && #[trigger] eligible(items[i], items[j]) && items[i].value == good::value(labels, id)
                && vstd::math::max(items[i].start as int, items[j].start as int) == good::start(id)
                && vstd::math::min(items[i].end as int, items[j].end as int) == good::end(id);
            super::proof::unique_interval(items, good::start(id), i, j);
        }
    }
}

pub(super) open spec fn axis_collected(labels: Seq<u16>, vertical: bool, items: Seq<ValuedChord>, line: int) -> bool {
    all_good(labels, vertical, items) && ids(items).no_duplicates() && ordered(items)
        && (forall|p: int| 0 <= p < items.len() ==> #[trigger] items[p].line() < line)
        && (forall|id: int| good::good(labels, id) && good::vertical(id) == vertical && good::line(id) < line
            ==> #[trigger] ids(items).contains(id))
}

pub(super) proof fn extraction_complete(labels: Seq<u16>, vertical: bool, items: Seq<ValuedChord>)
    requires axis_collected(labels, vertical, items, 64),
    ensures ids(items).to_set() == good::chords(labels, vertical),
{
    broadcast use vstd::seq_lib::group_seq_properties;
    broadcast use vstd::set_lib::group_set_properties;
    assert(ids(items).to_set() =~= good::chords(labels, vertical)) by {
        assert forall|id: int| #[trigger] ids(items).to_set().contains(id) implies good::chords(labels, vertical).contains(id) by {
            let i = choose|i: int| 0 <= i < items.len() && ids(items)[i] == id;
            native_id(labels, vertical, items[i]);
        }
        assert forall|id: int| #[trigger] good::chords(labels, vertical).contains(id) implies ids(items).to_set().contains(id) by {
            assert(good::good(labels, id));
            assert(good::line(id) < 64);
        }
    }
}

pub(super) proof fn graph_domain(labels: Seq<u16>, vertical: bool, items: Seq<ValuedChord>)
    requires all_good(labels, vertical, items), ids(items).no_duplicates(),
    ensures crate::optimal::graph::proof::chords_valid(items, vertical),
{
    assert forall|i: int| 0 <= i < items.len() implies #[trigger] items[i].well_formed(vertical) by {
        assert(native_good(labels, vertical, items[i]));
    }
    assert forall|i: int, j: int| 0 <= i < j < items.len() && items[i].line() == items[j].line()
        implies items[i].end() < items[j].start() || items[j].end() < items[i].start() by {
        assert(ids(items)[i] != ids(items)[j]);
        assert(items[i].id() != items[j].id());
        assert(native_good(labels, vertical, items[i]));
        assert(native_good(labels, vertical, items[j]));
        good::same_line_disjoint(labels, vertical, items[i].line(), items[i].start(), items[i].end(), items[i].value,
            items[j].start(), items[j].end(), items[j].value);
    }
}

/// 长度为一的闭 chord 也必须由真实边界收集器交付。
fn unit_chord_example() {
    let first = [Interval { start: 1, end: 3, value: 7 }];
    let second = [Interval { start: 2, end: 4, value: 7 }];
    let labels = Ghost(Seq::new(4096, |p: int| if (p / 64 == 0 && 1 <= p % 64 < 3)
        || (p / 64 == 1 && 2 <= p % 64 < 4) { 7u16 } else { 0u16 }));
    proof {
        assert(line_valid(first@)); assert(line_valid(second@));
        assert forall|p: int| 0 <= p < 64 implies #[trigger] line_value(first@, p)
            == cell(labels@, false, 0, p) by {
            corners::cell_coordinates(p, 0);
            if 1 <= p < 3 { value_at(first@, p, 0); }
        }
        assert forall|p: int| 0 <= p < 64 implies #[trigger] line_value(second@, p)
            == cell(labels@, false, 1, p) by {
            corners::cell_coordinates(p, 1);
            if 2 <= p < 4 { value_at(second@, p, 0); }
        }
    }
    let mut output = crate::optimal::matching::ChordBuffer::<ValuedChord>::new(super::EMPTY_CHORD);
    let result = super::collect_chords_for_boundary::<true>(&first, &second, 1, &mut output, labels);
    assert(result.is_ok());
    proof {
        pair_native(labels@, false, 1, first@, second@, 0, 0);
        native_id(labels@, false, converted(false, 1, first@[0], second@[0]));
    }
    assert(ids(output@).contains(4227));
    #[cfg(reject_chord_missing_unit)]
    assert(!ids(output@).contains(4227));
}

}
