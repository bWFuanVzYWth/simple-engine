//! 独占原生队列的地形命令。提交前失败释放候选，成功只凭 fence 完成证明退役。
use super::*;

pub(super) struct Recording {
    pool: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
}
impl Recording {
    pub(super) fn record(
        device: &Device,
        encode: impl FnOnce(vk::CommandBuffer) -> Result<(), Error>,
    ) -> Result<Self, Error> {
        if device._owner.is_some() {
            return Err(Error::Unsupported("原生地形提交需要独占宿主队列"));
        }
        let pool = unsafe {
            device.raw.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(device.queue_family)
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                None,
            )?
        };
        let mut recording = Guard::new(
            device,
            Self {
                pool,
                command: vk::CommandBuffer::null(),
                fence: vk::Fence::null(),
            },
        );
        unsafe {
            recording.command = device.raw.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            recording.fence = device
                .raw
                .create_fence(&vk::FenceCreateInfo::default(), None)?;
            device.raw.begin_command_buffer(
                recording.command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        encode(recording.command)?;
        Ok(recording.finish())
    }
}
impl Destroy for Recording {
    fn destroy(&mut self, device: &Device) {
        // SAFETY: 只处理未提交记录或已取得完成证明的在途记录。
        unsafe {
            device.raw.destroy_fence(self.fence, None);
            device.raw.destroy_command_pool(self.pool, None);
        }
    }
}
pub(super) struct Submitted {
    recording: Recording,
    flight: Flight,
    buffers: [BuildBuffers; 2],
}
impl Submitted {
    pub(super) fn memory(&self) -> (u64, usize) {
        self.buffers
            .iter()
            .map(BuildBuffers::memory)
            .fold((0, 0), |(bytes, count), (b, n)| (bytes + b, count + n))
    }
    pub(super) fn release(mut self, device: &Device, pool: &mut BufferPool) {
        self.recording.destroy(device);
        for b in self.buffers {
            b.recycle(device, pool);
        }
    }
}
impl Context {
    /// 从按预算传好的单一源 buffer 发布多个不重叠的设备补丁，元素依次为目标、源偏移、目标偏移、大小。
    /// # Errors
    /// 拒绝别名、非 word 对齐、越界或重叠写入，分配及提交错误逐层返回。
    pub fn enqueue_buffer_ranges(
        &mut self,
        source: Buffer,
        ranges: &[(Buffer, u64, u64, u64)],
    ) -> Result<(), Error> {
        if ranges.is_empty() {
            return Ok(());
        }
        let input = source.data(self)?.raw;
        let mut copies = Vec::new();
        let mut spans = Vec::new();
        copies.try_reserve_exact(ranges.len())?;
        spans.try_reserve_exact(ranges.len())?;
        for &(dst, from, to, bytes) in ranges {
            if source == dst
                || bytes == 0
                || (from | to | bytes) & 3 != 0
                || from.checked_add(bytes).is_none_or(|v| v > source.len())
                || to.checked_add(bytes).is_none_or(|v| v > dst.len())
            {
                return Err(Error::InvalidArgument("已上传补丁范围无效"));
            }
            spans.push((dst.key, to, to + bytes));
            copies.push((
                dst.data(self)?.raw,
                vk::BufferCopy::default()
                    .src_offset(from)
                    .dst_offset(to)
                    .size(bytes),
            ));
        }
        spans.sort_unstable();
        if spans
            .windows(2)
            .any(|v| v[0].0 == v[1].0 && v[0].2 > v[1].1)
        {
            return Err(Error::InvalidArgument("设备补丁重叠"));
        }
        let raw = Recording::record(&self.device, |cmd| {
            unsafe {
                memory_barrier(&self.device, cmd);
                for &(dst, copy) in &copies {
                    self.device.raw.cmd_copy_buffer(cmd, input, dst, &[copy]);
                }
                memory_barrier(&self.device, cmd);
            }
            Ok(())
        })?;
        self.submit_terrain(raw, Default::default(), |op| {
            op.access(source.into())?;
            for &(dst, _, _, _) in ranges {
                op.access(dst.into())?;
            }
            Ok(())
        })
    }
    /// 在原生队列复制独立缓冲的前缀，允许旧场景仍在 GPU 使用它。按提交顺序观察结果。
    /// # Errors
    /// 所有者、字节范围、分配及设备错误均在返回值中报告。
    pub fn enqueue_copy(&mut self, src: Buffer, dst: Buffer, bytes: u64) -> Result<(), Error> {
        if src == dst || bytes == 0 || !bytes.is_multiple_of(4) || bytes > src.len().min(dst.len())
        {
            return Err(Error::InvalidArgument("设备前缀复制范围无效"));
        }
        let a = src.data(self)?.raw;
        let b = dst.data(self)?.raw;
        let raw = Recording::record(&self.device, |cmd| {
            unsafe {
                memory_barrier(&self.device, cmd);
                self.device.raw.cmd_copy_buffer(
                    cmd,
                    a,
                    b,
                    &[vk::BufferCopy::default().size(bytes)],
                );
                memory_barrier(&self.device, cmd);
            }
            Ok(())
        })?;
        self.submit_terrain(raw, Default::default(), |op| {
            op.access(src.into())?;
            op.access(dst.into())
        })
    }

