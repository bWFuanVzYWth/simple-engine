//! 可信标准库适配；只声明实际使用的原语，不包含 SipHash round 或 hash。
//!
//! Verus 尚无这些 std 规范。Simd 不透明类型及 trait 绑定不声称内部布局。
//! 运算契约依据固定 Rust 1.99.1 portable-simd 的 vector.rs、ops.rs、uint.rs，
//! 外部标准库和编译器实现不由此证明。加法按模 2^64；移位仅规范合法移位量。
//! 此库单独编译导出；生产同源证明以 --no-cheating 导入，脚本单列可信编译。
//! 标准库来源：
//! [SIMD 运算](https://github.com/rust-lang/rust/blob/1.99.1/library/portable-simd/crates/core_simd/src/ops.rs)、
//! [向量与数组](https://github.com/rust-lang/rust/blob/1.99.1/library/portable-simd/crates/core_simd/src/vector.rs)、
//! [整数转换](https://github.com/rust-lang/rust/blob/1.99.1/library/portable-simd/crates/core_simd/src/simd/num/uint.rs)、
//! [只读分块](https://github.com/rust-lang/rust/blob/1.99.1/library/core/src/slice/mod.rs)、
//! [整数旋转](https://github.com/rust-lang/rust/blob/1.99.1/library/core/src/num/uint_macros.rs)。
#![feature(portable_simd)]

use std::simd::{Simd, SimdCast, SimdElement, num::SimdUint, u32x8, u64x8};
use vstd::prelude::*;

#[path = "../src/ops.rs"]
pub mod ops;

verus! {

#[verifier::external_trait_specification]
pub unsafe trait ExSimdElement: Copy {
    type ExternalTraitSpecificationFor: SimdElement;
}

#[verifier::external_trait_specification]
pub trait ExSimdCast: SimdElement {
    type ExternalTraitSpecificationFor: SimdCast;
}

#[verifier::external_type_specification]
#[verifier::external_body]
#[verifier::reject_recursive_types(T)]
pub struct ExSimd<T: SimdElement, const N: usize>(Simd<T, N>);

pub uninterp spec fn lanes<T: SimdElement, const N: usize>(x: Simd<T, N>) -> Seq<T>;

pub open spec fn view32(x: u32x8) -> Seq<u32> { lanes(x) }
pub open spec fn view64(x: u64x8) -> Seq<u64> { lanes(x) }

#[verifier::external_body]
pub broadcast proof fn shape<T: SimdElement, const N: usize>(x: Simd<T, N>)
    ensures #[trigger] lanes(x).len() == N,
{}

pub assume_specification<T: SimdElement, const N: usize>[Simd::<T, N>::splat](x: T) -> (r: Simd<T, N>)
    ensures lanes(r) == Seq::new(N as nat, |i: int| x)
    no_unwind;

pub assume_specification<T: SimdElement, const N: usize>[Simd::<T, N>::from_array](x: [T; N]) -> (r: Simd<T, N>)
    ensures lanes(r) == x@
    no_unwind;

pub assume_specification<T: SimdElement, const N: usize>[Simd::<T, N>::to_array](x: Simd<T, N>) -> (r: [T; N])
    ensures r@ == lanes(x)
    no_unwind;

pub assume_specification[u64::rotate_left](x: u64, n: u32) -> (r: u64)
    ensures r == (x << (n % 64)) | (x >> ((64 - n % 64) % 64))
    no_unwind;

pub assume_specification<T, const N: usize>[<[T]>::as_chunks::<N>](x: &[T]) -> (r: (&[[T; N]], &[T]))
    requires N > 0,
    ensures r.0@.len() == x@.len() / (N as nat),
        r.1@ == x@.subrange((x@.len() / (N as nat) * N) as int, x@.len() as int),
        forall|i: int, j: int| 0 <= i < r.0@.len() && 0 <= j < N ==>
            #[trigger] r.0@[i]@[j] == x@[i * N + j]
    no_unwind;

pub uninterp spec fn cast32<T: SimdCast>(x: u32) -> T;

pub assume_specification<const N: usize, T: SimdCast>[<Simd<u32, N> as SimdUint>::cast::<T>](x: Simd<u32, N>) -> (r: <Simd<u32, N> as SimdUint>::Cast<T>)
    ensures lanes(r) == Seq::new(N as nat, |i: int| cast32::<T>(lanes(x)[i]))
    no_unwind;

#[verifier::external_body]
pub broadcast proof fn widen32(x: u32)
    ensures #[trigger] cast32::<u64>(x) == x as u64,
{}

pub open spec fn valid_shifts<const N: usize>(x: Simd<u64, N>) -> bool {
    forall|i: int| 0 <= i < N ==> #[trigger] lanes(x)[i] < 64
}

pub broadcast group primitives {
    shape,
    widen32
}

}
