//! 实例身份发行；仅整数与原子单元进入运行布局，票据和历史完全擦除。
//!
//! 此同源核在 OwnedIdentity 阶段实际验证；其所在导出库也包含另行审查的标准原语。
use crate::identity_ops::{self, Step};
use std::sync::atomic::AtomicUsize;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
use vstd::{
    invariant::AtomicInvariant,
    resource::set::{GhostSetAuth, GhostSingleton},
};
#[cfg(verus_only)]
#[path = "proof/identity_owner.rs"]
pub(crate) mod proof;

/// 发行策略的静态适配；不持有运行期状态。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Checked;

#[cfg_attr(verus_only, verus_verify)]
impl Step for Checked {
    #[inline(always)]
    fn apply(value: usize) -> Option<usize> {
        if value < usize::MAX {
            Some(value + 1)
        } else {
            None
        }
    }
}

/// 物理计数器；不含执行期历史或额外同步。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Counter {
    pub(crate) value: AtomicUsize,
    #[cfg(verus_only)]
    pub(crate) inv: Tracked<AtomicInvariant<(int, vstd::resource::Loc), proof::Pair, proof::Pred>>,
}

#[cfg_attr(verus_only, verus_verify)]
impl Counter {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires value >= 1,
        ensures result.valid(),
    ))]
    pub(crate) const fn new_at(value: usize) -> Self {
        #[cfg(verus_only)]
        let mut permission = proof::empty();
        let value = identity_ops::primitive_new(
            value,
            #[cfg(verus_only)]
            &mut permission,
        );
        #[cfg(verus_only)]
        let inv = proof::initialize(&value, permission);
        Self {
            value,
            #[cfg(verus_only)]
            inv,
        }
    }

    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires self.valid(),
        ensures result is Ok ==> final(out)@ is Some
            && final(out)@->Some_0@ == result->Ok_0
            && final(out)@->Some_0.id() == self.authority()
            && 1 <= result->Ok_0 < usize::MAX,
            result is Err ==> final(out)@ == old(out)@,
    ))]
    pub(crate) fn issue(
        &self,
        #[cfg(verus_only)] out: &mut Tracked<Option<Ticket>>,
    ) -> Result<usize, ()> {
        let result;
        #[cfg_attr(verus_only, verifier::invariant_block)]
        {
            #[cfg(verus_only)]
            vstd::invariant::spend_open_invariant_credit(
                vstd::invariant::create_open_invariant_credit(),
            );
            #[cfg(verus_only)]
            let (guard, mut pair) = vstd::invariant::open_atomic_invariant_begin(self.inv.borrow());
            {
                #[cfg(verus_only)]
                proof_decl! { let tracked (mut permission, mut auth) = pair; }
                #[cfg_attr(verus_only, verus_spec(with Tracked(&mut permission)))]
                let current = identity_ops::permissioned_try_update::<Checked>(&self.value);
                result = current;
                #[cfg(verus_only)]
                proof! { proof::issued(result, &mut auth, permission, out); pair = (permission, auth); }
            }
            #[cfg(verus_only)]
            vstd::invariant::open_invariant_end(guard, pair);
        }
        match result {
            Ok(value) => Ok(value),
            Err(_) => Err(()),
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(ensures NEXT_OWNER.valid()))]
pub(crate) static NEXT_OWNER: Counter = Counter::new_at(1);

#[cfg(verus_only)]
pub type Ticket = GhostSingleton<usize>;
#[cfg(verus_only)]
pub use proof::ticket_valid;

/// 构造期间独占的发行结果；移交票据后不增加 HashDag 的运行布局。
#[cfg_attr(verus_only, verus_verify)]
pub struct Owner {
    pub(crate) number: usize,
    #[cfg(verus_only)]
    pub(crate) ticket: Tracked<Ticket>,
}

#[cfg_attr(verus_only, verus_verify)]
impl Owner {
    /// 从进程内唯一计数器取得新身份；饱和时不发行。
    #[cfg_attr(verus_only, verus_spec(result => ensures result is Ok ==> result->Ok_0.valid()))]
    pub fn new() -> Result<Self, ()> {
        let counter = identity_ops::global_counter();
        #[cfg(verus_only)]
        let mut ticket = proof::empty();
        let number = counter.issue(
            #[cfg(verus_only)]
            &mut ticket,
        )?;
        #[cfg(verus_only)]
        let ticket = proof::unpack(ticket);
        Ok(Self {
            number,
            #[cfg(verus_only)]
            ticket,
        })
    }

    /// 身份整数；相同实例的根表沿用此值。
    #[cfg_attr(verus_only, verus_spec(result => ensures result == self.number_spec(), no_unwind))]
    pub const fn number(&self) -> usize {
        self.number
    }

    #[cfg(verus_only)]
    #[cfg_attr(verus_only, verus_spec(result => requires self.valid(), ensures proof::ticket_valid(result@, self.number_spec())))]
    pub fn into_ticket(self) -> Tracked<Ticket> {
        self.ticket
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn identity_exhaustion_never_reuses_a_number() {
        let counter = Counter::new_at(usize::MAX - 1);
        assert_eq!(counter.issue().ok(), Some(usize::MAX - 1));
        assert!(counter.issue().is_err());
        assert_eq!(counter.value.load(Ordering::Relaxed), usize::MAX);
    }
}
