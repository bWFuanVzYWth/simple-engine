//! 闲置顺序与预算核的组合性质；不假定缓存条目就是 GPU allocation 身份。

#[cfg(verus_only)]
use super::{fits, give, oldest, take};
#[cfg(verus_only)]
use std::collections::VecDeque;
use vstd::prelude::*;

verus! {

pub(crate) open spec fn taken<T>(before: Seq<T>, after: Seq<T>, index: usize, entry: Option<T>) -> bool {
    match entry {
        Some(value) => {
            &&& index < before.len()
            &&& value == before[index as int]
            &&& after == before.remove(index as int)
        },
        None => after == before && before.len() <= index,
    }
}

pub(crate) open spec fn oldest_taken<T>(before: Seq<T>, after: Seq<T>, entry: Option<T>) -> bool {
    match entry {
        Some(value) => {
            &&& before.len() > 0
            &&& value == before[0]
            &&& after == before[1..]
        },
        None => after == before && before.len() == 0,
    }
}

pub(crate) open spec fn budget_fits(current: u64, incoming: u64, limit: u64) -> bool {
    current as nat + incoming as nat <= limit as nat
}

proof fn budget_addition_is_bounded(current: u64, incoming: u64, limit: u64)
    requires fits(current, incoming, limit),
    ensures current as nat + incoming as nat <= limit as nat,
{
}

#[cfg(verus_only)]
fn removing_middle_preserves_oldest(a: u64, b: u64, c: u64) {
    let mut idle = VecDeque::new();
    give(&mut idle, a);
    give(&mut idle, b);
    give(&mut idle, c);
    let borrowed = take(&mut idle, 1);
    match borrowed {
        Some(value) => give(&mut idle, value),
        None => { assert(false); },
    }
    assert(idle@ == seq![a, c, b]);
    let first = oldest(&mut idle);
    assert(first == Some(a));
}

}
