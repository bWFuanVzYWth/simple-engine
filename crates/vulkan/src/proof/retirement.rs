use super::*;
use crate::graph::model;
use vstd::prelude::*;

#[path = "reclaim.rs"]
mod reclaim;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) open spec fn valid<G: Graph>(graph: &G, plan: &Plan) -> bool {
    model::valid(graph@) && plan.valid(graph)
}
pub(crate) open spec fn eligible<G: Graph>(graph: &G, plan: &Plan, key: Key) -> bool {
    model::marked(graph@, plan.keep@, key) && !model::marked(graph@, plan.published@, key)
}
pub(crate) open spec fn views(groups: Seq<Retirement>) -> Seq<(Seq<Key>, Seq<RecordId>)> {
    groups.map_values(|g: Retirement| (g.resources@, g.waits@))
}
/// 各组资源按组顺序连接；仅用于归组唯一性，不构造执行期列表。
pub(crate) open spec fn resource_keys<W>(groups: Seq<(Seq<Key>, W)>) -> Seq<Key>
    decreases groups.len(),
{
    if groups.len() == 0 { Seq::empty() }
    else { resource_keys(groups.drop_last()) + groups.last().0 }
}
pub(crate) open spec fn unique(groups: Seq<Retirement>) -> bool {
    resource_keys(views(groups)).no_duplicates()
}
pub(crate) open spec fn covers<G: Graph>(graph: &G, plan: &Plan, groups: Seq<Retirement>) -> bool {
    forall|key: Key| #[trigger] resource_keys(views(groups)).contains(key) <==> eligible(graph, plan, key)
}
pub(crate) open spec fn live<R>(owner: u64, records: &Arena<R>) -> ISet<RecordId> {
    ISet::new(|id: RecordId| id.0.owner == owner && records.lookup(id.0) is Some)
}
pub(crate) open spec fn ready(group: Retirement, live: ISet<RecordId>) -> bool {
    forall|id: RecordId| #[trigger] group.waits@.contains(id) ==> !live.contains(id)
}
/// 较旧组尚存的等待身份也出现在较新组中；不要求保留已死亡的旧身份。
pub(crate) open spec fn waits_nested(groups: Seq<Retirement>, live: ISet<RecordId>) -> bool {
    forall|i: int, j: int, id: RecordId| #![auto]
        0 <= i < j < groups.len() && live.contains(id) && groups[i].waits@.contains(id)
        ==> groups[j].waits@.contains(id)
}
pub(crate) open spec fn taken(before: Seq<Retirement>, after: Seq<Retirement>,
    start: usize, cursor: usize, live: ISet<RecordId>, result: Option<Retirement>) -> bool
{
    start <= cursor && (forall|i: int| #![auto]
        start <= i < cursor && i < before.len() ==> !ready(before[i], live))
        && match result {
            Some(group) => cursor < before.len() && group == before[cursor as int]
                && ready(group, live) && after == before.remove(cursor as int),
            None => after == before && cursor >= before.len(),
        }
}
pub(super) proof fn found_wait(group: &Retirement, index: usize, live: ISet<RecordId>)
    requires index < group.waits@.len(), live.contains(group.waits@[index as int]),
    ensures !ready(*group, live),
{ assert(group.waits@.contains(group.waits@[index as int])); }

pub(crate) proof fn waits_shrink(groups: Seq<Retirement>, before: ISet<RecordId>, after: ISet<RecordId>)
    requires waits_nested(groups, before), after.subset_of(before),
    ensures waits_nested(groups, after),
{}
pub(crate) proof fn waits_insert(groups: Seq<Retirement>, live: ISet<RecordId>, fresh: RecordId)
    requires waits_nested(groups, live),
        forall|i: int| #![auto] 0 <= i < groups.len() ==> !groups[i].waits@.contains(fresh),
    ensures waits_nested(groups, live.insert(fresh)),
{}
pub(crate) proof fn ready_prefix(groups: Seq<Retirement>, live: ISet<RecordId>, i: int, j: int)
    requires waits_nested(groups, live), 0 <= i < j < groups.len(), !ready(groups[i], live),
    ensures !ready(groups[j], live),
{
    let id = choose|id: RecordId| #![auto] groups[i].waits@.contains(id) && live.contains(id);
    assert(groups[j].waits@.contains(id));
}

