//! 真实生命周期容器的数学视图与桥接证明。

use super::*;
use vstd::prelude::*;

verus! {
impl<P: Payloads, B> Life<P, B> {
    pub(crate) open(super) spec fn record_root_keys(&self) -> Seq<Key> {
        super::roots::proof::records(self.records.values_view())
    }

    pub(crate) proof fn record_root_member(&self, id: crate::handles::RecordId, key: Key)
        requires self.record_lookup(id) is Some,
            super::roots::proof::record(&self.record_lookup(id)->Some_0).contains(key),
        ensures self.record_root_keys().contains(key),
    {
        self.records.lookup_in_values(id.0);
        super::roots::proof::record_member(self.records.values_view(), self.record_lookup(id)->Some_0, key);
    }

    /// 所有退役组连接后的完整 Key 无重复；不合并槽号相同但代次或 owner 不同的身份。
    pub(crate) open(super) spec fn groups_unique(&self) -> bool {
        crate::retirement::proof::unique(self.retirements@)
    }

    pub(crate) open(super) spec fn resources_valid(&self) -> bool {
        self.resources.valid() && self.retiring@.len() == self.resources.slot_count()
    }

    pub(crate) open(super) spec fn resource_lookup(&self, key: Key) -> Option<Data<P>> {
        if key.owner == self.owner { self.resources.lookup(key) } else { None }
    }

    pub(crate) open(super) spec fn active_key(&self, key: Key) -> bool {
        self.resource_lookup(key) is Some && !self.retiring@[key.slot as int]
            && match self.resource_lookup(key)->Some_0.producer_value() {
                Some(id) => self.record_lookup(id) is Some, None => true,
            }
    }

    pub(crate) open(super) spec fn retired_key(&self, key: Key) -> bool {
        self.resource_lookup(key) is Some && self.retiring@[key.slot as int]
    }

    pub(crate) open(super) spec fn resource_revoked(&self, key: Key) -> bool {
        key.owner == self.owner && self.resources.revoked(key)
    }

    pub(crate) open(super) spec fn query_lookup(&self, key: Key) -> Option<P::Queries> {
        match self.resource_lookup(key) { Some(Data::Timestamps(raw)) => Some(raw), _ => None }
    }

    pub(crate) open(super) spec fn resource_graph(&self) -> crate::graph::Model {
        Resources::<P>::model(self.owner, &self.resources)
    }

    pub(crate) open(super) spec fn plan_valid(&self, plan: &crate::graph::Plan) -> bool {
        forall|graph: Resources<P>| #[trigger] graph@ == self.resource_graph() ==> plan.valid(&graph)
    }

    pub(crate) proof fn revoked_resource_absent(&self, key: Key)
        requires self.resource_revoked(key),
        ensures self.resource_lookup(key) is None,
    { crate::arena::revoked_absent(&self.resources, key); }
}

pub(crate) open(super) spec fn groups_unchanged<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>) -> bool {
    after.retirements == before.retirements
}

