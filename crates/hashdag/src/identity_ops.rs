//! 标准原子许可适配；发行策略与已发行集合不在此可信边界内。
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
#[path = "../verify/identity.rs"]
pub mod model;

/// 可放入原子线性化点的纯标量回调；调用不打开共享不变量，也不 unwind。
#[cfg_attr(verus_only, verus_verify)]
pub trait Step: StepModel {
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == Self::next(value),
        opens_invariants none,
        no_unwind
    ))]
    fn apply(value: usize) -> Option<usize>;
}

#[cfg(verus_only)]
verus! { pub trait StepModel { spec fn next(value: usize) -> Option<usize>; } }
#[cfg(not(verus_only))]
pub trait StepModel {}
#[cfg(not(verus_only))]
impl<T> StepModel for T {}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures final(permission)@ is Some
        && model::view(final(permission)@->Some_0) == (model::cell(&result), value),
    no_unwind
))]
pub(crate) const fn primitive_new(
    value: usize,
    #[cfg(verus_only)] permission: &mut Tracked<Option<model::Permission>>,
) -> AtomicUsize {
    AtomicUsize::new(value)
}

/// 标准 try_update 的一个线性化 RMW；回调实现的算术和发行策略另行验证。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verifier::atomic)]
#[cfg_attr(verus_only, verus_spec(result =>
    with Tracked(permission): Tracked<&mut model::Permission>,
    requires model::view(*old(permission)).0 == model::cell(counter),
    ensures model::view(*final(permission)).0 == model::view(*old(permission)).0,
    match result {
        Ok(value) => value == model::view(*old(permission)).1 && S::next(value) == Some(model::view(*final(permission)).1),
        Err(value) => value == model::view(*old(permission)).1 && model::view(*final(permission)).1 == value && S::next(value) == None,
    },
    opens_invariants none,
    no_unwind
))]
pub(crate) fn permissioned_try_update<S: Step>(counter: &AtomicUsize) -> Result<usize, usize> {
    counter.try_update(Ordering::Relaxed, Ordering::Relaxed, S::apply)
}

/// 固定 Verus 不允许在 spec 读取 exec static；仅关联 Rust 同一静态对象的不可变元数据。
/// const 初始化、checked 递增、MAX 和票据唯一性均由发行核证明，此处不假设 fresh owner。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result => ensures result.valid(), result.authority() == model::global_authority()))]
pub(crate) fn global_counter() -> &'static crate::identity::Counter {
    &crate::identity::NEXT_OWNER
}
