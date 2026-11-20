//! 顺序命令构建、资源访问和录制；提交与完成归Context所有。
use super::*;

/// 已绑定的计算阶段与输出图像。各阶段顺序执行，阶段之间及首尾都有内存屏障。
/// 首尾同步覆盖外部生产和消费，内部屏障覆盖 compute 阶段对存储资源的读写。
/// 本值持有描述句柄，调用方须在 Context 回收时保留其发布根。
/// 参见 [Vulkan Guide 的计算阶段间同步](https://docs.vulkan.org/guide/latest/synchronization_examples.html)。
pub struct ImagePasses {
    output: Image,
    dispatches: Vec<(ComputePipeline, [u32; 3])>,
}

impl ImagePasses {
    /// 接管调度列表。各管线须属于同一设备，构建方负责绑定输入、工作区与最终输出。
    pub const fn new(output: Image, dispatches: Vec<(ComputePipeline, [u32; 3])>) -> Self {
        Self { output, dispatches }
    }

    /// 序列写入的设备图像，内容在执行完成后可用。
    pub const fn output(&self) -> Image {
        self.output
    }

    /// 每次录制发射的工作组调度次数。
    pub const fn dispatch_count(&self) -> usize {
        self.dispatches.len()
    }

    /// 保活输出、管线及其绑定资源，直至视图撤下。
    pub fn roots(&self) -> impl Iterator<Item = Resource> + '_ {
        std::iter::once(self.output.into())
            .chain(self.dispatches.iter().map(|&(p, _)| Resource::from(p)))
    }

    /// 追加各阶段的调度与屏障，提交和完成由调用方管理。
    /// # Errors
    /// 管线归属或工作组数无效时返回底层错误，须丢弃已追加部分操作的命令。
    pub fn record_into(&self, commands: &mut Commands) -> Result<(), Error> {
        commands.barrier();
        if let Some((&(last, size), previous)) = self.dispatches.split_last() {
            for &(pipeline, groups) in previous {
                commands.dispatch(pipeline, groups)?;
                commands.operations.push(Op::ComputeBarrier);
            }
            commands.dispatch(last, size)?;
            commands.barrier();
        }
        Ok(())
    }
}

