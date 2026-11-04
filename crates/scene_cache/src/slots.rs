//! 空闲区间按地址有序，较大目录使用连续最大值树查找首个可容纳区间。
use crate::Error;
use std::ops::Range;

const LINEAR_LIMIT: usize = 16;

/// 连续属性槽的独占分配目录；准备事务持有自己的范围元数据。
/// 小目录串行查找，累计搜索达到F后建立一次索引，整批分配O(F+D log F)；
/// 提交前只排序新回收项并线性合并，成本O(F+D log D)。
pub struct Slots {
    end: u32,
    free: Vec<Range<u32>>,
    maxima: Vec<u32>,
    released: Vec<Range<u32>>,
    searched: usize,
    consumed: bool,
    pub(super) owner: u64,
    pub(super) generation: u64,
}

impl Slots {
    pub(super) const fn new(owner: u64) -> Self {
        Self {
            end: 0,
            free: Vec::new(),
            maxima: Vec::new(),
            released: Vec::new(),
            searched: 0,
            consumed: false,
            owner,
            generation: 0,
        }
    }

    pub(super) fn copy(&self) -> Result<Self, Error> {
        let mut next = Self::new(self.owner);
        next.end = self.end;
        next.generation = self.generation;
        next.free.try_reserve_exact(self.free.len())?;
        next.free.extend(self.free.iter().cloned());
        Ok(next)
    }

    pub(super) const fn retained_bytes(&self) -> usize {
        (self.free.capacity() + self.released.capacity()) * size_of::<Range<u32>>()
            + self.maxima.capacity() * size_of::<u32>()
    }

    /// 已占用与中间空闲范围的上界，供属性缓冲扩容使用。
    pub const fn end(&self) -> u32 {
        self.end
    }

    /// 预留地址最小的可容纳区间；零长度不消耗槽，溢出保留原状态。
    /// 未发布事务不会使用其回收列表，因此不能覆盖旧GPU读取。
    pub fn take(&mut self, count: u32) -> Result<Range<u32>, Error> {
        if count == 0 {
            return Ok(self.end..self.end);
        }
        if self.free.len() <= LINEAR_LIMIT {
            if let Some(index) = self
                .free
                .iter()
                .position(|range| range.end - range.start >= count)
            {
                let start = self.free[index].start;
                self.free[index].start += count;
                if self.free[index].is_empty() {
                    self.free.remove(index);
                }
                return Ok(start..start + count);
            }
            let start = self.end;
            self.end = start.checked_add(count).ok_or(Error::Capacity)?;
            return Ok(start..self.end);
        }
        if self.maxima.is_empty()
            && self.free.len() > LINEAR_LIMIT
            && self.searched >= self.free.len()
        {
            self.rebuild()?;
        }
        let index = if self.maxima.is_empty() {
            let found = self
                .free
                .iter()
                .position(|range| range.end - range.start >= count);
            self.searched = self
                .searched
                .saturating_add(found.map_or(self.free.len(), |index| index + 1));
            found
        } else if self.maxima[1] < count {
            None
        } else {
            let base = self.maxima.len() / 2;
            let mut node = 1;
            while node < base {
                node = if self.maxima[node * 2] >= count {
                    node * 2
                } else {
                    node * 2 + 1
                };
            }
            Some(node - base)
        };
        if let Some(index) = index {
            let start = self.free[index].start;
            self.free[index].start += count;
            self.consumed |= self.free[index].is_empty();
            if !self.maxima.is_empty() {
                let mut node = self.maxima.len() / 2 + index;
                self.maxima[node] = self.free[index].end - self.free[index].start;
                while node > 1 {
                    node /= 2;
                    self.maxima[node] = self.maxima[node * 2].max(self.maxima[node * 2 + 1]);
                }
            }
            return Ok(start..start + count);
        }
        let start = self.end;
        self.end = start.checked_add(count).ok_or(Error::Capacity)?;
        Ok(start..self.end)
    }

    pub(super) fn release(&mut self, range: Range<u32>) -> Result<(), Error> {
        if !range.is_empty() {
            self.released.try_reserve(1)?;
            self.released.push(range);
        }
        Ok(())
    }

    pub(super) fn merge(&mut self) -> Result<(), Error> {
        if self.released.is_empty() && !self.consumed {
            return Ok(());
        }
        self.free.try_reserve(self.released.len())?;
        self.free.retain(|range| !range.is_empty());
        self.released.sort_unstable_by_key(|range| range.start);
        let mut old = self.free.len();
        let mut extra = self.released.len();
        let mut output = old + extra;
        self.free.resize(output, 0..0);
        while extra != 0 {
            output -= 1;
            if old != 0 && self.free[old - 1].start > self.released[extra - 1].start {
                old -= 1;
                self.free[output] = self.free[old].clone();
            } else {
                extra -= 1;
                self.free[output] = self.released[extra].clone();
            }
        }
        self.released.clear();
        let mut length = 0;
        for index in 0..self.free.len() {
            let range = self.free[index].clone();
            if length != 0 && self.free[length - 1].end == range.start {
                self.free[length - 1].end = range.end;
            } else {
                self.free[length] = range;
                length += 1;
            }
        }
        self.free.truncate(length);
        while self.free.last().is_some_and(|range| range.end == self.end) {
            self.end = self.free.pop().expect("末项已检查").start;
        }
        self.maxima.clear();
        self.searched = 0;
        self.consumed = false;
        Ok(())
    }

    fn rebuild(&mut self) -> Result<(), Error> {
        self.maxima.clear();
        if self.free.len() <= LINEAR_LIMIT {
            return Ok(());
        }
        let base = self
            .free
            .len()
            .checked_next_power_of_two()
            .ok_or(Error::Capacity)?;
        let size = base.checked_mul(2).ok_or(Error::Capacity)?;
        self.maxima.try_reserve(size)?;
        self.maxima.resize(size, 0);
        for (index, range) in self.free.iter().enumerate() {
            self.maxima[base + index] = range.end - range.start;
        }
        for index in (1..base).rev() {
            self.maxima[index] = self.maxima[index * 2].max(self.maxima[index * 2 + 1]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_ranges_remain_disjoint_through_partial_reuse_and_coalescing() -> Result<(), Error>
    {
        let mut slots = Slots::new(1);
        let blocks: Vec<_> = (0..1024).map(|_| slots.take(4)).collect::<Result<_, _>>()?;
        for block in blocks.iter().step_by(2) {
            slots.release(block.clone())?;
        }
        slots.merge()?;
        for index in 0..512 {
            assert_eq!(slots.take(3)?, index * 8..index * 8 + 3);
        }
        assert_eq!(slots.take(2)?, 4096..4098);
        for block in blocks.iter().skip(1).step_by(2) {
            slots.release(block.clone())?;
        }
        slots.merge()?;
        for index in 0..512 {
            assert_eq!(slots.take(5)?, index * 8 + 3..index * 8 + 8);
        }
        assert_eq!(slots.end(), 4098);
        Ok(())
    }
}
