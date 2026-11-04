//! 标准 SIMD、整数与数组原语的同源适配，供 native 和可信原语库共用。
use std::simd::{cmp::SimdPartialEq, u32x8};
#[cfg(verus_only)]
use vstd::prelude::*;

/// 逐 lane 比较完整 `u32`，不解释节点类型。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result <==> forall|i: int| 0 <= i < 8 ==>
        #[trigger] crate::view32(a)[i] == crate::view32(b)[i],
    no_unwind
))]
pub fn equal8(a: u32x8, b: u32x8) -> bool {
    a.simd_eq(b).all()
}

/// 标准整数原语；非零输入返回最低置位的槽号。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == vstd::std_specs::bits::u64_trailing_zeros(bits), no_unwind
))]
pub fn trailing_zeros(bits: u64) -> u32 {
    bits.trailing_zeros()
}

/// 内建数组重复，不分配；验证库补齐 vstd 缺少的无 unwind 保证。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result@ == Seq::new(N as nat, |i: int| value), no_unwind
))]
pub const fn array_fill<T: Copy, const N: usize>(value: T) -> [T; N] {
    [value; N]
}

/// 标准 `u32` 数组相等比较；覆盖全部元素，不受哈希碰撞影响。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result => ensures result <==> a@ == b@, no_unwind))]
pub fn equal_words<const N: usize>(a: &[u32; N], b: &[u32; N]) -> bool {
    a == b
}

/// 标准 Copy 数组范围填充；合法边界内逐项复制同一值。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(
    requires start<=end<=N,
    ensures final(values)@ == old(values)@.subrange(0,start as int)
        +Seq::new((end-start) as nat, |i:int| value)+old(values)@.subrange(end as int,N as int), no_unwind
))]
pub fn fill_range<T: Copy, const N: usize>(
    values: &mut [T; N],
    start: usize,
    end: usize,
    value: T,
) {
    values[start..end].fill_with(|| value);
}
