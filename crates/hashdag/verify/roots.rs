//! 装配生产根表；实例身份原子分配与真实节点引用连接由 HashDag 入口另验。
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]
#![feature(portable_simd)]

#[path = "../src/error.rs"]
mod error;
#[path = "../src/morton.rs"]
mod morton;
#[path = "../src/node.rs"]
mod node;
#[path = "../src/roots.rs"]
mod roots;
#[path = "../src/wide.rs"]
mod wide;

mod hashdag {
    pub use crate::error::Error;
    use vstd::prelude::*;
    #[cfg_attr(verus_only, vstd::prelude::verus_verify)]
    pub const NODE_FANOUT: usize = 64;
}

pub use error::Error;
pub use morton::ZOrder20x3;
pub use node::{Child, NodeIdx};
use vstd::prelude::*;

verus! {

fn identity_roundtrip(owner: usize, child: Child) {
    let mut roots = match roots::RootSlots::new(owner) {
        Ok(roots) => roots,
        Err(_) => return,
    };
    let default = roots.default_root();
    let original = roots.child(default);
    assert(original is Ok && original->Ok_0 == Child::Leaf(0));
    if roots.prepare().is_err() { return; }
    let snapshot = roots.insert(child);
    assert(roots.lookup(default) == Some(Child::Leaf(0)));
    let found = roots.child(snapshot);
    assert(found is Ok && found->Ok_0 == child);
    let (_, generation) = match roots.prepare_release(snapshot) {
        Ok(parts) => parts,
        Err(_) => return,
    };
    roots.retire(snapshot, generation);
    assert(roots.revoked(snapshot));
    assert(roots.lookup(default) == Some(Child::Leaf(0)));
    let stale = roots.child(snapshot);
    assert(stale is Err);
    if roots.prepare().is_err() { return; }
    assert(roots.revoked(snapshot));
    assert(roots.lookup(default) == Some(Child::Leaf(0)));
    let reused = roots.insert(child);
    proof { roots::proof::revoked_absent(&roots, snapshot); }
    assert(roots.lookup(default) == Some(Child::Leaf(0)));
    let old = roots.child(snapshot);
    let live = roots.child(reused);
    assert(old is Err && live is Ok && live->Ok_0 == child);
    assert(snapshot != reused);
    let unchanged = roots.child(default);
    assert(unchanged is Ok && unchanged->Ok_0 == Child::Leaf(0));
    #[cfg(reject_stale_root)]
    assert(old is Ok);
}

fn foreign_rejected(roots: &roots::RootSlots, root: roots::RootId)
    requires root.owner != roots.owner,
{
    let result = roots.child(root);
    assert(result is Err && result->Err_0 is WrongOwner);
    #[cfg(reject_foreign_root)]
    assert(result is Ok);
}

fn saturation_preserves(roots: &roots::RootSlots, root: roots::RootId)
    requires roots.valid(), roots.lookup(root) is Some,
        root.slot != 0, root.generation == u64::MAX,
{
    let before = roots.child(root);
    let result = roots.prepare_release(root);
    let after = roots.child(root);
    assert(result is Err && result->Err_0 is RootGenerationExhausted);
    assert(before == after);
    #[cfg(reject_root_generation_wrap)]
    assert(result is Ok);
}

fn main() {}

}
