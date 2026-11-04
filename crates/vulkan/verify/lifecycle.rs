//! 装配真实 Life 容器；平台 payload 与 GPU 完成事实不在此证明。
#![allow(dead_code)]
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]
#[path = "../src/arena.rs"]
mod arena;
mod owner {
    pub(crate) use crate::arena::{Arena, InsertError, Key, Values};
}
#[path = "../src/graph.rs"]
mod graph;
#[path = "../src/handles.rs"]
mod handles;
#[path = "../src/lifecycle.rs"]
mod lifecycle;
#[path = "../src/record_ids.rs"]
mod record_ids;
#[path = "../src/retirement.rs"]
mod retirement;
#[path = "../src/submission.rs"]
mod submission;

use lifecycle::{
    Life,
    resource::{Data, Payloads},
};
use vstd::prelude::*;

verus! {
fn resource_consumed_once<P: Payloads, B>(life: &mut Life<P, B>, key: arena::Key)
    requires old(life).resources_valid(),
{
    let _first = life.take_resource(key);
    let second = life.take_resource(key);
    assert(second is None);
    #[cfg(reject_resource_double_take)]
    assert(second is Some);
}

fn resource_generation_never_revives<P: Payloads, B>(life: &mut Life<P, B>, key: arena::Key, value: Data<P>)
    requires old(life).resources_valid(), old(life).resource_lookup(key) is Some,
{
    let removed = life.take_resource(key);
    assert(removed is Some);
    let ghost before = *life;
    let ghost payload = value;
    let result = life.insert_resource(value);
    proof { lifecycle::proof::insertion_keeps_revoked(&before, life, payload, result, key); }
    let lookup = life.resource(key);
    assert(lookup is Err);
    #[cfg(reject_resource_generation_revival)]
    assert(lookup is Ok);
}

fn exact_ready_removal<P: Payloads, B>(life: &mut Life<P, B>, cursor: &mut usize) {
    let ghost before = *life;
    let ghost start = *cursor;
    let result = life.take_ready(cursor);
    assert(lifecycle::proof::ready_taken(&before, life, start, *cursor, result));
    #[cfg(reject_lifecycle_ready_mutation)]
    assert(!lifecycle::proof::ready_taken(&before, life, start, *cursor, result));
}

fn commit_matches_publication<P: Payloads, B>(life: &mut Life<P, B>, plan: &graph::Plan, key: arena::Key)
    requires old(life).resources_valid(), old(life).records_valid(), old(life).plan_valid(plan),
        old(life).resource_lookup(key) is Some,
{
    let ghost before = *life;
    life.commit_plan(plan);
    proof { lifecycle::proof::committed_partition(&before, life, plan, key); }
    assert(life.retired_key(key) == !plan.published@[key.slot as int]);
    #[cfg(reject_lifecycle_retiring_publication)]
    assert(life.retired_key(key) == plan.published@[key.slot as int]);
}

fn queries_change_payload_only<P: Payloads, B>(life: &mut Life<P, B>, key: arena::Key, value: P::Queries)
    requires old(life).resources_valid(),
{
    let ghost before = *life;
    if let Some(raw) = life.queries_mut(key) { *raw = value; }
    proof { lifecycle::proof::queries_keep_graph(&before, life, key); }
    assert(life.resource_graph() == before.resource_graph());
    #[cfg(reject_lifecycle_query_link_change)]
    assert(life.resource_graph() != before.resource_graph());
}

fn published_roots_keep_order(roots: &[handles::Resource])
    requires roots@.len() > 0,
{
    let mut source = lifecycle::roots::ResourceRoots::new(roots);
    let first = graph::Seeds::next(&mut source);
    assert(first == Some(roots@[0].0));
    #[cfg(reject_published_root_order)]
    assert(first is None);
}

fn record_roots_reset_exactly<P: Payloads, B>(life: &Life<P, B>) {
    let mut source = life.record_roots();
    let first = graph::Seeds::next(&mut source);
    graph::Seeds::reset(&mut source);
    let again = graph::Seeds::next(&mut source);
    assert(first == again);
    assert(source@.keys == life.record_root_keys());
    #[cfg(reject_record_root_reset)]
    assert(first != again);
}

fn collection_keeps_actual_consumers<P: Payloads, B>(life: &mut Life<P, B>, roots: &[handles::Resource],
    id: handles::RecordId, key: arena::Key)
    requires old(life).resources_valid(), old(life).records_valid(), old(life).record_lookup(id) is Some,
        graph::model::reachable(old(life).resource_graph(),
            lifecycle::roots::proof::record(&old(life).record_lookup(id)->Some_0), Seq::empty(), key),
{
    let ghost before = *life;
    match life.collect_plan(roots) {
        Ok(plan) => {
            proof { lifecycle::proof::record_consumers_kept(&before, life, roots@, plan, id, key); }
            assert(graph::model::marked(before.resource_graph(), plan.keep@, key));
            #[cfg(reject_collection_live_consumer)]
            assert(!plan.keep@[key.slot as int]);
        },
        Err(_) => {
            assert(*life == before);
            #[cfg(reject_collection_partial_commit)]
            assert(*life != before);
        },
    }
}
}
fn main() {}