pub(crate) open(super) spec fn inserted<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>,
    value: Data<P>, result: Result<Key, (Error, Data<P>)>) -> bool
{
    after.resources_valid() && after.owner == before.owner
        && after.records == before.records && after.timeline == before.timeline && after.retirements == before.retirements
        && (forall|key: Key| #[trigger] before.resource_revoked(key) ==> after.resource_revoked(key))
        && match result {
            Ok(key) => after.resource_lookup(key) == Some(value) && before.resource_lookup(key) is None
                && !after.retiring@[key.slot as int]
                && (forall|previous: Key| #![auto] !previous.identifies(key)
                    ==> after.resource_lookup(previous) == before.resource_lookup(previous))
                && (forall|i: int| #![auto] 0 <= i < before.retiring@.len() && i != key.slot
                    ==> after.retiring@[i] == before.retiring@[i]),
            Err((_, rejected)) => rejected == value && after.resources.same_entries(&before.resources)
                && after.retiring == before.retiring,
        }
}

pub(crate) open(super) spec fn removed<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>,
    key: Key, result: Option<Data<P>>) -> bool
{
    after.resources_valid() && after.owner == before.owner && after.retiring == before.retiring
        && after.records == before.records && after.timeline == before.timeline && after.retirements == before.retirements
        && result == before.resource_lookup(key) && after.resource_lookup(key) is None
        && (result is Some ==> after.resource_revoked(key))
        && (forall|previous: Key| #![auto] !previous.identifies(key)
            ==> after.resource_lookup(previous) == before.resource_lookup(previous))
        && (forall|previous: Key| #[trigger] before.resource_revoked(previous) ==> after.resource_revoked(previous))
}

pub(crate) open(super) spec fn queried<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, key: Key) -> bool {
    after.resources_valid() && after.owner == before.owner && after.records == before.records
        && after.retiring == before.retiring && after.retirements == before.retirements && after.timeline == before.timeline
        && after.resources.same_shape(&before.resources)
        && (forall|other: Key| #![auto] !other.identifies(key)
            ==> after.resources.lookup(other) == before.resources.lookup(other))
        && (before.query_lookup(key) is Some <==> after.query_lookup(key) is Some)
        && (before.query_lookup(key) is None ==> after.resources.lookup(key) == before.resources.lookup(key))
}

/// 查询 payload 的借用只改变数值；真实图的槽位、代际与链接保持不变。
pub(crate) proof fn queries_keep_graph<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, key: Key)
    requires before.resources_valid(), queried(before, after, key),
    ensures after.resource_graph() == before.resource_graph(),
{
    assert forall|other: Key| #![auto] before.resources.lookup(other) is Some && after.resources.lookup(other) is Some
        implies before.resources.lookup(other)->Some_0@ == after.resources.lookup(other)->Some_0@ by {
        if other.identifies(key) {
            before.resources.lookup_identity(other, key);
            after.resources.lookup_identity(other, key);
            if before.query_lookup(key) is Some {
                Data::<P>::timestamps_edges(before.query_lookup(key)->Some_0);
                Data::<P>::timestamps_edges(after.query_lookup(key)->Some_0);
            }
        }
    }
    Resources::<P>::same_model(before.owner, &after.resources, &before.resources);
}

pub(crate) open(super) spec fn ready_taken<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>,
    start: usize, cursor: usize, result: Option<Retirement>) -> bool
{
    after.owner == before.owner && after.resources == before.resources && after.records == before.records
        && after.retiring == before.retiring && after.timeline == before.timeline
        && (before.resources_valid() ==> after.resources_valid())
        && (before.records_valid() ==> after.records_valid())
        && crate::retirement::proof::taken(before.retirements@, after.retirements@, start, cursor,
            crate::retirement::proof::live(before.owner, &before.records), result)
        && (crate::retirement::proof::unique(before.retirements@) ==> crate::retirement::proof::unique(after.retirements@))
        && (crate::retirement::proof::waits_nested(before.retirements@, crate::retirement::proof::live(before.owner, &before.records))
            ==> crate::retirement::proof::waits_nested(after.retirements@, crate::retirement::proof::live(after.owner, &after.records)))
}

pub(crate) proof fn insertion_keeps_revoked<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>,
    value: Data<P>, result: Result<Key, (Error, Data<P>)>, key: Key)
    requires inserted(before, after, value, result), before.resource_revoked(key),
    ensures after.resources_valid(), after.resource_revoked(key), after.resource_lookup(key) is None,
{ after.revoked_resource_absent(key); }

