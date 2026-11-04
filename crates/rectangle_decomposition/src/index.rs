// 索引实现在 optimal 的原有代码生成边界内装配；验证入口直接复用本文件。

#[cfg(verus_only)]
use vstd::prelude::*;

/// Verus 将标准库 abort 映射为 panic，仅检查该分支不可达；生产仍直接 abort。
/// 此适配只信任标准库的终止行为，不假设任意调用方满足索引前提。
#[cold]
#[inline(never)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(requires false))]
pub(crate) fn invariant_failed() -> ! {
    #[cfg(not(verus_only))]
    std::process::abort();
    #[cfg(verus_only)]
    panic!("内部不变量失败");
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires index < values@.len(),
    ensures *result == values@[index as int],
))]
pub(crate) fn get<T>(values: &[T], index: usize) -> &T {
    debug_assert!(index < values.len());
    match values.get(index) {
        Some(value) => value,
        None => invariant_failed(),
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires index < old(values)@.len(),
    ensures *result == old(values)@[index as int],
        final(values)@ == old(values)@.update(index as int, *final(result)),
    no_unwind
))]
pub(crate) fn get_mut<T>(values: &mut [T], index: usize) -> &mut T {
    debug_assert!(index < values.len());
    if index >= values.len() {
        invariant_failed();
    }
    &mut values[index]
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires index < values@.len(),
    ensures result == values@[index as int],
))]
pub(crate) fn copy<T: Copy>(values: &[T], index: usize) -> T {
    *get(values, index)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires range.start <= range.end <= values@.len(),
    ensures result@ == values@.subrange(range.start as int, range.end as int),
))]
#[inline]
pub(crate) fn slice<T>(values: &[T], range: std::ops::Range<usize>) -> &[T] {
    debug_assert!(range.start <= range.end && range.end <= values.len());
    match values.get(range) {
        Some(value) => value,
        None => invariant_failed(),
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires range.start <= range.end <= old(values)@.len(),
    ensures result@ == old(values)@.subrange(range.start as int, range.end as int),
        final(result)@.len() == range.end - range.start,
        final(values)@ == old(values)@.subrange(0, range.start as int)
            + final(result)@ + old(values)@.subrange(range.end as int, old(values)@.len() as int),
))]
#[inline]
pub(crate) fn slice_mut<T>(values: &mut [T], range: std::ops::Range<usize>) -> &mut [T] {
    debug_assert!(range.start <= range.end && range.end <= values.len());
    match values.get_mut(range) {
        Some(value) => value,
        None => invariant_failed(),
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires index <= u16::MAX,
    ensures result as int == index as int,
))]
pub(crate) fn u16_index(index: usize) -> u16 {
    match u16::try_from(index) {
        Ok(value) => value,
        Err(_) => invariant_failed(),
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires index <= u32::MAX,
    ensures result as int == index as int,
))]
pub(crate) fn u32_index(index: usize) -> u32 {
    match u32::try_from(index) {
        Ok(value) => value,
        Err(_) => invariant_failed(),
    }
}
