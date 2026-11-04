//! 装配生产索引边界，负例不得越过切片或整数域。
#![allow(dead_code)]

#[path = "../src/index.rs"]
mod index;

use vstd::prelude::*;

verus! {

#[cfg(reject_index_out_of_bounds)]
fn reject_index_out_of_bounds(values: &[u8]) {
    let _ = index::get(values, values.len());
}

#[cfg(reject_index_narrowing)]
fn reject_index_narrowing() {
    let _ = index::u16_index(65_536);
}

fn main() {}

}
