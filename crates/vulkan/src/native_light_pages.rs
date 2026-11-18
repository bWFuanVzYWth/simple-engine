//! 区块子树共享大分配。子树版本拥有一段互不重叠的页内地址，收取退役后才归还范围。
use super::*;
use crate::lifecycle::resource::LightSlice;
use std::{collections::HashMap, ops::Range};

const PAGE_BYTES: u64 = 16 * 1024 * 1024;

struct Ranges {
    free: Vec<Range<u64>>,
    live: usize,
}

impl Ranges {
    fn new(len: u64) -> Result<Self, Error> {
        let mut free = Vec::new();
        free.try_reserve_exact(1)?;
        free.push(0..len);
        Ok(Self { free, live: 0 })
    }

    fn take(&mut self, len: u64) -> Result<Option<u64>, Error> {
        let Some(index) = self.free.iter().position(|r| r.end - r.start >= len) else {
            return Ok(None);
        };
        // n 个在途或发布范围最多分割出 n+1 个空洞。预留使完成回收无需分配。
        self.free.try_reserve(self.live + 2 - self.free.len())?;
        let range = &mut self.free[index];
        let offset = range.start;
        range.start += len;
        if range.is_empty() {
            self.free.remove(index);
        }
        self.live += 1;
        Ok(Some(offset))
    }

    fn give(&mut self, mut range: Range<u64>) {
        let mut index = self.free.partition_point(|span| span.start < range.start);
        if index > 0 && self.free[index - 1].end == range.start {
            index -= 1;
            range.start = self.free.remove(index).start;
        }
        if self
            .free
            .get(index)
            .is_some_and(|span| span.start == range.end)
        {
            range.end = self.free.remove(index).end;
        }
        self.free.insert(index, range);
        self.live -= 1;
    }
}

struct Page {
    buffer: Buffer,
    address: u64,
    ranges: Ranges,
}

#[derive(Default)]
pub(super) struct LightPages {
    pages: HashMap<Key, Page>,
}

impl LightPages {
    pub(super) fn forget(&mut self, page: Key) {
        self.pages.remove(&page);
    }

    pub(super) fn release(&mut self, slice: LightSlice) {
        if let Some(page) = self.pages.get_mut(&slice.page.key) {
            page.ranges.give(slice.offset..slice.offset + slice.len);
        }
    }

    pub(super) fn retained(&self) -> usize {
        self.pages.capacity() * (size_of::<Key>() + size_of::<Page>())
            + self
                .pages
                .values()
                .map(|page| page.ranges.free.capacity() * size_of::<Range<u64>>())
                .sum::<usize>()
    }
}

impl Context {
    pub(super) fn light_slice(
        &mut self,
        len: u64,
        pending_bytes: u64,
    ) -> Result<(Key, u64), Error> {
        self.device.ensure_active()?;
        let mut block = None;
        for page in self.light_pages.pages.values_mut() {
            if self.life.active(page.buffer.key).is_ok()
                && let Some(offset) = page.ranges.take(len)?
            {
                block = Some((
                    LightSlice {
                        page: page.buffer,
                        offset,
                        len,
                    },
                    page.address + offset,
                ));
                break;
            }
        }
        let (slice, address) = if let Some(block) = block {
            block
        } else {
            let limit = u64::from(self.max_storage_buffer_range()) & !15;
            let capacity = pending_bytes.min(PAGE_BYTES).min(limit).max(len);
            let mut ranges = Ranges::new(capacity)?;
            let offset = ranges.take(len)?.expect("新页的容量覆盖请求");
            self.light_pages.pages.try_reserve(1)?;
            let page = Buffer::new(self, capacity, false)?;
            let address = page.data(self)?.address(&self.device);
            self.light_pages.pages.insert(
                page.key,
                Page {
                    buffer: page,
                    address,
                    ranges,
                },
            );
            (LightSlice { page, offset, len }, address + offset)
        };
        match self.insert(Data::LightRegion(slice)) {
            Ok(key) => Ok((key, address)),
            Err(error) => {
                self.light_pages.release(slice);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_light_slices_coalesce_without_moving_live_ranges() -> Result<(), Error> {
        let mut ranges = Ranges::new(512)?;
        assert_eq!(ranges.take(128)?, Some(0));
        assert_eq!(ranges.take(128)?, Some(128));
        assert_eq!(ranges.take(128)?, Some(256));
        ranges.give(0..128);
        ranges.give(256..384);
        assert_eq!(ranges.take(288)?, None);
        assert_eq!(ranges.take(256)?, Some(256));
        ranges.give(128..256);
        assert_eq!(ranges.take(256)?, Some(0));
        Ok(())
    }
}