/// 顺序命令构建器；未 finish 的命令不访问队列或保留资源。
pub struct Commands {
    pub(super) owner: u64,
    pub(super) operations: Vec<Op>,
    pub(super) max_trace: [u64; 3],
    pub(super) max_groups: [u32; 3],
}
impl Commands {
    /// 开始同设备序列；构建器本身不克隆 owner。
    pub fn new(context: &Context) -> Result<Self, Error> {
        context.device.ensure_active()?;
        let limits = context.device.physical.properties().limits;
        Ok(Self {
            owner: context.owner(),
            operations: Vec::new(),
            max_trace: std::array::from_fn(|i| {
                u64::from(limits.max_compute_work_group_count[i])
                    * u64::from(limits.max_compute_work_group_size[i])
            }),
            max_groups: limits.max_compute_work_group_count,
        })
    }
    /// 发射二维主光线。
    pub fn trace(&mut self, pipeline: RayTracingPipeline, size: [u32; 2]) -> Result<(), Error> {
        self.trace_3d(pipeline, [size[0], size[1], 1])
    }
    /// 发射三维光线；完整 pipeline 范围在 finish 时校验。
    pub fn trace_3d(&mut self, pipeline: RayTracingPipeline, size: [u32; 3]) -> Result<(), Error> {
        if pipeline.key.owner != self.owner || size.contains(&0) {
            return Err(Error::InvalidArgument("光追设备或范围无效"));
        }
        self.operations.push(Op::Trace(pipeline, size));
        Ok(())
    }
    /// 发射 compute 工作组。
    pub fn dispatch(&mut self, pipeline: ComputePipeline, groups: [u32; 3]) -> Result<(), Error> {
        if pipeline.key.owner != self.owner
            || groups.contains(&0)
            || groups
                .into_iter()
                .zip(self.max_groups)
                .any(|(n, max)| n > max)
        {
            return Err(Error::InvalidArgument("compute 设备或范围无效"));
        }
        self.operations.push(Op::Dispatch(pipeline, groups));
        Ok(())
    }
    /// 保活资源及其传递依赖，不登记 host 访问冲突或发出设备操作。
    ///
    /// finish 校验活动状态；提交仍须满足 AS producer 顺序。外部工作实际访问
    /// buffer/image 时应使用 [`Self::access`]；仅保活不阻止其 host 读写。
    /// # Errors
    /// 资源属于其他设备时返回错误。
    pub fn keepalive(&mut self, resource: Resource) -> Result<(), Error> {
        if resource.0.owner != self.owner {
            return Err(Error::InvalidArgument("保活资源属于其他设备"));
        }
        self.operations.push(Op::Keepalive(resource));
        Ok(())
    }
    /// 声明外部工作访问 native buffer/image，并保活资源；不发出设备操作。
    ///
    /// finish 校验活动状态；直到取消或收取完成通知前，拒绝该资源的全部 host 读写。
    /// Trace/Dispatch 自动登记 native binding 的访问，外部 SDK/raw 工作由调用方显式登记。
    /// # Errors
    /// 资源属于其他设备时返回错误。
    pub fn access(&mut self, access: ResourceAccess) -> Result<(), Error> {
        if access.key().owner != self.owner {
            return Err(Error::InvalidArgument("访问资源属于其他设备"));
        }
        self.operations.push(Op::Access(access));
        Ok(())
    }
    /// 同步前后设备访问。
    pub fn barrier(&mut self) {
        self.operations.push(Op::Barrier);
    }
    /// 清空完整 buffer；后续 shader 访问前需 barrier。
    pub fn clear_buffer(&mut self, buffer: Buffer) -> Result<(), Error> {
        if buffer.key.owner != self.owner {
            return Err(Error::InvalidArgument("清零 buffer 属于其他设备"));
        }
        self.operations.push(Op::Clear(buffer));
        Ok(())
    }
    /// 重置完整诊断 pool；同一命令只允许 reset 一次，且此前不能有该 pool 的待执行使用。
    pub fn reset_timestamps(&mut self, queries: TimestampQueries) -> Result<(), Error> {
        if queries.key.owner != self.owner {
            return Err(Error::InvalidArgument("timestamp 属于其他设备"));
        }
        self.operations.push(Op::ResetTimestamps(queries));
        Ok(())
    }
    /// 在 ALL_COMMANDS 完成端写 timestamp；须先在本命令 reset，同一槽只能写一次。
    /// 与相邻 barrier 一起度量设备调度跨度，不能解释为隔离于其他 GPU 工作的耗时。
    pub fn write_timestamp(&mut self, queries: TimestampQueries, index: u32) -> Result<(), Error> {
        if queries.key.owner != self.owner || index >= queries.len() {
            return Err(Error::InvalidArgument("timestamp 设备或索引无效"));
        }
        self.operations.push(Op::Timestamp(queries, index));
        Ok(())
    }
    /// 固化 ID/ABI、保活根及直接资源访问；返回票据由 owner 显式提交或取消。
    pub fn finish(self, context: &mut Context) -> Result<Recorded, Error> {
        self.finish_build(context, Vec::new())
    }

    // 仅构建器将本次独占创建、未关联 producer 的输出传进记录。
    pub(super) fn finish_build(
        self,
        context: &mut Context,
        outputs: Vec<Key>,
    ) -> Result<Recorded, Error> {
        let uses = self.validate(context)?;
        let record = Record::new(self.operations, uses, outputs, BuildBuffers::default());
        let key = context
            .life
            .insert_record(record)
            .map_err(|(error, _)| Error::from(error))?;
        Ok(Recorded { key })
    }

