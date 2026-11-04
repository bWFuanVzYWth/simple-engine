//! 回收阶段组合引理；输入前提须由真实 Life 的状态转换维护。
//!
//! 不把这些条件式结论计为 native/FFI 已验证；没有新增执行期表、快照或历史。

use crate::{
    graph::{Model, model},
    handles::RecordId,
    owner::Key,
    retirement::{Retirement, proof as retirement},
};
use vstd::prelude::*;

verus! {
/// 非退休集合封闭；与含 AS producer 状态的 native active 不同。
pub(crate) open spec fn nonretiring_closed(graph: Model, available: ISet<Key>) -> bool {
    forall|parent: Key, child: Key| #![auto]
        available.contains(parent) && model::dependency(graph, parent, child) ==> available.contains(child)
}

pub(crate) open spec fn ranks_ordered(graph: Model, rank: spec_fn(Key) -> nat) -> bool {
    forall|parent: Key, child: Key| #[trigger] model::dependency(graph, parent, child)
        ==> rank(parent) < rank(child)
}

/// 父资源不得处于比依赖更新的退役组；同组由 rank 决定销毁顺序。
pub(crate) open spec fn groups_ordered(graph: Model, groups: Seq<Retirement>) -> bool {
    forall|i: int, j: int, parent: Key, child: Key| #![auto]
        0 <= i < groups.len() && 0 <= j < groups.len()
        && groups[i].resources@.contains(parent) && groups[j].resources@.contains(child)
        && model::dependency(graph, parent, child) ==> i <= j
}

/// 已完成较旧组与本组较低 rank 后，当前资源没有尚存的父资源。
pub(crate) proof fn no_remaining_parent(graph: Model, groups: Seq<Retirement>,
    available: ISet<Key>, present: ISet<Key>, rank: spec_fn(Key) -> nat, index: int, key: Key)
    requires
        nonretiring_closed(graph, available), ranks_ordered(graph, rank), groups_ordered(graph, groups),
        0 <= index < groups.len(), groups[index].resources@.contains(key), !available.contains(key),
        forall|parent: Key| #![auto] present.contains(parent) && !available.contains(parent)
            ==> exists|i: int| 0 <= i < groups.len() && #[trigger] groups[i].resources@.contains(parent),
        forall|i: int, parent: Key| #![auto] 0 <= i < groups.len() && groups[i].resources@.contains(parent)
            && (i < index || (i == index && rank(parent) < rank(key))) ==> !present.contains(parent),
    ensures forall|parent: Key| #![auto] present.contains(parent) ==> !model::dependency(graph, parent, key),
{
    assert forall|parent: Key| #![auto] present.contains(parent)
        implies !model::dependency(graph, parent, key) by {
        if model::dependency(graph, parent, key) {
            if available.contains(parent) { assert(available.contains(key)); }
            else {
                let i = choose|i: int| 0 <= i < groups.len() && #[trigger] groups[i].resources@.contains(parent);
                assert(i <= index);
                assert(rank(parent) < rank(key));
            }
        }
    }
}

/// 首次 waits 覆盖真实存活消费者时，ready 才能推出该组已无存活消费者。
pub(crate) proof fn no_live_consumers(graph: Model, group: Retirement, live: ISet<RecordId>,
    roots: spec_fn(RecordId) -> Seq<Key>)
    requires retirement::ready(group, live),
        forall|id: RecordId, key: Key| #![auto]
            live.contains(id) && group.resources@.contains(key)
            && model::reachable(graph, roots(id), Seq::empty(), key) ==> group.waits@.contains(id),
    ensures forall|id: RecordId, key: Key| #![auto]
        live.contains(id) && group.resources@.contains(key)
        ==> !model::reachable(graph, roots(id), Seq::empty(), key),
{}
}
