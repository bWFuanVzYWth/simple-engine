//! 叶内点写的逐项覆盖规范；直接查看生产输入与输出。
use super::{DenseRegion, SetCell};
use vstd::prelude::*;

verus! {
pub open spec fn code(cell: SetCell) -> u64 { crate::morton::proof::value(cell.path) }
pub(crate) open spec fn input_domain(cells: Seq<SetCell>) -> bool {
    forall|i: int| 0 <= i < cells.len() ==> #[trigger] code(cells[i]) < 0x1000_0000_0000_0000u64
}
pub open spec fn slot(cell: SetCell) -> int { (code(cell) % 64) as int }
pub open spec fn sorted(cells: Seq<SetCell>) -> bool {
    sorted_prefix(cells, cells.len())
}
pub open spec fn sorted_prefix(cells: Seq<SetCell>, count: nat) -> bool {
    forall|i: int| 0 <= i && i + 1 < count ==> #[trigger] code(cells[i]) <= code(cells[i + 1])
}
pub open spec fn same_group(cells: Seq<SetCell>, start: nat, end: nat) -> bool {
    start < end <= cells.len() && forall|i: int| start <= i < end ==> #[trigger] code(cells[i]) == code(cells[start as int])
}
pub open spec fn apply(initial: Seq<u32>, cells: Seq<SetCell>, count: nat) -> Seq<u32>
    recommends initial.len() == 64, count <= cells.len(),
    decreases count,
{
    if count == 0 { initial } else {
        let cell = cells[(count - 1) as int];
        apply(initial, cells, (count - 1) as nat).update(slot(cell), cell.data)
    }
}

/// 从尾部寻找最后一次同槽写入；没有记录时保留输入值。
pub open spec fn last_value(cells: Seq<SetCell>, count: nat, target: int, fallback: u32) -> u32
    recommends count <= cells.len(), 0 <= target < 64,
    decreases count,
{
    if count == 0 { fallback } else {
        let cell = cells[(count - 1) as int];
        if slot(cell) == target { cell.data }
        else { last_value(cells, (count - 1) as nat, target, fallback) }
    }
}

pub proof fn point_value(initial: Seq<u32>, cells: Seq<SetCell>, count: nat, target: int)
    requires initial.len() == 64, count <= cells.len(), 0 <= target < 64,
    ensures apply(initial, cells, count)[target] == last_value(cells, count, target, initial[target]),
    decreases count,
{
    apply_length(initial, cells, count);
    if count > 0 { point_value(initial, cells, (count - 1) as nat, target); }
}

proof fn apply_length(initial: Seq<u32>, cells: Seq<SetCell>, count: nat)
    requires initial.len() == 64, count <= cells.len(),
    ensures apply(initial, cells, count).len() == 64,
    decreases count,
{
    if count > 0 { apply_length(initial, cells, (count - 1) as nat); }
}

pub proof fn slot_bits(code: u64)
    ensures code & 63 == code % 64, code & 63 < 64,
{
    assert(code & 63 == code % 64 && code & 63 < 64) by(bit_vector);
}

pub proof fn ordering_failure(cells: Seq<SetCell>, index: nat)
    requires index + 1 < cells.len(), code(cells[index as int]) > code(cells[(index + 1) as int]),
    ensures !sorted(cells),
{
    assert(!(code(cells[index as int]) <= code(cells[(index + 1) as int])));
}

pub proof fn group_applied(initial: Seq<u32>, cells: Seq<SetCell>, start: nat, end: nat)
    requires initial.len() == 64, same_group(cells, start, end),
    ensures apply(initial, cells, end) == apply(initial, cells, start)
        .update(slot(cells[(end - 1) as int]), cells[(end - 1) as int].data),
    decreases end - start,
{
    broadcast use vstd::seq_lib::group_seq_properties;
    if end > start + 1 {
        group_applied(initial, cells, start, (end - 1) as nat);
        let previous = apply(initial, cells, start);
        let i = slot(cells[(end - 1) as int]);
        assert(previous.update(i, cells[(end - 2) as int].data).update(i, cells[(end - 1) as int].data)
            =~= previous.update(i, cells[(end - 1) as int].data));
    }
}

pub(super) proof fn pending_step(cells: Seq<SetCell>, start: nat, processed: nat)
    requires start <= processed < cells.len(),
        start == processed || (same_group(cells, start, processed)
            && code(cells[(processed - 1) as int]) == code(cells[processed as int])),
    ensures same_group(cells, start, processed + 1),
{
    assert forall|i: int| start <= i < processed + 1 implies
        #[trigger] code(cells[i]) == code(cells[start as int]) by {
        if start < processed && i == processed {
            assert(code(cells[(processed - 1) as int]) == code(cells[start as int]));
        }
    }
}

