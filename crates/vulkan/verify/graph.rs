//! 装配生产借用图闭包核；邻接适配不导入 Vulkan FFI。
#![allow(dead_code)]
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]
#[path = "../src/arena.rs"]
mod arena;
mod owner {
    pub(crate) use crate::arena::Key;
}
#[path = "../src/graph.rs"]
mod graph;
mod graph_fixture;
use graph::{Graph, Keys, model};
use vstd::prelude::*;

verus! {
fn leaf_root_is_marked<G: Graph>(graph: &G)
    requires graph@.nodes.len() > 0, graph@.nodes[0] is Some, graph@.nodes[0]->Some_0.edges.len() == 0,
{
    let (key, _) = graph.node(0).unwrap();
    let roots = vec![key];
    let empty = Vec::new();
    let mut source = Keys::new(&roots);
    let mut extra = Keys::new(&empty);
    let result = match graph::plan(graph, &mut source, &mut extra) { Ok(result) => result, Err(_) => return };
    proof {
        assert(roots@ == seq![key]);
        assert(empty@ == Seq::<arena::Key>::empty());
        root_is_reachable(graph@, key);
        assert(model::has_key(graph@, key));
        assert(model::marked(graph@, result.published@, key));
    }
    assert(result.published@[0]);
    assert(result.keep@[0]);
    #[cfg(reject_graph_unmarked_leaf)]
    assert(!result.keep@[0]);
}

proof fn root_is_reachable(graph: graph::Model, key: arena::Key)
    ensures model::reachable(graph, seq![key], Seq::empty(), key),
{
    assert forall|keys: ISet<arena::Key>| #![auto] model::seeded(seq![key], Seq::empty(), keys) && model::closed(graph, keys)
        implies keys.contains(key) by { assert(seq![key][0] == key); }
}

fn marked_generation_rejected<G: Graph>(graph: &G)
    requires graph@.nodes.len() > 0, graph@.nodes[0] is Some,
        graph@.nodes[0]->Some_0.generation > 0,
{
    let key = graph.present_key(0);
    let stale = arena::Key { owner: key.owner, slot: key.slot, generation: key.generation - 1 };
    let roots = vec![key, stale];
    let empty = Vec::new();
    let mut source = Keys::new(&roots);
    let mut extra = Keys::new(&empty);
    let result = graph::plan(graph, &mut source, &mut extra);
    proof {
        assert(roots@ == seq![key, stale]);
        assert forall|keys: ISet<arena::Key>| #![auto] model::seeded(roots@, empty@, keys)
            && model::closed(graph@, keys) implies keys.contains(stale) by {
            assert(roots@[1] == stale);
        }
        assert(model::reachable(graph@, roots@, empty@, stale));
        assert(!model::has_key(graph@, stale));
        if result is Ok {
            assert(model::marked(graph@, result->Ok_0.keep@, stale));
            assert(false);
        }
    }
    assert(result is Err);
    #[cfg(reject_graph_marked_generation)]
    assert(result is Ok);
}

#[cfg(reject_graph_missing_dependency)]
fn reject_graph_missing_dependency<G: Graph>(graph: &G, roots: &[arena::Key], from: arena::Key, to: arena::Key)
    requires roots@.contains(from), model::dependency(graph@, from, to),
{
    let extra = Vec::new();
    let mut source = Keys::new(roots);
    let mut extra_source = Keys::new(&extra);
    let result = match graph::plan(graph, &mut source, &mut extra_source) { Ok(result) => result, Err(_) => return };
    proof {
        assert forall|keys: ISet<arena::Key>| #![auto] model::seeded(roots@, extra@, keys) && model::closed(graph@, keys)
            implies keys.contains(to) by {
            let i = choose|i: int| 0 <= i < roots@.len() && roots@[i] == from;
            assert(keys.contains(from));
        }
        assert(model::reachable(graph@, roots@, extra@, to));
        assert(model::marked(graph@, result.keep@, to));
    }
    assert(!result.keep@[to.slot as int]);
}

#[cfg(reject_graph_unreachable_keep)]
fn reject_graph_unreachable_keep<G: Graph>(graph: &G)
    requires graph@.nodes.len() > 0, graph@.nodes[0] is Some,
{
    let roots = Vec::new();
    let mut source = Keys::new(&roots);
    let mut extra_source = Keys::new(&roots);
    let result = match graph::plan(graph, &mut source, &mut extra_source) { Ok(result) => result, Err(_) => return };
    proof {
        let empty = ISet::<arena::Key>::empty(); let id = model::key(graph@, 0);
        assert(model::seeded(roots@, roots@, empty)); assert(model::closed(graph@, empty)); assert(!empty.contains(id));
        assert(model::reachable(graph@, roots@, roots@, id) ==> empty.contains(id));
        assert(!model::reachable(graph@, roots@, roots@, id)); assert(model::has_key(graph@, id));
        assert(model::marked(graph@, result.keep@, id) <==> model::reachable(graph@, roots@, roots@, id));
        assert(!result.keep@[0]);
    }
    assert(result.keep@[0]);
}
}
fn main() {}
