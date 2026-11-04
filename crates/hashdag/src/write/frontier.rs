//! 有序写入的活动祖先帧；帧内 Child 只借用节点，不认领结构引用。

use crate::ZOrder20x3;
use crate::node::{Child, PoolEntry};
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "../proof/write_frontier.rs"]
#[cfg(verus_only)]
pub(super) mod proof;

/// 只保留尚未结束的空间前缀，离开子树时比较最终内容并提交。
#[cfg_attr(verus_only, verus_verify)]
pub struct BuildFrame {
    pub entry: PoolEntry,
    pub original: Child,
    pub path: ZOrder20x3,
    pub any_modified: bool,
    pub pool_lod: u8,
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires frame.pool_lod < 10,
    ensures result == proof::contains(*frame, path), no_unwind
))]
pub(super) const fn contains(frame: &BuildFrame, path: ZOrder20x3) -> bool {
    let shift = (frame.pool_lod as u32 + 1) * 6;
    path.value() >> shift == frame.path.value() >> shift
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires old(frame).pool_lod < 10,
    ensures final(frame).pool_lod == old(frame).pool_lod,
        final(frame).path == old(frame).path, final(frame).original == old(frame).original,
        crate::node::proof::children(final(frame).entry) == crate::node::proof::children(old(frame).entry)
            .update(proof::slot(path, old(frame).pool_lod), child),
        final(frame).any_modified == (old(frame).any_modified
            || crate::node::proof::children(old(frame).entry)[proof::slot(path, old(frame).pool_lod)] != child),
    no_unwind
))]
pub(super) fn replace_child(frame: &mut BuildFrame, path: ZOrder20x3, child: Child) {
    #[cfg(verus_only)]
    proof! { proof::slot_matches(path, frame.pool_lod); }
    if !frame.entry.child_at(path, frame.pool_lod).same(child) {
        frame.entry.set_child_at(path, frame.pool_lod, child);
        frame.any_modified = true;
    } else {
        #[cfg(verus_only)]
        proof! { proof::same_slot(crate::node::proof::children(frame.entry), proof::slot(path, frame.pool_lod), child); }
    }
}