pub(crate) open(super) spec fn committing<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>,
    plan: &crate::graph::Plan, graph: &Resources<P>, slot: usize) -> bool
{
    before.resources_valid() && before.records_valid() && after.resources_valid()
        && graph@ == before.resource_graph() && plan.valid(graph)
        && after.owner == before.owner && after.resources == before.resources && after.records == before.records
        && after.timeline == before.timeline
        && slot <= after.retiring@.len() && plan.published@.len() == after.retiring@.len()
        && (forall|i: int| #![auto] 0 <= i < slot ==> after.retiring@[i] == !plan.published@[i])
        && crate::retirement::proof::reconciled(graph, plan, before.retirements@, record_keys(before.owner, &before.records), after.retirements@)
        && crate::retirement::proof::covers(graph, plan, after.retirements@)
        && (crate::retirement::proof::unique(before.retirements@) ==> crate::retirement::proof::unique(after.retirements@))
        && (crate::retirement::proof::waits_nested(before.retirements@, crate::retirement::proof::live(before.owner, &before.records))
            ==> crate::retirement::proof::waits_nested(after.retirements@, crate::retirement::proof::live(after.owner, &after.records)))
}

pub(crate) open(super) spec fn committed<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>, plan: &crate::graph::Plan) -> bool {
    after.retiring@.len() <= usize::MAX && exists|graph: Resources<P>| #[trigger]
        committing(before, after, plan, &graph, after.retiring@.len() as usize)
}

pub(crate) open(super) spec fn collected<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>,
    roots: Seq<crate::handles::Resource>, result: &Result<crate::graph::Plan, crate::graph::PlanError>) -> bool
{
    match result {
        Err(_) => *after == *before,
        Ok(plan) => committed(before, after, plan)
            && crate::graph::model::matches(before.resource_graph(), plan.published@,
                super::roots::proof::resources(roots), Seq::empty())
            && crate::graph::model::matches(before.resource_graph(), plan.keep@,
                super::roots::proof::resources(roots), before.record_root_keys()),
    }
}

/// 计划成功时，任何真实存活记录可达的资源都保留；包括间接依赖及任意环。
/// 这不代替后续记录准入或首次 waits 的跨调用归纳。
pub(crate) proof fn record_consumers_kept<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>,
    roots: Seq<crate::handles::Resource>, plan: crate::graph::Plan, id: crate::handles::RecordId, key: Key)
    requires collected(before, after, roots, &Ok(plan)), before.record_lookup(id) is Some,
        crate::graph::model::reachable(before.resource_graph(),
            super::roots::proof::record(&before.record_lookup(id)->Some_0), Seq::empty(), key),
    ensures crate::graph::model::marked(before.resource_graph(), plan.keep@, key),
{
    let graph = before.resource_graph();
    let own = super::roots::proof::record(&before.record_lookup(id)->Some_0);
    let published = super::roots::proof::resources(roots);
    assert forall|set: ISet<Key>| crate::graph::model::seeded(published, before.record_root_keys(), set)
        && crate::graph::model::closed(graph, set) implies set.contains(key) by {
        assert forall|i: int| #![auto] 0 <= i < own.len() implies set.contains(own[i]) by {
            before.record_root_member(id, own[i]);
            let index = choose|index: int| 0 <= index < before.record_root_keys().len()
                && before.record_root_keys()[index] == own[i];
        }
        assert(crate::graph::model::seeded(own, Seq::empty(), set));
    }
    assert(crate::graph::model::reachable(graph, published, before.record_root_keys(), key));
}

pub(super) proof fn plan_ready<P: Payloads, B>(life: &Life<P, B>, plan: &crate::graph::Plan, graph: &Resources<P>)
    requires life.resources_valid(), life.plan_valid(plan), graph@ == life.resource_graph(),
    ensures crate::retirement::proof::valid(graph, plan),
{
    life.resources.slot_bound();
    assert(crate::graph::model::valid(graph@));
    assert(plan.valid(graph));
}

/// Plan.valid 只依赖真实图的视图，不扩大计划的有效输入域。
pub(crate) proof fn plan_matches_owner<P: Payloads, B>(life: &Life<P, B>, plan: &crate::graph::Plan, graph: &Resources<P>)
    requires graph@ == life.resource_graph(), plan.valid(graph),
    ensures life.plan_valid(plan),
{}

/// 提交后完整身份仍指向同一资源；退役标记精确对应未发布槽位。
pub(crate) proof fn committed_partition<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>,
    plan: &crate::graph::Plan, key: Key)
    requires committed(before, after, plan), before.resource_lookup(key) is Some,
    ensures after.resource_lookup(key) == before.resource_lookup(key),
        after.retired_key(key) == !plan.published@[key.slot as int],
{
    let graph = choose|graph: Resources<P>| #[trigger]
        committing(before, after, plan, &graph, after.retiring@.len() as usize);
    before.resources.lookup_bounds(key);
}

