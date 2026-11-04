//! 装配生产 SipHash 模块，检查公开入口与独立算法规范的连接。

#![feature(portable_simd, proc_macro_hygiene)]
#![allow(dead_code)]

#[path = "../src/lib.rs"]
mod sip_hash;

pub use sip_hash::SipHash;
use sip_hash::scalar::proof;
use sip_hash_std::view64;
use std::simd::u32x8;
use vstd::prelude::*;

verus! {

fn byte_contract<const C: usize, const D: usize>(seed: (u64, u64), message: &[u8])
    no_unwind
{
    let hasher = SipHash::<C, D>::new(seed);
    let result = hasher.hash(message);
    assert(result == proof::hash(proof::initial(seed), message@, C as nat, D as nat));
}

fn coordinate_contract<const C: usize, const D: usize>(seed: (u64, u64), a: u32, b: u32, c: u32)
    no_unwind
{
    let hasher = SipHash::<C, D>::new(seed);
    let result = hasher.hash_u32x3(a, b, c);
    assert(result == proof::coords(proof::initial(seed), a, b, c, C as nat, D as nat));
}

fn lane_contract<const C: usize, const D: usize>(seed: (u64, u64), a: [u32; 8], b: [u32; 8], c: [u32; 8])
    no_unwind
{
    let hasher = SipHash::<C, D>::new(seed);
    let result = hasher.hash_u32x3_x8(a, b, c);
    assert forall|lane: int| 0 <= lane < 8 implies
        result@[lane] == proof::coords(proof::initial(seed), a@[lane], b@[lane], c@[lane], C as nat, D as nat) by {}
}

fn vector_entry_contract<const C: usize, const D: usize>(seed: (u64, u64), a: [u32; 8], b: [u32; 8], c: [u32; 8])
    no_unwind
{
    let hasher = SipHash::<C, D>::new(seed);
    let result = hasher.hash_u32x3_simd(u32x8::from_array(a), u32x8::from_array(b), u32x8::from_array(c));
    assert forall|lane: int| 0 <= lane < 8 implies
        view64(result)[lane] == proof::coords(proof::initial(seed), a@[lane], b@[lane], c@[lane], C as nat, D as nat) by {}
}

#[cfg(reject_seed_mapping)]
fn reject_seed_mapping(seed: (u64, u64)) {
    let hasher = SipHash::<1, 3>::new(seed);
    assert(hasher.state() == proof::initial((seed.1, seed.0)));
}

#[cfg(reject_coordinate_order)]
fn reject_coordinate_order(seed: (u64, u64), a: u32, b: u32, c: u32) {
    let hasher = SipHash::<1, 3>::new(seed);
    let result = hasher.hash_u32x3(a, b, c);
    assert(result == proof::coords(proof::initial(seed), c, b, a, 1, 3));
}

#[cfg(reject_missing_tail)]
fn reject_missing_tail(seed: (u64, u64)) {
    let hasher = SipHash::<1, 3>::new(seed);
    let result = hasher.hash(&[]);
    assert(result == proof::finish(proof::initial(seed), 3));
}

#[cfg(reject_lane_reorder)]
fn reject_lane_reorder(seed: (u64, u64), a: [u32; 8], b: [u32; 8], c: [u32; 8]) {
    let hasher = SipHash::<1, 3>::new(seed);
    let result = hasher.hash_u32x3_x8(a, b, c);
    assert(result[0] == proof::coords(proof::initial(seed), a@[1], b@[1], c@[1], 1, 3));
}

#[cfg(reject_zero_round_output)]
fn reject_zero_round_output() {
    let hasher = SipHash::<0, 0>::new((0, 0));
    let result = hasher.hash_u32x3(0, 0, 0);
    assert(result != 0x0f1c1c0309061df5);
}

#[cfg(reject_unwind_contract)]
fn unspecified_unwind() {}

#[cfg(reject_unwind_contract)]
fn reject_unwind_contract()
    no_unwind
{
    unspecified_unwind();
}

}

fn main() {}