proof fn resource_keys_push<W>(groups: Seq<(Seq<Key>, W)>, group: (Seq<Key>, W))
    ensures resource_keys(groups.push(group)) == resource_keys(groups) + group.0,
{
    assert(groups.push(group).drop_last() =~= groups);
    assert(groups.push(group).last() == group);
    reveal_with_fuel(resource_keys, 1);
}
pub(crate) open spec fn survivors<G: Graph>(graph: &G, plan: &Plan, groups: Seq<Retirement>) -> Seq<Retirement> {
    groups.filter(|g: Retirement| g.resources@.filter(|k: Key| eligible(graph, plan, k)).len() > 0)
}
pub(crate) open spec fn kept_old<G: Graph>(graph: &G, plan: &Plan, groups: Seq<Retirement>)
    -> Seq<(Seq<Key>, Seq<RecordId>)>
{
    survivors(graph, plan, groups).map_values(|g: Retirement|
        (g.resources@.filter(|k: Key| eligible(graph, plan, k)), g.waits@))
}
pub(crate) open spec fn ordered_filter<G: Graph>(graph: &G, plan: &Plan, groups: Seq<Retirement>)
    -> Seq<(Seq<Key>, Seq<RecordId>)>
{
    groups.map_values(|g: Retirement|
        (g.resources@.filter(|k: Key| eligible(graph, plan, k)), g.waits@))
        .filter(|g: (Seq<Key>, Seq<RecordId>)| g.0.len() > 0)
}
pub(crate) open spec fn preserved<G: Graph>(graph: &G, plan: &Plan, old: Seq<Retirement>, after: Seq<Retirement>) -> bool {
    let kept = survivors(graph, plan, old);
    kept.len() <= after.len() && forall|i: int| #![auto] 0 <= i < kept.len() ==> after[i].waits == kept[i].waits
}
pub(crate) open spec fn covered<G: Graph>(graph: &G, plan: &Plan, old: Seq<Retirement>, key: Key) -> bool {
    eligible(graph, plan, key) && exists|i: int| 0 <= i < old.len() && #[trigger] old[i].resources@.contains(key)
}
pub(crate) open spec fn fresh_keys<G: Graph>(graph: &G, plan: &Plan, old: Seq<Retirement>, fresh: Seq<Key>) -> bool {
    fresh.no_duplicates() && forall|key: Key| #[trigger] fresh.contains(key)
        <==> eligible(graph, plan, key) && !covered(graph, plan, old, key)
}
/// 内容与原契约一致；另要求存续 waits 的 Vec 原值不变，排除克隆替换。
pub(crate) open spec fn reconciled<G: Graph>(graph: &G, plan: &Plan, old: Seq<Retirement>,
    records: Seq<RecordId>, after: Seq<Retirement>) -> bool
{
    preserved(graph, plan, old, after)
        && kept_old(graph, plan, old) == ordered_filter(graph, plan, old)
        && exists|fresh: Seq<Key>| #![auto]
        fresh_keys(graph, plan, old, fresh)
        && views(after) == kept_old(graph, plan, old)
            + if fresh.len() == 0 { Seq::empty() } else { seq![(fresh, records)] }
}
pub(crate) open spec fn filtered<G: Graph>(graph: &G, plan: &Plan, input: Seq<Key>, after: Seq<Key>,
    initial: Seq<bool>, waiting: Seq<bool>) -> bool
{
    after == input.filter(|k: Key| eligible(graph, plan, k))
        && initial.len() == waiting.len()
        && forall|key: Key| #![auto] model::has_key(graph@, key) ==> waiting[key.slot as int]
            == (initial[key.slot as int] || after.contains(key))
}
pub(crate) open spec fn resource_loop<G: Graph>(graph: &G, plan: &Plan, input: Seq<Key>, initial: Seq<bool>,
    current: Seq<Key>, waiting: Seq<bool>, read: int, write: int) -> bool
{
    valid(graph, plan) && 0 <= write <= read <= input.len() && current.len() == input.len()
        && initial.len() == graph@.nodes.len() && waiting.len() == initial.len()
        && current.subrange(0, write) == input.subrange(0, read).filter(|k: Key| eligible(graph, plan, k))
        && (forall|i: int| #![auto] read <= i < input.len() ==> current[i] == input[i])
        && (forall|key: Key| #![auto] model::has_key(graph@, key) ==> waiting[key.slot as int]
            == (initial[key.slot as int] || current.subrange(0, write).contains(key)))
}
pub(crate) open spec fn moved(before: Seq<Retirement>, after: Seq<Retirement>, read: int, write: int) -> bool {
    after.len() == before.len() && after[write] == before[read]
        && (forall|i: int| #![auto] 0 <= i < before.len() && i != read && i != write ==> after[i] == before[i])
        && (read != write ==> after[read].resources@.len() == 0 && after[read].waits@.len() == 0)
}
pub(crate) open spec fn group_loop<G: Graph>(graph: &G, plan: &Plan, original: Seq<Retirement>,
    current: Seq<Retirement>, waiting: Seq<bool>, read: int, write: int) -> bool
{
    valid(graph, plan) && 0 <= write <= read <= original.len() && current.len() == original.len()
        && waiting.len() == graph@.nodes.len()
        && write == survivors(graph, plan, original.subrange(0, read)).len()
        && views(current.subrange(0, write)) == kept_old(graph, plan, original.subrange(0, read))
        && preserved(graph, plan, original.subrange(0, read), current.subrange(0, write))
        && (forall|i: int| #![auto] read <= i < original.len() ==> current[i] == original[i])
        && (forall|key: Key| #![auto] model::has_key(graph@, key) ==> waiting[key.slot as int]
            == covered(graph, plan, original.subrange(0, read), key))
}
pub(crate) open spec fn old_done<G: Graph>(graph: &G, plan: &Plan, original: Seq<Retirement>,
    current: Seq<Retirement>, waiting: Seq<bool>) -> bool
{
    valid(graph, plan) && views(current) == kept_old(graph, plan, original)
        && preserved(graph, plan, original, current) && waiting.len() == graph@.nodes.len()
        && forall|key: Key| #![auto] model::has_key(graph@, key) ==> waiting[key.slot as int] == covered(graph, plan, original, key)
}
pub(crate) open spec fn fresh_scan<G: Graph>(graph: &G, plan: &Plan, waiting: Seq<bool>, fresh: Seq<Key>, slot: int) -> bool
{
    valid(graph, plan) && waiting.len() == graph@.nodes.len() && 0 <= slot <= graph@.nodes.len()
        && fresh.no_duplicates()
        && forall|key: Key| #[trigger] fresh.contains(key) <==>
            eligible(graph, plan, key) && !waiting[key.slot as int] && key.slot < slot
}
pub(crate) open spec fn unassigned<G: Graph>(graph: &G, plan: &Plan, waiting: Seq<bool>, fresh: Seq<Key>) -> bool {
    fresh.no_duplicates() && forall|key: Key| #[trigger] fresh.contains(key) <==>
        eligible(graph, plan, key) && !waiting[key.slot as int]
}

pub(super) proof fn prefix_step<T>(input: Seq<T>, i: int)
    requires 0 <= i < input.len(),
    ensures input.subrange(0, i + 1) == input.subrange(0, i).push(input[i]),
{ assert(input.subrange(0, i + 1) =~= input.subrange(0, i).push(input[i])); }

pub(super) proof fn kept_equivalent<G: Graph>(graph: &G, plan: &Plan, groups: Seq<Retirement>)
    ensures kept_old(graph, plan, groups) == ordered_filter(graph, plan, groups),
    decreases groups.len(),
{
    let mapper = |g: Retirement| (g.resources@.filter(|k: Key| eligible(graph, plan, k)), g.waits@);
    let present = |g: Retirement| g.resources@.filter(|k: Key| eligible(graph, plan, k)).len() > 0;
    let nonempty = |g: (Seq<Key>, Seq<RecordId>)| g.0.len() > 0;
    if groups.len() == 0 {
        assert(groups =~= Seq::<Retirement>::empty());
        reveal_with_fuel(Seq::filter, 1);
    } else {
        let i = groups.len() as int - 1;
        let prefix = groups.subrange(0, i);
        kept_equivalent(graph, plan, prefix);
        prefix_step(groups, i);
        assert(groups.subrange(0, groups.len() as int) =~= groups);
        prefix.lemma_filter_push(groups[i], present);
        prefix.lemma_push_map_commute(mapper, groups[i]);
        prefix.map_values(mapper).lemma_filter_push(mapper(groups[i]), nonempty);
        if present(groups[i]) {
            prefix.filter(present).lemma_push_map_commute(mapper, groups[i]);
        }
    }
}

pub(super) proof fn resource_started<G: Graph>(graph: &G, plan: &Plan, input: Seq<Key>, initial: Seq<bool>)
    requires valid(graph, plan), initial.len() == graph@.nodes.len(),
    ensures resource_loop(graph, plan, input, initial, input, initial, 0, 0),
{
    assert(input.subrange(0, 0) =~= Seq::<Key>::empty());
    reveal_with_fuel(Seq::filter, 1);
}

pub(super) proof fn groups_started<G: Graph>(graph: &G, plan: &Plan, original: Seq<Retirement>, waiting: Seq<bool>)
    requires valid(graph, plan), waiting.len() == graph@.nodes.len(),
        forall|i: int| #![auto] 0 <= i < waiting.len() ==> !waiting[i],
    ensures group_loop(graph, plan, original, original, waiting, 0, 0),
{
    assert(original.subrange(0, 0) =~= Seq::<Retirement>::empty());
    reveal_with_fuel(Seq::filter, 1);
    assert(survivors(graph, plan, original.subrange(0, 0)).len() == 0);
    assert(views(original.subrange(0, 0)) =~= kept_old(graph, plan, original.subrange(0, 0)));
    assert forall|key: Key| #![auto] model::has_key(graph@, key) implies waiting[key.slot as int]
        == covered(graph, plan, original.subrange(0, 0), key) by {
        assert(!waiting[key.slot as int]);
    }
}

pub(super) proof fn push_view(before: Seq<Retirement>, group: Retirement)
    ensures views(before.push(group)) == views(before).push((group.resources@, group.waits@)),
{ before.lemma_push_map_commute(|g: Retirement| (g.resources@, g.waits@), group); }

pub(super) proof fn resource_step<G: Graph>(graph: &G, plan: &Plan, input: Seq<Key>, initial: Seq<bool>,
    before: Seq<Key>, flags: Seq<bool>, read: int, write: int, after: Seq<Key>, waiting: Seq<bool>, next: int)
    requires resource_loop(graph, plan, input, initial, before, flags, read, write), read < input.len(),
        after == if eligible(graph, plan, input[read]) && write != read { before.update(write, input[read]) } else { before },
        waiting == if eligible(graph, plan, input[read]) { flags.update(input[read].slot as int, true) } else { flags },
        next == if eligible(graph, plan, input[read]) { write + 1 } else { write },
    ensures resource_loop(graph, plan, input, initial, after, waiting, read + 1, next),
{
    prefix_step(input, read);
    input.subrange(0, read).lemma_filter_push(input[read], |k: Key| eligible(graph, plan, k));
    assert(after.subrange(0, next) =~= input.subrange(0, read + 1).filter(|k: Key| eligible(graph, plan, k)));
}
pub(super) proof fn resource_done<G: Graph>(graph: &G, plan: &Plan, input: Seq<Key>, initial: Seq<bool>,
    before: Seq<Key>, waiting: Seq<bool>, write: int, after: Seq<Key>)
    requires resource_loop(graph, plan, input, initial, before, waiting, input.len() as int, write),
        after == before.subrange(0, write),
    ensures filtered(graph, plan, input, after, initial, waiting),
{
    assert(input.subrange(0, input.len() as int) =~= input);
}

proof fn filter_contains<G: Graph>(input: Seq<Key>, graph: &G, plan: &Plan, key: Key)
    ensures input.filter(|k: Key| eligible(graph, plan, k)).contains(key) <==> input.contains(key) && eligible(graph, plan, key),
{
    let p = |k: Key| eligible(graph, plan, k);
    if input.filter(p).contains(key) { input.lemma_filter_contains_rev(p, key); }
    if input.contains(key) && eligible(graph, plan, key) {
        let i = choose|i: int| 0 <= i < input.len() && input[i] == key;
        input.lemma_filter_contains(p, i);
    }
}

pub(super) proof fn group_step<G: Graph>(graph: &G, plan: &Plan, original: Seq<Retirement>,
    before: Seq<Retirement>, flags: Seq<bool>, mid: Seq<Retirement>, after: Seq<Retirement>, waiting: Seq<bool>,
    read: int, write: int, next: int)
    requires group_loop(graph, plan, original, before, flags, read, write), read < original.len(),
        mid.len() == before.len(),
        mid[read].waits == before[read].waits,
        filtered(graph, plan, before[read].resources@, mid[read].resources@, flags, waiting),
        forall|i: int| #![auto] 0 <= i < mid.len() && i != read ==> mid[i] == before[i],
        mid[read].resources@.len() > 0 ==> moved(mid, after, read, write) && next == write + 1,
        mid[read].resources@.len() == 0 ==> after == mid && next == write,
    ensures group_loop(graph, plan, original, after, waiting, read + 1, next),
{
    prefix_step(original, read);
    let kept = mid[read].resources@;
    let p = |g: Retirement| g.resources@.filter(|k: Key| eligible(graph, plan, k)).len() > 0;
    let mapper = |g: Retirement| (g.resources@.filter(|k: Key| eligible(graph, plan, k)), g.waits@);
    original.subrange(0, read).lemma_filter_push(original[read], p);
    if kept.len() > 0 {
        survivors(graph, plan, original.subrange(0, read)).lemma_push_map_commute(mapper, original[read]);
    }
    assert(views(after.subrange(0, next)) =~= kept_old(graph, plan, original.subrange(0, read + 1)));
    assert forall|i: int| #![auto] 0 <= i < next implies
        after[i].waits == survivors(graph, plan, original.subrange(0, read + 1))[i].waits by {}
    assert forall|key: Key| #![auto] model::has_key(graph@, key) implies waiting[key.slot as int]
        == covered(graph, plan, original.subrange(0, read + 1), key) by {
        filter_contains(original[read].resources@, graph, plan, key);
        if covered(graph, plan, original.subrange(0, read + 1), key) {
            let i = choose|i: int| 0 <= i < read + 1 && #[trigger] original[i].resources@.contains(key);
            if i < read { assert(original.subrange(0, read)[i].resources@.contains(key)); }
        }
        if covered(graph, plan, original.subrange(0, read), key) {
            let i = choose|i: int| 0 <= i < read && #[trigger] original[i].resources@.contains(key);
            assert(original.subrange(0, read + 1)[i].resources@.contains(key));
        }
        if kept.contains(key) { assert(original.subrange(0, read + 1)[read].resources@.contains(key)); }
    }
}
pub(super) proof fn groups_done<G: Graph>(graph: &G, plan: &Plan, original: Seq<Retirement>,
    before: Seq<Retirement>, after: Seq<Retirement>, waiting: Seq<bool>, write: int)
    requires group_loop(graph, plan, original, before, waiting, original.len() as int, write),
        after == before.subrange(0, write),
    ensures old_done(graph, plan, original, after, waiting),
{ assert(original.subrange(0, original.len() as int) =~= original); }

pub(super) proof fn fresh_step<G: Graph>(graph: &G, plan: &Plan, waiting: Seq<bool>,
    slot: int, before: Seq<Key>, after: Seq<Key>)
    requires valid(graph, plan), 0 <= slot < graph@.nodes.len(), waiting.len() == graph@.nodes.len(),
        before.no_duplicates(),
        forall|key: Key| #[trigger] before.contains(key) <==>
            eligible(graph, plan, key) && !waiting[key.slot as int] && key.slot < slot,
        after == if graph@.nodes[slot] is Some && plan.keep@[slot] && !plan.published@[slot] && !waiting[slot] {
            before.push(model::key(graph@, slot))
        } else { before },
    ensures after.no_duplicates(),
        forall|key: Key| #[trigger] after.contains(key) <==>
            eligible(graph, plan, key) && !waiting[key.slot as int] && key.slot < slot + 1,
{
    if graph@.nodes[slot] is Some && plan.keep@[slot] && !plan.published@[slot] && !waiting[slot] {
        let key = model::key(graph@, slot);
        assert(!before.contains(key));
        assert forall|i: int, j: int| 0 <= i < after.len() && 0 <= j < after.len() && i != j implies after[i] != after[j] by {
            if i < before.len() && j < before.len() { assert(before[i] != before[j]); }
        }
    }
}
pub(super) proof fn scan_done<G: Graph>(graph: &G, plan: &Plan, waiting: Seq<bool>, fresh: Seq<Key>)
    requires fresh_scan(graph, plan, waiting, fresh, graph@.nodes.len() as int),
    ensures unassigned(graph, plan, waiting, fresh),
{}
pub(super) proof fn fresh_complete<G: Graph>(graph: &G, plan: &Plan, old: Seq<Retirement>,
    current: Seq<Retirement>, waiting: Seq<bool>, fresh: Seq<Key>)
    requires old_done(graph, plan, old, current, waiting), unassigned(graph, plan, waiting, fresh),
    ensures fresh_keys(graph, plan, old, fresh),
{}
pub(super) proof fn assembled<G: Graph>(graph: &G, plan: &Plan, old: Seq<Retirement>, records: Seq<RecordId>,
    before: Seq<Retirement>, fresh: Seq<Key>, after: Seq<Retirement>)
    requires valid(graph, plan), views(before) == kept_old(graph, plan, old), preserved(graph, plan, old, before),
        fresh_keys(graph, plan, old, fresh),
        views(after) == if fresh.len() == 0 { views(before) } else { views(before).push((fresh, records)) },
        forall|i: int| #![auto] 0 <= i < before.len() ==> after[i] == before[i],
    ensures reconciled(graph, plan, old, records, after),
{
    assert(views(before) + seq![(fresh, records)] =~= views(before).push((fresh, records)));
    kept_equivalent(graph, plan, old);
}

pub(crate) proof fn resource_keys_contains<W>(groups: Seq<(Seq<Key>, W)>, key: Key)
    ensures resource_keys(groups).contains(key) <==>
        exists|i: int| 0 <= i < groups.len() && #[trigger] groups[i].0.contains(key),
    decreases groups.len(),
{
    if groups.len() > 0 {
        let prefix = groups.drop_last();
        resource_keys_contains(prefix, key);
        if resource_keys(groups).contains(key) && !groups.last().0.contains(key) {
            let i = choose|i: int| 0 <= i < prefix.len() && #[trigger] prefix[i].0.contains(key);
            assert(groups[i] == prefix[i]);
        }
        if exists|i: int| 0 <= i < groups.len() && #[trigger] groups[i].0.contains(key) {
            let i = choose|i: int| 0 <= i < groups.len() && #[trigger] groups[i].0.contains(key);
            if i < prefix.len() { assert(prefix[i] == groups[i]); }
        }
    }
}

proof fn resource_keys_add<W>(left: Seq<(Seq<Key>, W)>, right: Seq<(Seq<Key>, W)>)
    ensures resource_keys(left + right) == resource_keys(left) + resource_keys(right),
    decreases right.len(),
{
    if right.len() == 0 { assert(left + right =~= left); }
    else {
        let prefix = right.drop_last();
        let last = right.last();
        resource_keys_add(left, prefix);
        resource_keys_push(left + prefix, last);
        resource_keys_push(prefix, last);
        assert((left + prefix).push(last) =~= left + right);
        assert((resource_keys(left) + resource_keys(prefix)) + last.0
            =~= resource_keys(left) + (resource_keys(prefix) + last.0));
    }
}

proof fn no_dup_remove_middle<A>(left: Seq<A>, middle: Seq<A>, right: Seq<A>)
    requires (left + middle + right).no_duplicates(),
    ensures (left + right).no_duplicates(),
{
    let before = left + middle + right;
    let after = left + right;
    assert forall|i: int, j: int| 0 <= i < after.len() && 0 <= j < after.len() && i != j
        implies after[i] != after[j] by {
        let old_i = if i < left.len() { i } else { i + middle.len() };
        let old_j = if j < left.len() { j } else { j + middle.len() };
        assert(0 <= old_i < before.len() && 0 <= old_j < before.len() && old_i != old_j);
        assert(after[i] == before[old_i] && after[j] == before[old_j]);
        assert(before[old_i] != before[old_j]);
    }
}

pub(crate) proof fn removed_properties(before: Seq<Retirement>, after: Seq<Retirement>,
    index: int, live: ISet<RecordId>)
    requires 0 <= index < before.len(), after == before.remove(index),
    ensures unique(before) ==> unique(after), waits_nested(before, live) ==> waits_nested(after, live),
{
    let prefix = before.subrange(0, index);
    let suffix = before.subrange(index + 1, before.len() as int);
    assert(before =~= prefix + seq![before[index]] + suffix);
    assert(after =~= prefix + suffix);
    if unique(before) {
        assert(views(before) =~= views(prefix) + views(seq![before[index]]) + views(suffix));
        assert(views(after) =~= views(prefix) + views(suffix));
        assert(views(before) == views(prefix) + views(seq![before[index]]) + views(suffix));
        assert(views(after) == views(prefix) + views(suffix));
        resource_keys_add(views(prefix), views(seq![before[index]]));
        resource_keys_add(views(prefix) + views(seq![before[index]]), views(suffix));
        resource_keys_add(views(prefix), views(suffix));
        no_dup_remove_middle(resource_keys(views(prefix)), resource_keys(views(seq![before[index]])), resource_keys(views(suffix)));
    }
    if waits_nested(before, live) {
        assert forall|i: int, j: int, id: RecordId| #![auto] 0 <= i < j < after.len()
            && live.contains(id) && after[i].waits@.contains(id) implies after[j].waits@.contains(id) by {
            let old_i = if i < index { i } else { i + 1 };
            let old_j = if j < index { j } else { j + 1 };
            assert(0 <= old_i < old_j < before.len());
            assert(after[i] == before[old_i] && after[j] == before[old_j]);
        }
    }
}

proof fn filtered_unique(input: Seq<Key>, pred: spec_fn(Key) -> bool)
    requires input.no_duplicates(),
    ensures input.filter(pred).no_duplicates(),
    decreases input.len(),
{
    if input.len() == 0 {
        reveal_with_fuel(Seq::filter, 1);
    } else {
        let prefix = input.drop_last();
        let last = input.last();
        filtered_unique(prefix, pred);
        prefix.lemma_filter_push(last, pred);
        assert(prefix.push(last) =~= input);
        if pred(last) {
            if prefix.filter(pred).contains(last) { prefix.lemma_filter_contains_rev(pred, last); }
            assert(!prefix.filter(pred).contains(last));
            assert forall|i: int, j: int| 0 <= i < input.filter(pred).len()
                && 0 <= j < input.filter(pred).len() && i != j
                implies input.filter(pred)[i] != input.filter(pred)[j] by {
                if i < prefix.filter(pred).len() && j < prefix.filter(pred).len() {
                    assert(prefix.filter(pred)[i] != prefix.filter(pred)[j]);
                }
            }
        }
    }
}

proof fn kept_resource_keys<G: Graph>(graph: &G, plan: &Plan, old: Seq<Retirement>)
    ensures resource_keys(kept_old(graph, plan, old))
        == resource_keys(views(old)).filter(|key: Key| eligible(graph, plan, key)),
    decreases old.len(),
{
    let pred = |key: Key| eligible(graph, plan, key);
    let present = |group: Retirement| group.resources@.filter(pred).len() > 0;
    let mapper = |group: Retirement| (group.resources@.filter(pred), group.waits@);
    if old.len() == 0 {
        reveal_with_fuel(Seq::filter, 1);
        assert(views(old) =~= Seq::<(Seq<Key>, Seq<RecordId>)>::empty());
        assert(kept_old(graph, plan, old) =~= Seq::<(Seq<Key>, Seq<RecordId>)>::empty());
    } else {
        let prefix = old.drop_last();
        let last = old.last();
        kept_resource_keys(graph, plan, prefix);
        assert(prefix.push(last) =~= old);
        prefix.lemma_filter_push(last, present);
        prefix.lemma_push_map_commute(|group: Retirement| (group.resources@, group.waits@), last);
        assert(views(old) == views(prefix).push((last.resources@, last.waits@)));
        resource_keys_push(views(prefix), (last.resources@, last.waits@));
        assert(resource_keys(views(old)) == resource_keys(views(prefix)) + last.resources@);
        if present(last) {
            prefix.filter(present).lemma_push_map_commute(mapper, last);
            assert(kept_old(graph, plan, old) == kept_old(graph, plan, prefix).push(mapper(last)));
            resource_keys_push(kept_old(graph, plan, prefix), mapper(last));
            assert(resource_keys(kept_old(graph, plan, old))
                == resource_keys(kept_old(graph, plan, prefix)) + last.resources@.filter(pred));
        } else {
            assert(kept_old(graph, plan, old) == kept_old(graph, plan, prefix));
        }
        Seq::filter_distributes_over_add(resource_keys(views(prefix)), last.resources@, pred);
        if !present(last) { assert(last.resources@.filter(pred) =~= Seq::<Key>::empty()); }
    }
}

/// 任意旧输入仍保留原精确过滤结论；唯一归组只在旧状态已唯一时归纳保持。
pub(crate) proof fn unique_reconciled<G: Graph>(graph: &G, plan: &Plan, old: Seq<Retirement>,
    records: Seq<RecordId>, after: Seq<Retirement>)
    requires reconciled(graph, plan, old, records, after), unique(old),
    ensures unique(after),
{
    let fresh = choose|fresh: Seq<Key>| #![auto] fresh_keys(graph, plan, old, fresh)
        && views(after) == kept_old(graph, plan, old)
            + if fresh.len() == 0 { Seq::empty() } else { seq![(fresh, records)] };
    let kept = resource_keys(kept_old(graph, plan, old));
    kept_resource_keys(graph, plan, old);
    filtered_unique(resource_keys(views(old)), |key: Key| eligible(graph, plan, key));
    if fresh.len() > 0 {
        assert(views(after) =~= kept_old(graph, plan, old).push((fresh, records)));
        assert(resource_keys(views(after)) == kept + fresh);
        assert forall|i: int, j: int| 0 <= i < kept.len() && 0 <= j < fresh.len()
            implies kept[i] != fresh[j] by {
            let key = kept[i];
            assert(kept.contains(key));
            resource_keys(views(old)).lemma_filter_contains_rev(|key: Key| eligible(graph, plan, key), key);
            resource_keys_contains(views(old), key);
            if key == fresh[j] {
                assert(fresh.contains(key));
                let g = choose|g: int| 0 <= g < old.len() && #[trigger] views(old)[g].0.contains(key);
                assert(views(old)[g].0 == old[g].resources@);
                assert(covered(graph, plan, old, key));
            }
        }
        vstd::seq_lib::lemma_no_dup_in_concat(kept, fresh);
    }
}

proof fn filtered_pair<A>(input: Seq<A>, pred: spec_fn(A) -> bool, i: int, j: int) -> (pair: (int, int))
    requires 0 <= i < j < input.filter(pred).len(),
    ensures 0 <= pair.0 < pair.1 < input.len(),
        input[pair.0] == input.filter(pred)[i], input[pair.1] == input.filter(pred)[j],
    decreases input.len(),
{
    reveal_with_fuel(Seq::filter, 1);
    let prefix = input.drop_last();
    let filtered = prefix.filter(pred);
    if j < filtered.len() {
        filtered_pair(prefix, pred, i, j)
    } else {
        assert(pred(input.last()));
        assert(i < filtered.len());
        assert(filtered.contains(filtered[i]));
        prefix.lemma_filter_contains_rev(pred, filtered[i]);
        let index = choose|index: int| 0 <= index < prefix.len() && prefix[index] == filtered[i];
        (index, input.len() as int - 1)
    }
}

pub(crate) proof fn nested_reconciled<G: Graph>(graph: &G, plan: &Plan, old: Seq<Retirement>,
    records: Seq<RecordId>, after: Seq<Retirement>, live: ISet<RecordId>)
    requires reconciled(graph, plan, old, records, after), waits_nested(old, live),
        forall|id: RecordId| #[trigger] live.contains(id) ==> records.contains(id),
    ensures waits_nested(after, live),
{
    let fresh = choose|fresh: Seq<Key>| #![auto] fresh_keys(graph, plan, old, fresh)
        && views(after) == kept_old(graph, plan, old)
            + if fresh.len() == 0 { Seq::empty() } else { seq![(fresh, records)] };
    let kept = survivors(graph, plan, old);
    assert(kept_old(graph, plan, old).len() == kept.len());
    assert(views(after).len() == after.len());
    assert forall|i: int, j: int, id: RecordId| #![auto]
        0 <= i < j < after.len() && live.contains(id) && after[i].waits@.contains(id)
        implies after[j].waits@.contains(id) by {
        if j < kept.len() {
            let pair = filtered_pair(old,
                |group: Retirement| group.resources@.filter(|key: Key| eligible(graph, plan, key)).len() > 0,
                i, j);
            assert(after[i].waits == old[pair.0].waits && after[j].waits == old[pair.1].waits);
        } else {
            assert(j == kept.len() && fresh.len() > 0);
            assert(views(after)[j].1 == records);
        }
    }
}

pub(crate) proof fn reconciled_coverage<G: Graph>(graph: &G, plan: &Plan, old: Seq<Retirement>,
    records: Seq<RecordId>, after: Seq<Retirement>)
    requires reconciled(graph, plan, old, records, after),
    ensures covers(graph, plan, after),
{
    let fresh = choose|fresh: Seq<Key>| #![auto] fresh_keys(graph, plan, old, fresh)
        && views(after) == kept_old(graph, plan, old)
            + if fresh.len() == 0 { Seq::empty() } else { seq![(fresh, records)] };
    let kept = resource_keys(kept_old(graph, plan, old));
    kept_resource_keys(graph, plan, old);
    if fresh.len() > 0 {
        assert(views(after) =~= kept_old(graph, plan, old).push((fresh, records)));
        resource_keys_push(kept_old(graph, plan, old), (fresh, records));
        assert(resource_keys(views(after)) == kept + fresh);
    }
    assert forall|key: Key| #[trigger] resource_keys(views(after)).contains(key)
        <==> eligible(graph, plan, key) by {
        resource_keys_contains(views(old), key);
        filter_contains(resource_keys(views(old)), graph, plan, key);
        if eligible(graph, plan, key) {
            if covered(graph, plan, old, key) {
                let i = choose|i: int| 0 <= i < old.len() && #[trigger] old[i].resources@.contains(key);
                assert(views(old)[i].0 == old[i].resources@);
                assert(resource_keys(views(old)).contains(key));
                assert(kept.contains(key));
            } else { assert(fresh.contains(key)); }
        }
    }
}

pub(super) proof fn assembled_complete<G: Graph, R>(graph: &G, plan: &Plan, old: Seq<Retirement>,
    owner: u64, records: &Arena<R>, before: Seq<Retirement>, fresh: Seq<Key>, after: Seq<Retirement>)
    requires valid(graph, plan), records.valid(),
        views(before) == kept_old(graph, plan, old), preserved(graph, plan, old, before),
        fresh_keys(graph, plan, old, fresh),
        views(after) == if fresh.len() == 0 { views(before) } else { views(before).push((fresh, record_keys(owner, records))) },
        forall|i: int| #![auto] 0 <= i < before.len() ==> after[i] == before[i],
    ensures reconciled(graph, plan, old, record_keys(owner, records), after),
        covers(graph, plan, after),
        unique(old) ==> unique(after),
        waits_nested(old, live(owner, records)) ==> waits_nested(after, live(owner, records)),
{
    assembled(graph, plan, old, record_keys(owner, records), before, fresh, after);
    reconciled_coverage(graph, plan, old, record_keys(owner, records), after);
    if unique(old) { unique_reconciled(graph, plan, old, record_keys(owner, records), after); }
    if waits_nested(old, live(owner, records)) {
        assert forall|id: RecordId| #[trigger] live(owner, records).contains(id)
            implies record_keys(owner, records).contains(id) by {
            crate::record_ids::proof::contains(owner, records, id);
        }
        nested_reconciled(graph, plan, old, record_keys(owner, records), after, live(owner, records));
    }
}
}
