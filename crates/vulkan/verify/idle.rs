//! 装配生产闲置顺序与预算核及其证明；不分配 Vulkan 资源。

#![allow(dead_code)]

#[path = "../src/idle.rs"]
mod idle;

use idle::{fits, give, take};
use std::collections::VecDeque;
use vstd::prelude::*;

verus! {

#[cfg(reject_idle_reorder)]
fn reject_idle_reorder(a: u64, b: u64, c: u64)
    requires b != c,
{
    let mut idle = VecDeque::new();
    give(&mut idle, a);
    give(&mut idle, b);
    give(&mut idle, c);
    let _ = take(&mut idle, 0);
    assert(idle@ == seq![c, b]);
}

#[cfg(reject_idle_overflow)]
proof fn reject_idle_overflow() {
    assert(fits(u64::MAX, 1, u64::MAX));
}

}

fn main() {}
