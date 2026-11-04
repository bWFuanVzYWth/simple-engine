//! 原生叶内的批量修改；只借用调用方的 64 槽输出。

use super::input::{DenseRegion, SetCell};
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "../proof/write_patch.rs"]
#[cfg(verus_only)]
pub(crate) mod proof;

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == proof::sorted(cells@),
        result ==> proof::input_domain(cells@), no_unwind
))]
pub(super) fn sorted(cells: &[SetCell]) -> bool {
    if cells.is_empty() {
        return true;
    }
    let mut previous = cells[0].path.value();
    let mut index = 1;
    #[cfg_attr(verus_only, verus_spec(
        invariant 1 <= index <= cells@.len(), proof::sorted_prefix(cells@, index as nat),
            previous == proof::code(cells@[(index - 1) as int]),
            forall|i: int| 0 <= i < index ==> #[trigger] proof::code(cells@[i]) < 0x1000_0000_0000_0000u64,
        decreases cells@.len() - index,
    ))]
    while index < cells.len() {
        let current = cells[index].path.value();
        if previous > current {
            #[cfg(verus_only)]
            proof! { proof::ordering_failure(cells@, (index - 1) as nat); }
            return false;
        }
        previous = current;
        index += 1;
    }
    true
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires start < cells@.len(),
    ensures start < result <= cells@.len(), proof::same_group(cells@, start as nat, result as nat),
    no_unwind
))]
fn group_end(cells: &[SetCell], start: usize) -> usize {
    let code = cells[start].path.value();
    let mut end = start + 1;
    #[cfg_attr(verus_only, verus_spec(
        invariant start < end <= cells@.len(),
            code == proof::code(cells@[start as int]), proof::same_group(cells@, start as nat, end as nat),
        decreases cells@.len() - end,
    ))]
    while end < cells.len() && cells[end].path.value() == code {
        end += 1;
    }
    end
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures final(values)@ == old(values)@.update(proof::slot(*cell), cell.data),
        result == (old(values)@[proof::slot(*cell)] != cell.data),
        !result ==> final(values)@ == old(values)@,
    no_unwind
))]
#[inline]
const fn apply_last(values: &mut [u32; 64], cell: &SetCell) -> bool {
    #[cfg(verus_only)]
    proof_decl! { let ghost initial = values@; }
    let code = cell.path.value();
    #[cfg(verus_only)]
    proof! { proof::slot_bits(code); }
    let slot = (code & 63) as usize;
    let changed = values[slot] != cell.data;
    values[slot] = cell.data;
    #[cfg(verus_only)]
    proof! { if !changed { assert(values@ =~= initial); } }
    changed
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures final(values)@ == proof::apply(old(values)@, cells@, cells@.len()),
        !result ==> final(values)@ == old(values)@,
    no_unwind
))]
pub(crate) fn apply_cells(values: &mut [u32; 64], cells: &[SetCell]) -> bool {
    #[cfg(verus_only)]
    proof_decl! { let ghost initial = values@; }
    let Some((mut last, mut pending)) = cells.split_first() else {
        return false;
    };
    let mut changed = false;
    #[cfg(verus_only)]
    proof_decl! { let ghost mut start = 0nat; let ghost mut processed = 1nat; }
    #[cfg_attr(verus_only, verus_spec(
        invariant initial.len() == 64, initial == old(values)@, cells@.len() > 0,
            start < processed <= cells@.len(),
            pending@ == cells@.subrange(processed as int, cells@.len() as int),
            *last == cells@[(processed - 1) as int],
            proof::same_group(cells@, start, processed),
            values@ == proof::apply(initial, cells@, start),
            !changed ==> values@ == initial,
        decreases pending@.len(),
    ))]
    // 耗尽前须连接最后一组的擦除证明，不能直接改为 while let。
    #[allow(clippy::while_let_loop)]
    loop {
        let Some((cell, rest)) = pending.split_first() else {
            let differs = apply_last(values, last);
            #[cfg(verus_only)]
            proof! {
                assert(processed == cells@.len());
                proof::group_applied(initial, cells@, start, processed);
                assert(values@ == proof::apply(initial, cells@, cells@.len()));
            }
            return changed | differs;
        };
        if last.path.value() != cell.path.value() {
            changed |= apply_last(values, last);
            #[cfg(verus_only)]
            proof! { proof::group_applied(initial, cells@, start, processed); start = processed; }
        }
        #[cfg(verus_only)]
        proof! { proof::pending_step(cells@, start, processed); }
        last = cell;
        pending = rest;
        #[cfg(verus_only)]
        proof! { processed = processed + 1; }
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result ==> proof::apply(Seq::new(64, |i: int| value), cells@, cells@.len()) == Seq::new(64, |i: int| value),
    no_unwind
))]
pub(super) fn keeps_cells(cells: &[SetCell], value: u32) -> bool {
    let mut position = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant position <= cells@.len(),
            proof::apply(Seq::new(64, |i: int| value), cells@, position as nat) == Seq::new(64, |i: int| value),
        decreases cells@.len() - position,
    ))]
    while position < cells.len() {
        let end = group_end(cells, position);
        if cells[end - 1].data != value {
            return false;
        }
        #[cfg(verus_only)]
        proof! { proof::kept_group(cells@, position as nat, end as nat, value); }
        position = end;
    }
    true
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::leaf_regions(regions@),
    ensures final(values)@ == proof::dense_apply(old(values)@, regions@, regions@.len()),
        !result ==> final(values)@ == old(values)@,
))]
pub(super) fn apply_dense(values: &mut [u32; 64], regions: &[DenseRegion<'_>]) -> bool {
    #[cfg(verus_only)]
    proof_decl! { let ghost initial = values@; }
    let mut changed = false;
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant initial.len() == 64, proof::leaf_regions(regions@), index <= regions@.len(),
            values@ == proof::dense_apply(initial, regions@, index as nat),
            !changed ==> values@ == initial,
        decreases regions@.len() - index,
    ))]
    while index < regions.len() {
        #[cfg(verus_only)]
        proof! { proof::region_valid(regions@, index as nat); }
        let region = &regions[index];
        let code = region.path.value();
        #[cfg(verus_only)]
        proof! { proof::slot_bits(code); }
        let start = (code & 63) as usize;
        if region.lod == 0 {
            let data = region.values[0];
            changed |= values[start] != data;
            values[start] = data;
        } else {
            let target = &mut values[start..start + 8];
            let differs = &*target != region.values;
            changed |= differs;
            #[cfg(verus_only)]
            proof! { if !differs { assert(target@ =~= region.values@); } }
            target.copy_from_slice(region.values);
        }
        index += 1;
    }
    changed
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result ==> proof::all_values(regions@, regions@.len(), value), no_unwind
))]
pub(super) fn keeps_dense(regions: &[DenseRegion<'_>], value: u32) -> bool {
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= regions@.len(), proof::all_values(regions@, index as nat, value),
        decreases regions@.len() - index,
    ))]
    while index < regions.len() {
        let data = regions[index].values;
        let mut position = 0;
        #[cfg_attr(verus_only, verus_spec(
            invariant position <= data@.len(), data@ == regions@[index as int].values@,
                forall|i:int| 0 <= i < position ==> #[trigger] data@[i] == value,
            decreases data@.len() - position,
        ))]
        while position < data.len() {
            if data[position] != value {
                return false;
            }
            position += 1;
        }
        #[cfg(verus_only)]
        proof! { proof::values_step(regions@, index as nat, value); }
        index += 1;
    }
    true
}
