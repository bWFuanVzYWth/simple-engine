//! 全部元素已初始化的固定容量缓冲；清空仅重置长度，不调用分配器。

use core::ops::{Deref, DerefMut};

use crate::optimal::{copy, get_mut};
#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "../proof/fixed.rs"]
mod proof;

/// 验证目标：始终 `len <= N`，数学视图只包含 `items[..len]`。
/// push、resize、extend_from_slice 的调用者分别证明 `len < N`、`新长度 <= N`、
/// `len + 追加长度 <= N`，计算无溢出；pop 空时不变，否则返回旧末项。
/// clear 得到空视图，truncate 只保留指定长度的前缀，超出当前长度时不变。
/// 此类型不负责把容量不足转换为错误，不能把 get_mut 的 abort 当作正常失败。
/// 同源证明覆盖固有方法、Clone、Deref／DerefMut 与借用迭代转换。
#[derive(Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct FixedVec<T: Copy, const N: usize> {
    items: [T; N],
    len: usize,
}

// Verus 尚无非 Copy 结构的派生 Clone 规范；直接复制两个字段以保持类型不变量。
#[cfg_attr(verus_only, verus_verify)]
impl<T: Copy, const N: usize> Clone for FixedVec<T, N> {
    #[cfg_attr(verus_only, verus_spec(result => ensures result == *self))]
    fn clone(&self) -> Self {
        proof! { use_type_invariant(self); }
        Self {
            items: self.items,
            len: self.len,
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<T: Copy, const N: usize> FixedVec<T, N> {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.valid(), result@ == Seq::<T>::empty(),
        result.storage() == Seq::new(N as nat, |_: int| fill),
    ))]
    pub(crate) const fn new(fill: T) -> Self {
        Self {
            items: [fill; N],
            len: 0,
        }
    }

    #[cfg_attr(verus_only, verus_spec(result => requires self.valid(), ensures result == self@.len(), result <= N))]
    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    #[cfg_attr(verus_only, verus_spec(result => ensures result == N))]
    pub(crate) const fn capacity(&self) -> usize {
        self.items.len()
    }

    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(),
        ensures final(self).valid(), final(self)@ == Seq::<T>::empty(), final(self).storage() == old(self).storage(),
        no_unwind
    ))]
    pub(crate) const fn clear(&mut self) {
        self.len = 0;
    }

    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(),
        ensures final(self).valid(), final(self)@ == old(self)@.take(vstd::math::min(len as int, old(self)@.len() as int)),
            final(self).storage() == old(self).storage(),
    ))]
    pub(crate) const fn truncate(&mut self, len: usize) {
        if len < self.len {
            self.len = len;
        }
    }

    #[cfg_attr(verus_only, verus_spec(result => requires self.valid(), ensures result@ == self@))]
    pub(crate) const fn as_slice(&self) -> &[T] {
        self.items.split_at(self.len).0
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures result@ == old(self)@, final(self).valid(), final(self)@.len() == old(self)@.len(),
            final(self)@ == final(result)@,
            final(self).storage().skip(old(self)@.len() as int) == old(self).storage().skip(old(self)@.len() as int),
        no_unwind
    ))]
    pub(crate) const fn as_mut_slice(&mut self) -> &mut [T] {
        self.items.split_at_mut(self.len).0
    }

    /// 原位替换一个有效槽，不改变长度或其它槽。
    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(), index < old(self)@.len(),
        ensures final(self).valid(), final(self)@ == old(self)@.update(index as int, value),
            final(self).storage() == old(self).storage().update(index as int, value),
        no_unwind
    ))]
    pub(crate) const fn set(&mut self, index: usize, value: T) {
        assert!(index < self.len);
        self.items[index] = value;
    }

    /// 只借出一个有效槽；未来写入只影响该槽，长度与尾部均保持。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(), index < old(self)@.len(),
        ensures *result == old(self)@[index as int], final(self).valid(),
            final(self)@ == old(self)@.update(index as int, *final(result)),
            final(self).storage() == old(self).storage().update(index as int, *final(result)),
        no_unwind
    ))]
    pub(crate) fn slot_mut(&mut self, index: usize) -> &mut T {
        debug_assert!(index < self.len);
        if index >= self.len {
            crate::optimal::invariant_failed();
        }
        get_mut(&mut self.items, index)
    }

    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(), old(self)@.len() < N,
        ensures final(self).valid(), final(self)@ == old(self)@.push(value),
        no_unwind
    ))]
    pub(crate) fn push(&mut self, value: T) {
        *get_mut(&mut self.items, self.len) = value;
        self.len += 1;
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).valid(), proof::popped(old(self)@, final(self)@, result),
            final(self).storage() == old(self).storage(),
    ))]
    pub(crate) fn pop(&mut self) -> Option<T> {
        proof_decl! { let ghost length = self.len; }
        self.len = self.len.checked_sub(1)?;
        proof! { proof::pop_prefix(self.items@, length as int); }
        Some(copy(&self.items, self.len))
    }

    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(), len <= N,
        ensures final(self).valid(), final(self)@ == proof::resized(old(self)@, len as int, value),
        no_unwind
    ))]
    pub(crate) fn resize(&mut self, len: usize, value: T) {
        if len > self.len {
            debug_assert!(self.len <= len && len <= N);
            if len > N {
                crate::optimal::invariant_failed();
            }
            let (_, tail) = self.items.split_at_mut(self.len);
            let (slots, _) = tail.split_at_mut(len - self.len);
            proof_decl! { let ghost before = slots@; }
            let mut index = 0;
            #[cfg_attr(verus_only, verus_spec(
                invariant index <= slots@.len(), slots@.len() == before.len(),
                    slots@ == Seq::new(before.len(), |i: int| if i < index { value } else { before[i] }),
                decreases slots@.len() - index,
            ))]
            while index < slots.len() {
                slots[index] = value;
                index += 1;
            }
        }
        self.len = len;
    }

    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(), old(self)@.len() + values@.len() <= N,
        ensures final(self).valid(), final(self)@ == old(self)@ + values@,
    ))]
    pub(crate) fn extend_from_slice(&mut self, values: &[T]) {
        let end = self.len + values.len();
        crate::optimal::slice_mut(&mut self.items, self.len..end).copy_from_slice(values);
        self.len = end;
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<T: Copy, const N: usize> Deref for FixedVec<T, N> {
    type Target = [T];
    #[cfg_attr(verus_only, verus_spec(result => ensures result@ == self@))]
    fn deref(&self) -> &[T] {
        proof! { use_type_invariant(self); }
        self.as_slice()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<T: Copy, const N: usize> DerefMut for FixedVec<T, N> {
    #[cfg_attr(verus_only, verus_spec(result => ensures result@ == old(self)@,
        final(self)@ == final(result)@, final(self)@.len() == old(self)@.len(),
    ))]
    fn deref_mut(&mut self) -> &mut [T] {
        proof! { use_type_invariant(&*self); }
        self.as_mut_slice()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a, T: Copy, const N: usize> IntoIterator for &'a FixedVec<T, N> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;
    #[cfg_attr(verus_only, verus_spec(result => ensures
        vstd::std_specs::iter::IteratorSpec::obeys_prophetic_iter_laws(&result),
        vstd::std_specs::iter::IteratorSpec::remaining(&result) == self@.as_ref(),
        vstd::std_specs::iter::IteratorSpec::decrease(&result) is Some,
    ))]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
