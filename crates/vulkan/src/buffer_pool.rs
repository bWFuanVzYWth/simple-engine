//! Context 独占的 AS 分配复用；借出时从闲置表移除，完成退役后才归还。
//! input、scratch 各只保留一份最大闲置容量，在途另持有独占分配；backing 按容量匹配和 LRU 回收。

use crate::{Device, Error, buffer::Buffer, idle, owner::Destroy, vk};
use std::collections::VecDeque;

// 实测一批 backing 约 45MiB、临时输入和 scratch 约 26MiB；128MiB 可容纳一轮及容量余量。
const IDLE_LIMIT: u64 = 128 * 1024 * 1024;

/// AS 分配池的累计命中和当前闲置内存；闲置显存已计入 native allocation 总量。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AsPoolStats {
    /// 当前闲置 VkDeviceMemory allocation 字节，不含借出资源。
    pub idle_bytes: u64,
    /// 闲置总量硬上限，超过时同步销毁最旧闲置分配。
    pub idle_limit: u64,
    /// 当前闲置 buffer 数。
    pub idle_buffers: usize,
    /// build input 与 scratch 的累计命中数。
    pub temp_hits: u64,
    /// build input 与 scratch 的累计新分配数。
    pub temp_misses: u64,
    /// AS backing 的累计命中数。
    pub backing_hits: u64,
    /// AS backing 的累计新分配数。
    pub backing_misses: u64,
}

/// AS 构建记录独占的 input/scratch；均已包含在 native allocation 总量中。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AsBuildMemory {
    /// 已录制、尚未提交的实际 allocation 字节。
    pub pending_bytes: u64,
    /// 提交中、已提交、外部编码或故障隔离的实际 allocation 字节。
    pub inflight_bytes: u64,
    /// 尚未提交的临时 buffer 数。
    pub pending_buffers: usize,
    /// 在途或故障隔离的临时 buffer 数；借出期间不计入 pool idle。
    pub inflight_buffers: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Class {
    Input,
    Expanded,
    Scratch,
    Backing,
}

impl Class {
    fn usage(self) -> vk::BufferUsageFlags {
        (match self {
            Self::Input | Self::Expanded => {
                vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
                    | vk::BufferUsageFlags::STORAGE_BUFFER
            }
            Self::Scratch => vk::BufferUsageFlags::STORAGE_BUFFER,
            Self::Backing => vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR,
        }) | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
    }

    fn properties(self) -> vk::MemoryPropertyFlags {
        match self {
            Self::Input => {
                vk::MemoryPropertyFlags::DEVICE_LOCAL
                    | vk::MemoryPropertyFlags::HOST_VISIBLE
                    | vk::MemoryPropertyFlags::HOST_COHERENT
            }
            Self::Expanded | Self::Scratch | Self::Backing => vk::MemoryPropertyFlags::DEVICE_LOCAL,
        }
    }
}

struct Entry {
    class: Class,
    buffer: Buffer,
}

#[derive(Default)]
pub(crate) struct BufferPool {
    idle: VecDeque<Entry>,
    stats: AsPoolStats,
    enabled: bool,
}

impl BufferPool {
    pub(crate) const fn stats(&self) -> AsPoolStats {
        AsPoolStats {
            idle_limit: IDLE_LIMIT,
            idle_buffers: self.idle.len(),
            ..self.stats
        }
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.idle.capacity() * size_of::<Entry>()
    }

    pub(crate) fn take(
        &mut self,
        device: &Device,
        class: Class,
        len: usize,
    ) -> Result<Buffer, Error> {
        device.ensure_active()?;
        self.enabled = true;
        let found = self
            .idle
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.class == class && entry.buffer.len >= len)
            .min_by_key(|(_, entry)| entry.buffer.len)
            .map(|(index, _)| index);
        let hit = found.is_some();
        let buffer = if let Some(index) = found {
            let entry = idle::take(&mut self.idle, index).expect("命中项仍在闲置表中");
            self.stats.idle_bytes -= entry.buffer.allocation_bytes();
            entry.buffer
        } else {
            Buffer::new(device, len, class.usage(), class.properties())?
        };
        let counter = match (class, hit) {
            (Class::Backing, true) => &mut self.stats.backing_hits,
            (Class::Backing, false) => &mut self.stats.backing_misses,
            (_, true) => &mut self.stats.temp_hits,
            (_, false) => &mut self.stats.temp_misses,
        };
        *counter = counter.saturating_add(1);
        Ok(buffer)
    }

    pub(crate) fn give(&mut self, device: &Device, class: Class, mut buffer: Buffer) {
        if device.is_quarantined() {
            std::mem::forget(buffer);
            return;
        }
        let bytes = buffer.allocation_bytes();
        if !self.enabled || bytes > IDLE_LIMIT {
            buffer.destroy(device);
            return;
        }
        if class != Class::Backing
            && let Some(index) = self.idle.iter().position(|entry| entry.class == class)
        {
            if self.idle[index].buffer.len >= buffer.len {
                buffer.destroy(device);
                return;
            }
            let mut entry = idle::take(&mut self.idle, index).expect("同类项仍在闲置表中");
            self.stats.idle_bytes -= entry.buffer.allocation_bytes();
            entry.buffer.destroy(device);
        }
        if self.idle.try_reserve(1).is_err() {
            buffer.destroy(device);
            return;
        }
        while !idle::fits(self.stats.idle_bytes, bytes, IDLE_LIMIT) {
            let mut entry = idle::oldest(&mut self.idle).expect("超限时至少有一个闲置分配");
            self.stats.idle_bytes -= entry.buffer.allocation_bytes();
            entry.buffer.destroy(device);
        }
        self.stats.idle_bytes += bytes;
        idle::give(&mut self.idle, Entry { class, buffer });
    }
}

impl Destroy for BufferPool {
    fn destroy(&mut self, device: &Device) {
        for mut entry in self.idle.drain(..) {
            entry.buffer.destroy(device);
        }
        self.stats.idle_bytes = 0;
    }
}
