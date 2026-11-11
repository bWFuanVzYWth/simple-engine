//! 与宿主无关的 Vulkan 资源、描述符和录制。shader 接口由受控 Slang 构建维护。
//!
//! Context 唯一拥有资源和命令版本；标识不保活，Recorded/Flight 须显式取消或报告完成。
//! 导入边界负责宿主设备有效期，资源更新受待执行/在途使用约束；不引入 Naga 或额外访存插桩。

use crate::graph;
use crate::handles::RecordId;
#[doc(inline)]
pub use crate::handles::{
    Bindings, Buffer, ComputePipeline, ExternalBinding, ExternalKind, Image, RayTracingPipeline,
    Resource, ResourceAccess, Sampler,
};
use crate::interop::{RayShader, SampledTexture, SharedBuffer, SharedTexture, WgpuContext};
use crate::lifecycle::{self, Life, Op, resource as meta};
#[doc(inline)]
pub use crate::lifecycle::{Flight, Recorded};
use crate::submission::RecordState as State;
use crate::{Device, Error, Instance, PhysicalDevice, ShaderModule, buffer, khr, vk};
use crate::{
    acceleration::{
        AccelerationData, BatchMode, BlasData, BuildBuffers, ResolvedBlas, ResolvedInstance,
    },
    buffer_pool::BufferPool,
    micromap::MicromapData,
    owner::{Destroy, Guard, Key},
};
use ash::vk::Handle;
use ash::vk::TaggedStructure as _;
use ash_wgpu::vk::Handle as _;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};
use wgpu::hal::api::Vulkan;

#[path = "native_light_tree.rs"]
mod light_tree;
pub use light_tree::DeviceLightTree;
#[path = "terrain_flight.rs"]
mod terrain_flight;

#[path = "timestamp.rs"]
mod timestamp;
pub use timestamp::TimestampQueries;

#[path = "pipeline_statistics.rs"]
mod pipeline_statistics;
pub use pipeline_statistics::{ExecutableStatistic, ExecutableStatistics, StatisticValue};

#[path = "checkpoints.rs"]
mod checkpoints;
pub use checkpoints::{Checkpoint, CheckpointProgress, CheckpointReport};

/// 宿主创建设备时实际启用的能力。
#[derive(Clone, Copy, Default)]
pub struct Capabilities {
    /// Vulkan 1.4、device address commands、AS、RT pipeline 和 RayQuery。
    pub ray_tracing: bool,
    /// KHR position fetch 扩展及 feature。
    pub position_fetch: bool,
    /// KHR opacity micromap 扩展及 feature。
    pub opacity_micromap: bool,
    /// EXT invocation reorder 扩展及 feature；shader 须使用相同扩展。
    pub invocation_reorder: bool,
    /// KHR pipeline executable properties 扩展及 pipelineExecutableInfo feature。
    /// 启用后 RT 管线捕获统计；仅用于独立分析，不与正常计时混用。
    pub pipeline_statistics: bool,
}

/// 设备默认 subgroup 规模、compute 操作与可请求的规模范围。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubgroupProperties {
    /// 默认每subgroup的invocation数量。
    pub size: u32,
    /// 已启用 subgroupSizeControl，且 compute 支持 requiredSubgroupSize。
    pub compute_size_control: bool,
    /// 可请求规模的最小值；未启用规模控制时等于默认规模。
    pub min_size: u32,
    /// 可请求规模的最大值；未启用规模控制时等于默认规模。
    pub max_size: u32,
    /// compute阶段支持subgroup操作。
    pub compute: bool,
    /// 支持BASIC操作。
    pub basic: bool,
    /// 支持BALLOT操作。
    pub ballot: bool,
    /// 支持ARITHMETIC操作。
    pub arithmetic: bool,
}
impl Capabilities {
    pub(crate) const fn validate(self) -> Result<(), Error> {
        if !self.ray_tracing
            && (self.position_fetch
                || self.opacity_micromap
                || self.invocation_reorder
                || self.pipeline_statistics)
        {
            return Err(Error::InvalidArgument("附加光追能力依赖 RT"));
        }
        Ok(())
    }
}
/// 外部 Vulkan 设备与提交队列的原始句柄。
#[derive(Clone, Copy)]
pub struct RawDevice {
    /// VkInstance。
    pub instance: u64,
    /// VkPhysicalDevice。
    pub physical_device: u64,
    /// VkDevice。
    pub device: u64,
    /// 唯一提交用 VkQueue；外部编码命令也须提交到此队列。
    pub queue: u64,
    /// 支持 compute 的队列族。
    pub queue_family: u32,
    /// 宿主实例与物理设备共同支持的 Vulkan API 版本。
    pub api_version: u32,
    /// 宿主是否已启用 subgroupSizeControl；不改变默认 subgroup 规模。
    pub subgroup_size_control: bool,
    /// 实际启用能力。
    pub capabilities: Capabilities,
}
/// 设备、资源版本及命令的唯一所有者；标识复制不延长资源寿命。
///
/// 创建及更新通过独占借用进行。已录制/在途命令自动成为资源根；调用方在版本更换后
/// 用 [`Self::collect`] 保留当前发布根。原始宿主设备须存活至本对象销毁。
/// 析构取消 pending 记录；存在外部已编码或已提交记录时保守等待队列，再释放资源与设备。
pub struct Context {
    pub(crate) device: Device,
    pub(crate) life: Life<Native, BuildBuffers>,
    as_pool: BufferPool,
    buffer_retirement: Option<crate::buffer_retirement::BufferRetirement>,
    quad_expansion: Option<crate::quad_expansion::Expansion>,
    light_builder: Option<crate::quad_expansion::Expansion>,
    terrain_flights: Vec<terrain_flight::Submitted>,
    pipeline_cache: Option<crate::pipeline_cache::Native>,
    checkpoints: Option<checkpoints::Checkpoints>,
}

pub(crate) struct Native;

impl meta::Payloads for Native {
    type Queries = timestamp::QueryData;
    type Buffer = buffer::Buffer;
    type Image = ImageData;
    type Sampler = SamplerData;
    type Set = RawBindings;
    type Layout = DescriptorLayout;
    type RayProgram = RawRayProgram;
    type ComputeProgram = RawComputeProgram;
    type Structure = crate::acceleration::Structure;
    type Micromap = MicromapData;
    type SharedBuffer = SharedBuffer;
    type SharedTexture = SharedTexture;
    type SampledTexture = SampledTexture;
}

type Data = meta::Data<Native>;
type BindingData = meta::BindingData<Native>;
type StoredBinding = meta::Binding<Native>;
type RayProgram = meta::Program<RawRayProgram>;
type ComputeProgram = meta::Program<RawComputeProgram>;
type RayVersion = meta::Version;
type ComputeVersion = meta::Version;
type Record = lifecycle::Record<BuildBuffers>;

impl Context {
    /// 返回单个 storage buffer 的物理设备字节上限，查询不会分配资源。
    pub fn max_storage_buffer_range(&self) -> u32 {
        self.device
            .physical
            .properties()
            .limits
            .max_storage_buffer_range
    }

    /// 已启用 OMM 时返回设备精确二值地图允许的最大细分级别；未启用时返回 `None`。
    /// 场景按自身 UV 与过滤规则决定可表示性，本查询不改变设备或创建资源。
    pub fn max_opacity2_state_subdivision_level(&self) -> Option<u32> {
        if !self.device.opacity_micromap {
            return None;
        }
        let mut opacity = vk::PhysicalDeviceOpacityMicromapPropertiesKHR::default();
        let mut properties = vk::PhysicalDeviceProperties2::default().push(&mut opacity);
        // SAFETY: Context 保活物理设备与实例，输出链只借用本栈内有效的结构。
        unsafe {
            self.device
                .physical
                .instance
                .raw
                .get_physical_device_properties2(self.device.physical.raw, &mut properties);
        }
        Some(opacity.max_opacity2_state_subdivision_level)
    }

    /// 查询 subgroup 操作与可请求规模；不修改设备或 pipeline。
    pub fn subgroup_properties(&self) -> SubgroupProperties {
        let mut subgroup = vk::PhysicalDeviceSubgroupProperties::default();
        let mut sizes = vk::PhysicalDeviceSubgroupSizeControlProperties::default();
        let mut properties = vk::PhysicalDeviceProperties2::default().push(&mut subgroup);
        if self.device.subgroup_size_control {
            properties = properties.push(&mut sizes);
        }
        unsafe {
            self.device
                .physical
                .instance
                .raw
                .get_physical_device_properties2(self.device.physical.raw, &mut properties);
        }
        SubgroupProperties {
            size: subgroup.subgroup_size,
            compute_size_control: self.device.subgroup_size_control
                && sizes
                    .required_subgroup_size_stages
                    .contains(vk::ShaderStageFlags::COMPUTE),
            min_size: if self.device.subgroup_size_control {
                sizes.min_subgroup_size
            } else {
                subgroup.subgroup_size
            },
            max_size: if self.device.subgroup_size_control {
                sizes.max_subgroup_size
            } else {
                subgroup.subgroup_size
            },
            compute: subgroup
                .supported_stages
                .contains(vk::ShaderStageFlags::COMPUTE),
            basic: subgroup
                .supported_operations
                .contains(vk::SubgroupFeatureFlags::BASIC),
            ballot: subgroup
                .supported_operations
                .contains(vk::SubgroupFeatureFlags::BALLOT),
            arithmetic: subgroup
                .supported_operations
                .contains(vk::SubgroupFeatureFlags::ARITHMETIC),
        }
    }

