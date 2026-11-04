//! 根槽的唯一所有者；空槽显式无值，代次只在成功释放时前进。

#[cfg(not(verus_only))]
use crate::vec_ops;
use crate::{Child, Error};
#[cfg(verus_only)]
use hash_dag_std::vec_ops;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
#[path = "proof/roots.rs"]
pub(crate) mod proof;

/// 持久化 root 句柄；复制不增加节点引用。
///
/// 只在发行它的 [`crate::HashDag`] 中有效；独立快照须由
/// [`crate::HashDag::fork_root`] 创建，并显式释放。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
pub struct RootId {
    pub(super) owner: usize,
    pub(super) slot: usize,
    pub(super) generation: u64,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct RootSlot {
    pub(super) child: Option<Child>,
    pub(super) generation: u64,
}

/// 默认槽始终存活；其余空槽恰好在 free 中出现一次。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct RootSlots {
    pub(super) owner: usize,
    pub(super) slots: Vec<RootSlot>,
    pub(super) free: Vec<usize>,
}

#[cfg_attr(verus_only, verus_verify)]
impl RootSlots {
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is Ok ==> result->Ok_0.valid()
            && result->Ok_0.slots@.len() == 1 && result->Ok_0.free@.len() == 0
            && result->Ok_0.owner == owner
            && result->Ok_0.lookup(RootId { owner, slot: 0, generation: 0 }) == Some(Child::Leaf(0)),
    ))]
    pub(crate) fn new(owner: usize) -> Result<Self, Error> {
        let mut slots = vec_ops::new();
        if let Err(error) = vec_ops::reserve_exact(&mut slots, 1) {
            return Err(Error::Allocation(error));
        }
        vec_ops::push(
            &mut slots,
            RootSlot {
                child: Some(Child::Leaf(0)),
                generation: 0,
            },
        );
        Ok(Self {
            owner,
            slots,
            free: vec_ops::new(),
        })
    }

    #[must_use]
    #[cfg_attr(verus_only, verus_spec(root =>
        ensures root.owner == self.owner && root.slot == 0 && root.generation == 0,
        no_unwind
    ))]
    pub(crate) const fn default_root(&self) -> RootId {
        RootId {
            owner: self.owner,
            slot: 0,
            generation: 0,
        }
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is Ok <==> self.lookup(root) is Some,
            result is Ok ==> result->Ok_0 == self.lookup(root)->Some_0,
            root.owner != self.owner ==> result is Err && result->Err_0 is WrongOwner,
            root.owner == self.owner && self.lookup(root) is None ==>
                result is Err && result->Err_0 is RootMissing,
        no_unwind
    ))]
    #[inline]
    pub(crate) fn child(&self, root: RootId) -> Result<Child, Error> {
        if root.owner != self.owner {
            return Err(Error::WrongOwner);
        }
        if root.slot >= self.slots.len() {
            return Err(Error::RootMissing);
        }
        let slot = &self.slots[root.slot];
        if slot.generation != root.generation {
            return Err(Error::RootMissing);
        }
        match slot.child {
            Some(child) => Ok(child),
            None => Err(Error::RootMissing),
        }
    }

    /// 先准备插入和将来释放所需容量，尚不改变根表内容。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).valid(), final(self).same_entries(old(self)),
            result is Ok ==> final(self).insertion_ready(),
    ))]
    pub(crate) fn prepare(&mut self) -> Result<(), Error> {
        if let Some(&slot) = self.free.last() {
            self.slots.get(slot).ok_or(Error::RootMissing)?;
        } else {
            let count = self
                .slots
                .len()
                .checked_add(1)
                .ok_or(Error::CapacityOverflow)?;
            if let Err(error) = vec_ops::reserve(&mut self.slots, 1) {
                return Err(Error::Allocation(error));
            }
            let additional = count.saturating_sub(self.free.len());
            if let Err(error) = vec_ops::reserve(&mut self.free, additional) {
                return Err(Error::Allocation(error));
            }
        }
        Ok(())
    }

    /// 调用方已准备容量并取得根节点引用；提交不分配且不再失败。
    #[cfg_attr(verus_only, verus_spec(root =>
        requires old(self).valid(), old(self).insertion_ready(),
        ensures proof::inserted(old(self), final(self), child, root),
    ))]
    pub(crate) fn insert(&mut self, child: Child) -> RootId {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        if let Some(slot) = vec_ops::pop(&mut self.free) {
            let entry = &mut self.slots[slot];
            let root = RootId {
                owner: self.owner,
                slot,
                generation: entry.generation,
            };
            entry.child = Some(child);
            #[cfg(verus_only)]
            proof! { proof::insert_reused(before, *self, slot); }
            root
        } else {
            let slot = self.slots.len();
            vec_ops::push(
                &mut self.slots,
                RootSlot {
                    child: Some(child),
                    generation: 0,
                },
            );
            #[cfg(verus_only)]
            proof! { proof::insert_new(before, *self); }
            RootId {
                owner: self.owner,
                slot,
                generation: 0,
            }
        }
    }

    /// 验证释放资格；饱和错误保留原根，真实节点回收在所有者提交前完成。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires self.valid(),
        ensures result is Ok ==> self.lookup(root) == Some(result->Ok_0.0)
            && root.slot != 0 && root.generation < u64::MAX
            && result->Ok_0.1 == root.generation + 1,
            self.lookup(root) is Some && root.slot == 0 ==>
                result is Err && result->Err_0 is CannotReleaseDefaultRoot,
            self.lookup(root) is Some && root.slot != 0 && root.generation == u64::MAX ==>
                result is Err && result->Err_0 is RootGenerationExhausted,
        no_unwind
    ))]
    // 固定 Verus 的 FromResidual 未声明 no_unwind，显式返回保持此契约可验。
    #[allow(clippy::question_mark)]
    pub(crate) fn prepare_release(&self, root: RootId) -> Result<(Child, u64), Error> {
        let child = match self.child(root) {
            Ok(child) => child,
            Err(error) => return Err(error),
        };
        if root.slot == 0 {
            return Err(Error::CannotReleaseDefaultRoot);
        }
        let generation = match root.generation.checked_add(1) {
            Some(generation) => generation,
            None => return Err(Error::RootGenerationExhausted),
        };
        Ok((child, generation))
    }

    /// 调用方完成真实节点回收后，使旧身份永久失效；不分配。
    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(), old(self).lookup(root) is Some, root.slot != 0,
            root.generation < u64::MAX, generation == root.generation + 1,
        ensures proof::removed(old(self), final(self), root),
            hash_dag_std::vec_model::capacity(final(self).free) == hash_dag_std::vec_model::capacity(old(self).free),
        no_unwind
    ))]
    pub(crate) fn retire(&mut self, root: RootId, generation: u64) {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        #[cfg(verus_only)]
        proof! { proof::release_capacity(self, root); }
        let slot = &mut self.slots[root.slot];
        slot.child = None;
        slot.generation = generation;
        vec_ops::push(&mut self.free, root.slot);
        #[cfg(verus_only)]
        proof! { proof::retired(before, *self, root); }
    }

    /// 只替换已校验的活根内容；不发行身份或改变其它根。
    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(), old(self).lookup(root) is Some,
        ensures final(self).valid(), final(self).lookup(root) == Some(child),
            proof::preserves_others(old(self), final(self), root),
            proof::preserves_revoked(old(self), final(self)),
            proof::replaced(old(self), final(self), root, child),
        no_unwind
    ))]
    pub(crate) fn replace(&mut self, root: RootId, child: Child) {
        self.slots[root.slot].child = Some(child);
    }
}

impl RootSlots {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.slots.capacity() * size_of::<RootSlot>() + self.free.capacity() * size_of::<usize>()
    }
}