    /// 只读校验完整操作流，返回排序去重的 host 互斥资源；失败不登记记录。
    pub(super) fn validate(&self, context: &Context) -> Result<Vec<Key>, Error> {
        context.device.ensure_active()?;
        if self.owner != context.owner() {
            return Err(Error::InvalidArgument("命令属于其他设备"));
        }
        let mut uses = Vec::new();
        let mut timestamps = BTreeMap::<Key, Vec<bool>>::new();
        for op in &self.operations {
            let binding = match op {
                Op::Trace(p, size) => {
                    context.active(p.key)?;
                    let version = p.data(context)?;
                    if !valid_trace_range(
                        *size,
                        context.ray_program(version.program())?.raw().max_dispatch,
                        self.max_trace,
                    ) {
                        return Err(Error::InvalidArgument("光追范围无效"));
                    }
                    Some(version.bindings())
                }
                Op::Dispatch(p, _) => {
                    context.active(p.key)?;
                    Some(p.data(context)?.bindings())
                }
                Op::Clear(b) => {
                    context.active(b.key)?;
                    b.data(context)?;
                    None
                }
                Op::Keepalive(resource) => {
                    context.active(resource.0)?;
                    None
                }
                Op::Access(access) => {
                    context.active(access.key())?;
                    None
                }
                Op::Barrier | Op::ComputeBarrier => None,
                Op::ResetTimestamps(q) => {
                    context.active(q.key)?;
                    context.resource_idle(q.key)?;
                    q.data(context)?;
                    if timestamps
                        .insert(q.key, vec![false; q.len() as usize])
                        .is_some()
                    {
                        return Err(Error::InvalidArgument("timestamp 重复 reset"));
                    }
                    None
                }
                Op::Timestamp(q, index) => {
                    let slots = timestamps
                        .get_mut(&q.key)
                        .ok_or(Error::InvalidArgument("timestamp 尚未 reset"))?;
                    if std::mem::replace(&mut slots[*index as usize], true) {
                        return Err(Error::InvalidArgument("timestamp 槽重复写入"));
                    }
                    None
                }
            };
            if let Some(key) = op.direct_access() {
                uses.push(key);
            }
            if let Some(binding) = binding {
                for resource in binding.data(context)?.resources() {
                    if let Some(access) = resource.access() {
                        uses.push(access.key());
                    }
                }
            }
        }
        uses.sort_unstable();
        uses.dedup();
        Ok(uses)
    }
}
impl Record {
    pub(super) fn roots(&self, out: &mut Vec<Key>) {
        out.extend_from_slice(self.build_roots());
        out.extend(self.operations().iter().filter_map(Op::root));
    }
    pub(super) fn track(
        &self,
        context: &Context,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), Error> {
        for op in self.operations() {
            match op {
                Op::Trace(p, _) => p.data(context)?.bindings().data(context)?.track(encoder),
                Op::Dispatch(p, _) => p.data(context)?.bindings().data(context)?.track(encoder),
                _ => {}
            }
        }
        Ok(())
    }
    pub(super) fn encode(
        &self,
        context: &Context,
        command: vk::CommandBuffer,
    ) -> Result<(), Error> {
        self.encode_operations(context, command, self.operations())
    }
    pub(super) fn encode_operations(
        &self,
        context: &Context,
        command: vk::CommandBuffer,
        operations: &[Op],
    ) -> Result<(), Error> {
        let device = &context.device;
        unsafe {
            memory_barrier(device, command);
            for op in operations {
                match op {
                    Op::Trace(p, size) => {
                        let version = p.data(context)?;
                        let program = context.ray_program(version.program())?;
                        context.checkpoint(
                            command,
                            "trace/begin",
                            program.raw().pipeline.as_raw(),
                            *size,
                        )?;
                        device.raw.cmd_bind_pipeline(
                            command,
                            vk::PipelineBindPoint::RAY_TRACING_KHR,
                            program.raw().pipeline,
                        );
                        device.raw.cmd_bind_descriptor_sets(
                            command,
                            vk::PipelineBindPoint::RAY_TRACING_KHR,
                            program.raw().layout,
                            0,
                            &[version.bindings().data(context)?.raw().set],
                            &[],
                        );
                        program.raw().loader.cmd_trace_rays(
                            command,
                            &program.raw().regions[0],
                            &program.raw().regions[1],
                            &program.raw().regions[2],
                            &program.raw().regions[3],
                            size[0],
                            size[1],
                            size[2],
                        );
                        context.checkpoint(
                            command,
                            "trace/end",
                            program.raw().pipeline.as_raw(),
                            *size,
                        )?;
                    }
                    Op::Dispatch(p, groups) => {
                        let version = p.data(context)?;
                        let program = context.compute_program(version.program())?;
                        context.checkpoint(
                            command,
                            "dispatch/begin",
                            program.raw().pipeline.as_raw(),
                            *groups,
                        )?;
                        device.raw.cmd_bind_pipeline(
                            command,
                            vk::PipelineBindPoint::COMPUTE,
                            program.raw().pipeline,
                        );
                        device.raw.cmd_bind_descriptor_sets(
                            command,
                            vk::PipelineBindPoint::COMPUTE,
                            program.raw().layout,
                            0,
                            &[version.bindings().data(context)?.raw().set],
                            &[],
                        );
                        device
                            .raw
                            .cmd_dispatch(command, groups[0], groups[1], groups[2]);
                        context.checkpoint(
                            command,
                            "dispatch/end",
                            program.raw().pipeline.as_raw(),
                            *groups,
                        )?;
                    }
                    Op::Keepalive(_) | Op::Access(_) => {}
                    Op::Barrier => memory_barrier(device, command),
                    // 图像计算序列内部只有 compute 读写，外部生产与消费已由首尾屏障覆盖。
                    Op::ComputeBarrier => device.raw.cmd_pipeline_barrier(
                        command,
                        vk::PipelineStageFlags::COMPUTE_SHADER,
                        vk::PipelineStageFlags::COMPUTE_SHADER,
                        vk::DependencyFlags::empty(),
                        &[vk::MemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                            .dst_access_mask(
                                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                            )],
                        &[],
                        &[],
                    ),
                    Op::Clear(b) => {
                        device
                            .raw
                            .cmd_fill_buffer(command, b.data(context)?.raw, 0, b.len, 0)
                    }
                    Op::ResetTimestamps(q) => {
                        let data = q.data(context)?;
                        device
                            .raw
                            .cmd_reset_query_pool(command, data.raw, 0, data.count);
                    }
                    Op::Timestamp(q, index) => {
                        device.raw.cmd_write_timestamp(
                            command,
                            vk::PipelineStageFlags::ALL_COMMANDS,
                            q.data(context)?.raw,
                            *index,
                        );
                    }
                }
            }
            memory_barrier(device, command);
        }
        Ok(())
    }
}
impl Recorded {
    /// 录入外部 command；返回票据对应 owner 内待完成批次。
    /// 失败时取消本 owner 的当前记录；需要保留票据清理整批时使用 [`Self::try_encode`]。
    /// # Safety
    /// 非零 command 属于同设备/队列族且独占录制，最终须提交到 Context 配置的同一 VkQueue。
    /// 宿主保留设备，串行化该队列的宿主访问，并在此批次真实完成后调用 Context::complete；
    /// 同队列顺序保证已提交 AS producer 的依赖，也限定 Context 析构等待的范围。
    /// 失败可能已录入部分命令，该 raw command 不得提交。
    pub unsafe fn encode(self, context: &mut Context, raw_command: u64) -> Result<Flight, Error> {
        match unsafe { self.try_encode(context, raw_command) } {
            Ok(flight) => Ok(flight),
            Err((error, recorded)) => {
                let _ = context.cancel(recorded);
                Err(error)
            }
        }
    }
    /// 录入外部 command；失败时归还原票据，便于取消当前及尚未编码的同批记录。
    /// # Safety
    /// 与 [`Self::encode`] 相同；失败可能已录入部分命令，该 raw command 不得提交。
    pub unsafe fn try_encode(
        self,
        context: &mut Context,
        raw_command: u64,
    ) -> Result<Flight, (Error, Self)> {
        let result = (|| {
            if raw_command == 0 || context.record(self.key)?.state() != State::Pending {
                return Err(Error::InvalidArgument("外部 command 或命令状态无效"));
            }
            context.validate_submission(&[self.key])?;
            context.encode(self.key, vk::CommandBuffer::from_raw(raw_command))?;
            context.life.mark_external(self.key)?;
            Ok(Flight { key: self.key })
        })();
        result.map_err(|error| (error, self))
    }
    /// 初始化/诊断的同步提交；正常逐帧路径不调用。
    pub fn submit_and_wait(self, context: &mut Context) -> Result<(), Error> {
        context.device.ensure_active()?;
        if context.record(self.key)?.state() != State::Pending {
            return Err(Error::InvalidState("命令已编码"));
        }
        let result = (|| {
            context.validate_submission(&[self.key])?;
            context.life.begin_sync(self.key)?;
            if let (Some(device), Some(_)) = (&context.device._owner, &context.device.wgpu_queue) {
                let tracker = context.device.checked(|| {
                    let mut tracker = device.create_command_encoder(&Default::default());
                    context.track(self.key, &mut tracker)?;
                    Ok(tracker.finish())
                })?;
                let commands = crate::interop::record(
                    &context.device,
                    "native synchronous commands",
                    |command| context.encode(self.key, command),
                )?;
                let submission = context.device.submit([tracker, commands])?;
                crate::interop::wait(&context.device, Some(submission))
            } else {
                immediate(&context.device, |command| context.encode(self.key, command))
            }
        })();
        if context.device.health.is_active() {
            let record = context.life.take_sync(self.key, result.is_ok())?;
            context.recycle_record(record, result.is_ok());
            context.retire_ready();
        } else {
            context.life.quarantine_records(&[self.key]);
        }
        result
    }

