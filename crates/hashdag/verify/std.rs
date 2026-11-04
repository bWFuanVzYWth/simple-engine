//! 可信依赖适配：hashbrown 与固定 Verus 尚未描述的标准原语。
//!
//! hashbrown 按用户指定信任；此边界不包含节点判等、入池、引用维护、事务或差分算法。
//! 原语契约单独编译导入，不计入证明；同源 identity 发行核另以 OwnedIdentity 阶段验证。
#![feature(portable_simd, proc_macro_hygiene)]

use hashbrown::HashTable;
use vstd::prelude::*;

#[path = "simd.rs"]
mod simd;
pub use simd::{simd_ops, view32};

#[path = "../src/diff_ops.rs"]
pub mod diff_ops;
#[path = "../src/identity.rs"]
pub mod identity;
#[path = "../src/identity_ops.rs"]
pub mod identity_ops;
#[path = "sort.rs"]
pub mod sort_model;
#[path = "table.rs"]
pub mod table_model;
#[path = "../src/table_ops.rs"]
pub mod table_ops;
#[path = "vec.rs"]
pub mod vec_model;
#[path = "../src/vec_ops.rs"]
pub mod vec_ops;

verus! {

#[verifier::external_trait_specification]
pub unsafe trait ExAllocator {
    type ExternalTraitSpecificationFor: allocator_api2::alloc::Allocator;
}

#[verifier::external_type_specification]
pub struct ExGlobal(allocator_api2::alloc::Global);

#[verifier::external_type_specification]
#[verifier::external_body]
#[verifier::reject_recursive_types(T)]
#[verifier::reject_recursive_types(A)]
pub struct ExTable<T, A: allocator_api2::alloc::Allocator>(HashTable<T, A>);

#[verifier::external_type_specification]
#[verifier::external_body]
pub struct ExHashAllocation(hashbrown::TryReserveError);

pub assume_specification[usize::div_ceil](value: usize, divisor: usize) -> (result: usize)
    requires divisor > 0,
    ensures result == value / divisor + if value % divisor == 0 { 0usize } else { 1usize }
    no_unwind;

pub assume_specification[usize::checked_shl](value: usize, shift: u32) -> (result: Option<usize>)
    ensures result == if shift < usize::BITS { Some(value << shift) } else { None }
    no_unwind;

}
