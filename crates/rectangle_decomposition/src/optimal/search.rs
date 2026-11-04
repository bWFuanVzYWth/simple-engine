//! HK 搜索的初始化与未匹配编号稳定过滤；复用固定缓冲。
//! owner 字段分借用包装及后续增广搜索另行审查。

use crate::optimal::{copy, fixed::FixedVec, get_mut, u16_index};
#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "../proof/search.rs"]
pub(super) mod proof;
#[cfg(verus_only)]
pub(crate) use proof::free_prefix;

/// 输入左右顶点数量不超过输出容量，编号避开 `u16::MAX` sentinel。
/// 结果距离全部为 sentinel，访问标记全部清零；未匹配编号按原序各出现一次。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires pair_left@.len() <= N, right_size <= N, N <= u16::MAX,
        old(right_distance).valid(), old(reachable_left).valid(),
        old(reachable_right).valid(), old(unmatched_lefts).valid(),
    ensures final(right_distance).valid(), final(reachable_left).valid(),
        final(reachable_right).valid(), final(unmatched_lefts).valid(),
        final(right_distance)@ == Seq::new(right_size as nat, |_: int| u16::MAX),
        final(reachable_left)@ == Seq::new(pair_left@.len(), |_: int| false),
        final(reachable_right)@ == Seq::new(right_size as nat, |_: int| false),
        final(unmatched_lefts)@ == proof::free_prefix(pair_left@, pair_left@.len() as int),
        proof::enumerated(pair_left@, final(unmatched_lefts)@, pair_left@.len() as int),
))]
pub(crate) fn initialize<const N: usize>(
    pair_left: &[u16],
    right_size: usize,
    right_distance: &mut FixedVec<u16, N>,
    reachable_left: &mut FixedVec<bool, N>,
    reachable_right: &mut FixedVec<bool, N>,
    unmatched_lefts: &mut FixedVec<u16, N>,
) {
    right_distance.clear();
    right_distance.resize(right_size, u16::MAX);
    reachable_left.clear();
    reachable_left.resize(pair_left.len(), false);
    reachable_right.clear();
    reachable_right.resize(right_size, false);
    unmatched_lefts.clear();
    let mut left = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant left <= pair_left@.len() <= N, N <= u16::MAX,
            unmatched_lefts.valid(),
            unmatched_lefts@ == proof::free_prefix(pair_left@, left as int),
            unmatched_lefts@.len() <= left,
        decreases pair_left@.len() - left,
    ))]
    while left < pair_left.len() {
        if copy(pair_left, left) == u16::MAX {
            unmatched_lefts.push(u16_index(left));
        }
        proof! { proof::prefix_step(pair_left@, left as int); }
        left += 1;
    }
    proof! { proof::complete(pair_left@, pair_left@.len() as int); }
}

/// 原位移除已匹配左顶点，保留原有顺序和重复次数；只要求本次输入编号有效。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires old(unmatched).valid(), proof::valid_ids(pair_left@, old(unmatched)@),
    ensures final(unmatched).valid(), final(unmatched)@ == proof::unmatched_ids(pair_left@, old(unmatched)@),
))]
pub(crate) fn remove_matched<const N: usize>(pair_left: &[u16], unmatched: &mut FixedVec<u16, N>) {
    let len = remove_matched_ids(pair_left, unmatched.as_mut_slice());
    unmatched.truncate(len);
}

/// 压缩后的有效前缀长度；前缀之外的原有字节不变。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::valid_ids(pair_left@, old(ids)@),
    ensures result <= old(ids)@.len(), final(ids)@.len() == old(ids)@.len(),
        final(ids)@.take(result as int) == proof::unmatched_ids(pair_left@, old(ids)@),
        final(ids)@.skip(result as int) == old(ids)@.skip(result as int),
))]
fn remove_matched_ids(pair_left: &[u16], ids: &mut [u16]) -> usize {
    proof_decl! { let ghost original = ids@; }
    let mut read = 0;
    let mut write = 0;
    proof! { proof::filter_started(pair_left@, original); }
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::filter_loop(pair_left@, original, ids@, read as int, write as int),
        decreases original.len() - read,
    ))]
    while read < ids.len() {
        proof_decl! { let ghost before = ids@; let ghost previous_write = write; }
        let id = copy(ids, read);
        if copy(pair_left, usize::from(id)) == u16::MAX {
            *get_mut(ids, write) = id;
            write += 1;
        }
        proof! { proof::filter_step(pair_left@, original, before, ids@, read as int,
        previous_write as int, write as int); }
        read += 1;
    }
    proof! { proof::filter_done(pair_left@, original, ids@, write as int); }
    write
}
