//! 装配 HK 实际调用的初始化与未匹配编号过滤核，不装配增广搜索。
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]

#[path = "../src/optimal/fixed.rs"]
pub(crate) mod fixed;
#[path = "../src/index.rs"]
mod index;
mod optimal {
    pub(crate) use crate::fixed;
    pub(crate) use crate::index::*;
}
#[path = "../src/optimal/search.rs"]
mod search;

use vstd::prelude::*;

verus! {
fn main() {
    let pairs = [u16::MAX, 0, u16::MAX];
    let mut distance = fixed::FixedVec::<u16, 4>::new(0);
    let mut left = fixed::FixedVec::<bool, 4>::new(false);
    let mut right = fixed::FixedVec::<bool, 4>::new(false);
    let mut unmatched = fixed::FixedVec::<u16, 4>::new(0);
    search::initialize(&pairs, 2, &mut distance, &mut left, &mut right, &mut unmatched);
    assert(distance@ == seq![u16::MAX, u16::MAX]);
    assert(left@ == seq![false, false, false]);
    reveal_with_fuel(search::free_prefix, 4);
    assert(unmatched@ == seq![0u16, 2u16]);
    #[cfg(reject_search_distance)]
    assert(distance@ == seq![0u16, 0u16]);
    #[cfg(reject_search_missing_free)]
    assert(unmatched@ == seq![0u16]);
    #[cfg(reject_search_capacity)]
    search::initialize(&pairs, 5, &mut distance, &mut left, &mut right, &mut unmatched);
    unmatched.clear();
    unmatched.extend_from_slice(&[2, 1, 0, 2]);
    search::remove_matched(&pairs, &mut unmatched);
    reveal_with_fuel(Seq::filter, 5);
    assert(unmatched@ == seq![2u16, 0u16, 2u16]);
    #[cfg(reject_search_filtered_loss)]
    assert(unmatched@ == seq![2u16, 0u16]);
    #[cfg(reject_search_filter_order)]
    assert(unmatched@ == seq![0u16, 2u16, 2u16]);
    #[cfg(reject_search_invalid_id)]
    {
        unmatched.clear();
        unmatched.push(3);
        search::remove_matched(&pairs, &mut unmatched);
    }
}
}
