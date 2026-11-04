//! 在真实存储视图上检查规范化表示的语义唯一性。
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
proof fn semantic_identity(pools: state::Pools, a: Child, b: Child, height: nat)
    requires state::valid_pools(pools), state::child_valid(pools, a, height), state::child_valid(pools, b, height), height <= 10,
{
    canonical::equivalent_iff_identical(pools, a, b, height);
    assert(state::same_value(pools, a, b, height) <==> a == b);
}

#[cfg(reject_semantic_identity)]
proof fn reject_semantic_identity(pools: state::Pools)
    requires state::valid_pools(pools),
{
    canonical::equivalent_iff_identical(pools, Child::Leaf(3), Child::Leaf(5), 10);
    assert(state::same_value(pools, Child::Leaf(3), Child::Leaf(5), 10));
}
}
fn main() {}
