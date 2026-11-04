//! 十层独占池的初始化；不分配节点或发布指针。
use crate::{layout::DAG_DEPTH, pool::Pool};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "proof/layers.rs"]
pub(crate) mod proof;

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures crate::state::well_formed(&result),
        forall|level: int| 0 <= level < 10 ==> #[trigger] result@[level].state().nodes.len() == 0
            && result@[level].state().retained.len() == 0 && !result@[level].state().constructing,
))]
pub(crate) fn empty() -> [Pool; DAG_DEPTH as usize] {
    [
        Pool::new(),
        Pool::new(),
        Pool::new(),
        Pool::new(),
        Pool::new(),
        Pool::new(),
        Pool::new(),
        Pool::new(),
        Pool::new(),
        Pool::new(),
    ]
}

/// 只切换构造阶段标记，保持节点、计数和所有保留记录。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only,verus_spec(
    requires crate::state::well_formed(old(pools)),
    ensures crate::state::well_formed(final(pools)),
        proof::phase(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools)),ACTIVE),
        crate::state::all_occupied_live(crate::state::pools_view(old(pools)))
            <==>crate::state::all_occupied_live(crate::state::pools_view(final(pools))),
        crate::state::frozen(crate::state::pools_view(old(pools)),crate::state::pools_view(final(pools))),
        crate::state::frozen(crate::state::pools_view(final(pools)),crate::state::pools_view(old(pools))),
    no_unwind
))]
pub(crate) fn set_phase<const ACTIVE: bool>(pools: &mut [Pool; DAG_DEPTH as usize]) {
    #[cfg(verus_only)]
    proof_decl! {let ghost before=crate::state::pools_view(pools);}
    let mut level = 0;
    #[cfg_attr(verus_only,verus_spec(
        invariant level<=10,before==crate::state::pools_view(old(pools)),crate::state::valid_pools(before),
            forall|l:int|0<=l<10 ==> #[trigger] pools@[l].valid(),
            proof::prefix(before,crate::state::pools_view(pools),level as nat,ACTIVE),
        decreases 10-level,
    ))]
    while level < DAG_DEPTH as usize {
        #[cfg(verus_only)]
        proof_decl! {let ghost previous=crate::state::pools_view(pools);}
        if ACTIVE {
            pools[level].begin_write();
        } else {
            pools[level].end_write();
        }
        #[cfg(verus_only)]
        proof! {proof::next(before,previous,crate::state::pools_view(pools),level as nat,ACTIVE);}
        level += 1;
    }
    #[cfg(verus_only)]
    proof! {proof::valid_phase(before,crate::state::pools_view(pools),ACTIVE);}
}
