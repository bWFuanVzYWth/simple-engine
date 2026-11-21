//! native缓冲的映射、范围校验和设备内复制；资源由Context唯一保活。
use super::*;

impl Buffer {
    /// 创建非空4字节对齐 buffer，不初始化内容。
    pub fn new(context: &mut Context, len: u64, host_visible: bool) -> Result<Self, Error> {
        let properties = if host_visible {
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
        } else {
            vk::MemoryPropertyFlags::DEVICE_LOCAL
        };
        Self::allocate(context, len, properties, host_visible)
    }
    /// 初始化 coherent DEVICE_LOCAL 只读输入，不回退系统 heap。
    pub fn upload(context: &mut Context, bytes: &[u8]) -> Result<Self, Error> {
        let result = Self::allocate(
            context,
            bytes.len() as u64,
            vk::MemoryPropertyFlags::DEVICE_LOCAL
                | vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT,
            false,
        )?;
        if let Err(error) = result.data(context)?.write(&context.device, 0, bytes) {
            context.discard(result.key);
            return Err(error);
        }
        Ok(result)
    }
    /// 创建可复用的coherent DEVICE_LOCAL属性存储，不初始化内容，也不回退系统heap。
    ///
    /// 所有可被shader读取的范围须在发布绑定前写入；未写槽只能作为不可达的保留容量。
    /// 后续写入同[`Self::write_ranges`]，拒绝待执行或在途GPU使用。
    pub fn mapped_device_local(context: &mut Context, len: u64) -> Result<Self, Error> {
        Self::allocate(
            context,
            len,
            vk::MemoryPropertyFlags::DEVICE_LOCAL
                | vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT,
            true,
        )
    }
    pub(super) fn allocate(
        context: &mut Context,
        len: u64,
        properties: vk::MemoryPropertyFlags,
        host_visible: bool,
    ) -> Result<Self, Error> {
        if len == 0 || !len.is_multiple_of(4) || len > u64::from(context.max_storage_buffer_range())
        {
            return Err(Error::InvalidArgument("native storage buffer 大小无效"));
        }
        let usage = vk::BufferUsageFlags::STORAGE_BUFFER
            | vk::BufferUsageFlags::INDIRECT_BUFFER
            | vk::BufferUsageFlags::UNIFORM_BUFFER
            | vk::BufferUsageFlags::TRANSFER_SRC
            | vk::BufferUsageFlags::TRANSFER_DST
            | if context.device.ray_pipeline {
                vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                    | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
            } else {
                vk::BufferUsageFlags::empty()
            };
        let inner = buffer::Buffer::new(
            &context.device,
            usize::try_from(len).map_err(|_| Error::InvalidArgument("buffer 地址溢出"))?,
            usage,
            properties,
        )?;
        Ok(Self {
            key: context.insert(Data::Buffer(inner))?,
            len,
            host_visible,
        })
    }
    pub(super) fn data<'a>(&self, context: &'a Context) -> Result<&'a buffer::Buffer, Error> {
        match context.get(self.key)? {
            Data::Buffer(data) => Ok(data),
            _ => Err(Error::InvalidArgument("buffer 标识类型无效")),
        }
    }
    /// 完整字节数。
    pub const fn len(&self) -> u64 {
        self.len
    }
    /// 构造拒绝空 buffer。
    pub const fn is_empty(&self) -> bool {
        false
    }
    pub(super) fn validate(&self, context: &Context, offset: u64, len: usize) -> Result<(), Error> {
        context.active(self.key)?;
        self.data(context)?;
        if !self.host_visible
            || offset
                .checked_add(len as u64)
                .is_none_or(|end| end > self.len)
        {
            return Err(Error::InvalidArgument("buffer 映射类型或范围无效"));
        }
        context.resource_idle(self.key)
    }
    /// 写 coherent 映射；待执行或在途使用结束前拒绝。
    pub fn write(&self, context: &mut Context, offset: u64, bytes: &[u8]) -> Result<(), Error> {
        self.validate(context, offset, bytes.len())?;
        self.data(context)?
            .write(&context.device, offset as usize, bytes)
    }
    /// 在同一设备内同步复制前缀。两个缓冲须独立且无待执行或在途使用。
    /// `bytes` 须为非零的四字节倍数，且不超过任一缓冲容量。
    pub fn copy_prefix_to(
        &self,
        context: &mut Context,
        target: &Self,
        bytes: u64,
    ) -> Result<(), Error> {
        if self.key == target.key
            || bytes == 0
            || !bytes.is_multiple_of(4)
            || bytes > self.len.min(target.len)
        {
            return Err(Error::InvalidArgument("buffer 复制范围无效"));
        }
        context.active(self.key)?;
        context.active(target.key)?;
        context.resource_idle(self.key)?;
        context.resource_idle(target.key)?;
        let source = self.data(context)?.raw;
        let target = target.data(context)?.raw;
        immediate(&context.device, |command| {
            // 两个独立分配已验证范围与空闲状态，且均包含传输用途。
            unsafe {
                memory_barrier(&context.device, command);
                context.device.raw.cmd_copy_buffer(
                    command,
                    source,
                    target,
                    &[vk::BufferCopy::default().size(bytes)],
                );
                memory_barrier(&context.device, command);
            }
            Ok(())
        })
    }
    /// 全部范围及GPU空闲状态校验后只映射一次，再按输入顺序复制。
    ///
    /// 任一校验或映射失败不写入任何范围；成功返回后全部写入可见。重叠范围按顺序覆盖。
    /// 输入只在调用中借用，不改变buffer容量、绑定版本或GPU所有权。
    pub fn write_ranges(
        &self,
        context: &mut Context,
        writes: &[(usize, &[u8])],
    ) -> Result<(), Error> {
        for &(offset, bytes) in writes {
            self.validate(context, offset as u64, bytes.len())?;
        }
        self.data(context)?.write_slices(&context.device, writes)
    }
    /// 全部目标校验和映射成功后才复制；任一失败不写入任何目标。
    pub fn write_batch(context: &mut Context, writes: &[(&Self, u64, &[u8])]) -> Result<(), Error> {
        let mut targets = BTreeMap::new();
        for &(buffer, offset, bytes) in writes {
            buffer.validate(context, offset, bytes.len())?;
            if targets.insert(buffer.key, buffer).is_some() {
                return Err(Error::InvalidArgument("批量映射 buffer 重复"));
            }
        }
        struct Mapping<'a> {
            device: &'a Device,
            memory: vk::DeviceMemory,
            pointer: *mut u8,
        }
        impl Drop for Mapping<'_> {
            fn drop(&mut self) {
                unsafe {
                    self.device.raw.unmap_memory(self.memory);
                }
            }
        }
        let mut mappings = BTreeMap::new();
        for (key, buffer) in targets {
            let data = buffer.data(context)?;
            let pointer = unsafe {
                context.device.raw.map_memory(
                    data.memory,
                    0,
                    buffer.len,
                    vk::MemoryMapFlags::empty(),
                )?
            };
            mappings.insert(
                key,
                Mapping {
                    device: &context.device,
                    memory: data.memory,
                    pointer: pointer.cast(),
                },
            );
        }
        for &(buffer, offset, bytes) in writes {
            let mapping = &mappings[&buffer.key];
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    mapping.pointer.add(offset as usize),
                    bytes.len(),
                );
            }
        }
        Ok(())
    }
    /// 多个独立buffer的离散范围共同写入；全部校验和映射成功后才复制。
    ///
    /// 每个buffer只出现一次，内部范围可重叠并按顺序覆盖。任何输入、空闲状态或映射
    /// 失败均保留所有目标原内容；成功后调用者可共同发布关联的CPU目录。
    pub fn write_scattered_batches(
        context: &mut Context,
        batches: &[(&Self, &BufferWriteRanges<'_>)],
    ) -> Result<(), Error> {
        let mut targets = BTreeMap::new();
        for &(buffer, ranges) in batches {
            if ranges.is_empty() {
                continue;
            }
            buffer.validate(context, 0, 0)?;
            for &(offset, bytes) in ranges {
                if offset
                    .checked_add(bytes.len())
                    .is_none_or(|end| end as u64 > buffer.len)
                {
                    return Err(Error::InvalidArgument("离散批量映射范围无效"));
                }
            }
            if targets.insert(buffer.key, buffer).is_some() {
                return Err(Error::InvalidArgument("离散批量映射buffer重复"));
            }
        }
        struct Mapping<'a> {
            device: &'a Device,
            memory: vk::DeviceMemory,
            pointer: *mut u8,
        }
        impl Drop for Mapping<'_> {
            fn drop(&mut self) {
                // SAFETY: 此所有者仅在映射成功后建立，并在独占context调用结束前解除映射。
                unsafe { self.device.raw.unmap_memory(self.memory) };
            }
        }
        let mut mappings = BTreeMap::new();
        for (key, buffer) in targets {
            let data = buffer.data(context)?;
            // SAFETY: 完整范围已校验，所有GPU使用已结束；同一memory只由独占buffer映射一次。
            let pointer = unsafe {
                context.device.raw.map_memory(
                    data.memory,
                    0,
                    buffer.len,
                    vk::MemoryMapFlags::empty(),
                )?
            };
            mappings.insert(
                key,
                Mapping {
                    device: &context.device,
                    memory: data.memory,
                    pointer: pointer.cast(),
                },
            );
        }
        for &(buffer, ranges) in batches {
            if ranges.is_empty() {
                continue;
            }
            let mapping = &mappings[&buffer.key];
            for &(offset, bytes) in ranges {
                // SAFETY: 全部映射均成功且范围已校验；源借用在调用内保持有效、不与目标别名。
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        mapping.pointer.add(offset),
                        bytes.len(),
                    );
                }
            }
        }
        Ok(())
    }
    /// 回读 coherent 映射；待执行或在途使用结束前拒绝。
    pub fn read(&self, context: &mut Context, offset: u64, bytes: &mut [u8]) -> Result<(), Error> {
        self.validate(context, offset, bytes.len())?;
        if !bytes.is_empty() {
            let data = self.data(context)?;
            unsafe {
                let pointer = context.device.raw.map_memory(
                    data.memory,
                    0,
                    self.len,
                    vk::MemoryMapFlags::empty(),
                )?;
                std::ptr::copy_nonoverlapping(
                    pointer.cast::<u8>().add(offset as usize),
                    bytes.as_mut_ptr(),
                    bytes.len(),
                );
                context.device.raw.unmap_memory(data.memory);
            }
        }
        Ok(())
    }
}
