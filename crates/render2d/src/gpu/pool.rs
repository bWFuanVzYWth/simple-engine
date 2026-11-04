use super::{Budgets, RenderError};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Kind {
    Geometry,
    View,
}

struct Slot {
    buffer: wgpu::Buffer,
    bytes: u64,
    kind: Kind,
    busy: bool,
}

#[derive(Default)]
pub(super) struct BufferPool {
    slots: Vec<Option<Slot>>,
    bytes: u64,
}

impl BufferPool {
    pub fn acquire(
        &mut self,
        device: &wgpu::Device,
        bytes: u64,
        kind: Kind,
        budgets: &Budgets,
    ) -> Result<usize, RenderError> {
        if bytes > budgets.max_buffer_bytes || bytes > device.limits().max_buffer_size {
            return Err(RenderError::Budget);
        }
        let reserve = if kind == Kind::Geometry
            && !self
                .slots
                .iter()
                .flatten()
                .any(|slot| slot.kind == Kind::View && !slot.busy)
        {
            super::abi::VIEW_SIZE
        } else {
            0
        };
        let growth_limit = budgets.max_buffer_bytes.saturating_sub(reserve);
        let reuse_capacity = self.bytes <= growth_limit;
        if let Some((index, slot)) = self
            .slots
            .iter_mut()
            .enumerate()
            .filter_map(|(i, s)| s.as_mut().map(|s| (i, s)))
            .find(|(_, slot)| {
                slot.kind == kind
                    && slot.bytes >= bytes
                    && !slot.busy
                    && (reuse_capacity || slot.bytes == bytes)
            })
        {
            slot.busy = true;
            return Ok(index);
        }
        if let Some(index) = self.slots.iter().position(|slot| {
            slot.as_ref()
                .is_some_and(|slot| slot.kind == kind && !slot.busy)
        }) {
            self.bytes -= self.slots[index].take().expect("存在槽").bytes;
        }
        // 几何按档位增长，为随后一次 prepare_view 留空间；预算紧张时退回精确需求。
        let capacity = if kind == Kind::Geometry {
            bytes
                .checked_next_power_of_two()
                .filter(|capacity| {
                    *capacity <= device.limits().max_buffer_size
                        && *capacity <= growth_limit.saturating_sub(self.bytes)
                })
                .unwrap_or(bytes)
        } else {
            bytes
        };
        while capacity > budgets.max_buffer_bytes.saturating_sub(self.bytes)
            || self.len() >= budgets.max_buffer_slots
        {
            let Some(index) = self
                .slots
                .iter()
                .position(|slot| slot.as_ref().is_some_and(|slot| !slot.busy))
            else {
                return Err(RenderError::Busy);
            };
            self.bytes -= self.slots[index].take().expect("存在槽").bytes;
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("2D 上传槽"),
            size: capacity,
            usage: wgpu::BufferUsages::COPY_DST
                | match kind {
                    Kind::Geometry => wgpu::BufferUsages::STORAGE,
                    Kind::View => wgpu::BufferUsages::UNIFORM,
                },
            mapped_at_creation: false,
        });
        let slot = Slot {
            buffer,
            bytes: capacity,
            kind,
            busy: true,
        };
        let index = if let Some(index) = self.slots.iter().position(Option::is_none) {
            self.slots[index] = Some(slot);
            index
        } else {
            self.slots.push(Some(slot));
            self.slots.len() - 1
        };
        self.bytes += capacity;
        Ok(index)
    }

    pub fn buffer(&self, index: usize) -> &wgpu::Buffer {
        &self.slots[index].as_ref().expect("活动槽").buffer
    }
    pub fn release(&mut self, index: usize) {
        self.slots[index].as_mut().expect("活动槽").busy = false;
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }
}