    /// 原生诊断：在每次 trace/dispatch 后独立提交并等待，保留原操作顺序及资源所有权。
    /// `checkpoint` 接收操作序号、种类、尺寸和完成标志；false 在提交前调用，true 只在
    /// GPU 确认完成后调用。回调不得 panic。额外等待会改变时序，不能用于性能验收。
    /// 只接受外部原生设备上的普通待执行记录，不接受 wgpu 或 AS 构建记录。
    /// 调用方独占同一队列；输入不得依赖尚未提交的外部 command。
    /// 与普通同步提交共用等待失败策略；无法确认队列停止时保留在途资源并返回错误。
    /// # Errors
    /// 归属、记录类型、资源依赖或底层提交/等待失败时返回来源；不会报告未完成操作成功。
    pub fn submit_diagnostic_and_wait(
        self,
        context: &mut Context,
        mut checkpoint: impl FnMut(usize, &'static str, [u32; 3], bool),
    ) -> Result<(), Error> {
        context.device.ensure_active()?;
        let record = context.record(self.key)?;
        if record.state() != State::Pending {
            return Err(Error::InvalidState("命令已编码"));
        }
        let result = (|| {
            if context.device._owner.is_some()
                || !context.record(self.key)?.build_roots().is_empty()
            {
                return Err(Error::InvalidArgument("逐操作诊断需要原生普通待执行记录"));
            }
            context.validate_submission(&[self.key])?;
            context.life.begin_sync(self.key)?;
            let record = context.record(self.key)?;
            let operations = record.operations();
            let mut start = 0;
            for (index, operation) in operations.iter().enumerate() {
                let (kind, extent) = match operation {
                    Op::Trace(_, extent) => ("trace", *extent),
                    Op::Dispatch(_, extent) => ("dispatch", *extent),
                    _ if index + 1 == operations.len() => ("tail", [0; 3]),
                    _ => continue,
                };
                checkpoint(index, kind, extent, false);
                immediate(&context.device, |command| {
                    record.encode_operations(context, command, &operations[start..=index])
                })?;
                checkpoint(index, kind, extent, true);
                start = index + 1;
            }
            Ok(())
        })();
        if context.device.health.is_active() {
            let record = context.life.take_sync(self.key, result.is_ok())?;
            context.recycle_record(record, result.is_ok());
            context.retire_ready();
        } else {
            context.life.quarantine_records(&[self.key]);
        }
        result
    }
}
impl Flight {
    /// 关联 owner 的 native backing allocation 总量。
    pub fn allocated_bytes(&self, context: &Context) -> Result<u64, Error> {
        context.record(self.key)?;
        Ok(context.allocated_bytes())
    }
}
pub(super) unsafe fn memory_barrier(device: &Device, command: vk::CommandBuffer) {
    unsafe {
        device.raw.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::ALL_COMMANDS | vk::PipelineStageFlags::HOST,
            vk::PipelineStageFlags::ALL_COMMANDS | vk::PipelineStageFlags::HOST,
            vk::DependencyFlags::empty(),
            &[vk::MemoryBarrier::default()
                .src_access_mask(
                    vk::AccessFlags::MEMORY_READ
                        | vk::AccessFlags::MEMORY_WRITE
                        | vk::AccessFlags::HOST_WRITE,
                )
                .dst_access_mask(
                    vk::AccessFlags::MEMORY_READ
                        | vk::AccessFlags::MEMORY_WRITE
                        | vk::AccessFlags::HOST_READ,
                )],
            &[],
            &[],
        );
    }
}

pub(super) fn valid_trace_range(size: [u32; 3], max_dispatch: u32, max_axis: [u64; 3]) -> bool {
    !size.contains(&0)
        && size
            .into_iter()
            .zip(max_axis)
            .all(|(side, max)| u64::from(side) <= max)
        && size
            .into_iter()
            .try_fold(1u64, |count, n| count.checked_mul(u64::from(n)))
            .is_some_and(|count| count <= u64::from(max_dispatch))
}
