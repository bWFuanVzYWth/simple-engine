//! 原位重整退役组；存续组顺序、首次 waits 及其存储保持不变。
//!
//! 资源键与组均以读写游标压缩，不复制存续 waits；仅新增组复制当前记录身份。
//! Graph/Plan 的对应关系由调用边界维护；跨调用消费者约束及 GPU 完成事实仍由所有者维护。
//! 分配依赖标准 Vec；分配终止不属于可恢复的 Result 错误。

#[cfg(verus_only)]
use crate::lifecycle::record_keys;
use crate::{
    graph::{Graph, Plan},
    handles::RecordId,
    lifecycle::record_ids,
    owner::{Arena, Key},
};
#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "proof/retirement.rs"]
pub(crate) mod proof;

/// 一次撤销发布的资源及当时全部记录身份；后续收集不扩充 waits。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Retirement {
    pub(crate) resources: Vec<Key>,
    pub(crate) waits: Vec<RecordId>,
}

#[cfg_attr(verus_only, verus_spec(keep =>
    requires proof::valid(graph, plan),
    ensures keep == proof::eligible(graph, plan, key),
))]
fn keep_key<G: Graph>(graph: &G, plan: &Plan, key: Key) -> bool {
    if !crate::graph::current(graph, key) {
        return false;
    }
    let index = key.slot as usize;
    plan.keep[index] && !plan.published[index]
}

#[cfg_attr(verus_only, verus_spec(
    requires proof::valid(graph, plan), old(waiting)@.len() == graph@.nodes.len(),
    ensures proof::filtered(graph, plan, old(resources)@, final(resources)@, old(waiting)@, final(waiting)@),
))]
fn filter_resources<G: Graph>(
    graph: &G,
    plan: &Plan,
    resources: &mut Vec<Key>,
    waiting: &mut [bool],
) {
    proof_decl! { let ghost original = resources@; let ghost initial = waiting@; }
    let mut read = 0;
    let mut write = 0;
    proof! { proof::resource_started(graph, plan, original, initial); }
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::resource_loop(graph, plan, original, initial, resources@, waiting@, read as int, write as int),
        decreases original.len() - read,
    ))]
    while read < resources.len() {
        let key = resources[read];
        proof_decl! { let ghost before = resources@; let ghost flags = waiting@; let ghost previous_write = write; }
        if keep_key(graph, plan, key) {
            waiting[key.slot as usize] = true;
            if write != read {
                resources[write] = key;
            }
            write += 1;
        }
        proof! { proof::resource_step(graph, plan, original, initial, before, flags, read as int,
        previous_write as int, resources@, waiting@, write as int); }
        read += 1;
    }
    proof_decl! { let ghost before = resources@; }
    resources.truncate(write);
    proof! { proof::resource_done(graph, plan, original, initial, before, waiting@, write as int, resources@); }
}

#[cfg_attr(verus_only, verus_spec(
    requires proof::valid(graph, plan), old(waiting)@.len() == graph@.nodes.len(),
    ensures final(group).waits == old(group).waits,
        proof::filtered(graph, plan, old(group).resources@, final(group).resources@, old(waiting)@, final(waiting)@),
))]
fn filter_group<G: Graph>(graph: &G, plan: &Plan, group: &mut Retirement, waiting: &mut [bool]) {
    filter_resources(graph, plan, &mut group.resources, waiting);
}

#[cfg_attr(verus_only, verus_spec(
    requires read < old(groups)@.len(), write < old(groups)@.len(),
    ensures proof::moved(old(groups)@, final(groups)@, read as int, write as int),
))]
fn move_group(groups: &mut [Retirement], read: usize, write: usize) {
    let mut moving = Retirement {
        resources: Vec::new(),
        waits: Vec::new(),
    };
    std::mem::swap(&mut groups[read], &mut moving);
    groups[write] = moving;
}

