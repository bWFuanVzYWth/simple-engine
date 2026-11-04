use std::{
    collections::HashMap,
    num::NonZeroU64,
    sync::{Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};
use vulkan::interop::{CommandBuffer, WgpuContext};

/// 环境创建时启用的能力；不满足要求时返回错误，不跳过执行并算作成功。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// 生产 compute SPIR-V，不要求硬件光追。
    Compute,
    /// 同时启用 Vulkan KHR 光追管线。
    RayTracing,
}

/// 单个 compute 入口；接口须匹配输入 binding 0、输出 binding 1 的 storage buffer。
#[derive(Clone, Copy)]
pub struct Shader<'a> {
    /// 由受控 Slang 构建生成的原始 SPIR-V。
    pub spirv: &'a [u8],
    /// 与 SPIR-V 完全一致的入口名称。
    pub entry: &'a str,
    /// 入口的实际工作组大小，供 wgpu passthrough 描述使用。
    pub workgroup: [u32; 3],
}

/// 初始化、执行或结果检查失败；失效环境不得继续承载用例。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// native 设备或互操作操作失败。
    #[error(transparent)]
    Vulkan(#[from] vulkan::Error),
    /// 设备轮询失败或等待超时。
    #[error("等待 GPU 失败：{0}")]
    Poll(#[from] wgpu::PollError),
    /// GPU 回读映射失败。
    #[error("映射回读失败：{0}")]
    Map(#[from] wgpu::BufferAsyncError),
    /// 无法取得已映射范围。
    #[error("访问回读失败：{0}")]
    MapRange(#[from] wgpu::MapRangeError),
    /// 映射回调未在时限内返回或通道已断开。
    #[error("回读回调中断：{0}")]
    Callback(#[from] mpsc::RecvTimeoutError),
    /// wgpu 设备丢失回调报告的原因与驱动描述。
    #[error("GPU 设备丢失（{reason:?}）：{message}")]
    DeviceLost {
        /// wgpu 分类的丢失原因。
        reason: wgpu::DeviceLostReason,
        /// 后端提供的详细描述。
        message: String,
    },
    /// 全进程 logger 安装或检查失败。
    #[error("{0}")]
    Logger(#[from] gpu_log::Error),
    /// 本环境基线之后的累计 Error 数量；环境随即失效。
    #[error("GPU 记录了 {0} 个错误，停止复用环境")]
    Validation(u32),
    /// 前次操作已使环境失效，本次没有继续执行 GPU 工作。
    #[error("GPU 环境已失效，未执行后续用例")]
    Unhealthy,
    /// 共享环境的串行锁中毒，拒绝恢复用例执行。
    #[error("GPU 测试环境锁已中毒")]
    Lock,
    /// 调用参数违反 IO、批次或计时能力约束。
    #[error("{0}")]
    Argument(&'static str),
}

/// 本执行器的累计主机成本与保留容量，独立于具体属性的结果。
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// logger、设备、布局与可选查询资源的初始化耗时。
    pub init_seconds: f64,
    /// compute 缓存的管线创建；fixture 直接创建的 AS/RT 管线另计。
    pub pipeline_seconds: f64,
    /// IO 扩容、上传及 bind group 准备的累计主机耗时。
    pub prepare_seconds: f64,
    /// 主机提交、等待与回读，不是 GPU timestamp。
    pub execute_seconds: f64,
    /// 按 SPIR-V、入口与工作组大小缓存的 compute 管线数。
    pub pipelines: usize,
    /// 经本执行器提交的批次，不含 fixture 直接提交。
    pub submissions: u64,
    /// 复用 IO 和回读 buffer 的容量总和，不含驱动及 native AS/SBT。
    pub buffer_capacity_bytes: u64,
}

/// 一批 GPU 命令的独立计时口径，不等于单次 shader 函数延迟。
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// 同队列 timestamp 差值，含执行依赖及测量边界固定成本。
    pub gpu_seconds: f64,
    /// 主机录制或提交起点至查询回读完成，具体起点由执行入口定义。
    pub host_seconds: f64,
    /// 管线与 IO 准备成本；传入已录制命令时为零。
    pub prepare_seconds: f64,
}

/// 可变 IO 为单一所有者；每次调用结束前完成回读，跨调用复用容量。
pub struct Gpu {
    context: WgpuContext,
    layout: wgpu::BindGroupLayout,
    kernels: HashMap<blake3::Hash, wgpu::ComputePipeline>,
    io: Option<Io>,
    staging: Option<wgpu::Buffer>,
    timestamps: Option<Timestamps>,
    errors: u32,
    healthy: bool,
    lost: mpsc::Receiver<(wgpu::DeviceLostReason, String)>,
    lost_reason: OnceLock<(wgpu::DeviceLostReason, String)>,
    stats: Stats,
}

struct Io {
    input: wgpu::Buffer,
    output: wgpu::Buffer,
}
struct Timestamps {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    mask: u64,
}

#[derive(Default)]
struct Environments {
    compute: Option<Gpu>,
    rt: Option<Gpu>,
}
static SHARED: OnceLock<Mutex<Environments>> = OnceLock::new();

pub(crate) fn shutdown_shared() -> Result<(), Error> {
    let Some(shared) = SHARED.get() else {
        return Ok(());
    };
    // 锁中毒后只清理所有者，不恢复执行用例。
    let mut environments = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = gpu_log::errors();
    let mut result = Ok(());
    for (cap, gpu) in [
        (Capability::Compute, environments.compute.take()),
        (Capability::RayTracing, environments.rt.take()),
    ] {
        let Some(gpu) = gpu else {
            continue;
        };
        eprintln!("GPU {cap:?} 累计：{:?}", gpu.stats());
        if let Err(error) = gpu.finish() {
            result = Err(error);
        }
    }
    let errors = gpu_log::errors().wrapping_sub(before);
    if errors != 0 {
        return Err(Error::Validation(errors));
    }
    result
}

/// 同进程按能力复用设备，GPU 区间串行；CPU 生成与断言放在闭包外。
pub fn with_shared<T>(
    cap: Capability,
    run: impl FnOnce(&mut Gpu) -> Result<T, Error>,
) -> Result<T, Error> {
    let mut environments = SHARED
        .get_or_init(|| Mutex::new(Environments::default()))
        .lock()
        .map_err(|_| Error::Lock)?;
    let slot = match cap {
        Capability::Compute => &mut environments.compute,
        Capability::RayTracing => &mut environments.rt,
    };
    if slot.is_none() {
        *slot = Some(Gpu::new(cap, 0, false)?);
    }
    let gpu = slot.as_mut().unwrap();
    gpu.ensure_healthy()?;
    let result = run(gpu);
    gpu.check()?;
    result
}

impl Gpu {
    /// 创建指定适配器的独立环境；timestamp 请求不支持时初始化失败。
    ///
    /// `adapter` 按 Vulkan 枚举顺序索引。映射与轮询使用 30 秒等待上限；GPU hang 应由
    /// 外层进程超时处理，不能在线程内取消后继续复用设备。
    pub fn new(cap: Capability, adapter: usize, timestamps: bool) -> Result<Self, Error> {
        gpu_log::init()?;
        let errors = gpu_log::errors();
        let start = Instant::now();
        let features = if timestamps {
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
        } else {
            wgpu::Features::empty()
        };
        let context = pollster::block_on(WgpuContext::with_features(
            adapter,
            cap == Capability::RayTracing,
            features,
        ))?;
        let (callback_lost, lost) = mpsc::sync_channel(1);
        context.on_device_lost(move |reason, message| {
            // 回调只移交结果；环境独占错误缓存，关闭后发送失败即可结束。
            let _ = callback_lost.try_send((reason, message));
        });
        let layout = context
            .device()
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("GPU test IO"),
                entries: &[binding(0, true), binding(1, false)],
            });
        let timestamps = timestamps.then(|| {
            let queries = context
                .device()
                .create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("GPU batch timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                });
            let resolve = context.device().create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU timestamps"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let mask = u64::MAX >> (64 - context.timestamp_valid_bits());
            Timestamps {
                queries,
                resolve,
                mask,
            }
        });
        let stats = Stats {
            init_seconds: start.elapsed().as_secs_f64(),
            ..Default::default()
        };
        let mut gpu = Self {
            context,
            layout,
            kernels: HashMap::new(),
            io: None,
            staging: None,
            timestamps,
            errors,
            healthy: true,
            lost,
            lost_reason: OnceLock::new(),
            stats,
        };
        gpu.check()?;
        Ok(gpu)
    }

    /// 借用同设备、同队列的 fixture 构建入口；fixture 须先于环境清理。
    pub const fn context(&self) -> &WgpuContext {
        &self.context
    }
    /// 独占借入提交与 native 资源版本所有者。
    pub fn context_mut(&mut self) -> &mut WgpuContext {
        &mut self.context
    }

    /// 当前累计成本；仅包含经此执行器完成的工作。
    pub const fn stats(&self) -> &Stats {
        &self.stats
    }

    /// 释放本环境并检查销毁日志；调用方持有的 fixture 须先释放。
    pub fn finish(mut self) -> Result<(), Error> {
        let baseline = self.errors;
        let result = self.check();
        drop(self);
        let errors = gpu_log::errors().wrapping_sub(baseline);
        if errors != 0 {
            return Err(Error::Validation(errors));
        }
        result
    }

    fn ensure_healthy(&self) -> Result<(), Error> {
        if !self.healthy {
            return Err(Error::Unhealthy);
        }
        if self.lost_reason.get().is_none()
            && let Ok(reason) = self.lost.try_recv()
        {
            let _ = self.lost_reason.set(reason);
        }
        if let Some((reason, message)) = self.lost_reason.get() {
            return Err(Error::DeviceLost {
                reason: *reason,
                message: message.clone(),
            });
        }
        Ok(())
    }

    /// 等待已提交工作并检查丢失回调和日志差分；失败后拒绝复用。
    ///
    /// 日志计数覆盖全进程，调用方须避免其他 GPU 环境并发制造无法归属的错误。
    pub fn check(&mut self) -> Result<(), Error> {
        self.ensure_healthy()?;
        if let Err(error) = self.context.device().poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        }) {
            self.healthy = false;
            return Err(error.into());
        }
        let count = gpu_log::errors().wrapping_sub(self.errors);
        if count != 0 {
            self.healthy = false;
            return Err(Error::Validation(count));
        }
        self.ensure_healthy()
    }

    fn pipeline(&mut self, shader: Shader<'_>) -> Result<wgpu::ComputePipeline, Error> {
        let mut hash = blake3::Hasher::new();
        hash.update(shader.spirv);
        hash.update(shader.entry.as_bytes());
        for size in shader.workgroup {
            hash.update(&size.to_le_bytes());
        }
        let key = hash.finalize();
        if let Some(pipeline) = self.kernels.get(&key) {
            return Ok(pipeline.clone());
        }
        let start = Instant::now();
        let module = self
            .context
            .shader(shader.spirv, shader.entry, shader.workgroup)?;
        let layout =
            self.context
                .device()
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("GPU test compute"),
                    bind_group_layouts: &[Some(&self.layout)],
                    immediate_size: 0,
                });
        let pipeline =
            self.context
                .device()
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(shader.entry),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(shader.entry),
                    compilation_options: Default::default(),
                    cache: None,
                });
        self.stats.pipeline_seconds += start.elapsed().as_secs_f64();
        self.stats.pipelines += 1;
        self.kernels.insert(key, pipeline.clone());
        Ok(pipeline)
    }

    fn prepare(&mut self, input: &[u8], output_bytes: usize) -> Result<wgpu::BindGroup, Error> {
        let limit = self
            .context
            .device()
            .limits()
            .max_storage_buffer_binding_size;
        if input.is_empty()
            || output_bytes == 0
            || !input.len().is_multiple_of(4)
            || !output_bytes.is_multiple_of(4)
            || input.len() as u64 > limit
            || output_bytes as u64 > limit
        {
            return Err(Error::Argument(
                "IO 须非空、4 字节对齐且不超设备 storage 范围",
            ));
        }
        let start = Instant::now();
        if self.io.as_ref().is_none_or(|io| {
            io.input.size() < input.len() as u64 || io.output.size() < output_bytes as u64
        }) {
            let sizes = self
                .io
                .as_ref()
                .map(|io| (io.input.size(), io.output.size()))
                .unwrap_or_default();
            let create = |label, size, usage| {
                self.context
                    .device()
                    .create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size,
                        usage,
                        mapped_at_creation: false,
                    })
            };
            self.io = Some(Io {
                input: create(
                    "GPU test input",
                    sizes.0.max(input.len() as u64),
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                ),
                output: create(
                    "GPU test output",
                    sizes.1.max(output_bytes as u64),
                    wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                ),
            });
        }
        let io = self.io.as_ref().unwrap();
        self.context.queue().write_buffer(&io.input, 0, input);
        let bindings = self
            .context
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("GPU test IO"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &io.input,
                            offset: 0,
                            size: NonZeroU64::new(input.len() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &io.output,
                            offset: 0,
                            size: NonZeroU64::new(output_bytes as u64),
                        }),
                    },
                ],
            });
        self.stats.prepare_seconds += start.elapsed().as_secs_f64();
        self.update_capacity();
        Ok(bindings)
    }

    /// 清零输出、批量 dispatch 并等待完整回读；输入与输出均须非空、4 字节对齐。
    ///
    /// IO 不得超过设备 storage binding 上限；容量与管线跨调用复用。返回值只含本次
    /// `output_bytes`，不会暴露此前保留容量中的数据。shader 的布局与访存仍由用例维护。
    pub fn compute(
        &mut self,
        shader: Shader<'_>,
        input: &[u8],
        output_bytes: usize,
        workgroups: [u32; 3],
    ) -> Result<Vec<u8>, Error> {
        self.ensure_healthy()?;
        let pipeline = self.pipeline(shader)?;
        let bindings = self.prepare(input, output_bytes)?;
        let start = Instant::now();
        let mut encoder = self
            .context
            .device()
            .create_command_encoder(&Default::default());
        encoder.clear_buffer(
            &self.io.as_ref().unwrap().output,
            0,
            Some(output_bytes as u64),
        );
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(workgroups[0], workgroups[1], workgroups[2]);
        }
        let output = self.io.as_ref().unwrap().output.clone();
        let staging = self.copy_to_staging(&mut encoder, &output, output_bytes as u64);
        let submission = self.context.queue().submit([encoder.finish()]);
        self.stats.submissions += 1;
        let bytes = self.map_output(&staging, output_bytes as u64, submission)?;
        self.stats.execute_seconds += start.elapsed().as_secs_f64();
        Ok(bytes)
    }

    /// 等待并回读整个同设备 buffer；须具备 `COPY_SRC` 且大小满足 wgpu 拷贝约束。
    pub fn readback(&mut self, source: &wgpu::Buffer) -> Result<Vec<u8>, Error> {
        self.read_range(source, source.size())
    }

    fn read_range(&mut self, source: &wgpu::Buffer, bytes: u64) -> Result<Vec<u8>, Error> {
        self.ensure_healthy()?;
        let mut encoder = self
            .context
            .device()
            .create_command_encoder(&Default::default());
        let buffer = self.copy_to_staging(&mut encoder, source, bytes);
        let submission = self.context.queue().submit([encoder.finish()]);
        self.stats.submissions += 1;
        self.map_output(&buffer, bytes, submission)
    }

    fn copy_to_staging(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Buffer,
        bytes: u64,
    ) -> wgpu::Buffer {
        if self
            .staging
            .as_ref()
            .is_none_or(|buffer| buffer.size() < bytes)
        {
            self.staging = Some(
                self.context
                    .device()
                    .create_buffer(&wgpu::BufferDescriptor {
                        label: Some("GPU test readback"),
                        size: bytes,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    }),
            );
        }
        self.update_capacity();
        let buffer = self.staging.as_ref().unwrap();
        encoder.copy_buffer_to_buffer(source, 0, buffer, 0, bytes);
        buffer.clone()
    }

    fn map_output(
        &mut self,
        buffer: &wgpu::Buffer,
        bytes: u64,
        submission: wgpu::SubmissionIndex,
    ) -> Result<Vec<u8>, Error> {
        let (send, receive) = mpsc::channel();
        buffer
            .slice(..bytes)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        if let Err(error) = self.context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        }) {
            self.healthy = false;
            return Err(error.into());
        }
        let result = (|| {
            receive.recv_timeout(Duration::from_secs(30))??;
            Ok::<_, Error>(buffer.slice(..bytes).get_mapped_range()?.to_vec())
        })();
        buffer.unmap();
        if result.is_err() {
            self.healthy = false;
        }
        let result = result?;
        self.check()?;
        Ok(result)
    }

    /// 资源准备不计入计时；host 包含录制、提交、查询 resolve、等待和查询回读。
    /// 输出不逐轮清零；重复 dispatch 的依赖和工作负载由调用方设计。
    pub fn bench_compute(
        &mut self,
        shader: Shader<'_>,
        input: &[u8],
        output_bytes: usize,
        workgroups: [u32; 3],
        dispatches: u32,
    ) -> Result<Sample, Error> {
        self.ensure_healthy()?;
        if dispatches == 0 {
            return Err(Error::Argument("测量批次不能为空"));
        }
        let prepared = Instant::now();
        let pipeline = self.pipeline(shader)?;
        let bindings = self.prepare(input, output_bytes)?;
        let prepare_seconds = prepared.elapsed().as_secs_f64();
        let start = Instant::now();
        let mut encoder = self
            .context
            .device()
            .create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            for _ in 0..dispatches {
                pass.dispatch_workgroups(workgroups[0], workgroups[1], workgroups[2]);
            }
        }
        let mut sample = self.measure(vec![encoder.finish().into()])?;
        sample.host_seconds = start.elapsed().as_secs_f64();
        sample.prepare_seconds = prepare_seconds;
        Ok(sample)
    }

    /// GPU 时间覆盖命令及交接；host 从提交前开始，不含传入命令的录制。
    ///
    /// 环境须启用 timestamp；差值按队列有效位数回绕，单批必须短于一次计数回绕周期。
    /// 起点后插入执行依赖，防止工作提前越过起点；查询与同步固定成本计入 GPU 样本。
    /// 见 [Vulkan timestamp](https://docs.vulkan.org/spec/latest/chapters/queries.html#queries-timestamps)。
    pub fn measure(&mut self, commands: Vec<CommandBuffer>) -> Result<Sample, Error> {
        self.ensure_healthy()?;
        if commands.is_empty() {
            return Err(Error::Argument("测量批次不能为空"));
        }
        let timestamps = self
            .timestamps
            .as_ref()
            .ok_or(Error::Argument("环境未启用 timestamp"))?;
        let mut before = self
            .context
            .device()
            .create_command_encoder(&Default::default());
        before.write_timestamp(&timestamps.queries, 0);
        let mut after = self
            .context
            .device()
            .create_command_encoder(&Default::default());
        after.write_timestamp(&timestamps.queries, 1);
        after.resolve_query_set(&timestamps.queries, 0..2, &timestamps.resolve, 0);
        let source = timestamps.resolve.clone();
        let staging = self.copy_to_staging(&mut after, &source, 16);
        let mut batch = Vec::with_capacity(commands.len() + 3);
        batch.push(before.finish().into());
        // HAL 起点是 BOTTOM_OF_PIPE；显式阻止后续工作提前跨过起点。
        batch.push(self.context.execution_barrier()?);
        batch.extend(commands);
        batch.push(after.finish().into());
        let start = Instant::now();
        let submission = self.context.submit(batch)?;
        self.stats.submissions += 1;
        let bytes = self.map_output(&staging, 16, submission)?;
        Ok(self.timestamp_sample(&bytes, start, 0.0))
    }

    fn timestamp_sample(&self, bytes: &[u8], start: Instant, prepare_seconds: f64) -> Sample {
        let first = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        let last = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let gpu_seconds = timestamp_ticks(first, last, self.timestamps.as_ref().unwrap().mask)
            as f64
            * f64::from(self.context.queue().get_timestamp_period())
            * 1e-9;
        Sample {
            gpu_seconds,
            host_seconds: start.elapsed().as_secs_f64(),
            prepare_seconds,
        }
    }

    fn update_capacity(&mut self) {
        self.stats.buffer_capacity_bytes = self
            .io
            .as_ref()
            .map_or(0, |io| io.input.size() + io.output.size())
            + self.staging.as_ref().map_or(0, wgpu::Buffer::size);
    }
}

fn timestamp_ticks(first: u64, last: u64, mask: u64) -> u64 {
    last.wrapping_sub(first) & mask
}

fn binding(index: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: index,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_wrap_uses_queue_precision() {
        let mask = (1u64 << 36) - 1;
        assert_eq!(super::timestamp_ticks(mask - 4, 2, mask), 7);
    }
}