    pub(crate) fn new(device: Device) -> Result<Self, Error> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let owner = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::InvalidState("设备标识已耗尽"))?;
        Ok(Self {
            device,
            life: Life::new(owner),
            as_pool: BufferPool::default(),
            buffer_retirement: None,
            quad_expansion: None,
            light_builder: None,
            terrain_flights: Vec::new(),
            pipeline_cache: None,
            checkpoints: None,
        })
    }

    /// 在设备准备阶段创建压缩 quad 展开程序与大显存列回收线程；重复调用复用它们。
    /// # Errors
    /// 计算管线创建或底层设备失败时不发布程序。
    pub fn prepare_quad_expansion(&mut self) -> Result<(), Error> {
        if self.buffer_retirement.is_none() {
            // SAFETY: Drop 在任何返回路径上都先关闭并 join 线程，随后才释放宿主设备和 loader。
            self.buffer_retirement = Some(
                unsafe { crate::buffer_retirement::BufferRetirement::new(&self.device) }
                    .map_err(Error::RetirementWorker)?,
            );
        }
        if self.quad_expansion.is_none() {
            self.quad_expansion = Some(crate::quad_expansion::Expansion::new(
                &self.device,
                self.pipeline_cache(),
            )?);
        }
        if self.light_builder.is_none() {
            self.light_builder = Some(crate::quad_expansion::Expansion::with_shader(
                &self.device,
                self.pipeline_cache(),
                include_bytes!(concat!(env!("OUT_DIR"), "/light_build.spv")),
                c"build_level",
                88,
            )?);
        }
        Ok(())
    }

    /// 启用本设备独占的管线缓存。调用者提供应用私有、可信的可再生缓存目录。
    ///
    /// 应在管线初始化前调用。重复调用保留首次成功配置。缺失、不兼容或损坏的快照
    /// 使用空缓存，缓存故障不阻止管线创建。独立进程以完整快照原子替换同一文件。
    pub fn enable_pipeline_cache(&mut self, directory: &std::path::Path) {
        if self.pipeline_cache.is_some() || self.device.ensure_active().is_err() {
            return;
        }
        match crate::pipeline_cache::Native::new(&self.device, directory) {
            Ok(cache) => self.pipeline_cache = Some(cache),
            Err(error) => log::warn!("创建 Vulkan 管线缓存失败：{error}"),
        }
    }

    /// 保存上次快照后使用过的管线缓存。不提交命令或等待 GPU，IO 失败仅记录诊断。
    /// 应在初始化或资源世代发布边界调用，析构也会保存之后的变化。
    pub fn flush_pipeline_cache(&mut self) {
        if let Some(cache) = &mut self.pipeline_cache {
            cache.flush(&self.device);
        }
    }

    fn pipeline_cache(&self) -> vk::PipelineCache {
        self.pipeline_cache
            .as_ref()
            .map_or_else(vk::PipelineCache::null, crate::pipeline_cache::Native::used)
    }

    pub(crate) const fn owner(&self) -> u64 {
        self.life.owner()
    }
    /// 借入宿主设备，不销毁原始句柄。
    ///
    /// # Safety
    /// 句柄来自同一有效设备，API 版本和启用能力须如实声明，queue 属于 queue_family。
    /// instance、device 须由系统默认 Vulkan loader 创建；自定义 loader 应使用宿主适配边界。
    /// RT 须启用 Vulkan 1.4、deviceAddressCommands、bufferDeviceAddress、AS、RT pipeline、RayQuery 与 synchronization2。
    /// 外部命令也只提交到此队列。
    /// 宿主在所有调用和析构期间保持设备有效、串行化队列的宿主访问，并在报告批次完成前确认 GPU 完成。
    pub unsafe fn from_raw(raw: RawDevice) -> Result<Self, Error> {
        raw.capabilities.validate()?;
        if [raw.instance, raw.physical_device, raw.device, raw.queue].contains(&0) {
            return Err(Error::InvalidArgument("外部 Vulkan 句柄为空"));
        }
        let entry = unsafe { ash::Entry::load()? };
        let instance =
            unsafe { ash::Instance::load(entry.static_fn(), vk::Instance::from_raw(raw.instance)) };
        let device =
            unsafe { ash::Device::load(instance.fp_v1_0(), vk::Device::from_raw(raw.device)) };
        let physical = PhysicalDevice {
            raw: vk::PhysicalDevice::from_raw(raw.physical_device),
            instance: Instance {
                raw: instance,
                _owner: None,
            },
        };
        if !physical
            .queue_families()
            .get(raw.queue_family as usize)
            .is_some_and(|family| family.queue_flags.contains(vk::QueueFlags::COMPUTE))
        {
            return Err(Error::InvalidArgument("外部队列不支持 compute"));
        }
        if raw.api_version > physical.properties().api_version
            || raw.api_version < vk::API_VERSION_1_1
            || (raw.capabilities.ray_tracing && raw.api_version < vk::API_VERSION_1_4)
            || (raw.subgroup_size_control && raw.api_version < vk::API_VERSION_1_3)
        {
            return Err(Error::InvalidArgument("外部设备 API 版本与能力不匹配"));
        }
        Self::new(Device {
            health: crate::health::Health::new(),
            uncertain_use: Default::default(),
            raw: device,
            physical,
            buffer_bytes: AtomicU64::new(0),
            ray_pipeline: raw.capabilities.ray_tracing,
            pipeline_robustness: false,
            subgroup_size_control: raw.subgroup_size_control,
            _owner: None,
            wgpu_queue: None,
            _loader: Some(entry),
            queue: vk::Queue::from_raw(raw.queue),
            queue_family: raw.queue_family,
            position_fetch: raw.capabilities.position_fetch,
            opacity_micromap: raw.capabilities.opacity_micromap,
            invocation_reorder: raw.capabilities.invocation_reorder,
            pipeline_statistics: raw.capabilities.pipeline_statistics,
        })
    }
    /// 实际启用能力。
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            ray_tracing: self.device.ray_pipeline,
            position_fetch: self.device.position_fetch,
            opacity_micromap: self.device.opacity_micromap,
            invocation_reorder: self.device.invocation_reorder,
            pipeline_statistics: self.device.pipeline_statistics,
        }
    }
    /// native backing allocation 字节数，包含 AS 池及交给驱动回收线程、尚未确认释放的分配。
    pub fn allocated_bytes(&self) -> u64 {
        if let Some(retirement) = &self.buffer_retirement {
            retirement.account(&self.device);
        }
        self.device.buffer_bytes.load(Ordering::Relaxed)
    }

    /// AS 分配池的当前闲置字节与累计命中；借出的资源不计入 idle。
    pub const fn as_pool_stats(&self) -> crate::AsPoolStats {
        self.as_pool.stats()
    }

    /// AS 池当前闲置 allocation 字节，已包含在 [`Self::allocated_bytes`]；常数时间读取。
    pub const fn build_pool_idle_bytes(&self) -> u64 {
        self.as_pool.stats().idle_bytes
    }
    /// 遍历命令记录，统计 AS input/scratch 的实际待提交与在途分配，不等待 GPU。
    /// 不包含 AS backing 或 pool idle；这些和结果均已计入 [`Self::allocated_bytes`]。
    pub fn as_build_memory(&self) -> crate::AsBuildMemory {
        let mut result = crate::AsBuildMemory::default();
        for flight in &self.terrain_flights {
            let (bytes, buffers) = flight.memory();
            result.inflight_bytes += bytes;
            result.inflight_buffers += buffers;
        }
        for (_, record) in self.life.records() {
            let (bytes, buffers) = record.buffers().memory();
            match record.state() {
                State::Pending => {
                    result.pending_bytes += bytes;
                    result.pending_buffers += buffers;
                }
                State::Submitting(_)
                | State::Submitted(_)
                | State::External
                | State::Synchronous
                | State::Uncertain => {
                    result.inflight_bytes += bytes;
                    result.inflight_buffers += buffers;
                }
            }
        }
        result
    }
    /// 资源表、命令和退役元数据的 CPU 保留容量；不含 Self、驱动与外部 wgpu 内部分配。
    pub fn retained_cpu_bytes(&self) -> u64 {
        let data: usize = self
            .life
            .resources()
            .map(|(_, data)| data.retained_bytes())
            .sum();
        let records: usize = self
            .life
            .records()
            .map(|(_, record)| record.metadata_bytes())
            .sum();
        (self.life.retained_bytes() + data + records + self.as_pool.retained_bytes()) as u64
    }
    #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
    pub(crate) fn record_pending(&self, key: RecordId) -> bool {
        self.life
            .record(key)
            .is_ok_and(|record| record.state() == State::Pending)
    }
    fn get(&self, key: Key) -> Result<&Data, Error> {
        self.life.resource(key).map_err(Into::into)
    }
    fn insert(&mut self, data: Data) -> Result<Key, Error> {
        if let Err(error) = self.device.ensure_active() {
            data.recycle(&self.device, &mut self.as_pool);
            return Err(error);
        }
        match self.life.insert_resource(data) {
            Ok(key) => Ok(key),
            Err((error, data)) => {
                data.recycle(&self.device, &mut self.as_pool);
                Err(error.into())
            }
        }
    }
    fn discard(&mut self, key: Key) {
        if let Some(data) = self.life.take_resource(key) {
            match (data, &mut self.buffer_retirement) {
                (Data::Buffer(buffer), Some(queue)) => {
                    if let Err(mut immediate) = queue.enqueue(&self.device, buffer) {
                        immediate.destroy(&self.device);
                    }
                }
                (other, _) => other.recycle(&self.device, &mut self.as_pool),
            }
        }
    }
    fn active(&self, key: Key) -> Result<(), Error> {
        self.device.ensure_active()?;
        self.life.active(key).map_err(Into::into)
    }
    fn producer_submitted(&self, key: Key) -> Result<(), Error> {
        if let Some(producer) = self.get(key)?.producer()
            && !matches!(self.record(producer)?.state(), State::Submitted(_))
        {
            return Err(Error::InvalidState("AS 构建尚未提交"));
        }
        Ok(())
    }
    fn insert_build_record(&mut self, record: Record) -> Result<RecordId, Error> {
        match self.life.insert_record(record) {
            Ok(id) => Ok(id),
            Err((error, record)) => {
                record
                    .into_buffers()
                    .recycle(&self.device, &mut self.as_pool);
                Err(error.into())
            }
        }
    }

    fn recycle_record(&mut self, record: Record, completed: bool) {
        if completed {
            for operation in record.operations() {
                if let Some(key) = operation.resets()
                    && let Some(data) = self.life.queries_mut(key)
                {
                    data.initialized = true;
                }
            }
        }
        record
            .into_buffers()
            .recycle(&self.device, &mut self.as_pool);
    }

    pub(crate) fn validate_submission(&self, keys: &[RecordId]) -> Result<(), Error> {
        self.device.ensure_active()?;
        let mut preceding = std::collections::HashSet::new();
        preceding.try_reserve(keys.len())?;
        let mut visited = std::collections::HashSet::new();
        let mut stack = Vec::new();
        for &key in keys {
            let record = self.record(key)?;
            if record.state() != State::Pending || preceding.contains(&key) {
                return Err(Error::InvalidState("命令已经提交、重复或外部编码"));
            }
            record.roots(&mut stack);
            while let Some(resource) = stack.pop() {
                if visited.contains(&resource) {
                    continue;
                }
                let data = self.get(resource)?;
                if let Some(producer) = data.producer() {
                    let producer_record = self.record(producer)?;
                    if producer != key
                        && !preceding.contains(&producer)
                        && !matches!(producer_record.state(), State::Submitted(_))
                    {
                        return Err(Error::InvalidState("AS producer 必须先于消费者提交"));
                    }
                }
                visited.try_reserve(1)?;
                visited.insert(resource);
                // 已构建或已排队的 AS 已证明子 producer；仅本次新建输出检查其子依赖。
                if matches!(data, Data::Blas(_) | Data::Acceleration(_))
                    && data.producer() != Some(key)
                {
                    continue;
                }
                data.dependencies(&mut stack);
            }
            preceding.insert(key);
        }
        Ok(())
    }
    fn retire_ready(&mut self) {
        let mut cursor = 0;
        while let Some(mut group) = self.life.take_ready(&mut cursor) {
            group
                .resources
                .sort_unstable_by_key(|key| self.life.resource(*key).map_or(u8::MAX, Data::rank));
            for key in group.resources {
                self.discard(key);
            }
        }
    }

    fn record(&self, key: RecordId) -> Result<&Record, Error> {
        self.life.record(key).map_err(Into::into)
    }

    fn resource_idle(&self, key: Key) -> Result<(), Error> {
        if self
            .life
            .records()
            .any(|(_, record)| record.uses().binary_search(&key).is_ok())
        {
            return Err(Error::InvalidState("资源已被待执行或在途命令使用"));
        }
        Ok(())
    }
    /// 同步构建 BLAS；输入仅在构建期间借用。
    pub fn triangle_blas(
        &mut self,
        triangles: &[[[f32; 3]; 3]],
        opaque: bool,
    ) -> Result<crate::TriangleBlas, Error> {
        let data = BlasData::native(&self.device, triangles, opaque)?;
        Ok(crate::TriangleBlas {
            key: self.insert(Data::Blas(data))?,
        })
    }

    /// 同步构建一批独立 BLAS；共用一份输入与互不重叠的 scratch，只提交和等待一次。
    ///
    /// 支持借入的原生设备与 wgpu 适配设备，输入在返回后即可释放。每项显式指定
    /// opaque 覆盖和可选 OMM，输出次序与输入一致；空批不提交。新标识全部成功后才返回，
    /// 失败保留既有资源；临时分配与独立 backing 按 Context 的池上限复用。
    /// 原生宿主继续遵守 [`Self::from_raw`] 的队列宿主访问串行化与同队列提交契约。
    /// # Errors
    /// 非有限、空几何、数量超限、跨设备或无效 OMM、分配、录制及提交/完成失败保留来源。
    pub fn triangle_blas_batch(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
    ) -> Result<crate::TriangleBlasBatch, Error> {
        self.triangle_blas_batch_timed(inputs, false)
    }

    pub(crate) fn triangle_blas_batch_timed(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
        timestamps: bool,
    ) -> Result<crate::TriangleBlasBatch, Error> {
        self.prepare_triangle_blas_batch(
            crate::acceleration::Sources::Triangles(inputs),
            timestamps,
            false,
        )
        .map(|(batch, _)| batch)
    }

    pub(crate) fn record_triangle_blas_batch(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
    ) -> Result<crate::interop::PreparedBlasBatch, Error> {
        self.record_blas_inputs(crate::acceleration::Sources::Triangles(inputs))
    }

    pub(crate) fn record_blas_inputs(
        &mut self,
        inputs: crate::acceleration::Sources<'_>,
    ) -> Result<crate::interop::PreparedBlasBatch, Error> {
        let (batch, commands) = self.prepare_triangle_blas_batch(inputs, false, true)?;
        Ok(crate::interop::PreparedBlasBatch {
            blases: batch.blases,
            commands: commands.expect("录制型 BLAS 始终返回命令"),
            timing: batch.timing,
        })
    }

    fn resolve_blas_inputs<'a>(
        device: &Device,
        life: &'a Life<Native, BuildBuffers>,
        inputs: crate::acceleration::Sources<'a>,
    ) -> Result<Vec<ResolvedBlas<'a>>, Error> {
        let mut resolved = Vec::new();
        resolved.try_reserve_exact(inputs.len())?;
        use crate::acceleration::{Positions, Sources};
        let mut writes = Vec::new();
        for i in 0..inputs.len() {
            let mut destinations = [0; 4];
            let mut region = [0; 3];
            let (positions, opaque, opacity) = match inputs {
                Sources::Triangles(v) => (
                    Positions::Triangles(v[i].triangles),
                    v[i].opaque,
                    v[i].opacity,
                ),
                Sources::Quads(v) | Sources::Uploaded(v, _) => {
                    let src = &v[i];
                    region = src.output.region;
                    let count = src.quads.positions().headers().len() as u64;
                    for (j, (target, stride, base)) in src.output.columns().into_iter().enumerate()
                    {
                        let Some(target) = target else { continue };
                        life.active(target.key)?;
                        let Data::Buffer(data) = life.resource(target.key)? else {
                            return Err(Error::InvalidArgument("quad 输出不是 buffer"));
                        };
                        let start = base + u64::from(src.output.first) * stride;
                        let end = start + count * stride;
                        if end > target.len() {
                            return Err(Error::InvalidArgument("quad 输出槽超过设备缓冲范围"));
                        }
                        writes.try_reserve(1)?;
                        writes.push((target.key, start, end));
                        destinations[j] = data.address(device) + start;
                    }
                    (Positions::Quads(src.quads), src.opaque, src.opacity)
                }
            };
            let opacity = if let Some((id, indices)) = opacity {
                life.active(id.key)?;
                let Data::Micromap(map) = life.resource(id.key)? else {
                    return Err(Error::InvalidArgument("OMM 标识类型无效"));
                };
                Some((map, id, indices))
            } else {
                None
            };
            resolved.push(ResolvedBlas {
                positions,
                uploaded: if let Sources::Uploaded(_, staged) = inputs {
                    let b = staged[i].buffer;
                    life.active(b.key)?;
                    let Data::Buffer(data) = life.resource(b.key)? else {
                        return Err(Error::InvalidArgument("地形源不是设备缓冲"));
                    };
                    Some(data.address(device))
                } else {
                    None
                },
                destinations,
                region,
                opaque,
                opacity,
            });
        }
        writes.sort_unstable();
        if writes
            .windows(2)
            .any(|w| w[0].0 == w[1].0 && w[0].2 > w[1].1)
        {
            return Err(Error::InvalidArgument("quad 输出切片重叠"));
        }
        Ok(resolved)
    }

    fn prepare_triangle_blas_batch(
        &mut self,
        inputs: crate::acceleration::Sources<'_>,
        timestamps: bool,
        record_only: bool,
    ) -> Result<
        (
            crate::TriangleBlasBatch,
            Option<crate::interop::CommandBuffer>,
        ),
        Error,
    > {
        let started = std::time::Instant::now();
        let resolved = Self::resolve_blas_inputs(&self.device, &self.life, inputs)?;
        let mut access = Vec::new();
        let mut uses = Vec::new();
        if let crate::acceleration::Sources::Quads(v) | crate::acceleration::Sources::Uploaded(v, _) =
            inputs
            && record_only
        {
            let mut buffers = Vec::new();
            buffers.try_reserve_exact(
                v.len()
                    .checked_mul(5)
                    .ok_or(Error::InvalidArgument("输出列数量溢出"))?,
            )?;
            buffers.extend(
                v.iter()
                    .flat_map(|v| v.output.columns())
                    .filter_map(|(b, _, _)| b),
            );
            if let crate::acceleration::Sources::Uploaded(_, staged) = inputs {
                buffers.extend(staged.iter().map(|s| s.buffer));
            }
            buffers.sort_unstable_by_key(|b| b.key);
            buffers.dedup_by_key(|b| b.key);
            access.try_reserve_exact(buffers.len())?;
            uses.try_reserve_exact(buffers.len())?;
            for b in buffers {
                access.push(Op::Access(b.into()));
                uses.push(b.key);
            }
        }
        let mut blases = Vec::new();
        let mut roots = Vec::new();
        blases.try_reserve_exact(inputs.len())?;
        if record_only {
            roots.try_reserve_exact(inputs.len())?;
        }
        let validation_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let output = BlasData::batch(
            &self.device,
            &resolved,
            timestamps,
            if record_only {
                BatchMode::Wgpu
            } else {
                BatchMode::Sync
            },
            &mut self.as_pool,
            self.quad_expansion.as_ref(),
        )?;
        drop(resolved);
        let mut timing = output.timing;
        let mut pending = output.recorded;
        let mut data = output.blases.into_iter();
        while let Some(blas) = data.next() {
            match self.insert(Data::Blas(blas)) {
                Ok(key) => {
                    blases.push(crate::TriangleBlas { key });
                    if record_only {
                        roots.push(key);
                    }
                }
                Err(error) => {
                    if let Some((_, buffers)) = pending.take() {
                        buffers.recycle(&self.device, &mut self.as_pool);
                    }
                    for remaining in data {
                        remaining.recycle(&self.device, &mut self.as_pool);
                    }
                    for blas in blases {
                        self.discard(blas.key);
                    }
                    return Err(error);
                }
            }
        }
        let commands = if let Some((raw, buffers)) = pending {
            match self.insert_build_record(Record::new(access, uses, roots, buffers)) {
                Ok(key) => Some(crate::interop::CommandBuffer::recorded(raw, key)),
                Err(error) => {
                    for blas in blases {
                        self.discard(blas.key);
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        timing.validation_ns = timing.validation_ns.saturating_add(validation_ns);
        timing.pool_idle_bytes = self.as_pool.stats().idle_bytes;
        timing.total_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        Ok((crate::TriangleBlasBatch { blases, timing }, commands))
    }

    /// 以一次提交和一次完成等待构建新 BLAS 及其场景 TLAS。
    ///
    /// 实例可引用已有 BLAS 或同批输入索引；实例顺序与 BLAS 输入顺序分别保留。调用期间
    /// 保留全部输入和 scratch，实际完成后归池。宿主须按 [`Self::from_raw`] 独占队列。
    ///
    /// # Errors
    /// 设备不支持、来源无效或分配、录制、提交及等待失败时不返回任何新标识，已有资源有效。
    pub fn triangle_scene_batch(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
    ) -> Result<crate::TriangleSceneBatch, Error> {
        self.scene_batch(
            crate::acceleration::Sources::Triangles(inputs),
            instances,
            false,
            None,
        )
    }

    /// 在同一命令中把紧凑 quad 展开到设备临时缓冲，构建 BLAS，再构建 TLAS。
    /// 同步宿主返回时均已完成，输入顺序和两个半面的编号保持不变。
    /// # Errors
    /// 展开管线未准备、输入无效、设备或分配失败时，保留已有场景。
    pub fn quad_scene_batch(
        &mut self,
        inputs: &[crate::QuadBlasInput<'_>],
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
    ) -> Result<crate::TriangleSceneBatch, Error> {
        self.scene_batch(
            crate::acceleration::Sources::Quads(inputs),
            instances,
            false,
            None,
        )
    }

    /// 向借入的原生宿主队列提交地形展开、BLAS 与 TLAS，后续追踪按同队列顺序观察新场景。
    /// GPU 完成 fence 之前由 Context 保活输入、scratch、输出及全部加速结构。
    /// # Errors
    /// 输入、设备或提交失败保留上个场景。只支持由调用方独占队列的原生宿主。
    pub fn enqueue_quad_scene(
        &mut self,
        inputs: &[crate::QuadBlasInput<'_>],
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
    ) -> Result<crate::TriangleSceneBatch, Error> {
        self.scene_batch(
            crate::acceleration::Sources::Quads(inputs),
            instances,
            true,
            None,
        )
    }
    /// 按本帧可用字节提交区域。超额包分帧上传；返回 None 时旧覆盖继续渲染且不能提交候选目录。
    /// # Errors
    /// 同 [`Self::enqueue_quad_scene`]；帧预算必须来自此渲染所有者，暂存 buffer 参与 collect。
    pub fn stream_quad_scene(
        &mut self,
        uploads: &mut crate::TerrainUploads,
        available: usize,
        inputs: &[crate::QuadBlasInput<'_>],
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
    ) -> Result<Option<crate::TriangleSceneBatch>, Error> {
        use crate::{acceleration::Sources, terrain_upload::Progress};
        let tail = instances
            .len()
            .checked_mul(64)
            .ok_or(Error::InvalidArgument("TLAS 实例容量溢出"))?;
        let staged = uploads.advance(self, inputs, available, tail)?;
        let scene = match staged {
            Progress::Direct => self.scene_batch(Sources::Quads(inputs), instances, true, None)?,
            Progress::Staged { start, buffers } => {
                if !buffers.is_empty() {
                    let window = &inputs[start..start + buffers.len()];
                    let (batch, _, _) =
                        self.scene_parts(Sources::Uploaded(window, &buffers), None, true, None)?;
                    uploads.remember(window, &batch)?;
                }
                let Some(built) = uploads.ready(inputs.len(), tail)? else {
                    return Ok(None);
                };
                let mut current = Vec::new();
                current.try_reserve_exact(instances.len())?;
                let mut existing = Vec::new();
                existing.try_reserve_exact(instances.len())?;
                for i in instances {
                    let id = match i.geometry {
                        crate::BatchBlas::Existing(id) => id,
                        crate::BatchBlas::New(k) => *built
                            .blases
                            .get(k)
                            .ok_or(Error::InvalidArgument("新BLAS下标越界"))?,
                    };
                    current.push(crate::TriangleInstance {
                        geometry: id,
                        transform: i.transform,
                        custom_index: i.custom_index,
                    });
                    existing.push(crate::TriangleInstance {
                        geometry: crate::BatchBlas::Existing(id),
                        transform: i.transform,
                        custom_index: i.custom_index,
                    });
                }
                let buffer = if tail > uploads.limit() {
                    let bytes = self.instance_payload(&current)?;
                    let Some(b) = uploads.instance_buffer(self, &bytes)? else {
                        return Ok(None);
                    };
                    Some(b)
                } else {
                    None
                };
                let mut scene =
                    self.scene_batch(Sources::Triangles(&[]), &existing, true, buffer)?;
                scene.blas = built;
                scene
            }
        };
        uploads.clear();
        Ok(Some(scene))
    }

    fn scene_batch(
        &mut self,
        inputs: crate::acceleration::Sources<'_>,
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
        queued: bool,
        instance_buffer: Option<Buffer>,
    ) -> Result<crate::TriangleSceneBatch, Error> {
        let (blas, top, tlas_prepare_ns) =
            self.scene_parts(inputs, Some(instances), queued, instance_buffer)?;
        Ok(crate::TriangleSceneBatch {
            blas,
            top: top.expect("请求了 TLAS"),
            tlas_prepare_ns,
        })
    }

    fn scene_parts(
        &mut self,
        inputs: crate::acceleration::Sources<'_>,
        top_input: Option<&[crate::TriangleInstance<crate::BatchBlas>]>,
        queued: bool,
        instance_buffer: Option<Buffer>,
    ) -> Result<
        (
            crate::TriangleBlasBatch,
            Option<crate::AccelerationStructure>,
            u64,
        ),
        Error,
    > {
        let instances = top_input.unwrap_or(&[]);
        self.device.ensure_active()?;
        if queued {
            self.poll_terrain()?;
        }
        for instance in instances {
            match instance.geometry {
                crate::BatchBlas::Existing(blas) => {
                    self.active(blas.key)?;
                    self.producer_submitted(blas.key)?;
                }
                crate::BatchBlas::New(index) if index >= inputs.len() => {
                    return Err(Error::InvalidArgument("实例的新 BLAS 索引超出批输入"));
                }
                crate::BatchBlas::New(_) => {}
            }
        }
        let mut blases = Vec::new();
        blases.try_reserve_exact(inputs.len())?;
        let mut top = None;
        let mut blas_buffers = BuildBuffers::default();
        let mut tlas_buffers = BuildBuffers::default();
        let mut timing = crate::BuildTiming::default();
        let mut tlas_prepare_ns = 0;
        let mut preparation_ns = 0;
        let started = std::time::Instant::now();
        let elapsed = |instant: std::time::Instant| {
            u64::try_from(instant.elapsed().as_nanos()).unwrap_or(u64::MAX)
        };
        let instance_address = instance_buffer
            .map(|b| -> Result<u64, Error> { Ok(b.data(self)?.address(&self.device)) })
            .transpose()?;
        let device = &self.device;
        let pool = &mut self.as_pool;
        let life = &mut self.life;
        let encode = |command| {
            let preparation = std::time::Instant::now();
            let resolved = Self::resolve_blas_inputs(device, life, inputs)?;
            let validation_ns = elapsed(preparation);
            let output = BlasData::batch(
                device,
                &resolved,
                false,
                BatchMode::Raw(command),
                pool,
                self.quad_expansion.as_ref(),
            )?;
            drop(resolved);
            timing = output.timing;
            timing.validation_ns = timing.validation_ns.saturating_add(validation_ns);
            blas_buffers = output.raw_buffers;
            let mut data = output.blases.into_iter();
            while let Some(blas) = data.next() {
                match life.insert_resource(Data::Blas(blas)) {
                    Ok(key) => blases.push(crate::TriangleBlas { key }),
                    Err((error, blas)) => {
                        blas.recycle(device, pool);
                        for remaining in data {
                            remaining.recycle(device, pool);
                        }
                        return Err(error.into());
                    }
                }
            }
            if top_input.is_none() {
                preparation_ns = elapsed(preparation);
                return Ok(());
            }
            let tlas_started = std::time::Instant::now();
            let mut resolved = Vec::new();
            resolved.try_reserve_exact(instances.len())?;
            for instance in instances {
                let id = match instance.geometry {
                    crate::BatchBlas::Existing(blas) => blas,
                    crate::BatchBlas::New(index) => blases[index],
                };
                let Data::Blas(geometry) = life.resource(id.key)? else {
                    return Err(Error::InvalidArgument("BLAS 标识类型无效"));
                };
                resolved.push((
                    id,
                    ResolvedInstance {
                        geometry,
                        transform: instance.transform,
                        custom_index: instance.custom_index,
                    },
                ));
            }
            let (data, buffers) = AccelerationData::record_instances_raw(
                device,
                &resolved,
                pool,
                instance_address,
                command,
            )?;
            drop(resolved);
            tlas_buffers = buffers;
            match life.insert_resource(Data::Acceleration(data)) {
                Ok(key) => top = Some(crate::AccelerationStructure { key }),
                Err((error, data)) => {
                    data.recycle(device, pool);
                    return Err(error.into());
                }
            }
            tlas_prepare_ns = elapsed(tlas_started);
            preparation_ns = elapsed(preparation);
            Ok(())
        };
        let mut raw = None;
        let result = if queued {
            terrain_flight::Recording::record(device, encode).map(|recorded| raw = Some(recorded))
        } else {
            immediate(device, encode)
        };
        let result = if let Some(raw) = raw {
            let buffers = [blas_buffers, tlas_buffers];
            self.submit_terrain(raw, buffers, |op| {
                if let Some(top) = top {
                    op.keepalive(top.into())?;
                }
                for &blas in &blases {
                    op.keepalive(blas.into())?;
                }
                if let Some(buffer) = instance_buffer {
                    op.access(buffer.into())?;
                }
                if let crate::acceleration::Sources::Quads(v)
                | crate::acceleration::Sources::Uploaded(v, _) = inputs
                {
                    for b in v
                        .iter()
                        .flat_map(|v| v.output.columns())
                        .filter_map(|(b, _, _)| b)
                    {
                        op.access(b.into())?;
                    }
                }
                if let crate::acceleration::Sources::Uploaded(_, staged) = inputs {
                    for s in staged {
                        op.access(s.buffer.into())?;
                    }
                }
                Ok(())
            })
        } else {
            blas_buffers.recycle(&self.device, &mut self.as_pool);
            tlas_buffers.recycle(&self.device, &mut self.as_pool);
            result
        };
        timing.submit_wait_ns = elapsed(started).saturating_sub(preparation_ns);
        if let Err(error) = result {
            if let Some(top) = top {
                self.discard(top.key);
            }
            for blas in blases {
                self.discard(blas.key);
            }
            return Err(error);
        }
        timing.pool_idle_bytes = self.as_pool.stats().idle_bytes;
        timing.total_ns = elapsed(started);
        Ok((
            crate::TriangleBlasBatch { blases, timing },
            top,
            tlas_prepare_ns,
        ))
    }

    pub(crate) fn record_acceleration_instances(
        &mut self,
        instances: &[crate::TriangleInstance],
    ) -> Result<crate::interop::PreparedTlas, Error> {
        self.record_instances_from(instances, None)
    }
    pub(crate) fn instance_payload(
        &self,
        instances: &[crate::TriangleInstance],
    ) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(
            instances
                .len()
                .checked_mul(64)
                .ok_or(Error::InvalidArgument("TLAS 字节数溢出"))?,
        )?;
        for i in instances {
            self.active(i.geometry.key)?;
            let geometry = self.blas(i.geometry)?;
            bytes.extend_from_slice(&crate::acceleration::instance_bytes(&ResolvedInstance {
                geometry,
                transform: i.transform,
                custom_index: i.custom_index,
            })?);
        }
        Ok(bytes)
    }
    pub(crate) fn record_instances_from(
        &mut self,
        instances: &[crate::TriangleInstance],
        input: Option<Buffer>,
    ) -> Result<crate::interop::PreparedTlas, Error> {
        let address = input
            .map(|b| -> Result<u64, Error> { Ok(b.data(self)?.address(&self.device)) })
            .transpose()?;
        let mut resolved = Vec::new();
        resolved.try_reserve_exact(instances.len())?;
        let mut roots = Vec::new();
        roots.try_reserve_exact(1)?;
        for instance in instances {
            self.active(instance.geometry.key)?;
            let Data::Blas(geometry) = self.life.resource(instance.geometry.key)? else {
                return Err(Error::InvalidArgument("BLAS 标识类型无效"));
            };
            resolved.push((
                instance.geometry,
                ResolvedInstance {
                    geometry,
                    transform: instance.transform,
                    custom_index: instance.custom_index,
                },
            ));
        }
        let (data, raw, buffers) = AccelerationData::record_instances(
            &self.device,
            &resolved,
            &mut self.as_pool,
            address,
        )?;
        drop(resolved);
        let key = match self.insert(Data::Acceleration(data)) {
            Ok(key) => key,
            Err(error) => {
                buffers.recycle(&self.device, &mut self.as_pool);
                return Err(error);
            }
        };
        roots.push(key);
        let record = match self.insert_build_record(Record::new(
            input.into_iter().map(|b| Op::Access(b.into())).collect(),
            input.into_iter().map(|b| b.key).collect(),
            roots,
            buffers,
        )) {
            Ok(record) => record,
            Err(error) => {
                self.discard(key);
                return Err(error);
            }
        };
        Ok(crate::interop::PreparedTlas {
            acceleration: crate::AccelerationStructure { key },
            commands: crate::interop::CommandBuffer::recorded(raw, record),
        })
    }
    /// 同步构建 TLAS；同设备 BLAS/OMM 依赖由本 owner 保存。
    pub fn acceleration_instances(
        &mut self,
        instances: &[crate::TriangleInstance],
    ) -> Result<crate::AccelerationStructure, Error> {
        let data = AccelerationData::native_instances(&self.device, instances, |id| {
            self.active(id.key)?;
            self.producer_submitted(id.key)?;
            self.blas(id)
        })?;
        Ok(crate::AccelerationStructure {
            key: self.insert(Data::Acceleration(data))?,
        })
    }
    /// 同步构建精确二值 OMM；支持原生与 wgpu 设备，返回后释放输入借用和暂存。
    /// 逻辑设备须启用 KHR OMM。Context 管理其生命周期，BLAS 自动保留依赖；
    /// 尚未被 BLAS 引用的表须作为 collect 的根保活。失败保留原有资源。
    /// # Errors
    /// 不支持、输入或设备失败返回来源；无法确认完成时隔离在途资源。
    pub fn opacity_micromap(
        &mut self,
        maps: &[crate::OpacityMap<'_>],
    ) -> Result<crate::OpacityMicromap, Error> {
        let data = MicromapData::new(&self.device, maps)?;
        let count = data.map_count();
        Ok(crate::OpacityMicromap {
            key: self.insert(Data::Micromap(data))?,
            count,
        })
    }
    pub(crate) fn triangle_blas_with_opacity(
        &mut self,
        triangles: &[[[f32; 3]; 3]],
        micromap: crate::OpacityMicromap,
        indices: &[u32],
    ) -> Result<crate::TriangleBlas, Error> {
        self.active(micromap.key)?;
        let Data::Micromap(omm) = self.get(micromap.key)? else {
            return Err(Error::InvalidArgument("OMM 标识类型无效"));
        };
        let data = BlasData::with_opacity(&self.device, triangles, (omm, micromap), indices)?;
        Ok(crate::TriangleBlas {
            key: self.insert(Data::Blas(data))?,
        })
    }
    fn blas(&self, id: crate::TriangleBlas) -> Result<&BlasData, Error> {
        match self.get(id.key)? {
            Data::Blas(data) => Ok(data),
            _ => Err(Error::InvalidArgument("BLAS 标识类型无效")),
        }
    }
    fn acceleration(&self, id: crate::AccelerationStructure) -> Result<&AccelerationData, Error> {
        match self.get(id.key)? {
            Data::Acceleration(data) => Ok(data),
            _ => Err(Error::InvalidArgument("TLAS 标识类型无效")),
        }
    }
    fn layout(&self, key: Key) -> Result<&DescriptorLayout, Error> {
        match self.get(key)? {
            Data::Layout(data) => Ok(data),
            _ => Err(Error::InvalidArgument("layout 标识类型无效")),
        }
    }
    fn ray_program(&self, key: Key) -> Result<&RayProgram, Error> {
        match self.get(key)? {
            Data::RayProgram(data) => Ok(data),
            _ => Err(Error::InvalidArgument("RT program 标识类型无效")),
        }
    }
    fn compute_program(&self, key: Key) -> Result<&ComputeProgram, Error> {
        match self.get(key)? {
            Data::ComputeProgram(data) => Ok(data),
            _ => Err(Error::InvalidArgument("compute program 标识类型无效")),
        }
    }
    /// 取消尚未交给外部宿主或提交的命令；外部已编码命令不能冒充取消。
    pub fn cancel(&mut self, recorded: Recorded) -> Result<(), Error> {
        let record = self.life.take_pending(recorded.key)?;
        self.recycle_record(record, false);
        self.retire_ready();
        Ok(())
    }
    /// 取消本 owner 全部尚未提交的命令，用于整个准备批次失败后的清理。
    /// 已编码的外部批次和已提交批次不受影响。
    pub fn cancel_pending(&mut self) {
        let ids: Vec<_> = self
            .life
            .records()
            .filter_map(|(id, record)| (record.state() == State::Pending).then_some(id))
            .collect();
        for id in ids {
            let record = self.life.take_pending(id).expect("同一所有者的pending记录");
            self.recycle_record(record, false);
        }
        self.retire_ready();
    }
    /// 关闭已完成的外部批次。
    /// # Safety
    /// 宿主已确认此批次在 Context 同一 VkQueue 上的全部 GPU 使用结束；
    /// 丢弃已编码 command 需保证以后不提交。
    pub unsafe fn complete(&mut self, flight: Flight) -> Result<(), Error> {
        self.device.ensure_active()?;
        let record = self.life.take_external(flight.key)?;
        self.recycle_record(record, true);
        self.retire_ready();
        Ok(())
    }

    pub(crate) fn completed(&mut self) {
        if !self.device.health.is_active() {
            return;
        }
        let serial = self.life.completed_serial();
        let ids: Vec<_> = self
            .life
            .records()
            .filter_map(|(id, record)| record.state().completed(serial).then_some(id))
            .collect();
        for id in ids {
            let record = self.life.take_completed(id).expect("完成水位覆盖的记录");
            self.recycle_record(record, true);
        }
        self.retire_ready();
    }
    /// 保留发布根和全部待执行/在途依赖，退役其余版本；只在资源换代或预算回收时调用。
    ///
    /// 标识不自动保活；退休标识不能建立新依赖，已录制命令仍可提交。每组只等待首次
    /// 撤销发布时已有的命令；后续 collect 不扩充其等待集合，未来帧不延长旧版寿命。
    /// 仍存在的版本可重新发布；完整依赖遍历仅在本方法执行。
    /// 直接借用真实资源的邻接，只遍历可达边，不复制全图或存续组的等待集合。
    /// 闭包计划失败后不提交退役更新。地形设备把已完成 GPU 使用的大缓冲移交有界回收线程。
    /// 显式传空根时还会收齐该线程的释放回执，全程不等待 GPU。
    /// # Errors
    /// 根或依赖的归属/版本无效时返回错误；资源、记录及退役状态不变。
    /// 设备已失效时拒绝发布和回收，保留在途资源交由析构清理。
    pub fn collect(&mut self, roots: &[Resource]) -> Result<(), Error> {
        self.device.ensure_active()?;
        self.collect_inner(roots)
    }
    fn collect_inner(&mut self, roots: &[Resource]) -> Result<(), Error> {
        let plan = self.life.collect_plan(roots).map_err(|error| match error {
            graph::PlanError::ForeignOwner => Error::InvalidArgument("资源属于其他设备"),
            graph::PlanError::InvalidKey => Error::InvalidState("资源版本已失效"),
            graph::PlanError::InvalidShape => Error::InvalidState("资源邻接身份无效"),
            graph::PlanError::Allocation(error) => error.into(),
        })?;
        self.recycle_unkept(&plan.keep)?;
        self.retire_ready();
        if roots.is_empty()
            && let Some(queue) = &mut self.buffer_retirement
        {
            queue.drain(&self.device);
        }
        Ok(())
    }

    fn recycle_unkept(&mut self, keep: &[bool]) -> Result<(), Error> {
        let mut retired = Vec::new();
        for (key, data) in self
            .life
            .resources()
            .filter(|(key, _)| !keep[key.slot as usize])
        {
            retired.try_reserve(1)?;
            retired.push((data.rank(), key.slot, key.generation));
        }
        retired.sort_unstable_by_key(|v| v.0);
        for (_, slot, generation) in retired {
            // 本 owner 的临时排序表只携带槽位和代次。
            let key = Key {
                owner: self.owner(),
                slot,
                generation,
            };
            self.discard(key);
        }
        Ok(())
    }
    pub(crate) fn encode(&self, key: RecordId, command: vk::CommandBuffer) -> Result<(), Error> {
        self.record(key)?.encode(self, command)
    }
    pub(crate) fn track(
        &self,
        key: RecordId,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), Error> {
        self.record(key)?.track(self, encoder)
    }
}
impl Context {
    /// 无法证明 GPU 停止或外部 SDK 释放失败时，永久保留资源及其依赖所有者。
    ///
    /// 此错误边界不等待、不分配，也不回收保留对象。所有资源身份失效，设备进入故障状态；
    /// 后续操作不能恢复本 Context。若通过 [`Self::from_raw`] 借入宿主设备，宿主还须保留
    /// 原始设备、实例及外部 SDK 所有者，不能在本 Context 析构后销毁仍被保留的依赖。
    pub fn retain_failed_shutdown(&mut self) {
        self.device.quarantine();
        let owner = self.owner();
        std::mem::forget(std::mem::replace(&mut self.life, Life::new(owner)));
        std::mem::forget(std::mem::take(&mut self.as_pool));
        std::mem::forget(std::mem::take(&mut self.terrain_flights));
        std::mem::forget(self.pipeline_cache.take());
        std::mem::forget(self.device._owner.take());
        std::mem::forget(self.device.wgpu_queue.take());
        std::mem::forget(self.device.physical.instance._owner.take());
        std::mem::forget(self.device._loader.take());
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        drop(self.buffer_retirement.take());
        if self
            .life
            .records()
            .any(|(_, r)| r.state() != State::Pending)
            || self.device.is_quarantined()
        {
            let result = self.device.wait_idle();
            if !crate::context::execution_ended(result) {
                log::error!("无法确认 native 队列已结束，保留设备与在途资源：{result:?}");
                self.retain_failed_shutdown();
                return;
            }
        }
        let owner = self.owner();
        for flight in self.terrain_flights.drain(..) {
            flight.release(&self.device, &mut self.as_pool);
        }
        let life = std::mem::replace(&mut self.life, Life::new(owner));
        let resources = life.into_shutdown(|record| record.into_buffers().destroy(&self.device));
        for mut resource in resources {
            resource.destroy(&self.device);
        }
        self.as_pool.destroy(&self.device);
        if let Some(mut expansion) = self.quad_expansion.take() {
            expansion.destroy(&self.device);
        }
        if let Some(mut builder) = self.light_builder.take() {
            builder.destroy(&self.device);
        }
        if let Some(cache) = self.pipeline_cache.take() {
            cache.destroy(&self.device);
        }
    }
}
impl WgpuContext {
    /// 把互不重叠的旧缓冲前缀复制到新缓冲；同队列按序提交并保活两代，不等待 GPU。
    /// # Errors
    /// 缓冲未激活、源/目标别名、范围无效、分配或队列提交失败时返回错误。
    pub fn copy_buffer_prefixes(&mut self, copies: &[(Buffer, Buffer, u64)]) -> Result<(), Error> {
        if copies.is_empty() {
            return Ok(());
        }
        self.drain_completed();
        let context = &mut self.native;
        let mut operations = Commands::new(context)?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(copies.len())?;
        for (i, &(src, dst, bytes)) in copies.iter().enumerate() {
            if bytes == 0
                || copies[..i].iter().any(|&(_, written, _)| dst == written)
                || !bytes.is_multiple_of(4)
                || bytes > src.len().min(dst.len())
                || copies.iter().any(|(source, _, _)| *source == dst)
            {
                return Err(Error::InvalidArgument("设备前缀复制范围无效"));
            }
            operations.access(src.into())?;
            operations.access(dst.into())?;
            raw.push((src.data(context)?.raw, dst.data(context)?.raw, bytes));
        }
        let commands = crate::interop::record(&context.device, "copy terrain columns", |cmd| {
            unsafe {
                memory_barrier(&context.device, cmd);
                for &(src, dst, bytes) in &raw {
                    context.device.raw.cmd_copy_buffer(
                        cmd,
                        src,
                        dst,
                        &[vk::BufferCopy::default().size(bytes)],
                    );
                }
                memory_barrier(&context.device, cmd);
            }
            Ok(())
        })?;
        let record = operations.finish(context)?;
        self.submit([crate::interop::CommandBuffer::recorded(
            commands, record.key,
        )])?;
        Ok(())
    }

    /// 独占借入唯一 native owner。
    pub fn native(&mut self) -> &mut Context {
        self.drain_completed();
        &mut self.native
    }
    /// 借入 native owner 的能力与资源描述。
    pub const fn native_ref(&self) -> &Context {
        &self.native
    }
    /// 拆借 native owner 与外部 wgpu 设备/队列。
    pub fn parts(&mut self) -> (&mut Context, &wgpu::Device, &wgpu::Queue) {
        self.drain_completed();
        (&mut self.native, &self.device, &self.queue)
    }
}
pub(crate) fn immediate(
    device: &Device,
    f: impl FnOnce(vk::CommandBuffer) -> Result<(), Error>,
) -> Result<(), Error> {
    device.ensure_active()?;
    if let (Some(_), Some(_)) = (&device._owner, &device.wgpu_queue) {
        let commands = crate::interop::record(device, "native upload", f)?;
        let submission = device.submit([commands])?;
        return crate::interop::wait(device, Some(submission));
    }
    let raw = &device.raw;
    let pool = unsafe {
        raw.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(device.queue_family)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT),
            None,
        )?
    };
    let result = (|| {
        let command = unsafe {
            raw.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?
        }[0];
        unsafe {
            raw.begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        f(command)?;
        unsafe {
            raw.end_command_buffer(command)?;
        }
        let fence = unsafe { raw.create_fence(&vk::FenceCreateInfo::default(), None)? };
        let submitted = unsafe {
            raw.queue_submit(
                device.queue,
                &[vk::SubmitInfo::default().command_buffers(&[command])],
                fence,
            )
        };
        let result = match submitted {
            Ok(()) => {
                let result = unsafe { raw.wait_for_fences(&[fence], true, u64::MAX) };
                if result.is_err()
                    && !crate::context::execution_ended(result)
                    && !crate::context::execution_ended(device.wait_idle())
                {
                    device.quarantine();
                }
                result
            }
            Err(error) => Err(error),
        };
        if result == Err(vk::Result::ERROR_DEVICE_LOST) {
            device.health.fail(crate::DeviceFailure::Lost);
        }
        if !device.is_quarantined() {
            unsafe {
                raw.destroy_fence(fence, None);
            }
        }
        result.map_err(Error::from)
    })();
    if !device.is_quarantined() {
        unsafe {
            raw.destroy_command_pool(pool, None);
        }
    }
    result
}
/// 一个buffer内按输入顺序写入的字节范围；偏移与切片长度均以字节计。
pub type BufferWriteRanges<'a> = [(usize, &'a [u8])];

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
    fn allocate(
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
    fn data<'a>(&self, context: &'a Context) -> Result<&'a buffer::Buffer, Error> {
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
    fn validate(&self, context: &Context, offset: u64, len: usize) -> Result<(), Error> {
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
/// 单 mip、单 sample 的图像格式。
#[derive(Clone, Copy)]
pub enum ImageFormat {
    /// 单通道线性标量。
    R32Float,
    /// 双通道半精度向量。
    Rg16Float,
    /// 线性 HDR。
    Rgba32Float,
    /// 线性 LUT。
    Rgba16Float,
    /// UNORM 输出。
    Rgba8Unorm,
    /// 自动解码的 sRGB atlas。
    Rgba8Srgb,
}
impl ImageFormat {
    fn raw(self) -> vk::Format {
        match self {
            Self::R32Float => vk::Format::R32_SFLOAT,
            Self::Rg16Float => vk::Format::R16G16_SFLOAT,
            Self::Rgba32Float => vk::Format::R32G32B32A32_SFLOAT,
            Self::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
            Self::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
            Self::Rgba8Srgb => vk::Format::R8G8B8A8_SRGB,
        }
    }
    const fn bytes(self) -> usize {
        match self {
            Self::Rgba32Float => 16,
            Self::Rgba16Float => 8,
            _ => 4,
        }
    }
}
/// 原生图像的只读描述；不转移或延长句柄寿命。
///
/// 此值不保活资源。外部命令须使用同设备并维护同步/GENERAL 布局；未登记的外部使用须由调用方将 Image 留在 Context 发布根至完成。
/// `format`、`usage`、`layout` 分别为 Vulkan 对应枚举的原始值。
#[derive(Clone, Copy)]
pub struct RawImage {
    /// VkImage。
    pub image: u64,
    /// VkImageView。
    pub view: u64,
    /// backing VkDeviceMemory。
    pub memory: u64,
    /// 三维 texel 尺寸。
    pub extent: [u32; 3],
    /// VkFormat。
    pub format: u32,
    /// VkImageUsageFlags。
    pub usage: u32,
    /// 交接 VkImageLayout。
    pub layout: u32,
}
pub(crate) struct ImageData {
    raw: vk::Image,
    view: vk::ImageView,
    memory: vk::DeviceMemory,
    extent: [u32; 3],
    format: ImageFormat,
    allocation: u64,
}
fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1)
}
impl Image {
    fn data<'a>(&self, context: &'a Context) -> Result<&'a ImageData, Error> {
        match context.get(self.key)? {
            Data::Image(data) => Ok(data),
            _ => Err(Error::InvalidArgument("image 标识类型无效")),
        }
    }
    /// 创建图像，同步初始化布局为 GENERAL，不初始化 texel；sRGB 只允许 sampled。
    pub fn new(
        context: &mut Context,
        extent: [u32; 3],
        format: ImageFormat,
    ) -> Result<Self, Error> {
        let data = create_image(context, extent, format)?;
        Ok(Self {
            key: context.insert(Data::Image(data))?,
            extent,
        })
    }
    /// 三维尺寸，2D 深度为1。
    pub const fn extent(&self) -> [u32; 3] {
        self.extent
    }
    /// 借入原始描述；调用方保证外部使用完成及同步。
    pub fn raw(&self, context: &Context) -> Result<RawImage, Error> {
        context.active(self.key)?;
        let inner = self.data(context)?;
        Ok(RawImage {
            image: inner.raw.as_raw(),
            view: inner.view.as_raw(),
            memory: inner.memory.as_raw(),
            extent: inner.extent,
            format: inner.format.raw().as_raw() as u32,
            usage: (vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST
                | if matches!(inner.format, ImageFormat::Rgba8Srgb) {
                    vk::ImageUsageFlags::empty()
                } else {
                    vk::ImageUsageFlags::STORAGE
                })
            .as_raw(),
            layout: vk::ImageLayout::GENERAL.as_raw() as u32,
        })
    }
    /// 同步上传紧密 mip0；调用方须完成此前图像使用。
    pub fn upload(&self, context: &mut Context, bytes: &[u8]) -> Result<(), Error> {
        context.active(self.key)?;
        context.resource_idle(self.key)?;
        let inner = self.data(context)?;
        let expected = inner
            .extent
            .into_iter()
            .try_fold(inner.format.bytes(), |size, n| size.checked_mul(n as usize));
        if expected != Some(bytes.len()) {
            return Err(Error::InvalidArgument("图像上传设备或字节数无效"));
        }
        let staging = Guard::new(
            &context.device,
            buffer::Buffer::new(
                &context.device,
                bytes.len(),
                vk::BufferUsageFlags::TRANSFER_SRC,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?,
        );
        staging.write(&context.device, 0, bytes)?;
        immediate(&context.device, |command| {
            unsafe {
                memory_barrier(&context.device, command);
                context.device.raw.cmd_copy_buffer_to_image(
                    command,
                    staging.raw,
                    inner.raw,
                    vk::ImageLayout::GENERAL,
                    &[vk::BufferImageCopy::default()
                        .image_subresource(
                            vk::ImageSubresourceLayers::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .layer_count(1),
                        )
                        .image_extent(vk::Extent3D {
                            width: inner.extent[0],
                            height: inner.extent[1],
                            depth: inner.extent[2],
                        })],
                );
                memory_barrier(&context.device, command);
            }
            Ok(())
        })
    }
    /// 同步回读紧密 mip0；调用方须完成此前图像使用。
    pub fn download(&self, context: &mut Context) -> Result<Vec<u8>, Error> {
        context.active(self.key)?;
        context.resource_idle(self.key)?;
        let inner = self.data(context)?;
        let len = inner
            .extent
            .into_iter()
            .try_fold(inner.format.bytes(), |size, n| size.checked_mul(n as usize))
            .ok_or(Error::InvalidArgument("图像回读大小溢出"))?;
        let staging = Guard::new(
            &context.device,
            buffer::Buffer::new(
                &context.device,
                len,
                vk::BufferUsageFlags::TRANSFER_DST,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?,
        );
        immediate(&context.device, |command| {
            unsafe {
                memory_barrier(&context.device, command);
                context.device.raw.cmd_copy_image_to_buffer(
                    command,
                    inner.raw,
                    vk::ImageLayout::GENERAL,
                    staging.raw,
                    &[vk::BufferImageCopy::default()
                        .image_subresource(
                            vk::ImageSubresourceLayers::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .layer_count(1),
                        )
                        .image_extent(vk::Extent3D {
                            width: inner.extent[0],
                            height: inner.extent[1],
                            depth: inner.extent[2],
                        })],
                );
                memory_barrier(&context.device, command);
            }
            Ok(())
        })?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len)?;
        bytes.resize(len, 0);
        unsafe {
            let mapped = context.device.raw.map_memory(
                staging.memory,
                0,
                len as u64,
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(mapped.cast::<u8>(), bytes.as_mut_ptr(), len);
            context.device.raw.unmap_memory(staging.memory);
        }
        Ok(bytes)
    }
}
impl Destroy for ImageData {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_image_view(self.view, None);
            device.raw.destroy_image(self.raw, None);
            device.raw.free_memory(self.memory, None);
        }
        device
            .buffer_bytes
            .fetch_sub(self.allocation, Ordering::Relaxed);
    }
}
fn create_image(
    context: &Context,
    extent: [u32; 3],
    format: ImageFormat,
) -> Result<ImageData, Error> {
    context.device.ensure_active()?;
    let limits = context.device.physical.properties().limits;
    let limit = if extent[2] > 1 {
        limits.max_image_dimension3_d
    } else {
        limits.max_image_dimension2_d
    };
    if extent.contains(&0) || extent.into_iter().any(|n| n > limit) {
        return Err(Error::InvalidArgument("native 图像尺寸无效"));
    }
    let mut result = Guard::new(
        &context.device,
        ImageData {
            raw: vk::Image::null(),
            view: vk::ImageView::null(),
            memory: vk::DeviceMemory::null(),
            extent,
            format,
            allocation: 0,
        },
    );
    let usage = vk::ImageUsageFlags::SAMPLED
        | vk::ImageUsageFlags::TRANSFER_DST
        | vk::ImageUsageFlags::TRANSFER_SRC
        | if matches!(format, ImageFormat::Rgba8Srgb) {
            vk::ImageUsageFlags::empty()
        } else {
            vk::ImageUsageFlags::STORAGE
        };
    let raw = &context.device.raw;
    unsafe {
        result.raw = raw.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(if extent[2] > 1 {
                    vk::ImageType::TYPE_3D
                } else {
                    vk::ImageType::TYPE_2D
                })
                .format(format.raw())
                .extent(vk::Extent3D {
                    width: extent[0],
                    height: extent[1],
                    depth: extent[2],
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        )?;
        let requirements = raw.get_image_memory_requirements(result.raw);
        let properties = context
            .device
            .physical
            .instance
            .raw
            .get_physical_device_memory_properties(context.device.physical.raw);
        let kind = (0..properties.memory_type_count)
            .find(|&i| {
                requirements.memory_type_bits & (1 << i) != 0
                    && properties.memory_types[i as usize]
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .ok_or(Error::Unsupported("图像缺少 DEVICE_LOCAL 内存"))?;
        result.memory = raw.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(kind),
            None,
        )?;
        result.allocation = requirements.size;
        context
            .device
            .buffer_bytes
            .fetch_add(requirements.size, Ordering::Relaxed);
        raw.bind_image_memory(result.raw, result.memory, 0)?;
        result.view = raw.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(result.raw)
                .view_type(if extent[2] > 1 {
                    vk::ImageViewType::TYPE_3D
                } else {
                    vk::ImageViewType::TYPE_2D
                })
                .format(format.raw())
                .subresource_range(color_range()),
            None,
        )?;
    }
    immediate(&context.device, |command| {
        unsafe {
            raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(result.raw)
                    .subresource_range(color_range())],
            );
        }
        Ok(())
    })?;
    Ok(result.finish())
}
pub(crate) struct SamplerData {
    raw: vk::Sampler,
}
impl Sampler {
    /// 创建 clamp-to-edge sampler。
    pub fn new(context: &mut Context, linear: bool) -> Result<Self, Error> {
        context.device.ensure_active()?;
        let filter = if linear {
            vk::Filter::LINEAR
        } else {
            vk::Filter::NEAREST
        };
        let raw = unsafe {
            context.device.raw.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(filter)
                    .min_filter(filter)
                    .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .max_lod(0.0),
                None,
            )?
        };
        Ok(Self {
            key: context.insert(Data::Sampler(SamplerData { raw }))?,
        })
    }
    fn data<'a>(&self, context: &'a Context) -> Result<&'a SamplerData, Error> {
        match context.get(self.key)? {
            Data::Sampler(data) => Ok(data),
            _ => Err(Error::InvalidArgument("sampler 标识类型无效")),
        }
    }
}
impl Destroy for SamplerData {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_sampler(self.raw, None);
        }
    }
}
impl ExternalBinding {
    /// 导入同设备、已初始化资源。
    /// # Safety
    /// handle 匹配 kind，图像为 GENERAL；宿主保持句柄至所有引用批次真正完成。
    pub unsafe fn new(context: &Context, handle: u64, kind: ExternalKind) -> Result<Self, Error> {
        context.device.ensure_active()?;
        if handle == 0 || matches!(kind, ExternalKind::StorageBuffer { len: 0 }) {
            return Err(Error::InvalidArgument("外部描述符句柄或范围为空"));
        }
        Ok(Self {
            owner: context.owner(),
            handle,
            kind,
        })
    }
}
/// 单个 binding 资源，slice 下标直接对应 set0 binding 编号。
#[derive(Clone)]
pub enum Binding {
    /// native storage buffer。
    StorageBuffer(Buffer),
    /// native uniform buffer；范围须符合设备 uniform 限制。
    UniformBuffer(Buffer),
    /// wgpu storage buffer。
    SharedBuffer(SharedBuffer),
    /// native GENERAL storage image。
    StorageImage(Image),
    /// wgpu GENERAL storage image。
    SharedTexture(SharedTexture),
    /// native GENERAL sampled image。
    SampledImage(Image),
    /// wgpu SHADER_READ_ONLY_OPTIMAL sampled image。
    SampledTexture(SampledTexture),
    /// native sampler。
    Sampler(Sampler),
    /// 同设备 AS。
    Acceleration(crate::AccelerationStructure),
    /// 宿主资源。
    External(ExternalBinding),
}
impl Binding {
    fn stored(&self) -> StoredBinding {
        match self {
            Self::StorageBuffer(v) => StoredBinding::StorageBuffer(*v),
            Self::UniformBuffer(v) => StoredBinding::UniformBuffer(*v),
            Self::SharedBuffer(v) => StoredBinding::SharedBuffer(v.clone()),
            Self::StorageImage(v) => StoredBinding::StorageImage(*v),
            Self::SharedTexture(v) => StoredBinding::SharedTexture(v.clone()),
            Self::SampledImage(v) => StoredBinding::SampledImage(*v),
            Self::SampledTexture(v) => StoredBinding::SampledTexture(v.clone()),
            Self::Sampler(v) => StoredBinding::Sampler(*v),
            Self::Acceleration(v) => StoredBinding::Acceleration(*v),
            Self::External(v) => StoredBinding::External(*v),
        }
    }
    fn owner(&self) -> u64 {
        match self {
            Self::StorageBuffer(v) | Self::UniformBuffer(v) => v.key.owner,
            Self::StorageImage(v) | Self::SampledImage(v) => v.key.owner,
            Self::Sampler(v) => v.key.owner,
            Self::Acceleration(v) => v.key.owner,
            Self::SharedBuffer(v) => v.device,
            Self::SharedTexture(v) => v.device,
            Self::SampledTexture(v) => v.device,
            Self::External(v) => v.owner,
        }
    }
    fn dependency(&self) -> Option<Key> {
        match self {
            Self::StorageBuffer(v) | Self::UniformBuffer(v) => Some(v.key),
            Self::StorageImage(v) | Self::SampledImage(v) => Some(v.key),
            Self::Sampler(v) => Some(v.key),
            Self::Acceleration(v) => Some(v.key),
            _ => None,
        }
    }
    fn descriptor_type(&self) -> vk::DescriptorType {
        match self {
            Self::UniformBuffer(_) => vk::DescriptorType::UNIFORM_BUFFER,
            Self::StorageBuffer(_)
            | Self::SharedBuffer(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::StorageBuffer { .. },
                ..
            }) => vk::DescriptorType::STORAGE_BUFFER,
            Self::StorageImage(_)
            | Self::SharedTexture(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::StorageImage,
                ..
            }) => vk::DescriptorType::STORAGE_IMAGE,
            Self::SampledImage(_)
            | Self::SampledTexture(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::SampledImage,
                ..
            }) => vk::DescriptorType::SAMPLED_IMAGE,
            Self::Sampler(_) => vk::DescriptorType::SAMPLER,
            Self::Acceleration(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::Acceleration,
                ..
            }) => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
        }
    }
}
impl StoredBinding {
    fn descriptor_type(&self) -> vk::DescriptorType {
        match self {
            Self::UniformBuffer(_) => vk::DescriptorType::UNIFORM_BUFFER,
            Self::StorageBuffer(_)
            | Self::SharedBuffer(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::StorageBuffer { .. },
                ..
            }) => vk::DescriptorType::STORAGE_BUFFER,
            Self::StorageImage(_)
            | Self::SharedTexture(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::StorageImage,
                ..
            }) => vk::DescriptorType::STORAGE_IMAGE,
            Self::SampledImage(_)
            | Self::SampledTexture(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::SampledImage,
                ..
            }) => vk::DescriptorType::SAMPLED_IMAGE,
            Self::Sampler(_) => vk::DescriptorType::SAMPLER,
            Self::Acceleration(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::Acceleration,
                ..
            }) => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
        }
    }
}
pub(crate) struct RawBindings {
    views: Vec<wgpu::TextureView>,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
}
pub(crate) struct DescriptorLayout {
    raw: vk::DescriptorSetLayout,
}
impl Destroy for DescriptorLayout {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_descriptor_set_layout(self.raw, None);
        }
    }
}
impl Destroy for RawBindings {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_descriptor_pool(self.pool, None);
        }
    }
}
impl BindingData {
    fn compatible(&self, other: &Self) -> bool {
        self.resources().len() == other.resources().len()
            && self
                .resources()
                .iter()
                .zip(other.resources())
                .all(|(a, b)| a.descriptor_type() == b.descriptor_type())
    }
    pub(crate) fn track(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.transition_resources(
            self.resources()
                .iter()
                .filter_map(|resource| match resource {
                    StoredBinding::SharedBuffer(v) => Some(wgpu::BufferTransition {
                        buffer: &v.buffer,
                        state: wgpu::BufferUses::STORAGE_READ_WRITE,
                    }),
                    _ => None,
                }),
            self.resources()
                .iter()
                .filter_map(|resource| match resource {
                    StoredBinding::SharedTexture(v) => Some(wgpu::TextureTransition {
                        texture: &v.texture,
                        selector: None,
                        state: wgpu::TextureUses::STORAGE_READ_WRITE,
                    }),
                    StoredBinding::SampledTexture(v) => Some(wgpu::TextureTransition {
                        texture: &v.texture,
                        selector: None,
                        state: wgpu::TextureUses::RESOURCE,
                    }),
                    _ => None,
                }),
        );
    }
}
impl Bindings {
    fn data<'a>(&self, context: &'a Context) -> Result<&'a BindingData, Error> {
        match context.get(self.key)? {
            Data::Bindings(data) => Ok(data),
            _ => Err(Error::InvalidArgument("bindings 标识类型无效")),
        }
    }
    /// 创建连续set0；先验证资源归属及设备阶段/set数量上限，再建立独立descriptor版本。
    /// 每项对compute及已启用RT阶段可见，不依赖shader静态裁剪来规避容量限制。
    pub fn new(context: &mut Context, resources: &[Binding]) -> Result<Self, Error> {
        context.device.ensure_active()?;
        let limits = context.device.physical.properties().limits;
        for (kind, stage, set) in [
            (
                vk::DescriptorType::STORAGE_BUFFER,
                limits.max_per_stage_descriptor_storage_buffers,
                limits.max_descriptor_set_storage_buffers,
            ),
            (
                vk::DescriptorType::UNIFORM_BUFFER,
                limits.max_per_stage_descriptor_uniform_buffers,
                limits.max_descriptor_set_uniform_buffers,
            ),
            (
                vk::DescriptorType::STORAGE_IMAGE,
                limits.max_per_stage_descriptor_storage_images,
                limits.max_descriptor_set_storage_images,
            ),
            (
                vk::DescriptorType::SAMPLED_IMAGE,
                limits.max_per_stage_descriptor_sampled_images,
                limits.max_descriptor_set_sampled_images,
            ),
            (
                vk::DescriptorType::SAMPLER,
                limits.max_per_stage_descriptor_samplers,
                limits.max_descriptor_set_samplers,
            ),
        ] {
            let count = resources
                .iter()
                .filter(|resource| resource.descriptor_type() == kind)
                .count();
            if count > stage.min(set) as usize {
                return Err(Error::Unsupported("描述符数量超过设备阶段或set上限"));
            }
        }
        for resource in resources {
            if resource.owner() != context.owner() {
                return Err(Error::InvalidArgument("描述符资源属于其他设备"));
            }
        }
        for resource in resources {
            if let Some(key) = resource.dependency() {
                context.active(key)?;
            }
        }
        let stages = vk::ShaderStageFlags::COMPUTE
            | if context.device.ray_pipeline {
                vk::ShaderStageFlags::RAYGEN_KHR
                    | vk::ShaderStageFlags::MISS_KHR
                    | vk::ShaderStageFlags::CLOSEST_HIT_KHR
                    | vk::ShaderStageFlags::ANY_HIT_KHR
                    | vk::ShaderStageFlags::CALLABLE_KHR
            } else {
                vk::ShaderStageFlags::empty()
            };
        let entries: Vec<_> = resources
            .iter()
            .enumerate()
            .map(|(i, resource)| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(i as u32)
                    .descriptor_type(resource.descriptor_type())
                    .descriptor_count(1)
                    .stage_flags(stages)
            })
            .collect();

        let raw = unsafe {
            context.device.raw.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&entries),
                None,
            )?
        };
        let layout = context.insert(Data::Layout(DescriptorLayout { raw }))?;
        match create_bindings(context, resources, layout)
            .and_then(|data| context.insert(Data::Bindings(data)))
        {
            Ok(key) => Ok(Self { key }),
            Err(error) => {
                context.discard(layout);
                Err(error)
            }
        }
    }
}
fn create_bindings(
    context: &Context,
    resources: &[Binding],
    layout: Key,
) -> Result<BindingData, Error> {
    if resources.len() > u32::MAX as usize
        || resources
            .iter()
            .any(|resource| context.owner() != resource.owner())
    {
        return Err(Error::InvalidArgument("描述符数量或设备归属无效"));
    }
    for resource in resources {
        if let Some(key) = resource.dependency() {
            context.get(key)?;
        }
        if let Binding::StorageImage(image) = resource
            && matches!(image.data(context)?.format, ImageFormat::Rgba8Srgb)
        {
            return Err(Error::InvalidArgument("sRGB 图像不支持 storage 绑定"));
        }
    }
    if resources.iter().any(|resource| matches!(resource, Binding::UniformBuffer(buffer) if buffer.len() > u64::from(context.device.physical.properties().limits.max_uniform_buffer_range))) {
            return Err(Error::InvalidArgument("uniform buffer 范围超出设备限制"));
        }
    let raw = &context.device.raw;
    let mut result = Guard::new(
        &context.device,
        RawBindings {
            views: Vec::new(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
        },
    );
    let mut counts = BTreeMap::new();
    for resource in resources {
        *counts
            .entry(resource.descriptor_type().as_raw())
            .or_insert(0u32) += 1;
    }
    let sizes: Vec<_> = counts
        .into_iter()
        .map(|(kind, count)| {
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::from_raw(kind))
                .descriptor_count(count)
        })
        .collect();
    result.pool = unsafe {
        raw.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(&sizes),
            None,
        )?
    };
    result.set = unsafe {
        raw.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(result.pool)
                .set_layouts(&[context.layout(layout)?.raw]),
        )?
    }[0];
    let resources: Vec<_> = resources.iter().map(Binding::stored).collect();
    for (index, resource) in resources.iter().enumerate() {
        let mut write = vk::WriteDescriptorSet::default()
            .dst_set(result.set)
            .dst_binding(index as u32)
            .descriptor_type(resource.descriptor_type())
            .descriptor_count(1);
        let buffer_info;
        let image_info;
        let handles;
        let mut acceleration;
        match resource {
            StoredBinding::StorageBuffer(v) | StoredBinding::UniformBuffer(v) => {
                buffer_info = [vk::DescriptorBufferInfo::default()
                    .buffer(v.data(context)?.raw)
                    .range(v.len())];
                write = write.buffer_info(&buffer_info);
            }
            StoredBinding::SharedBuffer(v) => {
                let hal = unsafe { v.buffer.as_hal::<Vulkan>() }
                    .ok_or(Error::InvalidState("共享 buffer 已销毁"))?;
                buffer_info = [vk::DescriptorBufferInfo::default()
                    .buffer(vk::Buffer::from_raw(unsafe { hal.raw_handle() }.as_raw()))
                    .range(v.len())];
                write = write.buffer_info(&buffer_info);
            }
            StoredBinding::StorageImage(v) | StoredBinding::SampledImage(v) => {
                image_info = [vk::DescriptorImageInfo::default()
                    .image_view(v.data(context)?.view)
                    .image_layout(vk::ImageLayout::GENERAL)];
                write = write.image_info(&image_info);
            }
            StoredBinding::SharedTexture(v) => {
                let view = v.texture.create_view(&Default::default());
                let hal = unsafe { view.as_hal::<Vulkan>() }
                    .ok_or(Error::InvalidState("共享 view 已销毁"))?;
                image_info = [vk::DescriptorImageInfo::default()
                    .image_view(vk::ImageView::from_raw(
                        unsafe { hal.raw_handle() }.as_raw(),
                    ))
                    .image_layout(vk::ImageLayout::GENERAL)];
                drop(hal);
                result.views.push(view);
                write = write.image_info(&image_info);
            }
            StoredBinding::SampledTexture(v) => {
                let hal = unsafe { v.view.as_hal::<Vulkan>() }
                    .ok_or(Error::InvalidState("采样 view 已销毁"))?;
                image_info = [vk::DescriptorImageInfo::default()
                    .image_view(vk::ImageView::from_raw(
                        unsafe { hal.raw_handle() }.as_raw(),
                    ))
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
                write = write.image_info(&image_info);
            }
            StoredBinding::Sampler(v) => {
                image_info = [vk::DescriptorImageInfo::default().sampler(v.data(context)?.raw)];
                write = write.image_info(&image_info);
            }
            StoredBinding::Acceleration(v) => {
                handles = [context.acceleration(*v)?.handle()];
                acceleration = vk::WriteDescriptorSetAccelerationStructureKHR::default()
                    .acceleration_structures(&handles);
                write = write.push(&mut acceleration);
            }
            StoredBinding::External(v) => match v.kind {
                ExternalKind::StorageBuffer { len } => {
                    buffer_info = [vk::DescriptorBufferInfo::default()
                        .buffer(vk::Buffer::from_raw(v.handle))
                        .range(len)];
                    write = write.buffer_info(&buffer_info);
                }
                ExternalKind::StorageImage | ExternalKind::SampledImage => {
                    image_info = [vk::DescriptorImageInfo::default()
                        .image_view(vk::ImageView::from_raw(v.handle))
                        .image_layout(vk::ImageLayout::GENERAL)];
                    write = write.image_info(&image_info);
                }
                ExternalKind::Acceleration => {
                    handles = [vk::AccelerationStructureKHR::from_raw(v.handle)];
                    acceleration = vk::WriteDescriptorSetAccelerationStructureKHR::default()
                        .acceleration_structures(&handles);
                    write = write.push(&mut acceleration);
                }
            },
        }
        unsafe {
            raw.update_descriptor_sets(&[write], &[]);
        }
    }
    Ok(BindingData::new(layout, resources, result.finish()))
}
/// 驱动返回的各RT阶段最大调用栈需求，单位为字节；创建program时查询并缓存。
///
/// 这些值不是寄存器数、实际每ray分配、占用率或GPU内存用量；本类型不修改pipeline栈配置。
/// callable取所有叶子的最大值，不按顺序调用次数相加；无callable时最大值与数量均为0。
/// 来源：[vkGetRayTracingShaderGroupStackSizeKHR](https://docs.vulkan.org/refpages/latest/refpages/source/vkGetRayTracingShaderGroupStackSizeKHR.html)。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RayStackSizes {
    /// Raygen general shader的驱动栈需求。
    pub raygen: u64,
    /// Miss general shader的驱动栈需求。
    pub miss: u64,
    /// 所有三角形hit group中closest-hit shader的最大驱动栈需求。
    pub closest_hit: u64,
    /// 存在any-hit shader时的驱动栈需求；None表示未链接该阶段。
    pub any_hit: Option<u64>,
    /// 所有callable general shader的最大驱动栈需求。
    pub callable_max: u64,
    /// 参与最大值查询的callable数量。
    pub callable_count: u32,
}
pub(crate) struct RawRayProgram {
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    sbt: Option<buffer::Buffer>,
    regions: [vk::StridedDeviceAddressRegionKHR; 4],
    loader: khr::ray_tracing_pipeline::Device,
    max_dispatch: u32,
    stack_sizes: RayStackSizes,
}
impl Destroy for RawRayProgram {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_pipeline(self.pipeline, None);
            device.raw.destroy_pipeline_layout(self.layout, None);
        }
        if let Some(sbt) = &mut self.sbt {
            sbt.destroy(device);
        }
    }
}
impl RayTracingPipeline {
    /// 查询驱动公开的可执行统计；字段和执行单元划分由驱动定义，可能为空。
    ///
    /// 创建 Context 时须显式启用 [`Capabilities::pipeline_statistics`]；重绑定复用
    /// 同一已捕获 program。不会执行或等待 GPU；返回值不借用管线。
    /// # Errors
    /// 未启用能力、标识已回收或不属于此 Context、Vulkan 查询失败时返回错误。
    pub fn executable_statistics(
        &self,
        context: &Context,
    ) -> Result<Vec<ExecutableStatistics>, Error> {
        let program = self.data(context)?.program();
        pipeline_statistics::query(context, context.ray_program(program)?.raw().pipeline)
    }
    /// 返回创建program时缓存的驱动栈需求；descriptor重绑定复用同一份数值。
    /// 标识不属于此Context或资源已回收时返回错误；此查询不录制命令，也不设置dynamic stack。
    pub fn stack_sizes(&self, context: &Context) -> Result<RayStackSizes, Error> {
        let program = self.data(context)?.program();
        Ok(context.ray_program(program)?.raw().stack_sizes)
    }