    pub(super) fn submit_terrain(
        &mut self,
        mut raw: Recording,
        buffers: [BuildBuffers; 2],
        roots: impl FnOnce(&mut Commands) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let result = (|| {
            self.terrain_flights.try_reserve(1)?;
            let mut op = Commands::new(self)?;
            roots(&mut op)?;
            let record = op.finish(self)?;
            // SAFETY: 私有 command 属于同一设备队列；此 owner 接管唯一完成票据。
            let flight = unsafe { record.encode(self, raw.command.as_raw())? };
            let submit = unsafe {
                self.device
                    .raw
                    .end_command_buffer(raw.command)
                    .and_then(|()| {
                        self.device.raw.queue_submit(
                            self.device.queue,
                            &[vk::SubmitInfo::default().command_buffers(&[raw.command])],
                            raw.fence,
                        )
                    })
            };
            if let Err(error) = submit {
                // 录制结束失败或单批未被队列接受；设备丢失也会终止使用。
                unsafe {
                    self.complete(flight)?;
                }
                if error == vk::Result::ERROR_DEVICE_LOST {
                    self.device.health.fail(crate::DeviceFailure::Lost);
                }
                return Err(error.into());
            }
            Ok(flight)
        })();
        match result {
            Ok(flight) => {
                self.terrain_flights.push(Submitted {
                    recording: raw,
                    flight,
                    buffers,
                });
                Ok(())
            }
            Err(e) => {
                if !self.device.is_quarantined() {
                    raw.destroy(&self.device);
                    for b in buffers {
                        b.recycle(&self.device, &mut self.as_pool);
                    }
                }
                Err(e)
            }
        }
    }
    /// 非阻塞收取原生地形任务的完成通知。未完成的资源仍归当前 Context 独占。
    /// # Errors
    /// fence 查询或设备故障保留在途任务。
    pub fn poll_terrain(&mut self) -> Result<(), Error> {
        self.device.ensure_active()?;
        let mut i = 0;
        while i < self.terrain_flights.len() {
            if !unsafe {
                self.device
                    .raw
                    .get_fence_status(self.terrain_flights[i].recording.fence)?
            } {
                i += 1;
                continue;
            }
            let Submitted {
                mut recording,
                flight,
                buffers,
            } = self.terrain_flights.swap_remove(i);
            unsafe {
                self.complete(flight)?;
            }
            recording.destroy(&self.device);
            for b in buffers {
                b.recycle(&self.device, &mut self.as_pool);
            }
        }
        Ok(())
    }
}
