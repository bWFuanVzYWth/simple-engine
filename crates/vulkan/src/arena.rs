//! 独占槽表；完整身份由 owner、槽号和不回绕代次组成，owner 隔离由上层检查。
//!
//! 移除推进代次；MAX 代次允许最后一次插入，移除后不再加入 free。
//! 验证覆盖插入/查找/移除的真实 Vec 更新；分配依赖固定 vstd 的标准库规范。
//! 借用值游标覆盖空槽跳过和原值顺序；身份 iter 与保留容量统计尚未验证。

use std::collections::TryReserveError;
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "proof/arena.rs"]
#[cfg(verus_only)]
mod proof;
#[cfg(verus_only)]
pub(crate) use proof::{preserves_others, revoked_absent};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Key {
    pub(crate) owner: u64,
    pub(crate) slot: u32,
    pub(crate) generation: u32,
}

#[cfg_attr(verus_only, verus_verify)]
impl Key {
    /// 比较完整资源身份；供同源计划核连接执行比较与逻辑相等。
    #[cfg_attr(verus_only, verus_spec(result => ensures result == (self == other)))]
    pub(crate) const fn same(self, other: Self) -> bool {
        self.owner == other.owner && self.slot == other.slot && self.generation == other.generation
    }
}

#[cfg_attr(verus_only, verus_verify)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum InsertError {
    Full,
    Allocation(TryReserveError),
}

/// 空闲索引有效且唯一；空的非终代槽必须可复用。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

/// 只借用尚未扫描的槽位；克隆只复制位置，不复制条目或要求 T: Clone。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Values<'a, T> {
    remaining: &'a [Slot<T>],
}

#[cfg(not(verus_only))]
impl<T> vstd::view::View for Values<'_, T> {
    type V = ();
}