pub(super) proof fn commit_finished<P: Payloads, B>(before: &Life<P, B>, after: &Life<P, B>,
    plan: &crate::graph::Plan, graph: &Resources<P>, slot: usize)
    requires committing(before, after, plan, graph, slot), slot == after.retiring@.len(),
    ensures committed(before, after, plan),
{
    assert(committing(before, after, plan, graph, after.retiring@.len() as usize));
    assert(exists|candidate: Resources<P>| #[trigger]
        committing(before, after, plan, &candidate, after.retiring@.len() as usize)) by {
        assert(committing(before, after, plan, graph, after.retiring@.len() as usize));
    }
}

fn new_groups_are_unique<P: Payloads, B>(owner: u64) {
    let life = Life::<P, B>::new(owner);
    assert(life.groups_unique());
    #[cfg(reject_lifecycle_groups_unique)]
    assert(!life.groups_unique());
}

// 下面逐类调用真实 API，检查已有 exact frame 足以闭合归纳，不向各接口重复添加属性。
fn resource_mutators_keep_groups<P: Payloads, B>(life: &mut Life<P, B>, key: Key,
    resource: Data<P>, query: P::Queries, action: u8)
    requires old(life).resources_valid(),
    ensures old(life).groups_unique() ==> final(life).groups_unique(),
{
    let ghost before = *life;
    match action {
        0 => { let _ = life.insert_resource(resource); },
        1 => { if let Some(raw) = life.queries_mut(key) { *raw = query; } },
        _ => { let _ = life.take_resource(key); },
    }
    assert(life.retirements == before.retirements);
}

fn record_mutators_keep_groups<P: Payloads, B>(life: &mut Life<P, B>, id: crate::handles::RecordId,
    buffers: B, completed: bool, action: u8)
    requires old(life).resources_valid(), old(life).records_valid(),
    ensures old(life).groups_unique() ==> final(life).groups_unique(),
{
    let ghost before = *life;
    match action {
        0 => {
            let record = Record::new(Vec::new(), Vec::new(), Vec::new(), buffers);
            let _ = life.insert_record(record);
        },
        1 => { let _ = life.mark_external(id); },
        2 => { let _ = life.begin_sync(id); },
        3 => { let _ = life.take_pending(id); },
        4 => { let _ = life.take_external(id); },
        5 => { let _ = life.take_completed(id); },
        6 => { let _ = life.take_sync(id, completed); },
        7 => { let _ = life.take_shutdown(id); },
        _ => { life.quarantine_records(&[id]); },
    }
    assert(life.retirements == before.retirements);
}

fn completion_keeps_groups<P: Payloads, B>(life: &mut Life<P, B>, serial: u64)
    requires old(life).records_valid(),
    ensures old(life).groups_unique() ==> final(life).groups_unique(),
{
    let _ = life.observe_completion(serial);
}

fn attempt_mutators_keep_groups<P: Payloads, B>(life: &mut Life<P, B>, ids: &[crate::handles::RecordId], accept: bool)
    requires old(life).records_valid(),
    ensures old(life).groups_unique() ==> final(life).groups_unique(),
{
    let ghost before = *life;
    match life.prepare_batch(ids) {
        Ok(attempt) => {
            proof { record::proof::attempt_groups_unchanged(&before, &attempt, ids@); }
            if accept { let _ = attempt.accept(); } else { attempt.quarantine(); }
        },
        Err(_) => {},
    }
    assert(life.retirements == before.retirements);
}

fn collection_mutators_keep_groups<P: Payloads, B>(life: &mut Life<P, B>, plan: &crate::graph::Plan, cursor: &mut usize)
    requires old(life).resources_valid(), old(life).records_valid(), old(life).plan_valid(plan),
    ensures old(life).groups_unique() ==> final(life).groups_unique(),
{
    life.commit_plan(plan);
    let _ = life.take_ready(cursor);
}
}
