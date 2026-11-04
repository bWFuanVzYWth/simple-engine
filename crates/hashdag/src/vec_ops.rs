//! 预留容量后的标准 Vec 操作；仅复制类型可用于无 unwind 的清理路径。
#[cfg(verus_only)]
use vstd::prelude::*;

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result => ensures result@.len() == 0, no_unwind))]
pub const fn new<T>() -> Vec<T> {
    Vec::new()
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == crate::vec_model::capacity(*values), result >= values@.len(),
    no_unwind
))]
pub fn capacity<T>(values: &Vec<T>) -> usize {
    values.capacity()
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures final(values)@ == old(values)@,
        crate::vec_model::capacity(*final(values)) >= crate::vec_model::capacity(*old(values)),
        result is Ok ==> crate::vec_model::capacity(*final(values)) >= old(values)@.len() + additional,
    no_unwind
))]
pub fn reserve<T>(
    values: &mut Vec<T>,
    additional: usize,
) -> Result<(), std::collections::TryReserveError> {
    values.try_reserve(additional)
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures final(values)@ == old(values)@,
        crate::vec_model::capacity(*final(values)) >= crate::vec_model::capacity(*old(values)),
        result is Ok ==> crate::vec_model::capacity(*final(values)) >= old(values)@.len() + additional,
    no_unwind
))]
pub fn reserve_exact<T>(
    values: &mut Vec<T>,
    additional: usize,
) -> Result<(), std::collections::TryReserveError> {
    values.try_reserve_exact(additional)
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(
    requires old(values)@.len() <= len <= crate::vec_model::capacity(*old(values)), factory.requires(()),
    ensures final(values)@.len() == len,
        final(values)@.take(old(values)@.len() as int) == old(values)@,
        forall|i: int| old(values)@.len() <= i < len ==> factory.ensures((), #[trigger] final(values)@[i]),
        crate::vec_model::capacity(*final(values)) == crate::vec_model::capacity(*old(values)),
))]
pub fn resize_with<T, F: Fn() -> T>(values: &mut Vec<T>, len: usize, factory: F) {
    values.resize_with(len, factory);
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(
    requires old(values)@.len() < crate::vec_model::capacity(*old(values)),
    ensures final(values)@ == old(values)@.push(value),
        crate::vec_model::capacity(*final(values)) == crate::vec_model::capacity(*old(values)),
    no_unwind
))]
pub fn push<T: Copy>(values: &mut Vec<T>, value: T) {
    values.push(value);
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == if old(values)@.len() == 0 { None } else { Some(old(values)@.last()) },
        final(values)@ == if old(values)@.len() == 0 { old(values)@ } else { old(values)@.drop_last() },
        crate::vec_model::capacity(*final(values)) == crate::vec_model::capacity(*old(values)),
    no_unwind
))]
pub fn pop<T: Copy>(values: &mut Vec<T>) -> Option<T> {
    values.pop()
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(
    ensures final(values)@.len() == 0,
        crate::vec_model::capacity(*final(values)) == crate::vec_model::capacity(*old(values)),
    no_unwind
))]
pub fn clear<T: Copy>(values: &mut Vec<T>) {
    values.clear();
}
