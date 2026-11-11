//! 从四顶点驻留列并行生成采样树；各层有显式设备依赖，正反向选择共享同一棵树。
use super::*;

/// 新建的灯树及可选的四角发光面记录。与生产它们的命令共同发布。
#[derive(Clone, Copy)]
pub struct DeviceLightTree {
    /// 与 light_bvh 查询 ABI 相同的 sibling packets 和叶目录，从 word 0 开始。
    pub tree: Buffer,
    /// 请求时每灯 64B（面积提案、材质、四角）。顶点按传入 anchor 平移。
    pub records: Option<Buffer>,
}

struct Build {
    output: DeviceLightTree,
    sources: Buffer,
    vertices: Buffer,
    count: u32,
    scratch: Option<Buffer>,
    anchor: [i32; 3],
}
impl Build {
    fn new(
        context: &mut Context,
        vertices: Buffer,
        leaves: &[[u32; 4]],
        anchor: [i32; 3],
        records: bool,
        source: Option<Buffer>,
    ) -> Result<Self, Error> {
        let count = u32::try_from(leaves.len()).map_err(|_| Error::InvalidArgument("灯数超限"))?;
        if count > 1 << 24 {
            return Err(Error::InvalidArgument("灯数超出24位采样域"));
        }
        context.active(vertices.key)?;
        let mut power = 0f64;
        for &[q, p, _, _] in leaves {
            let p = f32::from_bits(p);
            if (u64::from(q) + 1) * 64 > vertices.len() || !p.is_finite() || p <= 0.0 {
                return Err(Error::InvalidArgument("光源顶点槽或功率无效"));
            }
            power += f64::from(p);
        }
        if power > f64::from(f32::MAX) {
            return Err(Error::InvalidArgument("光源总功率溢出"));
        }
        let count64 = u64::from(count);
        let words = (count64.saturating_sub(1) * 10).next_multiple_of(8) + count64 * 8;
        let tree = Buffer::new(context, (words * 4).max(16), false)?;
        let records = records
            .then(|| Buffer::new(context, (count64 * 64).max(64), false))
            .transpose()?;
        let sources = if let Some(buffer) = source {
            context.active(buffer.key)?;
            if buffer.len() < count64 * 16 {
                return Err(Error::InvalidArgument("发光活动列范围不足"));
            }
            buffer
        } else if leaves.is_empty() {
            vertices
        } else {
            Buffer::upload(context, bytemuck::cast_slice(leaves))?
        };
        let scratch = if count > 1 {
            Some(Buffer::new(
                context,
                count64 * 16 * if count > 64 { 2 } else { 1 },
                false,
            )?)
        } else {
            None
        };
        Ok(Self {
            scratch,
            output: DeviceLightTree { tree, records },
            vertices,
            sources,
            count,
            anchor,
        })
    }
    fn buffers(&self) -> impl Iterator<Item = Buffer> {
        [
            Some(self.vertices),
            Some(self.sources),
            Some(self.output.tree),
            self.output.records,
            self.scratch,
        ]
        .into_iter()
        .flatten()
    }
    fn record(&self, context: &Context, cmd: vk::CommandBuffer) -> Result<(), Error> {
        if self.count == 0 {
            return Ok(());
        }
        let program = context
            .light_builder
            .as_ref()
            .ok_or(Error::InvalidState("灯树程序未准备"))?;
        let device = &context.device;
        let address = |b: Buffer| -> Result<u64, Error> { Ok(b.data(context)?.address(device)) };
        let pointers = [
            address(self.vertices)?,
            address(self.sources)?,
            address(self.output.tree)?,
            self.output.records.map(address).transpose()?.unwrap_or(0),
        ];
        let mut push = [0u8; 88];
        for (dst, p) in push[..32].as_chunks_mut::<8>().0.iter_mut().zip(pointers) {
            dst.copy_from_slice(&p.to_ne_bytes());
        }
        push[32..36].copy_from_slice(&self.count.to_ne_bytes());
        push[44..48].copy_from_slice(&u32::from(self.output.records.is_some()).to_ne_bytes());
        for (dst, k) in push[48..60]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(self.anchor)
        {
            dst.copy_from_slice(&k.to_ne_bytes());
        }
        push[60..64].copy_from_slice(&((self.vertices.len() / 64) as u32).to_ne_bytes());
        let highest = self.count.next_power_of_two().ilog2();
        let max_groups = device
            .physical
            .properties()
            .limits
            .max_compute_work_group_count[0];
        // 输入顶点可由之前的 BLAS 批次写出，宿主叶表已完成 coherent 写入。
        unsafe { memory_barrier(device, cmd) };
        let dispatch = |push: &mut [u8; 88], size: u32| {
            let mut first = 0u32;
            while first < size {
                let groups = (size - first).div_ceil(64).min(max_groups);
                push[40..44].copy_from_slice(&first.to_ne_bytes());
                program.dispatch(device, cmd, push, groups);
                first += groups * 64;
            }
            unsafe {
                memory_barrier(device, cmd);
            }
        };
        if let Some(scratch) = self.scratch {
            let mut sorted = address(scratch)?;
            let mut other = sorted + u64::from(self.count) * 16;
            push[72..80].copy_from_slice(&sorted.to_ne_bytes());
            push[80..84].copy_from_slice(&1u32.to_ne_bytes());
            dispatch(&mut push, self.count);
            let mut width = 64u32;
            while width < self.count {
                push[64..72].copy_from_slice(&sorted.to_ne_bytes());
                push[72..80].copy_from_slice(&other.to_ne_bytes());
                push[80..84].copy_from_slice(&2u32.to_ne_bytes());
                push[84..88].copy_from_slice(&width.to_ne_bytes());
                dispatch(&mut push, self.count);
                std::mem::swap(&mut sorted, &mut other);
                width *= 2;
            }
            push[64..72].copy_from_slice(&sorted.to_ne_bytes());
            push[80..84].fill(0);
        }
        for level in (0..=highest).rev() {
            push[36..40].copy_from_slice(&level.to_ne_bytes());
            dispatch(&mut push, 1u32 << level);
        }
        Ok(())
    }
}
impl Context {
    /// 异步向原生宿主队列提交灯树。可直接排在生成顶点的地形命令之后。
    /// # Errors
    /// 输入、容量、设备或队列失败保留已发布树。
    pub fn enqueue_light_tree(
        &mut self,
        vertices: Buffer,
        leaves: &[[u32; 4]],
        anchor: [i32; 3],
        records: bool,
    ) -> Result<DeviceLightTree, Error> {
        self.enqueue_light_tree_from(vertices, leaves, anchor, records, None)
    }
    /// 复用已经按预算传输的活动叶列。缓冲的四个 word 与 leaves 同序，跨提交保持只读。
    /// # Errors
    /// 与 [`Self::enqueue_light_tree`] 相同，设备缓冲须完整覆盖叶表。
    pub fn enqueue_light_tree_from(
        &mut self,
        vertices: Buffer,
        leaves: &[[u32; 4]],
        anchor: [i32; 3],
        records: bool,
        source: Option<Buffer>,
    ) -> Result<DeviceLightTree, Error> {
        let build = Build::new(self, vertices, leaves, anchor, records, source)?;
        let raw =
            super::terrain_flight::Recording::record(&self.device, |cmd| build.record(self, cmd))?;
        self.submit_terrain(raw, Default::default(), |op| {
            for b in build.buffers() {
                op.access(b.into())?;
            }
            Ok(())
        })?;
        Ok(build.output)
    }
    /// 由驻留四角及活动发光槽构树。每行依次为 quad 槽、正功率的 FP32 bits、表面 word、保留字。
    /// 仅上传活动槽，树及记录由 GPU 写出。原生同步宿主返回时资源可直接读取。
    /// # Errors
    /// 底层设备、容量、非有限功率和越界槽均返回错误。
    pub fn build_light_tree(
        &mut self,
        vertices: Buffer,
        leaves: &[[u32; 4]],
        anchor: [i32; 3],
        records: bool,
    ) -> Result<DeviceLightTree, Error> {
        let build = Build::new(self, vertices, leaves, anchor, records, None)?;
        immediate(&self.device, |cmd| build.record(self, cmd))?;
        Ok(build.output)
    }
}
impl WgpuContext {
    /// 录制 GPU 构树，与生产顶点的 BLAS 命令按同一队列顺序提交。不等待 GPU。
    /// 取消/完成前通过记录保活输入、输出及叶数据。
    /// # Errors
    /// 输入、分配、录制错误与 [`Context::build_light_tree`] 相同。
    pub fn record_light_tree(
        &mut self,
        vertices: Buffer,
        leaves: &[[u32; 4]],
        anchor: [i32; 3],
        records: bool,
    ) -> Result<(DeviceLightTree, crate::interop::CommandBuffer), Error> {
        self.record_light_tree_from(vertices, leaves, anchor, records, None)
    }
    /// 复用按帧预算传入的只读活动叶列，buffer 内容须与 leaves 同序。
    /// # Errors
    /// 与 [`Self::record_light_tree`] 相同。
    pub fn record_light_tree_from(
        &mut self,
        vertices: Buffer,
        leaves: &[[u32; 4]],
        anchor: [i32; 3],
        records: bool,
        source: Option<Buffer>,
    ) -> Result<(DeviceLightTree, crate::interop::CommandBuffer), Error> {
        let context = self.native();
        let build = Build::new(context, vertices, leaves, anchor, records, source)?;
        let mut op = Commands::new(context)?;
        for b in build.buffers() {
            op.access(b.into())?;
        }
        let raw = crate::interop::record(&context.device, "build light tree", |cmd| {
            build.record(context, cmd)
        })?;
        let record = op.finish(context)?;
        Ok((
            build.output,
            crate::interop::CommandBuffer::recorded(raw, record.key),
        ))
    }
}
