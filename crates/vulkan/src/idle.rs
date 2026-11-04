//! 闲置项从最旧到最新排列；删除保持其余相对顺序，不使用会回绕的时间戳。
//!
//! VecDeque 的任意删除移动较短一侧，最旧驱逐为 O(1)；搜索仍由分配池执行。

use std::collections::VecDeque;
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "proof/idle.rs"]
#[cfg(verus_only)]
mod proof;

/// 借出或替换一个闲置项，其余条目保持归还顺序。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(entry =>
    ensures proof::taken(old(idle)@, final(idle)@, index, entry),
))]
pub(crate) fn take<T>(idle: &mut VecDeque<T>, index: usize) -> Option<T> {
    idle.remove(index)
}

/// 把归还项放在最新端；调用者在此前准备容量并移交唯一所有权。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    ensures final(idle)@ == old(idle)@.push(entry),
))]
pub(crate) fn give<T>(idle: &mut VecDeque<T>, entry: T) {
    idle.push_back(entry);
}

/// 仅移出最旧闲置项；借出项不在此序列中。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(entry =>
    ensures proof::oldest_taken(old(idle)@, final(idle)@, entry),
))]
pub(crate) fn oldest<T>(idle: &mut VecDeque<T>) -> Option<T> {
    idle.pop_front()
}

/// 不先做可能溢出的加法，判断归还后总量是否在预算内。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::allow_in_spec)]
#[cfg_attr(verus_only, verus_spec(fits =>
    returns proof::budget_fits(current, incoming, limit),
))]
pub(crate) const fn fits(current: u64, incoming: u64, limit: u64) -> bool {
    incoming <= limit && current <= limit - incoming
}

#[cfg(test)]
mod tests {
    use super::{give, oldest, take};
    use std::collections::VecDeque;

    #[test]
    fn returned_entry_becomes_newest() {
        let mut idle = VecDeque::from([1, 2, 3]);
        let borrowed = take(&mut idle, 1).unwrap();
        give(&mut idle, borrowed);
        assert_eq!(idle, VecDeque::from([1, 3, 2]));
        assert_eq!(oldest(&mut idle), Some(1));
    }
}
