use crate::Device;
use std::ops::{Deref, DerefMut};

#[path = "arena.rs"]
mod arena;
pub(crate) use arena::{Arena, InsertError, Key, Values};

pub(crate) trait Destroy {
    fn destroy(&mut self, device: &Device);
}

pub(crate) struct Guard<'a, T: Destroy> {
    device: &'a Device,
    value: Option<T>,
}
impl<'a, T: Destroy> Guard<'a, T> {
    pub(crate) const fn new(device: &'a Device, value: T) -> Self {
        Self {
            device,
            value: Some(value),
        }
    }
    pub(crate) fn finish(mut self) -> T {
        self.value.take().expect("构造 guard 仅完成一次")
    }
}
impl<T: Destroy> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value.as_ref().expect("构造尚未完成")
    }
}
impl<T: Destroy> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("构造尚未完成")
    }
}
impl<T: Destroy> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        if self.device.is_quarantined() {
            std::mem::forget(self.value.take());
            return;
        }
        if let Some(value) = &mut self.value {
            value.destroy(self.device);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_slot_rejects_old_generation() {
        let mut arena = Arena::default();
        let old = arena.try_insert(7, 42).unwrap_or_else(|_| panic!());
        assert_eq!(arena.remove(old), Some(42));
        let current = arena.try_insert(7, 99).unwrap_or_else(|_| panic!());
        assert!(arena.find(old).is_none());
        assert_eq!(*arena.find(current).unwrap(), 99);
    }
}