#[cfg_attr(verus_only, verus_verify)]
impl<T> Clone for Values<'_, T> {
    #[cfg_attr(verus_only, verus_spec(copy => ensures copy == *self))]
    fn clone(&self) -> Self {
        Self {
            remaining: self.remaining,
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a, T> Values<'a, T> {
    /// 跳过空槽，交出下一原值的借用；耗尽后保持末尾。
    #[cfg_attr(verus_only, verus_spec(result => ensures
        result is Some <==> old(self)@.len() > 0,
        result is Some ==> *result->Some_0 == old(self)@[0]
            && final(self)@ == old(self)@.drop_first()
            && final(self).slots_left() < old(self).slots_left(),
        result is None ==> final(self)@.len() == 0 && final(self).slots_left() == 0,
    ))]
    pub(crate) fn advance(&mut self) -> Option<&'a T> {
        #[cfg(verus_only)]
        proof_decl! { let ghost original = self@; }
        #[cfg_attr(verus_only, verus_spec(invariant self@ == original, original == old(self)@,
            self.slots_left() <= old(self).slots_left(),
            decreases self.slots_left(),
        ))]
        while !self.remaining.is_empty() {
            let (slot, remaining) = self.remaining.split_first().unwrap();
            #[cfg(verus_only)]
            proof! { proof::values_step(self.remaining@); }
            self.remaining = remaining;
            if let Some(value) = slot.value.as_ref() {
                return Some(value);
            }
        }
        None
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<T> Arena<T> {
    #[cfg_attr(verus_only, verus_spec(arena => ensures arena.valid(), arena.slot_count() == 0))]
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    /// 失败归还原值；已存在条目的内容不变，保留容量允许变化。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures proof::inserted(old(self), final(self), owner, value, result),
    ))]
    pub(crate) fn try_insert(&mut self, owner: u64, value: T) -> Result<Key, (InsertError, T)> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            let key = Key {
                owner,
                slot: index,
                generation: slot.generation,
            };
            #[cfg(verus_only)]
            proof! { proof::insert_reused(before, *self, index); }
            Ok(key)
        } else {
            if self.slots.len() > u32::MAX as usize {
                return Err((InsertError::Full, value));
            }
            let index = self.slots.len() as u32;
            if let Err(error) = self.slots.try_reserve(1) {
                return Err((InsertError::Allocation(error), value));
            }
            self.slots.push(Slot {
                generation: 0,
                value: Some(value),
            });
            #[cfg(verus_only)]
            proof! { proof::insert_new(before, *self); }
            Ok(Key {
                owner,
                slot: index,
                generation: 0,
            })
        }
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        ensures
            result is Some <==> self.lookup(key) is Some,
            result is Some ==> *result->Some_0 == self.lookup(key)->Some_0,
    ))]
    pub(crate) fn find(&self, key: Key) -> Option<&T> {
        let slot = self.slots.get(key.slot as usize)?;
        if slot.generation == key.generation {
            return slot.value.as_ref();
        }
        None
    }

    /// 按槽借用当前条目并补入 owner；空槽或越界返回 None，不接受外部代次。
    #[cfg_attr(verus_only, verus_spec(entry =>
        requires self.valid(),
        ensures proof::read_at(self, slot, owner, entry),
    ))]
    pub(crate) fn at(&self, slot: usize, owner: u64) -> Option<(Key, &T)> {
        let entry = self.slots.get(slot)?;
        let value = entry.value.as_ref()?;
        let key = Key {
            owner,
            slot: slot as u32,
            generation: entry.generation,
        };
        Some((key, value))
    }

    /// 读取已确认存在的槽位身份；只访问代次，不检查或借用原值。
    #[cfg_attr(verus_only, verus_spec(key =>
        requires self.valid(), self.occupied(slot),
        ensures key.owner == owner, key.slot as usize == slot,
            key.generation == self.generation_at(slot), self.lookup(key) is Some,
    ))]
    pub(crate) const fn present_key(&self, slot: usize, owner: u64) -> Key {
        Key {
            owner,
            slot: slot as u32,
            generation: self.slots.as_slice()[slot].generation,
        }
    }

    #[cfg_attr(verus_only, verus_spec(length => ensures length == self.slot_count()))]
    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }

    /// 借用全部槽位作为值游标；无分配，按槽顺序跳过空项。
    #[cfg_attr(verus_only, verus_spec(cursor => ensures cursor@ == self.values_view()))]
    pub(crate) const fn values_cursor(&self) -> Values<'_, T> {
        Values {
            remaining: self.slots.as_slice(),
        }
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures
            proof::borrowed_state(old(self), final(self), key, result is Some),
            result is Some ==> *result->Some_0 == old(self).lookup(key)->Some_0,
            result is Some ==> final(self).lookup(key) == Some(*final(result->Some_0)),
    ))]
    pub(crate) fn find_mut(&mut self, key: Key) -> Option<&mut T> {
        if (key.slot as usize) >= self.slots.len() {
            return None;
        }
        let slot = &mut self.slots[key.slot as usize];
        if slot.generation != key.generation {
            return None;
        }
        slot.value.as_mut()
    }

    /// 只移出匹配的当前值；成功后完整旧身份永久失效。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures proof::removed_entry(old(self), final(self), key, result),
    ))]
    pub(crate) fn remove(&mut self, key: Key) -> Option<T> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        if (key.slot as usize) >= self.slots.len() {
            return None;
        }
        let slot = &mut self.slots[key.slot as usize];
        if slot.generation != key.generation {
            return None;
        }
        let value = slot.value.take()?;
        if slot.generation < u32::MAX {
            slot.generation += 1;
            self.free.push(key.slot);
        }
        #[cfg(verus_only)]
        proof! { proof::removed(before, *self, key); }
        Some(value)
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<T> Default for Arena<T> {
    #[cfg_attr(verus_only, verus_spec(arena => ensures arena.valid()))]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Arena<T> {
    pub(crate) fn iter(&self, owner: u64) -> impl Iterator<Item = (Key, &T)> + Clone {
        self.slots
            .iter()
            .enumerate()
            .filter_map(move |(index, slot)| {
                slot.value.as_ref().map(|value| {
                    (
                        Key {
                            owner,
                            slot: index as u32,
                            generation: slot.generation,
                        },
                        value,
                    )
                })
            })
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.slots.capacity() * size_of::<Slot<T>>() + self.free.capacity() * size_of::<u32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_generation_is_used_once() {
        let mut arena = Arena {
            slots: vec![Slot {
                generation: u32::MAX - 1,
                value: Some(1),
            }],
            free: Vec::new(),
        };
        let previous = Key {
            owner: 7,
            slot: 0,
            generation: u32::MAX - 1,
        };
        assert_eq!(arena.remove(previous), Some(1));
        let final_key = arena.try_insert(7, 2).ok().unwrap();
        assert_eq!(final_key.generation, u32::MAX);
        assert_eq!(arena.remove(final_key), Some(2));
        let fresh = arena.try_insert(7, 3).ok().unwrap();
        assert_ne!(fresh.slot, final_key.slot);
        assert!(arena.find(final_key).is_none());
    }
}
