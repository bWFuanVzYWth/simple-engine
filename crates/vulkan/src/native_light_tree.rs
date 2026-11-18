//! 从四顶点驻留列并行生成采样树；各层有显式设备依赖，正反向选择共享同一棵树。
use super::*;

/// 新建的灯树及可选的四角发光面记录。与生产它们的命令共同发布。
#[derive(Clone, Copy)]
pub struct DeviceLightTree {
    /// 单树参考编码，从 word 0 开始。
    pub tree: Buffer,
    /// 请求时每灯 64B（面积提案、材质、四角）。顶点按传入 anchor 平移。
    pub records: Option<Buffer>,
}

pub(super) const BUILD_PUSH_BYTES: usize = 120;

/// 校验灯的驻留顶点索引和相对功率。合成树另校验各区块功率之和。
pub(super) fn light_power(vertices: Buffer, leaves: &[[u32; 4]]) -> Result<f64, Error> {
    let mut power = 0.0;
    for &[quad, bits, _, _] in leaves {
        let weight = f32::from_bits(bits);
        if (u64::from(quad) + 1) * 64 > vertices.len() || !weight.is_finite() || weight <= 0.0 {
            return Err(Error::InvalidArgument("光源顶点槽或功率无效"));
        }
        power += f64::from(weight);
    }
    if power > f64::from(f32::MAX) {
        return Err(Error::InvalidArgument("光源总功率溢出"));
    }
    Ok(power)
}

pub(super) fn light_source(
    context: &mut Context,
    vertices: Buffer,
    leaves: &[[u32; 4]],
    source: Option<Buffer>,
) -> Result<Buffer, Error> {
    if let Some(buffer) = source {
        context.active(buffer.key)?;
        if buffer.len() < size_of_val(leaves) as u64 {
            return Err(Error::InvalidArgument("发光活动列范围不足"));
        }
        Ok(buffer)
    } else if leaves.is_empty() {
        Ok(vertices)
    } else {
        Buffer::upload(context, bytemuck::cast_slice(leaves))
    }
}

pub(super) struct Dispatch<'a> {
    pub context: &'a Context,
    cmd: vk::CommandBuffer,
    program: &'a crate::quad_expansion::Expansion,
    max_groups: u32,
}

impl<'a> Dispatch<'a> {
    pub(super) fn new(context: &'a Context, cmd: vk::CommandBuffer) -> Result<Self, Error> {
        let program = context
            .light_builder
            .as_ref()
            .ok_or(Error::InvalidState("灯树程序未准备"))?;
        let device = &context.device;
        let max_groups = device
            .physical
            .properties()
            .limits
            .max_compute_work_group_count[0];
        // 驻留顶点和已发布区块可由之前提交的命令写出。
        unsafe { memory_barrier(device, cmd) };
        Ok(Self {
            context,
            cmd,
            program,
            max_groups,
        })
    }

    pub(super) fn address(&self, b: Buffer) -> Result<u64, Error> {
        Ok(b.data(self.context)?.address(&self.context.device))
    }

    pub(super) fn run(&self, push: &mut [u8; BUILD_PUSH_BYTES], size: u32) {
        push[112..116].copy_from_slice(&size.to_ne_bytes());
        let mut first = 0u32;
        while first < size {
            let groups = (size - first).div_ceil(64).min(self.max_groups);
            push[40..44].copy_from_slice(&first.to_ne_bytes());
            self.program
                .dispatch(&self.context.device, self.cmd, push, groups);
            first += groups * 64;
        }
        // 后一层读取本层写出的排序键、节点或区块摘要。
        unsafe { memory_barrier(&self.context.device, self.cmd) };
    }

    pub(super) fn local(
        &self,
        push: &mut [u8; BUILD_PUSH_BYTES],
        jobs: u32,
        count: u32,
        depth: u32,
        scratch: Option<Buffer>,
    ) -> Result<(), Error> {
        if depth != 0 {
            let mut sorted = self.address(scratch.expect("多灯需要排序缓冲"))?;
            let mut other = sorted + u64::from(count) * 16;
            let shift = depth.max(6);
            let padded = if jobs == 0 {
                count.next_multiple_of(64)
            } else {
                jobs << shift
            };
            push[108..112].copy_from_slice(&shift.to_ne_bytes());
            push[72..80].copy_from_slice(&sorted.to_ne_bytes());
            push[80..84].copy_from_slice(&1u32.to_ne_bytes());
            self.run(push, padded);
            for pass in 6..depth {
                push[64..72].copy_from_slice(&sorted.to_ne_bytes());
                push[72..80].copy_from_slice(&other.to_ne_bytes());
                push[80..84].copy_from_slice(&2u32.to_ne_bytes());
                push[84..88].copy_from_slice(&(1u32 << pass).to_ne_bytes());
                self.run(push, if jobs == 0 { count } else { padded });
                std::mem::swap(&mut sorted, &mut other);
            }
            push[64..72].copy_from_slice(&sorted.to_ne_bytes());
        }
        push[80..84].fill(0);
        for level in (0..=depth).rev() {
            push[36..40].copy_from_slice(&level.to_ne_bytes());
            push[108..112].copy_from_slice(&level.to_ne_bytes());
            self.run(push, jobs.max(1) << level);
        }
        Ok(())
    }
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
        light_power(vertices, leaves)?;
        let count64 = u64::from(count);
        let words = (count64.saturating_sub(1) * 10).next_multiple_of(8) + count64 * 8;
        let tree = Buffer::new(context, (words * 4).max(16), false)?;
        let records = records
            .then(|| Buffer::new(context, (count64 * 64).max(64), false))
            .transpose()?;
        let sources = light_source(context, vertices, leaves, source)?;
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
        let dispatch = Dispatch::new(context, cmd)?;
        let pointers = [
            dispatch.address(self.vertices)?,
            dispatch.address(self.sources)?,
            dispatch.address(self.output.tree)?,
            self.output
                .records
                .map(|b| dispatch.address(b))
                .transpose()?
                .unwrap_or(0),
        ];
        let mut push = [0u8; BUILD_PUSH_BYTES];
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
        dispatch.local(
            &mut push,
            0,
            self.count,
            self.count.next_power_of_two().ilog2(),
            self.scratch,
        )
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
    /// 复用已经按预算传输的活动叶列。缓冲内容与 `leaves` 一致，跨提交保持只读。
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
        self.submit_terrain(raw, Default::default(), Vec::new(), |op| {
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
    /// 复用按帧预算传入的只读活动叶列，buffer 内容须与 `leaves` 一致。
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
