//! 真实所有者适配与 DFS 精确差集；不装配射线或 GPU。
#![feature(portable_simd, proc_macro_hygiene)]
#![allow(dead_code)]

#[path = "../src/proof/balance.rs"]
mod balance;
#[path = "../src/proof/canonical.rs"]
mod canonical;
#[path = "../src/proof/commit.rs"]
mod commit_proof;
#[path = "../src/diff.rs"]
mod diff;
#[path = "../src/error.rs"]
mod error;
#[path = "../src/hashdag.rs"]
mod hashdag;
#[path = "../src/layers.rs"]
mod layers;
#[path = "../src/layout.rs"]
mod layout;
#[path = "../src/morton.rs"]
mod morton;
#[path = "../src/node.rs"]
mod node;
#[path = "../src/pool.rs"]
mod pool;
#[path = "../src/rc.rs"]
mod rc;
#[path = "../src/roots.rs"]
mod roots;
#[path = "../src/proof/state.rs"]
mod state;
#[path = "../src/wide.rs"]
mod wide;
#[path = "../src/write.rs"]
mod write;

pub use error::Error;
pub use hashdag::HashDag;
pub use morton::ZOrder20x3;
pub use node::{Child, NodeIdx};
pub use roots::RootId;
use vstd::prelude::*;

verus! {
fn actual_difference(dag: &HashDag, before: RootId, after: RootId)
    requires dag.valid(),
{
    let mut difference = diff::NodeDiff::new();
    if let Ok(addresses) = difference.between(dag, before, after) {
        assert(diff::proof::exact(dag@, before, after, addresses@));
        assert(diff::proof::sorted(addresses@));
    }
}

fn exact_difference<G: diff::DiffGraph>(graph: &G, before: RootId, after: RootId)
    requires diff::proof::model::valid(graph@),
{
    let mut difference = diff::NodeDiff::new();
    let result = difference.between_graph(graph, before, after);
    if let Ok(addresses) = result {
        assert(diff::proof::exact(graph@, before, after, addresses@));
        assert(diff::proof::sorted(addresses@));
    }
}

#[cfg(reject_diff_old_reuse)]
fn reject_diff_old_reuse<G: diff::DiffGraph>(graph: &G, before: RootId, after: RootId, shared: diff::NodeAddress)
    requires diff::proof::model::valid(graph@),
        graph@.roots.lookup(before) is Some, graph@.roots.lookup(after) is Some,
        diff::proof::model::reachable(graph@, graph@.roots.lookup(before)->Some_0, shared),
        diff::proof::model::reachable(graph@, graph@.roots.lookup(after)->Some_0, shared),
{
    let mut difference = diff::NodeDiff::new();
    if let Ok(addresses) = difference.between_graph(graph, before, after) {
        assert(addresses@.contains(shared));
    }
}

#[cfg(reject_diff_missing_new)]
fn reject_diff_missing_new<G: diff::DiffGraph>(graph: &G, before: RootId, after: RootId, added: diff::NodeAddress)
    requires diff::proof::model::valid(graph@),
        graph@.roots.lookup(before) is Some, graph@.roots.lookup(after) is Some,
        !diff::proof::model::reachable(graph@, graph@.roots.lookup(before)->Some_0, added),
        diff::proof::model::reachable(graph@, graph@.roots.lookup(after)->Some_0, added),
{
    let mut difference = diff::NodeDiff::new();
    if let Ok(addresses) = difference.between_graph(graph, before, after) {
        assert(!addresses@.contains(added));
    }
}

#[cfg(reject_diff_error_output)]
fn reject_diff_error_output<G: diff::DiffGraph>(graph: &G, before: RootId, after: RootId)
    requires diff::proof::model::valid(graph@),
{
    let mut difference = diff::NodeDiff::new();
    let result = difference.between_graph(graph, before, after);
    if result.is_err() {
        let addresses = difference.nodes();
        assert(addresses@.len() > 0);
    }
}
fn main() {}
}
