//! 按真实槽顺序生成记录身份快照；不先创建资源 Key 中转表。

use crate::{handles::RecordId, owner::Arena};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(not(verus_only))]
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "proof/record_ids.rs"]
pub(crate) mod proof;

#[cfg_attr(verus_only, verus_spec(ids =>
    requires records.valid(),
    ensures ids@ == proof::keys(owner, records),
))]
pub(crate) fn record_ids<R>(owner: u64, records: &Arena<R>) -> Vec<RecordId> {
    let mut ids = Vec::new();
    let count = records.len();
    let mut slot = 0;
    proof! { records.keys_through_empty(owner); }
    #[cfg_attr(verus_only, verus_spec(
        invariant records.valid(), count == records.slot_count(), slot <= count,
            ids@ == proof::typed(records.keys_through(owner, slot as nat)),
        decreases count - slot,
    ))]
    while slot < count {
        proof_decl! { let ghost before = ids@; }
        let entry = records.at(slot, owner);
        if let Some((key, _)) = entry {
            ids.push(RecordId::new(key));
        }
        proof! {
            records.keys_through_step(owner, slot, entry);
            proof::step(records, owner, slot, entry, before, ids@);
        }
        slot += 1;
    }
    proof! { records.keys_through_all(owner); }
    ids
}
