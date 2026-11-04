//! 标准 SIMD 原语的最小同源适配；值语义与无 unwind 由标准库契约保证。
//!
//! native 与可信验证库编译同一函数体。Verus 的外部运算 trait 不允许具体实现
//! 单独加强 `no_unwind`，因此用单一标准运算的普通函数隔离该工具限制。

use std::simd::u64x8;
#[cfg(verus_only)]
use vstd::prelude::*;

/// 逐 lane 模 `2^64` 相加，不混合 lane。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures crate::view64(result) == Seq::new(8, |i: int|
        crate::view64(a)[i].wrapping_add(crate::view64(b)[i])),
    no_unwind
))]
pub fn add(a: u64x8, b: u64x8) -> u64x8 {
    a + b
}

/// 逐 lane XOR，不混合 lane。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures crate::view64(result) == Seq::new(8, |i: int|
        crate::view64(a)[i] ^ crate::view64(b)[i]),
    no_unwind
))]
pub fn xor(a: u64x8, b: u64x8) -> u64x8 {
    a ^ b
}

/// 逐 lane OR，不混合 lane。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures crate::view64(result) == Seq::new(8, |i: int|
        crate::view64(a)[i] | crate::view64(b)[i]),
    no_unwind
))]
pub fn or(a: u64x8, b: u64x8) -> u64x8 {
    a | b
}

/// 逐 lane 左移；内部调用须保证每个移位量小于 64。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires crate::valid_shifts(b),
    ensures crate::view64(result) == Seq::new(8, |i: int|
        crate::view64(a)[i] << crate::view64(b)[i]),
    no_unwind
))]
pub fn shl(a: u64x8, b: u64x8) -> u64x8 {
    a << b
}

/// 逐 lane 逻辑右移；内部调用须保证每个移位量小于 64。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires crate::valid_shifts(b),
    ensures crate::view64(result) == Seq::new(8, |i: int|
        crate::view64(a)[i] >> crate::view64(b)[i]),
    no_unwind
))]
pub fn shr(a: u64x8, b: u64x8) -> u64x8 {
    a >> b
}