#[cfg_attr(verus_only, verus_spec(waiting =>
    requires proof::valid(graph, plan),
    ensures proof::old_done(graph, plan, old(groups)@, final(groups)@, waiting@),
))]
fn compact_groups<G: Graph>(graph: &G, plan: &Plan, groups: &mut Vec<Retirement>) -> Vec<bool> {
    proof_decl! { let ghost original = groups@; }
    let mut waiting = vec![false; graph.len()];
    let mut read = 0;
    let mut write = 0;
    let len = groups.len();
    proof! { proof::groups_started(graph, plan, original, waiting@); }
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::group_loop(graph, plan, original, groups@, waiting@, read as int, write as int), len == original.len(),
        decreases original.len() - read,
    ))]
    while read < len {
        proof_decl! { let ghost before = groups@; let ghost flags = waiting@; let ghost previous_write = write; }
        filter_group(graph, plan, &mut groups[read], &mut waiting);
        proof_decl! { let ghost filtered = groups@; }
        if !groups[read].resources.is_empty() {
            if write != read {
                move_group(groups, read, write);
            }
            write += 1;
        }
        proof! { proof::group_step(graph, plan, original, before, flags, filtered, groups@,
        waiting@, read as int, previous_write as int, write as int); }
        read += 1;
    }
    proof_decl! { let ghost before = groups@; }
    groups.truncate(write);
    proof! { proof::groups_done(graph, plan, original, before, groups@, waiting@, write as int); }
    waiting
}

#[cfg_attr(verus_only, verus_spec(fresh =>
    requires proof::valid(graph, plan), waiting@.len() == graph@.nodes.len(),
    ensures proof::unassigned(graph, plan, waiting@, fresh@),
))]
fn fresh_resources<G: Graph>(graph: &G, plan: &Plan, waiting: &[bool]) -> Vec<Key> {
    let mut fresh = Vec::new();
    let mut slot = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::fresh_scan(graph, plan, waiting@, fresh@, slot as int),
        decreases graph@.nodes.len() - slot,
    ))]
    while slot < graph.len() {
        proof_decl! { let ghost previous = fresh@; }
        if plan.keep[slot] && !plan.published[slot] && !waiting[slot] {
            let (key, _) = graph.node(slot).unwrap();
            fresh.push(key);
        }
        proof! { proof::fresh_step(graph, plan, waiting@, slot as int, previous, fresh@); }
        slot += 1;
    }
    proof! { proof::scan_done(graph, plan, waiting@, fresh@); }
    fresh
}

/// 原位保留未重新发布的旧组，只为尚无旧组的保留资源追加一次当前记录快照。
/// 存续组只移动，不复制其 waits；完整资源身份包含 owner、slot 和 generation。
#[cfg_attr(verus_only, verus_spec(
    requires proof::valid(graph, plan), records.valid(),
    ensures proof::reconciled(graph, plan, old(groups)@, record_keys(owner, records), final(groups)@),
        proof::covers(graph, plan, final(groups)@),
        proof::unique(old(groups)@) ==> proof::unique(final(groups)@),
        proof::waits_nested(old(groups)@, proof::live(owner, records))
            ==> proof::waits_nested(final(groups)@, proof::live(owner, records)),
))]
pub(crate) fn reconcile<G: Graph, R>(
    graph: &G,
    plan: &Plan,
    groups: &mut Vec<Retirement>,
    owner: u64,
    records: &Arena<R>,
) {
    proof_decl! { let ghost original = groups@; }
    let waiting = compact_groups(graph, plan, groups);
    let fresh = fresh_resources(graph, plan, &waiting);
    proof! { proof::fresh_complete(graph, plan, original, groups@, waiting@, fresh@); }
    proof_decl! { let ghost previous = groups@; let ghost new_resources = fresh@; }
    if !fresh.is_empty() {
        let group = Retirement {
            resources: fresh,
            waits: record_ids(owner, records),
        };
        proof! { proof::push_view(groups@, group); }
        groups.push(group);
    }
    proof! {
        proof::assembled_complete(graph, plan, original, owner, records, previous, new_resources, groups@);
    }
}

#[cfg_attr(verus_only, verus_spec(found =>
    ensures found == !proof::ready(*group, proof::live(owner, records)),
))]
fn has_live_wait<R>(group: &Retirement, owner: u64, records: &Arena<R>) -> bool {
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= group.waits@.len(),
            forall|i: int| #![auto] 0 <= i < index ==> !proof::live(owner, records).contains(group.waits@[i]),
        decreases group.waits@.len() - index,
    ))]
    while index < group.waits.len() {
        let id = group.waits[index];
        if id.key().owner == owner && records.find(id.key()).is_some() {
            proof! { proof::found_wait(group, index, proof::live(owner, records)); }
            return true;
        }
        index += 1;
    }
    false
}

