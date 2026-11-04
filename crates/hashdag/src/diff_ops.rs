//! 差分使用的标准切片原语；地址键与集合完整性由调用者证明。
#[cfg(verus_only)]
use vstd::prelude::*;

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(
    ensures final(words)@.len() == old(words)@.len(),
        forall|i: int| 0 <= i < final(words)@.len() ==> #[trigger] final(words)@[i] == 0,
    no_unwind
))]
pub fn clear(words: &mut [u64]) {
    words.fill(0);
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(
    requires crate::sort_model::ready(old(items)@, key),
    ensures final(items)@.to_multiset() == old(items)@.to_multiset(),
        crate::sort_model::ordered(final(items)@, key),
))]
pub fn sort<T: Copy, F: Fn(&T) -> (u8, u32)>(items: &mut [T], key: F) {
    items.sort_unstable_by_key(key);
}
