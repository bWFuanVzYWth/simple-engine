//! 与宿主无关的 Vulkan 资源、描述符和录制。shader 接口由受控 Slang 构建维护。
//!
//! Context 唯一拥有资源和命令版本；标识不保活，Recorded/Flight 须显式取消或报告完成。
//! 导入边界负责宿主设备有效期，资源更新受待执行/在途使用约束；不引入 Naga 或额外访存插桩。

use crate::graph;
use crate::handles::RecordId;
#[doc(inline)]
pub use crate::handles::{
    Bindings, Buffer, ComputePipeline, ExternalBinding, ExternalKind, Image, LightRegion,
    LightWorld, RayTracingPipeline, Resource, ResourceAccess, Sampler,
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

#[path = "native_scene.rs"]
mod scene;

#[path = "native_buffer.rs"]
mod buffer_access;
#[path = "native_image.rs"]
mod images;
use images::{ImageData, SamplerData};
#[doc(inline)]
pub use images::{ImageFormat, RawImage};

#[path = "native_bindings.rs"]
mod descriptors;
#[doc(inline)]
pub use descriptors::Binding;
use descriptors::{DescriptorLayoutData, DescriptorSetData};

#[path = "native_pipeline.rs"]
mod pipelines;
#[doc(inline)]
pub use pipelines::RayStackSizes;
use pipelines::{ComputePipelineData, RayPipelineData};

#[path = "native_commands.rs"]
mod commands;
#[doc(inline)]
pub use commands::Commands;
use commands::memory_barrier;

#[path = "native_light_tree.rs"]
mod light_tree;
pub use light_tree::DeviceLightTree;
#[path = "native_light_world.rs"]
mod light_world;
pub use light_world::{LightRegionInput, LightWorldInput};
#[path = "native_light_pages.rs"]
mod light_pages;
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
    light_pages: light_pages::LightPages,
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
    type Set = DescriptorSetData;
    type Layout = DescriptorLayoutData;
    type RayProgram = RayPipelineData;
    type ComputeProgram = ComputePipelineData;
    type Structure = crate::acceleration::Structure;
    type Micromap = MicromapData;
    type SharedBuffer = SharedBuffer;
    type SharedTexture = SharedTexture;
    type SampledTexture = SampledTexture;
}

type Data = meta::Data<Native>;
type BindingData = meta::BindingData<Native>;
type StoredBinding = meta::Binding<Native>;
type RayProgram = meta::Program<RayPipelineData>;
type ComputeProgram = meta::Program<ComputePipelineData>;
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
            light_pages: light_pages::LightPages::default(),
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
                light_tree::BUILD_PUSH_BYTES as u32,
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
        (self.life.retained_bytes()
            + data
            + records
            + self.as_pool.retained_bytes()
            + self.light_pages.retained()) as u64
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
            if matches!(data, Data::Buffer(_)) {
                self.light_pages.forget(key);
            }
            match (data, &mut self.buffer_retirement) {
                (Data::LightRegion(slice), _) => self.light_pages.release(slice),
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
            return Err(Error::InvalidState("资源构建尚未提交"));
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
                        return Err(Error::InvalidState("构建命令必须先于消费者提交"));
                    }
                }
                visited.try_reserve(1)?;
                visited.insert(resource);
                // 已构建或已排队的资源已验证子依赖；仅本次新建输出再检查。
                if matches!(
                    data,
                    Data::Blas(_) | Data::Acceleration(_) | Data::LightWorld(_)
                ) && data.producer() != Some(key)
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
    fn layout(&self, key: Key) -> Result<&DescriptorLayoutData, Error> {
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
            Self::LightWorld(data) => data.regions.capacity() * size_of::<LightRegion>(),
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
            Self::LightWorld(v) => v.raw.destroy(device),
            Self::LightRegion(_) => {}
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
        assert!(super::commands::valid_trace_range(
            [1, 4, 1],
            64,
            [64, 4, 64]
        ));
        assert!(!super::commands::valid_trace_range(
            [1, 5, 1],
            64,
            [64, 4, 64]
        ));
    }
}
