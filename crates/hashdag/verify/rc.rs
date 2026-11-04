//! 真实 Pool 与显式栈级联回收；根持有由整数余量参数化。
#![feature(portable_simd, proc_macro_hygiene)]
#![allow(dead_code)]
#[path = "../src/proof/balance.rs"]
mod balance;
#[path = "../src/proof/canonical.rs"]
mod canonical;
#[path = "../src/error.rs"]
mod error;
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
#[path = "../src/proof/state.rs"]
mod state;
#[path = "../src/wide.rs"]
mod wide;
mod hashdag {
    pub use crate::{
        error::Error,
        layout::{DAG_DEPTH, NODE_FANOUT, ROOT_LOD},
    };
}
pub use error::Error;
pub use morton::ZOrder20x3;
pub use node::{Child, NodeIdx};
use vstd::prelude::*;

verus! {
fn release_contract(pools: &mut [pool::Pool; 10], root: NodeIdx, level: u8, scratch: &mut Vec<(NodeIdx, u8)>)
    requires rc::proof::eligible(old(pools), root, level),
        hash_dag_std::vec_model::capacity(*old(scratch)) >= rc::proof::capacity(),
{
    let ghost before = state::pools_view(pools);
    let result = rc::rc_dec_or_remove(pools, root, level, scratch);
    assert(result is Ok);
    assert(rc::proof::release_step(before, state::pools_view(pools), root, level));
    assert(scratch@.len() == 0);
}

#[cfg(reject_rc_no_decrement)]
fn reject_rc_no_decrement(pools: &mut [pool::Pool; 10], root: NodeIdx, level: u8, scratch: &mut Vec<(NodeIdx, u8)>)
    requires rc::proof::eligible(old(pools), root, level),
        hash_dag_std::vec_model::capacity(*old(scratch)) >= rc::proof::capacity(),
{
    let ghost before = state::pools_view(pools);
    let result = rc::rc_dec_or_remove(pools, root, level, scratch);
    assert(result is Ok);
    assert(state::net_credit(state::pools_view(pools), level as int, node::proof::idx(root))
        == state::net_credit(before, level as int, node::proof::idx(root)));
}

#[cfg(reject_rc_edge_set)]
proof fn reject_rc_edge_set(entry: node::PoolEntry, index: NodeIdx)
    requires state::occurrences(node::proof::children(entry), Child::Ptr(index)) > 1,
{
    pool::proof::pointer_occurrences(entry, index);
    assert(state::occurrences(pool::proof::entry_edges(entry), index) == 1);
}

#[cfg(reject_rc_shared_drop)]
fn reject_rc_shared_drop(pools: &mut [pool::Pool; 10], root: NodeIdx, level: u8,
    other: NodeIdx, other_level: u8, scratch: &mut Vec<(NodeIdx, u8)>)
    requires rc::proof::eligible(old(pools), root, level), other_level < 10,
        other != root || other_level != level,
        state::net_credit(state::pools_view(old(pools)), other_level as int, node::proof::idx(other)) > 0,
        hash_dag_std::vec_model::capacity(*old(scratch)) >= rc::proof::capacity(),
{
    let result = rc::rc_dec_or_remove(pools, root, level, scratch);
    assert(result is Ok);
    assert(!state::live(state::pools_view(pools)[other_level as int], node::proof::idx(other)));
}
fn main() {}
}
