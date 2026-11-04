//! 装配真实 FixedVec 与索引辅助，不导入几何或平台后端。
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]

#[path = "../src/index.rs"]
mod index;
mod optimal {
    pub(crate) use crate::index::*;
}
#[path = "../src/optimal/fixed.rs"]
mod fixed;

use vstd::prelude::*;

verus! {
fn main() {
    let mut values = fixed::FixedVec::<u16, 4>::new(0);
    values.push(3);
    values.extend_from_slice(&[5, 7]);
    assert(values@ == seq![3u16, 5u16, 7u16]);
    let last = values.pop();
    assert(last == Some(7u16));
    values.resize(4, 9);
    assert(values@ == seq![3u16, 5u16, 9u16, 9u16]);
    #[cfg(reject_fixed_full_push)]
    values.push(11);
    values.clear();
    assert(values@ == Seq::<u16>::empty());
    #[cfg(reject_fixed_clear_contents)]
    assert(values@.len() == 4);
    values.extend_from_slice(&[2, 3, 4]);
    let snapshot = values.clone();
    assert(snapshot@ == values@);
    values.truncate(2);
    assert(values@ == seq![2u16, 3u16]);
    values.truncate(4);
    assert(values@ == seq![2u16, 3u16]);
    #[cfg(reject_fixed_truncate_growth)]
    assert(values@.len() == 4);
}
}
