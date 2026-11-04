//! 直接借用真实记录表的整批转换；失败回滚本次前缀，不分配状态副本。

use super::{Error, Record, RecordId, RecordState};
use crate::owner::Arena;
#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "proof/record_batch.rs"]
pub(crate) mod proof;
#[cfg(verus_only)]
pub(crate) use proof::{framed, ready};

/// 完整身份、Pending 和批内唯一性全部满足才准备成功；失败恢复原值，保留容量不作承诺。
#[cfg_attr(verus_only, verus_spec(result => requires old(records).valid(),
    ensures final(records).valid(), proof::prepared(old(records), final(records), owner, ids@, serial, result),
))]
pub(crate) fn prepare<B>(
    records: &mut Arena<Record<B>>,
    owner: u64,
    ids: &[RecordId],
    serial: u64,
) -> Result<(), Error> {
    proof_decl! { let ghost original = *records; }
    proof! { proof::empty_write(records, owner, ids@, RecordState::Submitting(serial)); }
    let mut count = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant original == *old(records), proof::preparing(&original, records, owner, ids@, count as int, serial),
        decreases ids@.len() - count,
    ))]
    while count < ids.len() {
        proof_decl! { let ghost previous = *records; }
        if let Err(error) = begin(records, owner, ids[count], serial) {
            proof! { proof::rejected_input(&original, &previous, owner, ids@, count as int, serial); }
            proof_decl! { let ghost failed = *records; }
            rollback(records, owner, ids, count, serial);
            proof! {
                proof::write_identical(&previous, &failed, records, owner, ids@, count as int, RecordState::Pending);
                proof::rolled_back(&original, &previous, records, owner, ids@, count as int, serial);
            }
            return Err(error);
        }
        proof! { proof::prepared_step(&original, &previous, records, owner, ids@, count as int, serial); }
        count += 1;
    }
    proof! { proof::write_framed(&original, records, owner, ids@, ids@.len() as int, RecordState::Submitting(serial)); }
    Ok(())
}

#[cfg_attr(verus_only, verus_spec(result => requires old(records).valid(),
    ensures final(records).valid(), proof::begun(old(records), final(records), owner, id, serial, result),
))]
fn begin<B>(
    records: &mut Arena<Record<B>>,
    owner: u64,
    id: RecordId,
    serial: u64,
) -> Result<(), Error> {
    proof_decl! { let ghost original = *records; }
    proof! { records.shape_reflexive(); }
    let key = id.key();
    if key.owner != owner {
        return Err(Error::ForeignOwner);
    }
    let Some(record) = records.find_mut(key) else {
        proof! { records.shape_preserves_presence(&original, key); }
        proof! { proof::unchanged_one(&original, records, key); }
        return Err(Error::InvalidKey);
    };
    let next = match record.state.begin(serial) {
        Some(next) => next,
        None => {
            proof! { proof::unchanged_one(&original, records, key); }
            return Err(Error::InvalidState);
        }
    };
    record.state = next;
    proof! { proof::begun_done(&original, records, owner, id, serial); }
    Ok(())
}

#[cfg_attr(verus_only, verus_spec(
    requires old(records).valid(), proof::prefix(old(records), _owner, ids@, count as int, RecordState::Submitting(_serial)),
    ensures final(records).valid(), proof::written(old(records), final(records), _owner, ids@, count as int, RecordState::Pending),
))]
#[inline]
fn rollback<B>(
    records: &mut Arena<Record<B>>,
    _owner: u64,
    ids: &[RecordId],
    count: usize,
    _serial: u64,
) {
    proof_decl! { let ghost original = *records; }
    proof! { proof::empty_write(records, _owner, ids@, RecordState::Pending); }
    let mut i = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant original == *old(records), proof::changing(&original, records, _owner, ids@, count as int, i as int, RecordState::Submitting(_serial), RecordState::Pending),
        decreases count - i,
    ))]
    while i < count {
        proof_decl! { let ghost previous = *records; }
        records
            .find_mut(ids[i].key())
            .expect("本次已修改的前缀")
            .state = RecordState::Pending;
        proof! { proof::changed_step(&original, &previous, records, _owner, ids@, count as int, i as int, RecordState::Submitting(_serial), RecordState::Pending); }
        i += 1;
    }
}

/// 仅供仍独占已准备批次的 Attempt 接受；操作、依赖和缓冲保持原值。
#[cfg_attr(verus_only, verus_spec(
    requires old(records).valid(), ready(old(records), _owner, ids@, serial),
    ensures final(records).valid(), proof::written(old(records), final(records), _owner, ids@, ids@.len() as int, RecordState::Submitted(serial)),
))]
pub(crate) fn accept<B>(
    records: &mut Arena<Record<B>>,
    _owner: u64,
    ids: &[RecordId],
    serial: u64,
) {
    proof_decl! { let ghost original = *records; }
    proof! { proof::empty_write(records, _owner, ids@, RecordState::Submitted(serial)); }
    let mut i = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant original == *old(records), proof::changing(&original, records, _owner, ids@, ids@.len() as int, i as int, RecordState::Submitting(serial), RecordState::Submitted(serial)),
        decreases ids@.len() - i,
    ))]
    while i < ids.len() {
        proof_decl! { let ghost previous = *records; }
        let record = records
            .find_mut(ids[i].key())
            .expect("预检成功且独占 owner");
        record.state = record.state.accept(serial).expect("同一提交尝试");
        proof! { proof::changed_step(&original, &previous, records, _owner, ids@, ids@.len() as int, i as int, RecordState::Submitting(serial), RecordState::Submitted(serial)); }
        i += 1;
    }
}

/// 隔离本 owner 的当前身份；外来或失效身份不影响现存记录。
#[cfg_attr(verus_only, verus_spec(
    requires old(records).valid(),
    ensures final(records).valid(), proof::written(old(records), final(records), owner, ids@, ids@.len() as int, RecordState::Uncertain),
))]
pub(crate) fn quarantine<B>(records: &mut Arena<Record<B>>, owner: u64, ids: &[RecordId]) {
    proof_decl! { let ghost original = *records; }
    proof! { proof::empty_write(records, owner, ids@, RecordState::Uncertain); }
    let mut i = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant original == *old(records), records.valid(), 0 <= i <= ids@.len(), proof::written(&original, records, owner, ids@, i as int, RecordState::Uncertain),
        decreases ids@.len() - i,
    ))]
    while i < ids.len() {
        proof_decl! { let ghost previous = *records; }
        proof! { records.shape_reflexive(); }
        quarantine_one(records, owner, ids[i]);
        proof! { proof::quarantined_step(&original, &previous, records, owner, ids@, i as int); }
        i += 1;
    }
}

#[inline]
#[cfg_attr(verus_only, verus_spec(
    requires old(records).valid(),
    ensures final(records).valid(), proof::written(old(records), final(records), owner, seq![id], 1, RecordState::Uncertain),
))]
fn quarantine_one<B>(records: &mut Arena<Record<B>>, owner: u64, id: RecordId) {
    proof_decl! { let ghost original = *records; }
    proof! { proof::empty_write(records, owner, seq![id], RecordState::Uncertain); }
    let key = id.key();
    if key.owner != owner {
        proof! { proof::single_quarantined(&original, records, owner, id); }
        return;
    }
    if let Some(record) = records.find_mut(key) {
        record.state = record.state.quarantine();
    }
    proof! { proof::single_quarantined(&original, records, owner, id); }
}