pub proof fn kept_group(cells: Seq<SetCell>, start: nat, end: nat, value: u32)
    requires same_group(cells, start, end), cells[(end - 1) as int].data == value,
        apply(Seq::new(64, |i: int| value), cells, start) == Seq::new(64, |i: int| value),
    ensures apply(Seq::new(64, |i: int| value), cells, end) == Seq::new(64, |i: int| value),
{
    group_applied(Seq::new(64, |i: int| value), cells, start, end);
    assert(Seq::new(64, |i: int| value).update(slot(cells[(end - 1) as int]), value)
        =~= Seq::new(64, |i: int| value));
}

pub open spec fn region_slot(region: DenseRegion) -> int {
    (crate::morton::proof::value(region.path) % 64) as int
}
pub open spec fn leaf_regions(regions: Seq<DenseRegion>) -> bool {
    forall|i:int| 0 <= i < regions.len() ==> #[trigger] leaf_region(regions[i])
}
pub open spec fn leaf_region(region: DenseRegion) -> bool {
    region.lod <= 1 && region.values@.len() == (if region.lod == 0 { 1nat } else { 8nat })
        && region_slot(region) + region.values@.len() <= 64
}
pub proof fn region_valid(regions: Seq<DenseRegion>, index: nat)
    requires leaf_regions(regions), index < regions.len(),
    ensures leaf_region(regions[index as int]),
{}
pub open spec fn region_apply(before: Seq<u32>, region: DenseRegion) -> Seq<u32> {
    Seq::new(64, |i:int| if region_slot(region) <= i < region_slot(region) + region.values@.len() {
        region.values@[i-region_slot(region)]
    } else { before[i] })
}
pub open spec fn dense_apply(initial: Seq<u32>, regions: Seq<DenseRegion>, count: nat) -> Seq<u32>
    recommends initial.len() == 64, count <= regions.len(),
    decreases count,
{
    if count == 0 { initial } else {
        region_apply(dense_apply(initial, regions, (count - 1) as nat), regions[(count - 1) as int])
    }
}
pub open spec fn all_values(regions: Seq<DenseRegion>, count: nat, value: u32) -> bool {
    count <= regions.len() && forall|i:int,j:int| 0 <= i < count && 0 <= j < regions[i].values@.len()
        ==> #[trigger] regions[i].values@[j] == value
}
pub proof fn values_step(regions: Seq<DenseRegion>, count: nat, value: u32)
    requires count < regions.len(), all_values(regions, count, value),
        forall|j:int| 0 <= j < regions[count as int].values@.len() ==> #[trigger] regions[count as int].values@[j] == value,
    ensures all_values(regions, count + 1, value),
{}
pub proof fn dense_kept(regions: Seq<DenseRegion>, count: nat, value: u32)
    requires count <= regions.len(), all_values(regions, count, value),
    ensures dense_apply(Seq::new(64, |i:int| value), regions, count) == Seq::new(64, |i:int| value),
    decreases count,
{
    if count > 0 {
        dense_kept(regions, (count - 1) as nat, value);
        assert(region_apply(Seq::new(64, |i:int| value), regions[(count - 1) as int])
            =~= Seq::new(64, |i:int| value));
    }
}
pub open spec fn dense_last_value(regions: Seq<DenseRegion>, count: nat, target: int, fallback: u32) -> u32
    recommends count <= regions.len(), 0 <= target < 64,
    decreases count,
{
    if count == 0 { fallback } else {
        let region = regions[(count - 1) as int];
        if region_slot(region) <= target < region_slot(region) + region.values@.len() {
            region.values@[target-region_slot(region)]
        } else { dense_last_value(regions, (count - 1) as nat, target, fallback) }
    }
}
pub proof fn dense_point_value(initial: Seq<u32>, regions: Seq<DenseRegion>, count: nat, target: int)
    requires initial.len() == 64, count <= regions.len(), 0 <= target < 64,
    ensures dense_apply(initial, regions, count)[target] == dense_last_value(regions, count, target, initial[target]),
    decreases count,
{
    if count > 0 { dense_point_value(initial, regions, (count - 1) as nat, target); }
}
}