/// 从游标开始移出首个已无存活等待者的组；等待中的组保持顺序和存储。
#[cfg_attr(verus_only, verus_spec(result =>
    ensures proof::taken(old(groups)@, final(groups)@, *old(cursor), *final(cursor),
        proof::live(owner, records), result),
        proof::unique(old(groups)@) ==> proof::unique(final(groups)@),
        proof::waits_nested(old(groups)@, proof::live(owner, records))
            ==> proof::waits_nested(final(groups)@, proof::live(owner, records)),
))]
pub(crate) fn take_ready<R>(
    groups: &mut Vec<Retirement>,
    cursor: &mut usize,
    owner: u64,
    records: &Arena<R>,
) -> Option<Retirement> {
    proof_decl! { let ghost original = groups@; }
    #[cfg_attr(verus_only, verus_spec(
        invariant groups@ == old(groups)@, original == old(groups)@, *old(cursor) <= *cursor,
            forall|i: int| #![auto] *old(cursor) <= i < *cursor && i < groups@.len()
                ==> !proof::ready(groups@[i], proof::live(owner, records)),
        decreases groups@.len() - *cursor,
    ))]
    while *cursor < groups.len() {
        if has_live_wait(&groups[*cursor], owner, records) {
            *cursor += 1;
        } else {
            let group = groups.remove(*cursor);
            proof! { proof::removed_properties(original, groups@, *cursor as int, proof::live(owner, records)); }
            return Some(group);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Adjacency, EdgeModel, Model};
    use vstd::view::View;

    struct Resources([Key; 3]);
    struct Node;
    impl View for Resources {
        type V = Model;
    }
    impl View for Node {
        type V = EdgeModel;
    }
    impl Graph for Resources {
        type Node = Node;
        fn owner(&self) -> u64 {
            7
        }
        fn len(&self) -> usize {
            self.0.len()
        }
        fn node(&self, slot: usize) -> Option<(Key, Node)> {
            self.0.get(slot).copied().map(|key| (key, Node))
        }
    }
    impl Adjacency for Node {
        fn len(&self) -> usize {
            0
        }
        fn edge(&self, _index: usize) -> Option<Key> {
            None
        }
    }

    #[test]
    fn compaction_preserves_wait_allocation() {
        let keys = [0, 1, 2].map(|slot| Key {
            owner: 7,
            slot,
            generation: 0,
        });
        let source = Resources(keys);
        let plan = Plan {
            published: vec![false, true, true],
            keep: vec![true; 3],
        };
        let mut records = Arena::default();
        let first = RecordId::new(records.try_insert(7, ()).unwrap_or_else(|_| panic!()));
        let mut waits = Vec::with_capacity(64);
        waits.push(first);
        let pointer = waits.as_ptr();
        let capacity = waits.capacity();
        let mut groups = vec![
            Retirement {
                resources: vec![keys[1]],
                waits: vec![first],
            },
            Retirement {
                resources: vec![keys[1], keys[0]],
                waits,
            },
            Retirement {
                resources: vec![keys[2]],
                waits: vec![first],
            },
        ];
        reconcile(&source, &plan, &mut groups, 7, &records);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].waits.as_ptr(), pointer);
        assert_eq!(groups[0].waits.capacity(), capacity);
        assert_eq!(groups[0].waits, [first]);
    }

    #[test]
    fn stale_wait_does_not_block_reused_record_slot() {
        let mut records = Arena::new();
        let stale = records.try_insert(7, ()).unwrap_or_else(|_| panic!());
        records.remove(stale);
        let _current = records.try_insert(7, ()).unwrap_or_else(|_| panic!());
        let mut groups = vec![Retirement {
            resources: Vec::new(),
            waits: vec![RecordId::new(stale)],
        }];
        let mut cursor = 0;
        assert!(take_ready(&mut groups, &mut cursor, 7, &records).is_some());
    }
}
