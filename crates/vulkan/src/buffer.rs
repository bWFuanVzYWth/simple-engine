use crate::owner::{Destroy, Guard};
use crate::{Device, Error, vk};
use ash::vk::TaggedStructure as _;
use std::sync::atomic::Ordering;

pub(crate) struct Buffer {
    pub(crate) raw: vk::Buffer,
    pub(crate) memory: vk::DeviceMemory,
    pub(crate) len: usize,
    allocated_bytes: u64,
}

impl Buffer {
    pub(crate) const fn allocation_bytes(&self) -> u64 {
        self.allocated_bytes
    }

    pub(crate) fn new(
        device: &Device,
        len: usize,
        usage: vk::BufferUsageFlags,
        properties: vk::MemoryPropertyFlags,
    ) -> Result<Self, Error> {
        device.ensure_active()?;
        let mut buffer = Guard::new(
            device,
            Self {
                raw: vk::Buffer::null(),
                memory: vk::DeviceMemory::null(),
                len,
                allocated_bytes: 0,
            },
        );
        let create = vk::BufferCreateInfo::default()
            .size(len as u64)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        buffer.raw = unsafe { device.raw.create_buffer(&create, None)? };
        let required = unsafe { device.raw.get_buffer_memory_requirements(buffer.raw) };
        let memory = unsafe {
            device
                .physical
                .instance
                .raw
                .get_physical_device_memory_properties(device.physical.raw)
        };
        let memory_type = (0..memory.memory_type_count)
            .find(|&index| {
                required.memory_type_bits & (1 << index) != 0
                    && memory.memory_types[index as usize]
                        .property_flags
                        .contains(properties)
            })
            .ok_or(Error::Unsupported("设备没有满足 buffer 要求的内存类型"))?;
        let mut address =
            vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
        let mut allocate = vk::MemoryAllocateInfo::default()
            .allocation_size(required.size)
            .memory_type_index(memory_type);
        if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
            allocate = allocate.push(&mut address);
        }
        buffer.memory = unsafe { device.raw.allocate_memory(&allocate, None)? };
        buffer.allocated_bytes = required.size;
        device
            .buffer_bytes
            .fetch_add(required.size, Ordering::Relaxed);
        unsafe {
            device
                .raw
                .bind_buffer_memory(buffer.raw, buffer.memory, 0)?
        };
        Ok(buffer.finish())
    }

    pub(crate) fn address(&self, device: &Device) -> vk::DeviceAddress {
        unsafe {
            device
                .raw
                .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(self.raw))
        }
    }

    /// CPU 直接写入 coherent 可映射显存；不回退系统 heap，返回对齐后的设备地址。
    pub(crate) fn upload(
        device: &Device,
        bytes: &[u8],
        usage: vk::BufferUsageFlags,
        alignment: usize,
    ) -> Result<(Self, vk::DeviceAddress), Error> {
        let buffer = Guard::new(
            device,
            Self::new(
                device,
                bytes
                    .len()
                    .checked_add(alignment - 1)
                    .ok_or(Error::InvalidArgument("buffer 大小溢出"))?,
                usage,
                vk::MemoryPropertyFlags::DEVICE_LOCAL
                    | vk::MemoryPropertyFlags::HOST_VISIBLE
                    | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?,
        );
        let address = buffer.address(device);
        let offset = (alignment - address as usize % alignment) % alignment;
        buffer.write(device, offset, bytes)?;
        Ok((buffer.finish(), address + offset as u64))
    }

    pub(crate) fn write(&self, device: &Device, offset: usize, bytes: &[u8]) -> Result<(), Error> {
        if !bytes.is_empty() {
            unsafe {
                let mapped = device.raw.map_memory(
                    self.memory,
                    0,
                    self.len as u64,
                    vk::MemoryMapFlags::empty(),
                )?;
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    mapped.cast::<u8>().add(offset),
                    bytes.len(),
                );
                device.raw.unmap_memory(self.memory);
            }
        }
        Ok(())
    }

    /// 校验全部范围后只映射一次；输入是独立 CPU 切片，调用期间不与映射区别名。
    pub(crate) fn write_slices(
        &self,
        device: &Device,
        writes: &[(usize, &[u8])],
    ) -> Result<(), Error> {
        for &(offset, bytes) in writes {
            if offset
                .checked_add(bytes.len())
                .is_none_or(|end| end > self.len)
            {
                return Err(Error::InvalidArgument("buffer 写入范围越界"));
            }
        }
        if writes.iter().all(|(_, bytes)| bytes.is_empty()) {
            return Ok(());
        }
        // 范围已校验；所有源切片来自 CPU 输入，映射期间没有其他可失败调用。
        unsafe {
            let mapped = device.raw.map_memory(
                self.memory,
                0,
                self.len as u64,
                vk::MemoryMapFlags::empty(),
            )?;
            for &(offset, bytes) in writes {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    mapped.cast::<u8>().add(offset),
                    bytes.len(),
                );
            }
            device.raw.unmap_memory(self.memory);
        }
        Ok(())
    }
}

impl Destroy for Buffer {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_buffer(self.raw, None);
            device.raw.free_memory(self.memory, None);
        }
        device
            .buffer_bytes
            .fetch_sub(self.allocated_bytes, Ordering::Relaxed);
    }
}
