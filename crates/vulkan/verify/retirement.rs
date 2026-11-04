//! 装配生产原位退役更新；不验证 native 邻接适配或 GPU 完成事实。
#![allow(dead_code)]
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]
#[path = "../src/arena.rs"]
mod arena;
mod owner {
    pub(crate) use crate::arena::{Arena, Key};
}
#[path = "../src/handles.rs"]
mod handles;
#[path = "../src/record_ids.rs"]
mod record_ids;
mod lifecycle {
    pub(crate) use crate::record_ids::proof::keys as record_keys;
    pub(crate) use crate::record_ids::record_ids;
}
#[path = "../src/graph.rs"]
mod graph;
#[path = "../src/retirement.rs"]
mod retirement;

use graph::{Graph, model};
use retirement::proof as spec;
use vstd::prelude::*;

verus! {
fn unchanged_waits<G: Graph, R>(graph: &G, key: owner::Key, first: handles::RecordId, owner: u64, records: &arena::Arena<R>)
    requires model::has_key(graph@, key), records.valid(), lifecycle::record_keys(owner, records).len() >= 2,
        lifecycle::record_keys(owner, records).contains(first),
{
    let n = graph.len();
    let mut keep = vec![false; n];
    keep[key.slot as usize] = true;
    let plan = graph::Plan { published: vec![false; n], keep };
    let mut groups = vec![retirement::Retirement { resources: vec![key], waits: vec![first] }];
    let ghost original = groups@;
    retirement::reconcile(graph, &plan, &mut groups, owner, records);
    proof {
        reveal_with_fuel(Seq::filter, 3);
        assert(spec::eligible(graph, &plan, key));
        assert(spec::survivors(graph, &plan, original) =~= original);
        assert(spec::preserved(graph, &plan, original, groups@));
        assert(groups@[0].waits == original[0].waits);
    }
    assert(groups.len() >= 1);
    assert(groups[0].waits@ == seq![first]);
    #[cfg(reject_retirement_wait_growth)]
    assert(groups[0].waits@ == lifecycle::record_keys(owner, records));
}

fn republished_not_retired<G: Graph, R>(graph: &G, key: owner::Key, first: handles::RecordId, owner: u64, records: &arena::Arena<R>)
    requires model::has_key(graph@, key), records.valid(),
{
    let n = graph.len();
    let mut published = vec![false; n];
    let mut keep = vec![false; n];
    published[key.slot as usize] = true;
    keep[key.slot as usize] = true;
    let plan = graph::Plan { published, keep };
    let mut groups = vec![retirement::Retirement { resources: vec![key], waits: vec![first] }];
    let ghost original = groups@;
    retirement::reconcile(graph, &plan, &mut groups, owner, records);
    proof {
        assert(plan.published@ =~= plan.keep@);
        assert forall|candidate: owner::Key| !spec::eligible(graph, &plan, candidate) by {}
        reveal_with_fuel(Seq::filter, 3);
        assert(spec::kept_old(graph, &plan, original).len() == 0);
        let fresh = choose|fresh: Seq<owner::Key>| #![auto] spec::fresh_keys(graph, &plan, original, fresh)
            && spec::views(groups@) == spec::kept_old(graph, &plan, original)
                + if fresh.len() == 0 { Seq::empty() } else { seq![(fresh, lifecycle::record_keys(owner, records))] };
        if fresh.len() > 0 { assert(fresh.contains(fresh[0])); }
        assert(fresh.len() == 0);
        assert(spec::views(groups@).len() == groups@.len());
    }
    assert(groups.len() == 0);
    #[cfg(reject_retirement_republish)]
    assert(exists|i: int| 0 <= i < groups@.len() && groups@[i].resources@.contains(key));
}

fn unique_assignment<G: Graph, R>(graph: &G, plan: &graph::Plan, groups: &mut Vec<retirement::Retirement>,
    owner: u64, records: &arena::Arena<R>)
    requires spec::valid(graph, plan), records.valid(), spec::unique(old(groups)@),
{
    retirement::reconcile(graph, plan, groups, owner, records);
    assert(spec::unique(groups@));
    #[cfg(reject_retirement_duplicate_assignment)]
    assert(!spec::unique(groups@));
}

fn live_wait_not_released<R>(owner: u64, records: &arena::Arena<R>, id: handles::RecordId)
    requires records.valid(), lifecycle::record_keys(owner, records).contains(id),
{
    let mut groups = vec![retirement::Retirement { resources: Vec::new(), waits: vec![id] }];
    let ghost original = groups@;
    let mut cursor = 0;
    let result = retirement::take_ready(&mut groups, &mut cursor, owner, records);
    proof {
        record_ids::proof::contains(owner, records, id);
        if result is Some {
            assert(cursor == 0);
            assert(result->Some_0 == original[0]);
            assert(result->Some_0.waits@.contains(id));
        }
    }
    assert(result is None);
    #[cfg(reject_retirement_live_wait_release)]
    assert(result is Some);
}
}
fn main() {}