    /// 不可变program与SBT的资源身份；重绑descriptor时保持相同，可用于复用诊断或保活。
    pub fn program_root(&self, context: &Context) -> Result<Resource, Error> {
        Ok(Resource(self.data(context)?.program()))
    }

    fn data<'a>(&self, context: &'a Context) -> Result<&'a RayVersion, Error> {
        match context.get(self.key)? {
            Data::Ray(data) => Ok(data),
            _ => Err(Error::InvalidArgument("RT pipeline 标识类型无效")),
        }
    }
    /// 创建单层 RT pipeline；shader 的 payload/调用图契约由受控构建保证。
    pub fn new(
        context: &mut Context,
        bindings: Bindings,
        shaders: [RayShader<'_>; 3],
        any_hit: Option<RayShader<'_>>,
        callables: &[RayShader<'_>],
    ) -> Result<Self, Error> {
        let [raygen, miss, closest_hit] = shaders;
        Self::with_hit_groups(
            context,
            bindings,
            raygen,
            miss,
            &[closest_hit],
            any_hit,
            callables,
        )
    }

    /// 创建单层 RT pipeline，按切片顺序建立三角形 Hit Group 的 SBT 记录。
    ///
    /// `closest_hits` 至少含一项；所有记录共享可选覆盖率过滤。普通追踪及
    /// HitObject 的表索引均相对此切片，callable 索引独立从零开始。
    /// shader 的 payload、索引范围及无环调用图由受控构建保证；本函数不保留借用。
    ///
    /// # Errors
    /// 空命中表、数量或布局溢出、设备不支持及 Vulkan 创建失败时返回错误。
    pub fn with_hit_groups(
        context: &mut Context,
        bindings: Bindings,
        raygen: RayShader<'_>,
        miss: RayShader<'_>,
        closest_hits: &[RayShader<'_>],
        any_hit: Option<RayShader<'_>>,
        callables: &[RayShader<'_>],
    ) -> Result<Self, Error> {
        context.active(bindings.key)?;
        let data = create_ray_program(
            context,
            bindings,
            raygen,
            miss,
            closest_hits,
            any_hit,
            callables,
        )?;
        let program = context.insert(Data::RayProgram(data))?;
        match context.insert(Data::Ray(RayVersion::new(bindings, program))) {
            Ok(key) => Ok(Self { key }),
            Err(error) => {
                context.discard(program);
                Err(error)
            }
        }
    }
    /// 复用 program/SBT，仅创建绑定版本；旧命令依赖不变。
    pub fn rebind(&self, context: &mut Context, bindings: Bindings) -> Result<Self, Error> {
        context.active(self.key)?;
        context.active(bindings.key)?;
        let old = self.data(context)?;
        if !old
            .bindings()
            .data(context)?
            .compatible(bindings.data(context)?)
        {
            return Err(Error::InvalidArgument("RT 重绑定 layout 不兼容"));
        }
        let program = old.program();
        Ok(Self {
            key: context.insert(Data::Ray(RayVersion::new(bindings, program)))?,
        })
    }
}
pub(crate) struct RawComputeProgram {
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
}
impl Destroy for RawComputeProgram {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_pipeline(self.pipeline, None);
            device.raw.destroy_pipeline_layout(self.layout, None);
        }
    }
}
impl ComputePipeline {
    /// 不可变compute program的资源身份；重绑descriptor时保持相同。
    pub fn program_root(&self, context: &Context) -> Result<Resource, Error> {
        Ok(Resource(self.data(context)?.program()))
    }

    fn data<'a>(&self, context: &'a Context) -> Result<&'a ComputeVersion, Error> {
        match context.get(self.key)? {
            Data::Compute(data) => Ok(data),
            _ => Err(Error::InvalidArgument("compute pipeline 标识类型无效")),
        }
    }
    /// 创建直接消费 SPIR-V 的 compute program。
    pub fn new(
        context: &mut Context,
        bindings: Bindings,
        shader: RayShader<'_>,
    ) -> Result<Self, Error> {
        Self::create(context, bindings, shader, None)
    }

    /// 创建固定 subgroup 规模的 compute program，不依赖设备默认规模。
    ///
    /// # Errors
    /// size 非 2 幂、超出范围，或宿主未启用 compute 规模控制时返回错误；其余同 [`Self::new`]。
    pub fn with_subgroup_size(
        context: &mut Context,
        bindings: Bindings,
        shader: RayShader<'_>,
        size: u32,
    ) -> Result<Self, Error> {
        let properties = context.subgroup_properties();
        if !size.is_power_of_two()
            || !properties.compute_size_control
            || size < properties.min_size
            || size > properties.max_size
        {
            return Err(Error::Unsupported("compute subgroup 规模不受设备支持"));
        }
        Self::create(context, bindings, shader, Some(size))
    }

    fn create(
        context: &mut Context,
        bindings: Bindings,
        shader: RayShader<'_>,
        subgroup_size: Option<u32>,
    ) -> Result<Self, Error> {
        context.active(bindings.key)?;
        let data = create_compute_program(context, bindings, shader, subgroup_size)?;
        let program = context.insert(Data::ComputeProgram(data))?;
        match context.insert(Data::Compute(ComputeVersion::new(bindings, program))) {
            Ok(key) => Ok(Self { key }),
            Err(error) => {
                context.discard(program);
                Err(error)
            }
        }
    }
    /// 复用 program，仅创建不可变 descriptor 版本。
    pub fn rebind(&self, context: &mut Context, bindings: Bindings) -> Result<Self, Error> {
        context.active(self.key)?;
        context.active(bindings.key)?;
        let old = self.data(context)?;
        if !old
            .bindings()
            .data(context)?
            .compatible(bindings.data(context)?)
        {
            return Err(Error::InvalidArgument("compute 重绑定 layout 不兼容"));
        }
        let program = old.program();
        Ok(Self {
            key: context.insert(Data::Compute(ComputeVersion::new(bindings, program)))?,
        })
    }
}
fn create_ray_program(
    context: &Context,
    bindings: Bindings,
    raygen: RayShader<'_>,
    miss: RayShader<'_>,
    closest_hits: &[RayShader<'_>],
    any_hit: Option<RayShader<'_>>,
    callables: &[RayShader<'_>],
) -> Result<RayProgram, Error> {
    let bindings = bindings.data(context)?;
    let set_layout = bindings.layout();
    if closest_hits.is_empty() {
        return Err(Error::InvalidArgument("RT 命中表不能为空"));
    }
    let callable_group_base = closest_hits
        .len()
        .checked_add(2)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or(Error::InvalidArgument("RT hit group 数量溢出"))?;
    let group_count = callables
        .len()
        .checked_add(callable_group_base as usize)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or(Error::InvalidArgument("RT shader group 数量溢出"))?;
    if any_hit.is_some() && group_count == u32::MAX {
        return Err(Error::InvalidArgument("RT shader stage 数量溢出"));
    }
    if !context.device.ray_pipeline {
        return Err(Error::InvalidArgument("RT 能力或绑定设备无效"));
    }
    let loader = khr::ray_tracing_pipeline::Device::load(
        &context.device.physical.instance.raw,
        &context.device.raw,
    );
    let mut result = Guard::new(
        &context.device,
        RawRayProgram {
            layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            sbt: None,
            regions: Default::default(),
            loader,
            max_dispatch: 0,
            stack_sizes: RayStackSizes::default(),
        },
    );
    result.layout = unsafe {
        context.device.raw.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&[context.layout(bindings.layout())?.raw]),
            None,
        )?
    };
    let mut kinds = vec![
        vk::ShaderStageFlags::RAYGEN_KHR,
        vk::ShaderStageFlags::MISS_KHR,
    ];
    kinds.extend(std::iter::repeat_n(
        vk::ShaderStageFlags::CLOSEST_HIT_KHR,
        closest_hits.len(),
    ));
    let mut sources = vec![&raygen, &miss];
    sources.extend(closest_hits);
    let any_hit_stage = any_hit.as_ref().map(|_| sources.len() as u32);
    if let Some(ref shader) = any_hit {
        sources.push(shader);
        kinds.push(vk::ShaderStageFlags::ANY_HIT_KHR);
    }
    let callable_base = sources.len() as u32;
    sources.extend(callables);
    kinds.extend(std::iter::repeat_n(
        vk::ShaderStageFlags::CALLABLE_KHR,
        callables.len(),
    ));
    let modules: Vec<_> = sources
        .iter()
        .map(|shader| ShaderModule::new(&context.device, shader.spirv))
        .collect::<Result<_, _>>()?;
    let stages: Vec<_> = sources
        .iter()
        .zip(&modules)
        .zip(kinds)
        .map(|((shader, module), stage)| {
            vk::PipelineShaderStageCreateInfo::default()
                .stage(stage)
                .module(module.raw)
                .name(shader.entry)
        })
        .collect();
    let blank = vk::RayTracingShaderGroupCreateInfoKHR::default()
        .general_shader(vk::SHADER_UNUSED_KHR)
        .closest_hit_shader(vk::SHADER_UNUSED_KHR)
        .any_hit_shader(vk::SHADER_UNUSED_KHR)
        .intersection_shader(vk::SHADER_UNUSED_KHR);
    let mut groups = vec![
        blank
            .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
            .general_shader(0),
        blank
            .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
            .general_shader(1),
    ];
    groups.extend((0..closest_hits.len()).map(|i| {
        blank
            .ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
            .closest_hit_shader(2 + i as u32)
            .any_hit_shader(any_hit_stage.unwrap_or(vk::SHADER_UNUSED_KHR))
    }));
    groups.extend((0..callables.len()).map(|i| {
        blank
            .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
            .general_shader(callable_base + i as u32)
    }));
    let mut flags = if context.device.opacity_micromap {
        vk::PipelineCreateFlags::RAY_TRACING_OPACITY_MICROMAP_KHR
    } else {
        vk::PipelineCreateFlags::empty()
    };
    if context.device.pipeline_statistics {
        flags |= vk::PipelineCreateFlags::CAPTURE_STATISTICS_KHR;
    }
    let mut robustness = vk::PipelineRobustnessCreateInfo::default()
        .storage_buffers(vk::PipelineRobustnessBufferBehavior::DISABLED)
        .uniform_buffers(vk::PipelineRobustnessBufferBehavior::DISABLED)
        .vertex_inputs(vk::PipelineRobustnessBufferBehavior::DISABLED)
        .images(vk::PipelineRobustnessImageBehavior::DISABLED);
    let mut create = vk::RayTracingPipelineCreateInfoKHR::default()
        .flags(flags)
        .stages(&stages)
        .groups(&groups)
        .max_pipeline_ray_recursion_depth(1)
        .layout(result.layout);
    if context.device.pipeline_robustness {
        create = create.push(&mut robustness);
    }
    result.pipeline = match unsafe {
        result.loader.create_ray_tracing_pipelines(
            vk::DeferredOperationKHR::null(),
            context.pipeline_cache(),
            &[create],
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((pipelines, error)) => {
            for pipeline in pipelines {
                unsafe {
                    context.device.raw.destroy_pipeline(pipeline, None);
                }
            }
            return Err(error.into());
        }
    };
    // SAFETY: 同一Device成功创建的pipeline；group0/1为general，其后三角形组均含
    // closest-hit及同一个可选any-hit。callable区间独立于shader stage编号。
    result.stack_sizes = unsafe {
        let stack = |group, shader| {
            result
                .loader
                .get_ray_tracing_shader_group_stack_size(result.pipeline, group, shader)
        };
        RayStackSizes {
            raygen: stack(0, vk::ShaderGroupShaderKHR::GENERAL),
            miss: stack(1, vk::ShaderGroupShaderKHR::GENERAL),
            closest_hit: (2..callable_group_base)
                .map(|group| stack(group, vk::ShaderGroupShaderKHR::CLOSEST_HIT))
                .max()
                .unwrap_or(0),
            any_hit: any_hit
                .is_some()
                .then(|| stack(2, vk::ShaderGroupShaderKHR::ANY_HIT)),
            callable_max: (callable_group_base..group_count)
                .map(|group| stack(group, vk::ShaderGroupShaderKHR::GENERAL))
                .max()
                .unwrap_or(0),
            callable_count: group_count - callable_group_base,
        }
    };
    drop(modules);
    let mut properties = vk::PhysicalDeviceRayTracingPipelinePropertiesKHR::default();
    let mut physical = vk::PhysicalDeviceProperties2::default().push(&mut properties);
    unsafe {
        context
            .device
            .physical
            .instance
            .raw
            .get_physical_device_properties2(context.device.physical.raw, &mut physical);
    }
    result.max_dispatch = properties.max_ray_dispatch_invocation_count;
    let handle_size = properties.shader_group_handle_size as usize;
    let stride = handle_size
        .checked_next_multiple_of(properties.shader_group_handle_alignment as usize)
        .ok_or(Error::InvalidArgument("SBT stride 溢出"))?;
    if stride > properties.max_shader_group_stride as usize {
        return Err(Error::InvalidArgument("SBT stride 超限"));
    }
    let spacing = stride
        .checked_next_multiple_of(properties.shader_group_base_alignment as usize)
        .ok_or(Error::InvalidArgument("SBT 对齐溢出"))?;
    let callable_size = stride
        .checked_mul(callables.len())
        .ok_or(Error::InvalidArgument("callable SBT 大小溢出"))?;
    let hit_size = stride
        .checked_mul(closest_hits.len())
        .ok_or(Error::InvalidArgument("hit SBT 大小溢出"))?;
    let callable_offset = spacing
        .checked_mul(2)
        .and_then(|offset| offset.checked_add(hit_size))
        .and_then(|offset| {
            offset.checked_next_multiple_of(properties.shader_group_base_alignment as usize)
        })
        .ok_or(Error::InvalidArgument("SBT 对齐溢出"))?;
    let byte_count = callable_offset
        .checked_add(callable_size)
        .ok_or(Error::InvalidArgument("SBT 大小溢出"))?;
    let handles = unsafe {
        result.loader.get_ray_tracing_shader_group_handles(
            result.pipeline,
            0,
            group_count,
            handle_size
                .checked_mul(groups.len())
                .ok_or(Error::InvalidArgument("SBT handle 溢出"))?,
        )?
    };
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(byte_count)?;
    bytes.resize(byte_count, 0);
    for i in 0..groups.len() {
        let offset = if i < 2 {
            i * spacing
        } else if i < callable_group_base as usize {
            spacing * 2 + (i - 2) * stride
        } else {
            callable_offset + (i - callable_group_base as usize) * stride
        };
        bytes[offset..offset + handle_size]
            .copy_from_slice(&handles[i * handle_size..(i + 1) * handle_size]);
    }
    let (sbt, address) = buffer::Buffer::upload(
        &context.device,
        &bytes,
        vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        properties.shader_group_base_alignment as usize,
    )?;
    result.regions = std::array::from_fn(|i| {
        if i == 3 && callables.is_empty() {
            Default::default()
        } else {
            vk::StridedDeviceAddressRegionKHR {
                device_address: address + if i == 3 { callable_offset } else { i * spacing } as u64,
                stride: stride as u64,
                size: match i {
                    2 => hit_size,
                    3 => callable_size,
                    _ => stride,
                } as u64,
            }
        }
    });
    result.sbt = Some(sbt);
    Ok(RayProgram::new(set_layout, result.finish()))
}
fn create_compute_program(
    context: &Context,
    bindings: Bindings,
    shader: RayShader<'_>,
    subgroup_size: Option<u32>,
) -> Result<ComputeProgram, Error> {
    let bindings = bindings.data(context)?;
    let set_layout = bindings.layout();
    let mut result = Guard::new(
        &context.device,
        RawComputeProgram {
            layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
        },
    );
    result.layout = unsafe {
        context.device.raw.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&[context.layout(bindings.layout())?.raw]),
            None,
        )?
    };
    let module = ShaderModule::new(&context.device, shader.spirv)?;
    let mut size = vk::PipelineShaderStageRequiredSubgroupSizeCreateInfo::default()
        .required_subgroup_size(subgroup_size.unwrap_or(0));
    let mut stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(module.raw)
        .name(shader.entry);
    if subgroup_size.is_some() {
        stage = stage.push(&mut size);
    }
    let create = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(result.layout);
    result.pipeline = match unsafe {
        context
            .device
            .raw
            .create_compute_pipelines(context.pipeline_cache(), &[create], None)
    } {
        Ok(pipelines) => pipelines[0],
        Err((pipelines, error)) => {
            for pipeline in pipelines {
                unsafe {
                    context.device.raw.destroy_pipeline(pipeline, None);
                }
            }
            return Err(error.into());
        }
    };
    drop(module);
    Ok(ComputeProgram::new(set_layout, result.finish()))
}
/// 顺序命令构建器；未 finish 的命令不访问队列或保留资源。
pub struct Commands {
    owner: u64,
    operations: Vec<Op>,
    max_trace: [u64; 3],
    max_groups: [u32; 3],
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
        let uses = self.validate(context)?;
        let record = Record::new(self.operations, uses, Vec::new(), BuildBuffers::default());
        let key = context
            .life
            .insert_record(record)
            .map_err(|(error, _)| Error::from(error))?;
        Ok(Recorded { key })
    }

    /// 只读校验完整操作流，返回排序去重的 host 互斥资源；失败不登记记录。
    fn validate(&self, context: &Context) -> Result<Vec<Key>, Error> {
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
                Op::Barrier => None,
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
    fn roots(&self, out: &mut Vec<Key>) {
        out.extend_from_slice(self.build_roots());
        out.extend(self.operations().iter().filter_map(Op::root));
    }
    fn track(&self, context: &Context, encoder: &mut wgpu::CommandEncoder) -> Result<(), Error> {
        for op in self.operations() {
            match op {
                Op::Trace(p, _) => p.data(context)?.bindings().data(context)?.track(encoder),
                Op::Dispatch(p, _) => p.data(context)?.bindings().data(context)?.track(encoder),
                _ => {}
            }
        }
        Ok(())
    }
    fn encode(&self, context: &Context, command: vk::CommandBuffer) -> Result<(), Error> {
        self.encode_operations(context, command, self.operations())
    }
    fn encode_operations(
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
unsafe fn memory_barrier(device: &Device, command: vk::CommandBuffer) {
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

fn valid_trace_range(size: [u32; 3], max_dispatch: u32, max_axis: [u64; 3]) -> bool {
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

impl Data {
    fn recycle(self, device: &Device, pool: &mut BufferPool) {
        match self {
            Self::Blas(data) => data.recycle(device, pool),
            Self::Acceleration(data) => data.recycle(device, pool),
            mut data => data.destroy(device),
        }
    }

    fn retained_bytes(&self) -> usize {
        match self {
            Self::Bindings(data) => {
                data.resource_capacity() * size_of::<StoredBinding>()
                    + data.raw().views.capacity() * size_of::<wgpu::TextureView>()
            }
            Self::Acceleration(data) => data.bottom_capacity() * size_of::<crate::TriangleBlas>(),
            _ => 0,
        }
    }
}
impl Destroy for Data {
    fn destroy(&mut self, device: &Device) {
        match self {
            Self::Timestamps(v) => v.destroy(device),
            Self::Buffer(v) => v.destroy(device),
            Self::Image(v) => v.destroy(device),
            Self::Sampler(v) => v.destroy(device),
            Self::Bindings(v) => v.raw_mut().destroy(device),
            Self::Layout(v) => v.destroy(device),
            Self::RayProgram(v) => v.raw_mut().destroy(device),
            Self::ComputeProgram(v) => v.raw_mut().destroy(device),
            Self::Blas(v) => v.destroy(device),
            Self::Acceleration(v) => v.destroy(device),
            Self::Micromap(v) => v.destroy(device),
            Self::Ray(_) | Self::Compute(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "需要真实Vulkan与coherent DEVICE_LOCAL映射；主任务串行调度"]
    fn scattered_buffers_reject_the_whole_transaction_before_mutation() -> Result<(), crate::Error>
    {
        let mut owner = pollster::block_on(crate::interop::WgpuContext::compute())?;
        let context = owner.native();
        let a = super::Buffer::mapped_device_local(context, 16)?;
        let b = super::Buffer::mapped_device_local(context, 16)?;
        a.write(context, 0, &[7; 16])?;
        b.write(context, 0, &[8; 16])?;
        let first: &[(usize, &[u8])] = &[(0, &[1; 4]), (12, &[2; 4])];
        let bad: &[(usize, &[u8])] = &[(15, &[3; 4])];
        assert!(
            super::Buffer::write_scattered_batches(context, &[(&a, first), (&b, bad)]).is_err()
        );
        let mut bytes = [0; 16];
        a.read(context, 0, &mut bytes)?;
        assert_eq!(bytes, [7; 16]);
        b.read(context, 0, &mut bytes)?;
        assert_eq!(bytes, [8; 16]);
        let second: &[(usize, &[u8])] = &[(4, &[3; 4])];
        super::Buffer::write_scattered_batches(context, &[(&a, first), (&b, second)])?;
        a.read(context, 0, &mut bytes)?;
        assert_eq!(bytes, [1, 1, 1, 1, 7, 7, 7, 7, 7, 7, 7, 7, 2, 2, 2, 2]);
        b.read(context, 0, &mut bytes)?;
        assert_eq!(bytes, [8, 8, 8, 8, 3, 3, 3, 3, 8, 8, 8, 8, 8, 8, 8, 8]);
        Ok(())
    }
    #[test]
    #[ignore = "需要真实Vulkan与coherent DEVICE_LOCAL映射；主任务串行调度"]
    fn mapped_device_ranges_reject_the_whole_write_before_mutation() -> Result<(), crate::Error> {
        let mut owner = pollster::block_on(crate::interop::WgpuContext::compute())?;
        let context = owner.native();
        let buffer = super::Buffer::mapped_device_local(context, 16)?;
        buffer.write(context, 0, &[7; 16])?;
        assert!(
            buffer
                .write_ranges(context, &[(0, &[1; 4]), (15, &[2; 4])])
                .is_err()
        );
        let mut bytes = [0u8; 16];
        buffer.read(context, 0, &mut bytes)?;
        assert_eq!(bytes, [7; 16]);
        buffer.write_ranges(context, &[(0, &[1; 4]), (12, &[2; 4])])?;
        buffer.read(context, 0, &mut bytes)?;
        assert_eq!(&bytes[..4], &[1; 4]);
        assert_eq!(&bytes[4..12], &[7; 8]);
        assert_eq!(&bytes[12..], &[2; 4]);
        Ok(())
    }
    #[test]
    #[ignore = "需要真实 Vulkan；主任务统一调度"]
    fn failed_external_encode_returns_cancelable_record() -> Result<(), crate::Error> {
        let mut context = pollster::block_on(crate::interop::WgpuContext::compute())?;
        let buffer = super::Buffer::new(context.native(), 4, true)?;
        let mut commands = super::Commands::new(context.native())?;
        commands.clear_buffer(buffer)?;
        let recorded = commands.finish(context.native())?;
        let Err((error, recorded)) = (unsafe { recorded.try_encode(context.native(), 0) }) else {
            panic!("空 command 不得产生在途票据");
        };
        assert!(matches!(error, crate::Error::InvalidArgument(_)));
        context.native().cancel(recorded)?;
        buffer.write(context.native(), 0, &11u32.to_le_bytes())?;
        Ok(())
    }

    #[test]
    fn invocation_reorder_requires_ray_tracing() {
        let capabilities = super::Capabilities {
            invocation_reorder: true,
            ..Default::default()
        };
        assert!(matches!(
            capabilities.validate(),
            Err(crate::Error::InvalidArgument(_))
        ));
        assert!(
            super::Capabilities {
                ray_tracing: true,
                ..capabilities
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn trace_axis_limit_applies_within_invocation_budget() {
        assert!(super::valid_trace_range([1, 4, 1], 64, [64, 4, 64]));
        assert!(!super::valid_trace_range([1, 5, 1], 64, [64, 4, 64]));
    }
}
