//! 真实节点字段、编码与 SIMD 折叠的同源验证装配。
#![feature(portable_simd, proc_macro_hygiene)]
#![allow(dead_code)]

#[path = "../src/error.rs"]
mod error;
#[path = "../src/morton.rs"]
mod morton;
#[path = "../src/node.rs"]
mod node;
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
fn encoding_contract(child: Child)
    no_unwind
{
    let encoded = child.encode();
    let decoded = encoded.decode();
    proof { node::proof::encoding_round_trip(child); }
    assert(decoded == child);
}
fn slots_contract(words: [u32; 64], mask: u64, slot: u8, child: Child)
    no_unwind
{
    let mut entry = node::PoolEntry::new(&words, mask);
    assert(node::proof::values(entry.values) == words@);
    assert(node::proof::mask(entry.mask) == mask);
    let ghost before = node::proof::children(entry);
    entry.set_child(slot, child);
    assert(node::proof::children(entry) == if slot < 64 { before.update(slot as int, child) } else { before });
}
fn comparison_contract(a: [u32; 64], am: u64, b: [u32; 64], bm: u64)
    no_unwind
{
    let first = node::PoolEntry::new(&a, am);
    let second = node::PoolEntry::new(&b, bm);
    let equal = first.as_ref().same(&second);
    assert(equal <==> node::proof::children(first) == node::proof::children(second));
    assert(equal <==> a@ == b@ && am == bm) by {
        if equal {
            assert(a@ == b@);
            assert(node::proof::mask_words(first.mask) == node::proof::mask_words(second.mask));
        }
    }
}
fn pointer_contract(words: [u32; 64], mask: u64)
    no_unwind
{
    let entry = node::PoolEntry::new(&words, mask);
    let mut edges = entry.pointers();
    let ghost original = node::proof::remaining(edges);
    assert(original.len() <= 64);
    let first = edges.next();
    assert(first is None <==> original.len() == 0);
    if let Some(index) = first {
        assert(index == original[0]);
        assert(node::proof::remaining(edges) == original.drop_first());
    }
    proof { node::proof::pointer_sequence_matches(words@, mask); }
    assert(original == node::proof::pointer_slots(words@, mask, 0));
}
fn fold_contract(words: [u32; 64], mask: u64)
    no_unwind
{
    let entry = node::PoolEntry::new(&words, mask);
    let folded = entry.uniform_value();
    assert(folded is Some <==> node::proof::uniform(node::proof::children(entry)));
}
fn morton_contract(x: u32, y: u32, z: u32, raw: u64)
    no_unwind
{
    let encoded = ZOrder20x3::encode(x, y, z);
    assert(encoded is Some <==> x < 1_048_576 && y < 1_048_576 && z < 1_048_576);
    if let Some(path) = encoded {
        let point = path.decode();
        assert(point == (x, y, z));
        let code = path.value();
        assert(morton::proof::bit_encoding(code, x, y, z));
    }
    if let Some(path) = ZOrder20x3::from_value(raw) {
        let point = path.decode();
        let encoded = ZOrder20x3::encode(point.0, point.1, point.2);
        assert(encoded is Some);
        if let Some(back) = encoded {
            assert(morton::proof::value(back) == raw);
        }
    }
}

#[cfg(reject_node_mask)]
fn reject_node_mask() {
    let words = hash_dag_std::simd_ops::array_fill(0);
    let leaf = node::PoolEntry::new(&words, 0);
    let pointer = node::PoolEntry::new(&words, 1);
    let same = leaf.as_ref().same(&pointer);
    assert(same);
}
#[cfg(reject_node_fold)]
fn reject_node_fold() {
    let words = hash_dag_std::simd_ops::array_fill(7);
    let pointer = node::PoolEntry::new(&words, 1);
    let folded = pointer.uniform_value();
    assert(folded == Some(7));
}
#[cfg(reject_pointer_dedup)]
fn reject_pointer_dedup() {
    let words = hash_dag_std::simd_ops::array_fill(7);
    let entry = node::PoolEntry::new(&words, 3);
    let mut pointers = entry.pointers();
    let first = pointers.next();
    let second = pointers.next();
    assert(first != second);
}
#[cfg(reject_morton_axis)]
fn reject_morton_axis() {
    let encoded = ZOrder20x3::encode(1, 0, 0).unwrap();
    let decoded = encoded.decode();
    assert(decoded == (0, 1, 0));
}
fn main() {}
}
