//! 最小标准 SIMD 规范；不包含 uniform/fold 的实现或目标结论。
use std::simd::{Simd, SimdElement, u32x8};
use vstd::prelude::*;

#[path = "../src/simd.rs"]
pub mod simd_ops;

verus! {
#[verifier::external_trait_specification]
pub unsafe trait ExSimdElement: Copy {
    type ExternalTraitSpecificationFor: SimdElement;
}
#[verifier::external_type_specification]
#[verifier::external_body]
#[verifier::reject_recursive_types(T)]
pub struct ExSimd<T: SimdElement, const N: usize>(Simd<T, N>);

pub uninterp spec fn lanes<T: SimdElement, const N: usize>(value: Simd<T, N>) -> Seq<T>;
pub open spec fn view32(value: u32x8) -> Seq<u32> { lanes(value) }

pub assume_specification<T: SimdElement, const N: usize>[Simd::<T, N>::splat](value: T) -> (result: Simd<T, N>)
    ensures lanes(result) == Seq::new(N as nat, |i: int| value)
    no_unwind;
pub assume_specification<T: SimdElement, const N: usize>[Simd::<T, N>::from_array](value: [T; N]) -> (result: Simd<T, N>)
    ensures lanes(result) == value@
    no_unwind;
pub assume_specification<T, const N: usize>[<[T]>::as_chunks::<N>](values: &[T]) -> (result: (&[[T; N]], &[T]))
    requires N > 0,
    ensures result.0@.len() == values@.len() / (N as nat),
        result.1@ == values@.subrange((values@.len() / (N as nat) * N) as int, values@.len() as int),
        forall|i: int, j: int| 0 <= i < result.0@.len() && 0 <= j < N ==>
            #[trigger] result.0@[i]@[j] == values@[i * N + j]
    no_unwind;
}
