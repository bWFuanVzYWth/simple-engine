//! 实际 HashDag 所有者的初始化、根发布、快照与事务组合入口。
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
#[path = "../src/read.rs"]
mod read;
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
pub use node::{Child, NodeIdx, NodeMask, NodeValues};
pub use roots::RootId;
use vstd::prelude::*;
verus! {
fn initial_owner() {
    let dag = match HashDag::new() { Ok(dag) => dag, Err(_) => return };
    assert(dag.valid());
    let root = dag.root();
    let value = dag.root_child(root);
    assert(value is Ok && value->Ok_0 == Child::Leaf(0));
}

fn distinct_owners() {
    let mut first = match HashDag::new() { Ok(dag) => dag, Err(_) => return };
    let second = match HashDag::new() { Ok(dag) => dag, Err(_) => return };
    proof { first.owner_ticket.borrow_mut().disjoint(second.owner_ticket.borrow()); }
    assert(first.roots.owner != second.roots.owner);
    let foreign = second.root();
    let found = first.root_child(foreign);
    assert(found is Err && found->Err_0 is WrongOwner);
    #[cfg(reject_owner_number_reuse)]
    assert(first.roots.owner == second.roots.owner);
}

fn local_result_owner(values: &[u32]) {
    let mut tree = write::built::LocalTree::default();
    assert(tree.valid_local());
    let mut builder = write::built::LocalBuilder::new();
    let _ = builder.build(&mut tree, 2, values);
    assert(tree.valid_local());
}

fn snapshot_lifecycle(dag: &mut HashDag, source: RootId)
    requires old(dag).valid(), old(dag).roots.lookup(source) is Some,
    ensures final(dag).valid(), final(dag).roots.lookup(source) == old(dag).roots.lookup(source),
{
    let ghost child = dag.roots.lookup(source)->Some_0;
    let copy = match dag.fork_root(source) { Ok(copy) => copy, Err(_) => return };
    assert(copy != source);
    let value = dag.root_child(copy);
    assert(value == Ok(child));
    let released = dag.release_root(copy);
    let source_value = dag.root_child(source);
    assert(source_value == Ok(child));
    if let Ok(()) = released {
        let old_value = dag.root_child(copy);
        assert(old_value is Err);
    }
}

#[cfg(reject_owner_fork_alias)]
fn reject_owner_fork_alias(dag: &mut HashDag, source: RootId)
    requires old(dag).valid(), old(dag).roots.lookup(source) is Some,
{
    let copy = match dag.fork_root(source) { Ok(copy) => copy, Err(_) => return };
    assert(copy == source);
}

#[cfg(reject_owner_shared_release)]
fn reject_owner_shared_release(dag: &mut HashDag, source: RootId)
    requires old(dag).valid(), old(dag).roots.lookup(source) is Some,
{
    let copy = match dag.fork_root(source) { Ok(copy) => copy, Err(_) => return };
    if let Ok(()) = dag.release_root(copy) {
        assert(dag.roots.lookup(source) is None);
    }
}

#[cfg(reject_owner_stale_read)]
fn reject_owner_stale_read(dag: &mut HashDag, root: RootId)
    requires old(dag).valid(),
{
    if let Ok(()) = dag.release_root(root) {
        let value = dag.root_child(root);
        assert(value is Ok);
    }
}
}
fn main() {}
