use crate::Handle;

#[derive(Clone, Copy, Default)]
struct Link {
    handle: Option<Handle>,
    previous: Option<usize>,
    next: Option<usize>,
    bucket: u8,
}

// 每槽至多一个队列项；覆盖旧代号时直接摘链，不累积过期项。
// 常规出队取最近桶，每八次轮询下一非空桶；稳定目标下持续近处事件不能永久饿死远处。
pub(crate) struct DistanceQueue {
    links: Vec<Link>,
    heads: [Option<usize>; 128],
    tails: [Option<usize>; 128],
    occupied: u128,
    len: usize,
    fairness_turn: u8,
    fairness_bucket: u8,
}

impl DistanceQueue {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            links: vec![Link::default(); capacity],
            heads: [None; 128],
            tails: [None; 128],
            occupied: 0,
            len: 0,
            fairness_turn: 0,
            fairness_bucket: 0,
        }
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn reserved_bytes(&self) -> usize {
        self.links.capacity() * std::mem::size_of::<Link>()
    }

    pub(crate) fn insert(&mut self, handle: Handle, bucket: u8) {
        let index = handle.index;
        if self.links[index].handle == Some(handle) && self.links[index].bucket == bucket {
            return;
        }
        self.remove(index);
        let previous = self.tails[bucket as usize];
        self.links[index] = Link {
            handle: Some(handle),
            previous,
            next: None,
            bucket,
        };
        if let Some(previous) = previous {
            self.links[previous].next = Some(index);
        } else {
            self.heads[bucket as usize] = Some(index);
        }
        self.tails[bucket as usize] = Some(index);
        self.occupied |= 1_u128 << bucket;
        self.len += 1;
    }

    pub(crate) fn pop(&mut self) -> Option<(Handle, u8)> {
        self.pop_mask(u128::MAX)
    }

    pub(crate) const fn has_mask(&self, mask: u128) -> bool {
        self.occupied & mask != 0
    }

    pub(crate) fn reserve_capacity(
        &mut self,
        capacity: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        self.links
            .try_reserve_exact(capacity.saturating_sub(self.links.len()))
    }

    pub(crate) fn grow_capacity(&mut self, capacity: usize) {
        self.links.resize(capacity, Link::default());
    }

    pub(crate) fn pop_mask(&mut self, mask: u128) -> Option<(Handle, u8)> {
        let occupied = self.occupied & mask;
        if occupied == 0 {
            return None;
        }
        self.fairness_turn = (self.fairness_turn + 1) % 8;
        let following = occupied & (u128::MAX << self.fairness_bucket);
        let candidates = if self.fairness_turn == 0 && following != 0 {
            following
        } else {
            occupied
        };
        let bucket = candidates.trailing_zeros() as usize;
        if self.fairness_turn == 0 {
            self.fairness_bucket = ((bucket + 1) % 128) as u8;
        }
        let index = self.heads[bucket].expect("非空距离桶拥有首项");
        let handle = self.links[index].handle.expect("队列项拥有句柄");
        self.remove(index);
        Some((handle, bucket as u8))
    }

    pub(crate) fn discard(&mut self, handle: Handle) {
        if self.links[handle.index].handle == Some(handle) {
            self.remove(handle.index);
        }
    }

    fn remove(&mut self, index: usize) {
        let link = self.links[index];
        if link.handle.is_none() {
            return;
        }
        if let Some(previous) = link.previous {
            self.links[previous].next = link.next;
        } else {
            self.heads[link.bucket as usize] = link.next;
        }
        if let Some(next) = link.next {
            self.links[next].previous = link.previous;
        } else {
            self.tails[link.bucket as usize] = link.previous;
        }
        if self.heads[link.bucket as usize].is_none() {
            self.occupied &= !(1_u128 << link.bucket);
        }
        self.links[index] = Link::default();
        self.len -= 1;
    }
}

pub(crate) const fn distance_bucket(distance2: u128) -> u8 {
    if distance2 == 0 {
        0
    } else {
        (127 - distance2.leading_zeros()) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(index: usize, generation: u64) -> Handle {
        Handle { index, generation }
    }

    #[test]
    fn nearer_buckets_precede_farther_buckets() {
        let mut queue = DistanceQueue::new(4);
        queue.insert(handle(0, 1), 5);
        queue.insert(handle(1, 2), 1);
        queue.insert(handle(2, 3), 1);
        assert_eq!(queue.pop(), Some((handle(1, 2), 1)));
        assert_eq!(queue.pop(), Some((handle(2, 3), 1)));
        assert_eq!(queue.pop(), Some((handle(0, 1), 5)));
        assert!(queue.is_empty());
    }

    #[test]
    fn slot_reuse_replaces_its_old_queue_entry() {
        let mut queue = DistanceQueue::new(1);
        queue.insert(handle(0, 1), 3);
        queue.insert(handle(0, 2), 7);
        assert_eq!(queue.pop(), Some((handle(0, 2), 7)));
        assert!(queue.is_empty());
    }

    #[test]
    fn continuous_near_work_cannot_starve_a_far_bucket() {
        let mut queue = DistanceQueue::new(2);
        let near = handle(0, 1);
        let far = handle(1, 1);
        queue.insert(far, 127);
        let mut served = false;
        for _ in 0..16 {
            queue.insert(near, 0);
            served |= queue.pop().unwrap().0 == far;
        }
        assert!(served, "持续近处工作不能永久阻塞已发现的远处任务");
    }
}
