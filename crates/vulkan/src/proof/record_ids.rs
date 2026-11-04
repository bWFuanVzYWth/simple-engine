use super::*;
use crate::owner::Key;
use vstd::prelude::*;

verus! {
pub(crate) open spec fn typed(keys: Seq<Key>) -> Seq<RecordId> {
    keys.map_values(|key: Key| RecordId(key))
}

pub(crate) open spec fn keys<R>(owner: u64, records: &Arena<R>) -> Seq<RecordId> {
    typed(records.keys_view(owner))
}

pub(crate) proof fn contains<R>(owner: u64, records: &Arena<R>, id: RecordId)
    requires records.valid(),
    ensures keys(owner, records).contains(id) <==> id.0.owner == owner && records.lookup(id.0) is Some,
{
    records.keys_contains(owner, id.0);
    let raw = records.keys_view(owner);
    if keys(owner, records).contains(id) {
        let i = choose|i: int| 0 <= i < raw.len() && typed(raw)[i] == id;
        assert(raw[i] == id.0);
    }
    if raw.contains(id.0) {
        let i = choose|i: int| 0 <= i < raw.len() && raw[i] == id.0;
        assert(typed(raw)[i] == id);
    }
}

pub(super) proof fn step<R>(records: &Arena<R>, owner: u64, slot: usize,
    entry: Option<(Key, &R)>, before: Seq<RecordId>, after: Seq<RecordId>)
    requires
        before == typed(records.keys_through(owner, slot as nat)),
        after == match entry { Some((key, _)) => before.push(RecordId(key)), None => before },
        records.keys_through(owner, slot as nat + 1) == match entry {
            Some((key, _)) => records.keys_through(owner, slot as nat).push(key),
            None => records.keys_through(owner, slot as nat),
        },
    ensures after == typed(records.keys_through(owner, slot as nat + 1)),
{
    match entry {
        Some((key, _)) => {
            let raw = records.keys_through(owner, slot as nat);
            assert(typed(raw.push(key)) =~= typed(raw).push(RecordId(key)));
        }
        None => {},
    }
}
}
