//! wgpu Vulkan 互操作；native 命令单独录制，共享 storage buffer、storage 与 sampled 图像。
//! SPIR-V 沿用 crate 级契约，不经过 Naga。
//!
//! # 交接与生命周期
//!
//! 普通 wgpu 命令和 native 命令分别录制，由 [`WgpuContext::submit`] 统一排序。完整 PT
//! 阶段内部无需返回 wgpu；前段 wgpu 完整初始化图像，storage 以 `GENERAL`、sampled 以
//! `SHADER_READ_ONLY_OPTIMAL` 交接。tracker 与边界 barrier 维护状态，buffer 使用前须取消映射。
//!
//! native 录制在唯一 Context 中登记资源根；Copy ID 不拥有资源。提交层补充 tracker，
//! 经 submit 的批次（含纯 wgpu）得到实际提交序号，完成回调仅发送序号。收取通知后关闭在途记录，
//! 并退役已被 collect 撤销的版本；不得显式 destroy 仍被原生命令使用的外部 wgpu 资源。
//! HAL guard 使用设备的非递归锁，必须逐个借入并及时释放，持有时不得调用其他 wgpu API。
//!
//! # 第三方故障边界
//!
//! 固定的 wgpu 30.0.1 在 `Device::poll` 收到部分 HAL 设备故障时走内部 fatal panic；
//! [`wgpu::PollError`] 仅公开超时与提交索引错误。本模块无法经稳定接口取得全部底层来源，
//! 正常轮询仍受此限制；不匹配 panic 文本或修改依赖实现。析构改用原生队列等待：
//! 完成或 DEVICE_LOST 后释放，其他等待失败保留设备和资源并记录错误。
//!
//! 来源：[HAL 录制](https://docs.rs/wgpu/30.0.1/wgpu/struct.CommandEncoder.html#method.as_hal_mut)、
//! [资源状态交接](https://docs.rs/wgpu/30.0.1/wgpu/struct.CommandEncoder.html#method.transition_resources)。

use crate::handles::RecordId;
use crate::native::{Context, Recorded};
use crate::{
    AccelerationStructure, Device, Error, Instance, OpacityMap, OpacityMicromap, PhysicalDevice,
    TriangleBlas, TriangleInstance, khr, shader::decode_spirv, vk,
};
use ash::vk::Handle as _;
use ash::vk::TaggedStructure as _;
use ash_wgpu::vk::Handle as _;
use std::{
    ffi::CStr,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};
use wgpu::hal::api::Vulkan;

/// 已声明 Vulkan 1.4 的 wgpu 实例；通过 Deref 使用普通 adapter/surface 接口。
///
/// 该身份保证 native 核心接口与宿主 API 版本一致；wgpu 默认实例目前只声明 1.3。
#[derive(Clone, Debug)]
pub struct VulkanInstance {
    inner: wgpu::Instance,
}

impl VulkanInstance {
    /// 接管外部宿主已创建的 Vulkan 1.4 实例。
    ///
    /// # Safety
    /// 实例必须声明至少 Vulkan 1.4，且 HAL 的实例版本与实际 VkApplicationInfo 一致。
    pub const unsafe fn from_wgpu(inner: wgpu::Instance) -> Self {
        Self { inner }
    }
}

impl std::ops::Deref for VulkanInstance {
    type Target = wgpu::Instance;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// 同设备、同 Vulkan 队列的唯一提交所有者；内含唯一 native Context。
///
/// wgpu 资源使用外部 handle，native 资源/版本由 Context 资源表独占。析构等待全部提交；无法确认完成时
/// 记录错误并保留设备与在途资源，避免提前释放。枚举索引属于本次 Vulkan 实例，不是跨运行的设备身份。
/// Windows dlss_rr 在设备创建时独立查询并可选启用 SDK 能力；不支持时仍可正常使用原生 RT。
/// early bootstrap 进程只允许一个 Context 设备 owner；其析构先关闭 SDK，再释放 VkDevice。
/// 该进程关闭后不重新初始化 SDK，普通 native Context 不受此限制。
/// 本 owner 的操作捕获 wgpu 验证/内存/内部错误；发生故障后拒绝继续构建、提交或发布，
/// 不确定批次保留至受控清理。宿主直接使用 device/queue 时仍自行处理其错误和同步。
pub struct WgpuContext {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) native: Context,
    pipeline_cache: Option<(wgpu::PipelineCache, crate::pipeline_cache::Disk)>,
    #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
    pub(crate) streamline: crate::streamline_interop::Session,
    info: wgpu::AdapterInfo,
    completed_tx: mpsc::Sender<u64>,
    completed_rx: mpsc::Receiver<u64>,
}

impl WgpuContext {
    /// 创建声明 Vulkan 1.4 的 wgpu 实例，保留平台 surface 扩展与环境诊断开关。
    ///
    /// adapter 和 surface 均须由此实例创建；普通 graphics/compute 不额外要求 RT。
    /// # Errors
    /// Vulkan loader 低于 1.4、实例或诊断资源创建失败时返回保留来源的错误。
    pub fn instance() -> Result<VulkanInstance, Error> {
        crate::host::instance().map(|inner| VulkanInstance { inner })
    }

    /// 使用第一个 Vulkan 适配器，要求 KHR 光追与 position fetch；支持 OMM、EXT SER 时自动启用。
    pub async fn new() -> Result<Self, Error> {
        Self::with_index(0, true).await
    }

    /// 使用第一个 Vulkan 适配器，不请求硬件光追或查询 SER；仍支持普通 graphics/compute。
    pub async fn compute() -> Result<Self, Error> {
        Self::with_index(0, false).await
    }

    /// 按本次 Vulkan 枚举索引选择设备；光追要求 position fetch，并自动检测 OMM、EXT SER。
    pub async fn with_index(index: usize, ray_tracing: bool) -> Result<Self, Error> {
        Self::with_features(index, ray_tracing, wgpu::Features::empty()).await
    }

    /// `features` 追加到 SPIR-V 直通与光追所需能力；光追时自动检测 OMM、EXT SER，其余请求缺失即失败。
    pub async fn with_features(
        index: usize,
        ray_tracing: bool,
        features: wgpu::Features,
    ) -> Result<Self, Error> {
        Self::with_options(
            index,
            crate::native::Capabilities {
                ray_tracing,
                position_fetch: ray_tracing,
                opacity_micromap: false,
                invocation_reorder: false,
                pipeline_statistics: false,
            },
            features,
            ray_tracing,
        )
        .await
    }

    /// 按场景请求 RT 及附加能力；position fetch、OMM 和 EXT SER 须依赖 RT。
    /// 未请求 SER 时不启用该设备能力；显式请求但不支持时返回 Unsupported。
    pub async fn with_capabilities(
        index: usize,
        capabilities: crate::native::Capabilities,
        features: wgpu::Features,
    ) -> Result<Self, Error> {
        Self::with_options(index, capabilities, features, false).await
    }

    async fn with_options(
        index: usize,
        capabilities: crate::native::Capabilities,
        features: wgpu::Features,
        optional_accelerations: bool,
    ) -> Result<Self, Error> {
        let instance = Self::instance()?;
        let adapter = instance
            .enumerate_adapters(wgpu::Backends::VULKAN)
            .await
            .into_iter()
            .nth(index)
            .ok_or(Error::AdapterIndex(index))?;
        Self::from_adapter_options(
            &instance,
            &adapter,
            capabilities,
            features,
            optional_accelerations,
        )
    }

    /// 实例须启用 Vulkan；光追、ray query、position fetch 和同步能力在创建设备时追加。
    /// 支持 OMM、EXT SER 时自动启用；普通渲染不查询或请求这些光追能力。
    /// 可选核心 `pipelineRobustness` 仅关闭原生光追管线保护；不支持时沿用设备默认。
    /// `instance` 须由 [`Self::instance`] 或声明 Vulkan 1.4 的外部宿主创建，并为 adapter 的原始实例。
    pub fn from_adapter(
        instance: &VulkanInstance,
        adapter: &wgpu::Adapter,
        ray_tracing: bool,
    ) -> Result<Self, Error> {
        Self::from_adapter_with_features(instance, adapter, ray_tracing, wgpu::Features::empty())
    }

    /// 从已有 Vulkan 适配器创建同队列设备，并追加请求的 wgpu features。
    ///
    /// `instance` 必须是 `adapter` 的原始实例，并声明 Vulkan 1.4；光追扩展
    /// 须在创建设备时启用。主队列族须同时支持 graphics/compute；`ray_tracing` 为 true
    /// 时要求 `VK_KHR_ray_query`、position fetch 及 synchronization2 对应能力，
    /// 并在支持时启用 OMM、EXT SER；未支持的可选能力由调用方选择对应 shader。
    /// # Errors
    /// 缺少请求能力时返回具体 Unsupported；查询与设备创建失败时保留平台来源。
    pub fn from_adapter_with_features(
        instance: &VulkanInstance,
        adapter: &wgpu::Adapter,
        ray_tracing: bool,
        features: wgpu::Features,
    ) -> Result<Self, Error> {
        Self::from_adapter_options(
            instance,
            adapter,
            crate::native::Capabilities {
                ray_tracing,
                position_fetch: ray_tracing,
                opacity_micromap: false,
                invocation_reorder: false,
                pipeline_statistics: false,
            },
            features,
            ray_tracing,
        )
    }

    /// 从 Vulkan 适配器创建按场景能力配置的设备；instance 须为 adapter 原始实例。
    /// position fetch、OMM 和 EXT SER 必须同时请求 RT；缺少请求能力时返回具体 Unsupported。
    /// 不自动检测 SER；`invocation_reorder: false` 保持普通光追设备。
    pub fn from_adapter_with_capabilities(
        instance: &VulkanInstance,
        adapter: &wgpu::Adapter,
        capabilities: crate::native::Capabilities,
        features: wgpu::Features,
    ) -> Result<Self, Error> {
        Self::from_adapter_options(instance, adapter, capabilities, features, false)
    }

    fn from_adapter_options(
        instance: &VulkanInstance,
        adapter: &wgpu::Adapter,
        capabilities: crate::native::Capabilities,
        mut features: wgpu::Features,
        optional_accelerations: bool,
    ) -> Result<Self, Error> {
        capabilities.validate()?;
        let ray_tracing = capabilities.ray_tracing;
        let position_fetch_enabled = capabilities.position_fetch
            || features.contains(wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN);
        let mut micromap_enabled = false;
        let mut invocation_reorder_enabled = false;
        if !ray_tracing && position_fetch_enabled {
            return Err(Error::InvalidArgument("position fetch 依赖 RT"));
        }
        features |= wgpu::Features::PASSTHROUGH_SHADERS;
        features |= adapter.features() & wgpu::Features::PIPELINE_CACHE;
        if ray_tracing {
            features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }
        if !adapter.features().contains(features) {
            return Err(Error::Unsupported("wgpu 适配器不支持请求的 features"));
        }
        let hal = unsafe { adapter.as_hal::<Vulkan>() }
            .ok_or(Error::Unsupported("wgpu 未使用 Vulkan 后端"))?;
        let instance_hal = unsafe { instance.as_hal::<Vulkan>() }
            .ok_or(Error::Unsupported("实例未使用 Vulkan 后端"))?;
        if instance_hal.shared_instance().raw_instance().handle()
            != hal.shared_instance().raw_instance().handle()
        {
            return Err(Error::InvalidArgument("adapter 不属于给定的 Vulkan 实例"));
        }
        // 只桥接 Vulkan 稳定函数指针 ABI；原始 wgpu instance 保活其实际 loader。
        let entry = unsafe {
            ash::Entry::from_static_fn(ash::StaticFn {
                get_instance_proc_addr: std::mem::transmute::<
                    ash_wgpu::vk::PFN_vkGetInstanceProcAddr,
                    vk::PFN_vkGetInstanceProcAddr,
                >(
                    hal.shared_instance()
                        .entry()
                        .static_fn()
                        .get_instance_proc_addr,
                ),
            })
        };
        let raw_instance = unsafe {
            ash::Instance::load(
                entry.static_fn(),
                vk::Instance::from_raw(hal.shared_instance().raw_instance().handle().as_raw()),
            )
        };
        let physical = PhysicalDevice {
            raw: vk::PhysicalDevice::from_raw(hal.raw_physical_device().as_raw()),
            instance: Instance {
                raw: raw_instance,
                _owner: Some(instance.inner.clone()),
            },
        };
        let api_version = physical
            .properties()
            .api_version
            .min(hal.shared_instance().instance_api_version());
        if ray_tracing && api_version < vk::API_VERSION_1_4 {
            return Err(Error::Unsupported("native RT 需要 Vulkan 1.4"));
        }
        // wgpu 30 的 HAL open 固定选队列族 0，不改变其归属。
        if !physical.queue_families().first().is_some_and(|family| {
            family
                .queue_flags
                .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
        }) {
            return Err(Error::Unsupported("wgpu 主队列族不支持 graphics/compute"));
        }
        #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
        let mut sdk = crate::streamline_interop::DeviceFeatures::query(&physical)?;
        #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
        let sdk_capable = sdk.capable && ray_tracing;
        let extensions = [
            khr::acceleration_structure::NAME,
            khr::deferred_host_operations::NAME,
            khr::ray_tracing_pipeline::NAME,
            khr::device_address_commands::NAME,
        ];
        let mut address = vk::PhysicalDeviceBufferDeviceAddressFeatures::default();
        let mut address_commands = vk::PhysicalDeviceDeviceAddressCommandsFeaturesKHR::default();
        let mut acceleration = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
        let mut tracing = vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default();
        let mut query = vk::PhysicalDeviceRayQueryFeaturesKHR::default();
        let mut position_fetch = vk::PhysicalDeviceRayTracingPositionFetchFeaturesKHR::default();
        let mut micromap = vk::PhysicalDeviceOpacityMicromapFeaturesKHR::default();
        let mut synchronization2 = vk::PhysicalDeviceSynchronization2Features::default();
        let mut robustness = vk::PhysicalDevicePipelineRobustnessFeatures::default();
        let mut invocation_reorder =
            vk::PhysicalDeviceRayTracingInvocationReorderFeaturesEXT::default();
        let mut pipeline_robustness = false;
        let mut pipeline_statistics =
            vk::PhysicalDevicePipelineExecutablePropertiesFeaturesKHR::default();
        if capabilities.pipeline_statistics
            && !physical.supports_extensions(&[khr::pipeline_executable_properties::NAME])?
        {
            return Err(Error::Unsupported(
                "设备不支持 VK_KHR_pipeline_executable_properties",
            ));
        }
        if ray_tracing {
            if !physical.supports_extensions(&[khr::ray_query::NAME])? {
                return Err(Error::Unsupported("设备不支持 VK_KHR_ray_query"));
            }
            let query_micromap = (capabilities.opacity_micromap || optional_accelerations)
                && physical.supports_extensions(&[khr::opacity_micromap::NAME])?;
            if capabilities.opacity_micromap && !query_micromap {
                return Err(Error::Unsupported("设备不支持 VK_KHR_opacity_micromap"));
            }
            if position_fetch_enabled
                && !physical.supports_extensions(&[khr::ray_tracing_position_fetch::NAME])?
            {
                return Err(Error::Unsupported(
                    "设备不支持 VK_KHR_ray_tracing_position_fetch",
                ));
            }
            let query_reorder = (capabilities.invocation_reorder || optional_accelerations)
                && physical
                    .supports_extensions(&[ash::ext::ray_tracing_invocation_reorder::NAME])?;
            if capabilities.invocation_reorder && !query_reorder {
                return Err(Error::Unsupported(
                    "设备不支持 VK_EXT_ray_tracing_invocation_reorder",
                ));
            }
            let mut supported = vk::PhysicalDeviceFeatures2::default()
                .push(&mut address)
                .push(&mut address_commands)
                .push(&mut acceleration)
                .push(&mut tracing)
                .push(&mut query)
                .push(&mut robustness)
                .push(&mut synchronization2);
            if capabilities.pipeline_statistics {
                supported = supported.push(&mut pipeline_statistics);
            }
            if position_fetch_enabled {
                supported = supported.push(&mut position_fetch);
            }
            if query_micromap {
                supported = supported.push(&mut micromap);
            }
            if query_reorder {
                supported = supported.push(&mut invocation_reorder);
            }
            unsafe {
                physical
                    .instance
                    .raw
                    .get_physical_device_features2(physical.raw, &mut supported)
            };
            pipeline_robustness = robustness.pipeline_robustness != 0;
            if capabilities.pipeline_statistics && pipeline_statistics.pipeline_executable_info == 0
            {
                return Err(Error::Unsupported("设备不支持 pipelineExecutableInfo"));
            }
            if !physical.supports_extensions(&extensions)?
                || address.buffer_device_address == 0
                || address_commands.device_address_commands == 0
                || acceleration.acceleration_structure == 0
                || tracing.ray_tracing_pipeline == 0
            {
                return Err(Error::Unsupported(
                    "设备不支持 Vulkan 1.4 KHR RT 与 device address commands",
                ));
            }
            if position_fetch_enabled && position_fetch.ray_tracing_position_fetch == 0 {
                return Err(Error::Unsupported("设备不支持 rayTracingPositionFetch"));
            }
            if query.ray_query == 0 {
                return Err(Error::Unsupported("设备不支持 rayQuery feature"));
            }
            if capabilities.opacity_micromap && micromap.micromap == 0 {
                return Err(Error::Unsupported("设备不支持 opacity micromap feature"));
            }
            micromap_enabled = query_micromap && micromap.micromap != 0;
            if synchronization2.synchronization2 == 0 {
                return Err(Error::Unsupported("设备不支持 synchronization2 feature"));
            }
            invocation_reorder_enabled =
                query_reorder && invocation_reorder.ray_tracing_invocation_reorder != 0;
            if invocation_reorder_enabled && !capabilities.invocation_reorder {
                let mut reorder =
                    vk::PhysicalDeviceRayTracingInvocationReorderPropertiesEXT::default();
                let mut properties = vk::PhysicalDeviceProperties2::default().push(&mut reorder);
                unsafe {
                    physical
                        .instance
                        .raw
                        .get_physical_device_properties2(physical.raw, &mut properties);
                }
                invocation_reorder_enabled = reorder.ray_tracing_invocation_reorder_reordering_hint
                    == vk::RayTracingInvocationReorderModeEXT::REORDER;
            }
            if capabilities.invocation_reorder && !invocation_reorder_enabled {
                return Err(Error::Unsupported("设备不支持 rayTracingInvocationReorder"));
            }
        }
        address =
            vk::PhysicalDeviceBufferDeviceAddressFeatures::default().buffer_device_address(true);
        address_commands = vk::PhysicalDeviceDeviceAddressCommandsFeaturesKHR::default()
            .device_address_commands(true);
        acceleration = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
            .acceleration_structure(true);
        tracing =
            vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default().ray_tracing_pipeline(true);
        query = vk::PhysicalDeviceRayQueryFeaturesKHR::default().ray_query(true);
        position_fetch = vk::PhysicalDeviceRayTracingPositionFetchFeaturesKHR::default()
            .ray_tracing_position_fetch(true);
        micromap = vk::PhysicalDeviceOpacityMicromapFeaturesKHR::default().micromap(true);
        synchronization2 =
            vk::PhysicalDeviceSynchronization2Features::default().synchronization2(true);
        robustness = vk::PhysicalDevicePipelineRobustnessFeatures::default()
            .pipeline_robustness(pipeline_robustness);
        invocation_reorder = vk::PhysicalDeviceRayTracingInvocationReorderFeaturesEXT::default()
            .ray_tracing_invocation_reorder(invocation_reorder_enabled);
        pipeline_statistics = vk::PhysicalDevicePipelineExecutablePropertiesFeaturesKHR::default()
            .pipeline_executable_info(capabilities.pipeline_statistics);
        let pipeline_statistics = &mut pipeline_statistics;
        let address = &mut address;
        let address_commands = &mut address_commands;
        let acceleration = &mut acceleration;
        let tracing = &mut tracing;
        let query = &mut query;
        let position_fetch = &mut position_fetch;
        let micromap = &mut micromap;
        let synchronization2 = &mut synchronization2;
        let robustness = &mut robustness;
        let invocation_reorder = &mut invocation_reorder;
        let wgpu_position_fetch =
            features.contains(wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN);
        let wgpu_query = features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
        let wgpu_tracing = features.contains(wgpu::Features::EXPERIMENTAL_RAY_TRACING_PIPELINES);
        let wgpu_acceleration = wgpu_query || wgpu_tracing;
        #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
        let sdk_indexing = &mut sdk.indexing;
        #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
        let sdk_private = &mut sdk.private_data;
        #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
        let sdk_extended = sdk.extended_formats;
        let callback: Option<Box<wgpu::hal::vulkan::CreateDeviceCallback<'_>>> = if ray_tracing
            || cfg!(all(feature = "dlss_rr", target_os = "windows"))
        {
            Some(Box::new(move |args| {
                // VkDeviceCreateInfo 是稳定的 Vulkan 1.0 C ABI；仅 pNext 借入新绑定的 feature 结构。
                // HAL 同步创建设备，所有结构及其借用覆盖本次 callback/open 调用。
                const {
                    assert!(
                        size_of::<vk::DeviceCreateInfo>()
                            == size_of::<ash_wgpu::vk::DeviceCreateInfo>()
                    );
                }
                const {
                    assert!(
                        align_of::<vk::DeviceCreateInfo>()
                            == align_of::<ash_wgpu::vk::DeviceCreateInfo>()
                    );
                }
                let create_info =
                    unsafe { &mut *(args.create_info as *mut _ as *mut vk::DeviceCreateInfo<'_>) };
                if ray_tracing {
                    for extension in extensions.into_iter().chain([khr::ray_query::NAME]) {
                        if !args.extensions.contains(&extension) {
                            args.extensions.push(extension);
                        }
                    }
                    if capabilities.pipeline_statistics {
                        args.extensions
                            .push(khr::pipeline_executable_properties::NAME);
                        *create_info = (*create_info).push(pipeline_statistics);
                    }
                    if micromap_enabled {
                        args.extensions.push(khr::opacity_micromap::NAME);
                        *create_info = (*create_info).push(micromap);
                    }
                    *create_info = (*create_info).push(address_commands).push(synchronization2);
                    if position_fetch_enabled
                        && !args
                            .extensions
                            .contains(&khr::ray_tracing_position_fetch::NAME)
                    {
                        args.extensions.push(khr::ray_tracing_position_fetch::NAME);
                    }
                    // HAL 已为显式请求的 wgpu feature 挂接同一结构，不能重复 sType。
                    if !wgpu_acceleration {
                        *create_info = (*create_info).push(address).push(acceleration);
                    }
                    if !wgpu_tracing {
                        *create_info = (*create_info).push(tracing);
                    }
                    if !wgpu_query {
                        *create_info = (*create_info).push(query);
                    }
                    if position_fetch_enabled && !wgpu_position_fetch {
                        *create_info = (*create_info).push(position_fetch);
                    }
                    if pipeline_robustness {
                        *create_info = (*create_info).push(robustness);
                    }
                    if invocation_reorder_enabled {
                        args.extensions
                            .push(ash::ext::ray_tracing_invocation_reorder::NAME);
                        *create_info = (*create_info).push(invocation_reorder);
                    }
                }
                #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
                {
                    let has_indexing = crate::streamline_interop::enable_hal_features(
                        args.device_features,
                        sdk_extended,
                        sdk_capable,
                        sdk_capable.then_some(*sdk_indexing),
                    );
                    if sdk_capable {
                        for extension in crate::streamline_interop::EXTENSIONS {
                            if !args.extensions.contains(&extension) {
                                args.extensions.push(extension);
                            }
                        }
                        if !has_indexing {
                            *create_info = (*create_info).push(sdk_indexing);
                        }
                    }
                    // interposer 在创建设备时使用 private-data slot，与当前是否执行 RR 无关。
                    if sdk_private.private_data != 0 {
                        *create_info = (*create_info).push(sdk_private);
                    }
                }
            }))
        } else {
            None
        };
        let desc = wgpu::DeviceDescriptor {
            label: Some("native wgpu"),
            required_features: features,
            required_limits: adapter.limits(),
            ..Default::default()
        };
        // callback 仅追加已查询的能力；借入的 feature 结构存活至同步设备创建返回。
        #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
        let streamline_process = crate::streamline_interop::ProcessOwner::claim()?;
        let open = unsafe {
            hal.open_with_callback(
                features,
                &desc.required_limits,
                &desc.memory_hints,
                callback,
            )?
        };
        let (device, queue) = unsafe { adapter.create_device_from_hal::<Vulkan>(open, &desc)? };
        let hal_device = unsafe { device.as_hal::<Vulkan>() }
            .ok_or(Error::Unsupported("无法借入 wgpu Vulkan device"))?;
        let health = crate::health::Health::new();
        health.attach(&device);
        let native = Context::new(Device {
            health,
            uncertain_use: Default::default(),
            raw: unsafe {
                ash::Device::load(
                    physical.instance.raw.fp_v1_0(),
                    vk::Device::from_raw(hal_device.raw_device().handle().as_raw()),
                )
            },
            physical,
            buffer_bytes: AtomicU64::new(0),
            ray_pipeline: ray_tracing,
            pipeline_robustness,
            subgroup_size_control: api_version >= vk::API_VERSION_1_3,
            _owner: Some(device.clone()),
            wgpu_queue: Some(queue.clone()),
            _loader: Some(entry),
            queue: vk::Queue::from_raw(hal_device.raw_queue().as_raw()),
            queue_family: hal_device.queue_family_index(),
            position_fetch: position_fetch_enabled,
            opacity_micromap: micromap_enabled,
            invocation_reorder: invocation_reorder_enabled,
            pipeline_statistics: capabilities.pipeline_statistics,
        });
        drop(hal_device);
        let (completed_tx, completed_rx) = mpsc::channel();
        Ok(Self {
            device,
            queue,
            native,
            pipeline_cache: None,
            #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
            streamline: crate::streamline_interop::Session::new(sdk_capable, streamline_process),
            info: adapter.get_info(),
            completed_tx,
            completed_rx,
        })
    }

    /// 借入外部 wgpu 设备 handle，用于普通资源与命令录制。
    /// 宿主须自行处理其直接操作的验证错误，不得替换本 owner 的 device-lost callback。
    pub const fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// 在应用私有、可信目录启用 native 和 wgpu 两套独立的驱动缓存。
    /// 缺少 wgpu 管线缓存能力时只启用 native。应在构建应用管线前调用。
    pub fn enable_pipeline_caches(&mut self, directory: &std::path::Path) {
        self.native.enable_pipeline_cache(directory);
        if self.pipeline_cache.is_some()
            || self.check_status().is_err()
            || !self
                .device
                .features()
                .contains(wgpu::Features::PIPELINE_CACHE)
        {
            return;
        }
        let disk = crate::pipeline_cache::Disk::new(
            directory,
            "wgpu",
            &self.native.device.physical.properties(),
        );
        let bytes = disk.read();
        // SAFETY: 非空数据由本路径的同设备 wgpu 快照生成，读取已校验完整性。
        let cache = unsafe {
            self.device
                .create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("应用管线缓存"),
                    data: (!bytes.is_empty()).then_some(bytes.as_slice()),
                    fallback: true,
                })
        };
        self.pipeline_cache = Some((cache, disk));
    }

    /// 为同设备 wgpu 管线构造提供缓存，不适用于 raw Vulkan 描述符。
    pub fn pipeline_cache(&self) -> Option<&wgpu::PipelineCache> {
        let (cache, disk) = self.pipeline_cache.as_ref()?;
        disk.used();
        Some(cache)
    }

    /// 在初始化或资源发布边界保存新增缓存数据，不提交命令或等待 GPU。
    pub fn flush_pipeline_caches(&mut self) {
        self.native.flush_pipeline_cache();
        if let Some((cache, disk)) = &mut self.pipeline_cache {
            disk.flush(|| cache.get_data());
        }
    }

    /// 设置宿主丢失通知，同时保留本 owner 的失效事件登记。
    /// 回调先登记故障再通知宿主；不得再通过 device() 替换底层回调。
    pub fn on_device_lost(
        &self,
        observer: impl Fn(wgpu::DeviceLostReason, String) + Send + 'static,
    ) {
        self.native.device.health.observe(&self.device, observer);
    }

    /// 收取设备故障；首个原始错误由触发操作返回，失效后只允许诊断和受控清理。
    pub fn check_status(&self) -> Result<(), Error> {
        self.native.device.ensure_active()
    }

    /// 借用唯一提交队列；native 批次或依赖完成序号的资源回收须经 [`Self::submit`]。
    /// 直接提交此队列不推进本 owner 的序号。
    pub const fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// 复制所选适配器的设备与驱动身份，供诊断与测量归档。
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.info.clone()
    }

    /// 原生主队列的 timestamp 位宽；0 表示不支持。
    pub fn timestamp_valid_bits(&self) -> u32 {
        self.native.device.physical.queue_families()[0].timestamp_valid_bits
    }

    /// 本 crate 显式申请的 buffer/image/AS/SBT 及池闲置 allocation，不含 wgpu 和驱动内部分配。
    pub fn allocated_native_bytes(&self) -> u64 {
        self.native.device.buffer_bytes.load(Ordering::Relaxed)
    }

    /// 完整初始化的共享 storage buffer；其余用途由 usage 指定。
    /// 长度须非零、4 字节对齐且不超过 storage binding 上限，`usage` 必须包含 `STORAGE`。
    /// # Errors
    /// 参数或映射失败返回原始错误；wgpu 校验、内存或设备故障同时停用所有者。
    pub fn buffer(
        &self,
        label: &str,
        bytes: &[u8],
        usage: wgpu::BufferUsages,
    ) -> Result<SharedBuffer, Error> {
        if bytes.is_empty()
            || !bytes.len().is_multiple_of(4)
            || bytes.len() as u64 > self.device.limits().max_storage_buffer_binding_size
            || !usage.contains(wgpu::BufferUsages::STORAGE)
        {
            return Err(Error::InvalidArgument("共享 storage buffer 大小或用途无效"));
        }
        let buffer = self.native.device.checked(|| {
            Ok(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.len() as u64,
                usage,
                mapped_at_creation: true,
            }))
        })?;
        self.native.device.checked(|| {
            buffer
                .slice(..)
                .get_mapped_range_mut()?
                .copy_from_slice(bytes);
            buffer.unmap();
            Ok(SharedBuffer {
                buffer,
                device: self.native.owner(),
            })
        })
    }

    /// 创建共享 storage buffer，并在同一队列上完整清零，不分配等长 CPU 数组。
    ///
    /// 长度须非零、4字节对齐且在设备和 storage binding 上限内，usage 须含 STORAGE。
    /// 自动补充 COPY_DST；清零在返回前提交，后续同队列写入与 native 提交按序观察结果，
    /// 本方法不等待 GPU 完成。所有者和在途命令保活资源，调用方不得显式 destroy。
    /// # Errors
    /// 大小或用途无效时返回参数错误；wgpu 校验、内存或设备故障返回错误并停用所有者。
    pub fn zeroed_buffer(
        &self,
        label: &str,
        bytes: u64,
        usage: wgpu::BufferUsages,
    ) -> Result<SharedBuffer, Error> {
        let limits = self.device.limits();
        if bytes == 0
            || !bytes.is_multiple_of(4)
            || bytes > limits.max_buffer_size
            || bytes > limits.max_storage_buffer_binding_size
            || !usage.contains(wgpu::BufferUsages::STORAGE)
        {
            return Err(Error::InvalidArgument("共享 storage buffer 大小或用途无效"));
        }
        let buffer = self.native.device.checked(|| {
            Ok(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }))
        })?;
        let command = self.native.device.checked(|| {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.clear_buffer(&buffer, 0, None);
            Ok(encoder.finish())
        })?;
        self.native.device.submit([command])?;
        Ok(SharedBuffer {
            buffer,
            device: self.native.owner(),
        })
    }

    /// 创建非零尺寸的单层 RGBA32F storage/sample 图像，native 使用前须由 wgpu 完整初始化。
    /// # Errors
    /// 尺寸无效时返回参数错误；wgpu 校验、内存或设备故障返回错误并停用所有者。
    pub fn texture(&self, label: &str, size: [u32; 2]) -> Result<SharedTexture, Error> {
        if size.contains(&0)
            || size
                .into_iter()
                .any(|side| side > self.device.limits().max_texture_dimension_2d)
        {
            return Err(Error::InvalidArgument("共享图像尺寸超出设备范围"));
        }
        self.native.device.checked(|| {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            Ok(SharedTexture {
                texture,
                device: self.native.owner(),
            })
        })
    }

    /// shader 绑定布局与工作组大小由应用显式提供给 wgpu 管线。
    /// wgpu Vulkan HAL 会压紧 binding 编号；透传 SPIR-V 须匹配实际编号，示例从 0 连续绑定。
    ///
    /// `entry` 与 `workgroup` 须与受控 Slang 产物一致；只解码 SPIR-V 字节，不验证接口和访存。
    /// # Errors
    /// SPIR-V 字节无效时返回参数错误；wgpu 校验、内存或设备故障返回错误并停用所有者。
    pub fn shader(
        &self,
        spirv: &[u8],
        entry: &str,
        workgroup: [u32; 3],
    ) -> Result<wgpu::ShaderModule, Error> {
        let code = decode_spirv(spirv)?;
        self.native.device.checked(|| {
            Ok(unsafe {
                self.device.create_shader_module_passthrough(
                    wgpu::ShaderModuleDescriptorPassthrough {
                        entry_points: vec![wgpu::PassthroughShaderEntryPoint {
                            name: entry.into(),
                            workgroup_size: (workgroup[0], workgroup[1], workgroup[2]),
                        }]
                        .into(),
                        spirv: Some(code.into()),
                        ..Default::default()
                    },
                )
            })
        })
    }

    /// 创建同设备 mip0 浮点采样图像；返回外部 wgpu handle 供写入和 native 导入。
    ///
    /// 支持单采样、单 mip 的 RGBA16F／RGBA32F 2D（单层）或 3D 图像。
    /// RGBA16F 支持线性采样；RGBA32F 仅承诺 texel Load，不要求 float32 filtering。
    /// 资源由本设备创建，归属由所有者维护；不接受无法证明归属的裸 wgpu 图像导入。
    /// 后续调用方不得显式 destroy 图像；在原生读取前须完整初始化。
    /// # Errors
    /// 格式、用途、维度或过滤能力不符合契约时返回错误；wgpu 校验、内存或设备故障同时停用所有者。
    pub fn sampled_texture(
        &self,
        descriptor: &wgpu::TextureDescriptor<'_>,
    ) -> Result<SampledTexture, Error> {
        if descriptor.sample_count != 1
            || descriptor.mip_level_count != 1
            || !matches!(
                descriptor.format,
                wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
            )
            || !descriptor
                .usage
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(Error::InvalidArgument(
                "采样图像须为单 mip/sample RGBA16F/32F",
            ));
        }
        match descriptor.dimension {
            wgpu::TextureDimension::D2 if descriptor.size.depth_or_array_layers == 1 => (),
            wgpu::TextureDimension::D3 => (),
            _ => return Err(Error::InvalidArgument("采样图像只支持单层 2D 或 3D")),
        }
        let format = match descriptor.format {
            wgpu::TextureFormat::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
            _ => vk::Format::R32G32B32A32_SFLOAT,
        };
        let properties = unsafe {
            self.native
                .device
                .physical
                .instance
                .raw
                .get_physical_device_format_properties(self.native.device.physical.raw, format)
        };
        if descriptor.format == wgpu::TextureFormat::Rgba16Float
            && !properties
                .optimal_tiling_features
                .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR)
        {
            return Err(Error::Unsupported("采样图像格式不支持线性过滤"));
        }
        let texture = self
            .native
            .device
            .checked(|| Ok(self.device.create_texture(descriptor)))?;
        self.native.device.checked(|| {
            let view = texture.create_view(&Default::default());
            Ok(SampledTexture {
                texture,
                view,
                device: self.native.owner(),
            })
        })
    }

    /// 初始化期同步构建 opaque 三角形 BLAS 与 identity 实例 TLAS，等待构建完成。
    ///
    /// primitive index 保持输入顺序；空输入产生零实例 TLAS。坐标须有限，数量须在设备
    /// 范围内；不支持增量更新，后续帧的追踪提交本身无需逐帧等待。
    pub fn acceleration_structure(
        &mut self,
        triangles: &[[[f32; 3]; 3]],
    ) -> Result<AccelerationStructure, Error> {
        let mut instances = Vec::new();
        if !triangles.is_empty() {
            instances.push(TriangleInstance {
                geometry: self.native.triangle_blas(triangles, true)?,
                transform: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]],
                custom_index: 0,
            });
        }
        self.native.acceleration_instances(&instances)
    }
    /// 同步构建 opaque BLAS，输入随后可释放。
    pub fn triangle_blas(&mut self, triangles: &[[[f32; 3]; 3]]) -> Result<TriangleBlas, Error> {
        self.native.triangle_blas(triangles, true)
    }

    /// 一次命令构建整批独立 BLAS，使用不重叠的 scratch 切片，只提交和等待一次。
    ///
    /// 输入显式指定 opaque 标记，可混合已有 OMM 和普通三角形；次序与输出一一对应，返回后输入即可释放。
    /// 全批成功后才返回正式标识；输入、scratch、AS backing 的峰值包含整批。
    /// Context 复用已完成的 input、scratch 与退役 backing；闲置显存合计最多128MiB，
    /// 超出时立即释放，当前值见 [`crate::native::Context::build_pool_idle_bytes`]。
    /// 空批直接返回，不提交或等待。TLAS 仍通过 [`Self::acceleration_instances`] 单独构建。
    ///
    /// # Errors
    /// 任一输入为空、非有限、数量超限、OMM 跨设备或索引不符时不提交；分配、录制和等待
    /// 失败保留底层来源。失败不返回部分标识，旧场景和已有资源不被替换。
    pub fn triangle_blas_batch(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
    ) -> Result<crate::TriangleBlasBatch, Error> {
        self.native.triangle_blas_batch(inputs)
    }

    /// 同 [`Self::triangle_blas_batch`]，可选记录本批 GPU timestamp，不增加 host 等待。
    ///
    /// 队列不支持 timestamp 时设备耗时为 `None`；两端位于输入 barrier 后的 AS 阶段，
    /// 区间仍受 GPU 调度、并发 PT 竞争及驱动 timestamp 实现影响，不能据差值精确分离此前 PT 工作。
    ///
    /// # Errors
    /// 与普通批入口相同；开启计时另可能返回 query pool 创建或结果读取错误。
    pub fn triangle_blas_batch_timed(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
        timestamps: bool,
    ) -> Result<crate::TriangleBlasBatch, Error> {
        self.native.triangle_blas_batch_timed(inputs, timestamps)
    }

    /// 复制输入并录制整批 BLAS，不提交队列、不等待 GPU，也不读取 timestamp。
    ///
    /// 返回的标识可立即建立 TLAS 和描述符；消费者提交时，producer 必须已在同队列提交，
    /// 或位于同次 [`Self::submit`] 的更早段。输入切片返回后可释放，GPU input/scratch
    /// 由 Context 记录独占，真实完成后才归池。准备过程中 collect 仍须包含待发布根，
    /// 或推迟到提交后；命令根只保寿命，不取消 collect 对新依赖的退休限制。
    ///
    /// 所有返回命令须交给同一 Context 的 submit 或 cancel；取消 producer 后标识失效，
    /// 依赖它的待提交命令也须取消。空批返回可正常提交或取消的空命令。
    ///
    /// # Errors
    /// 输入和分配错误与 [`Self::triangle_blas_batch`] 相同；失败不返回部分标识或命令。
    pub fn record_triangle_blas_batch(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
    ) -> Result<PreparedBlasBatch, Error> {
        self.drain_completed();
        self.native.record_triangle_blas_batch(inputs)
    }

    /// 复制实例并录制 TLAS，不提交或等待；空实例生成可追踪的空 TLAS。
    ///
    /// 实例可引用先前录制的 BLAS；合并命令时使用 `blas.commands.then(tlas.commands)`，
    /// 并在成功 submit 后发布新场景。BLAS→TLAS 及 TLAS→追踪的 barrier 已录入，
    /// 后续 PT 可在同次或同队列更晚提交；输入与 scratch 按真实完成序号归池。
    /// 取消和 collect 的契约同 [`Self::record_triangle_blas_batch`]。
    ///
    /// # Errors
    /// 跨设备、已取消或退休 BLAS、无效变换、数量超限及分配/录制失败保留错误来源。
    pub fn record_acceleration_instances(
        &mut self,
        instances: &[TriangleInstance],
    ) -> Result<PreparedTlas, Error> {
        self.drain_completed();
        self.native.record_acceleration_instances(instances)
    }
    /// 同步构建精确2-state OMM。
    pub fn opacity_micromap(&mut self, maps: &[OpacityMap<'_>]) -> Result<OpacityMicromap, Error> {
        self.native.opacity_micromap(maps)
    }
    /// 同步构建附带 OMM 的 BLAS，primitive 顺序不变。
    pub fn triangle_blas_with_opacity(
        &mut self,
        triangles: &[[[f32; 3]; 3]],
        micromap: OpacityMicromap,
        indices: &[u32],
    ) -> Result<TriangleBlas, Error> {
        self.native
            .triangle_blas_with_opacity(triangles, micromap, indices)
    }
    /// 同步构建实例 TLAS；依赖保存在唯一 native owner。
    pub fn acceleration_instances(
        &mut self,
        instances: &[TriangleInstance],
    ) -> Result<AccelerationStructure, Error> {
        self.native.acceleration_instances(instances)
    }

    /// 仅建立命令执行依赖；不提供资源内存可见性或状态转换。
    pub fn execution_barrier(&self) -> Result<CommandBuffer, Error> {
        let raw = record(&self.native.device, "execution barrier", |command| {
            unsafe {
                self.native.device.raw.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[],
                );
            }
            Ok(())
        })?;
        Ok(CommandBuffer::from(raw))
    }

    /// 将已在 Context 登记的命令录入同队列 command buffer；须通过本 owner 提交或取消。
    /// 返回命令须经 [`Self::submit`] 或 [`Self::cancel`]；失败时取消本 owner 的当前记录。
    pub fn record_native(&mut self, commands: Recorded) -> Result<CommandBuffer, Error> {
        self.drain_completed();
        let key = commands.key;
        let raw = match record(&self.native.device, "native commands", |raw| {
            self.native.encode(key, raw)
        }) {
            Ok(raw) => raw,
            Err(error) => {
                let _ = self.native.cancel(commands);
                return Err(error);
            }
        };
        let mut result = CommandBuffer::new(raw);
        result.first.recorded = Some(key);
        Ok(result)
    }
    /// 丢弃从未提交的完整命令批次，解除相应 pending 资源使用。
    pub fn cancel(&mut self, commands: CommandBuffer) -> Result<(), Error> {
        let mut result = Ok(());
        for segment in std::iter::once(commands.first).chain(commands.following) {
            #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
            if let Some(key) = segment.recorded {
                self.streamline.cancel(key);
            }
            if let Some(key) = segment.recorded
                && let Err(error) = self.native.cancel(Recorded { key })
                && result.is_ok()
            {
                result = Err(error);
            }
        }
        result
    }
    fn cancel_records(&mut self, records: impl IntoIterator<Item = RecordId>) {
        for key in records {
            #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
            self.streamline.cancel(key);
            let _ = self.native.cancel(Recorded { key });
        }
    }
    pub(crate) fn drain_completed(&mut self) {
        if !self.native.device.health.is_active() {
            return;
        }
        for serial in self.completed_rx.try_iter() {
            if !self.native.life.observe_completion(serial) {
                self.native
                    .device
                    .health
                    .fail(crate::DeviceFailure::Internal);
                return;
            }
        }
        self.native.completed();
    }
    /// 排序并提交混合命令；native 资源在真实完成消息到达前留在唯一 owner。
    /// 提交前失败会取消输入中本 owner 的全部 pending 记录，不影响其余批次。
    /// wgpu 验证结果被接受后才推进序号；验证错误或设备丢失使 owner 失效。
    /// 可能已经交付队列的失败批次转为不确定状态，完成回调不能将其视为成功构建。
    /// 错误返回前确认队列可安全清理，否则终止；不承诺设备恢复或 GPU 结果回滚。
    pub fn submit(
        &mut self,
        commands: impl IntoIterator<Item = CommandBuffer>,
    ) -> Result<wgpu::SubmissionIndex, Error> {
        self.drain_completed();
        let mut buffers = Vec::new();
        let mut records = Vec::new();
        let mut commands = commands
            .into_iter()
            .flat_map(|command| std::iter::once(command.first).chain(command.following));
        if let Err(error) = self.native.device.ensure_active() {
            self.cancel_records(commands.filter_map(|command| command.recorded));
            return Err(error);
        }
        while let Some(command) = commands.next() {
            buffers.extend(command.preceding);
            if let Some(key) = command.recorded {
                records.push(key);
                let tracker = self.native.device.checked(|| {
                    let mut tracker = self.device.create_command_encoder(&Default::default());
                    self.native.track(key, &mut tracker)?;
                    Ok(tracker.finish())
                });
                let tracker = match tracker {
                    Ok(tracker) => tracker,
                    Err(error) => {
                        self.cancel_records(
                            records
                                .into_iter()
                                .chain(commands.filter_map(|c| c.recorded)),
                        );
                        return Err(error);
                    }
                };
                buffers.push(tracker);
            }
            buffers.push(command.raw);
        }
        let Some(_) = self.native.life.next_serial() else {
            self.cancel_records(records);
            return Err(Error::InvalidState("提交序号已耗尽"));
        };
        if let Err(error) = self.native.validate_submission(&records) {
            self.cancel_records(records);
            return Err(error);
        }
        let attempt = self
            .native
            .life
            .prepare_batch(&records)
            .expect("整批已校验且独占所有者");
        let serial = attempt.serial();
        let result = self.native.device.submit(buffers);
        let index = match result {
            Ok(index) => index,
            Err(error) => {
                attempt.quarantine();
                return Err(error);
            }
        };
        assert_eq!(attempt.accept(), serial);
        let completed = self.completed_tx.clone();
        self.queue.on_submitted_work_done(move || {
            let _ = completed.send(serial);
        });
        #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
        self.streamline.submitted(&records, serial);
        Ok(index)
    }
    /// 最后一次实际提交的单队列序号；录制不会推进此值。
    pub const fn last_submission_serial(&self) -> u64 {
        self.native.life.submitted_serial()
    }
    /// 收取实际完成通知并返回水位；不等待 GPU。
    pub fn completed_serial(&mut self) -> u64 {
        self.drain_completed();
        self.native.life.completed_serial()
    }
}

impl Drop for WgpuContext {
    fn drop(&mut self) {
        let idle = self.native.device.wait_idle();
        if !crate::context::execution_ended(idle) {
            log::error!("无法确认互操作队列已结束，保留设备与在途资源：{idle:?}");
            self.native.retain_failed_shutdown();
            #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
            {
                std::mem::forget(self.streamline.runtime.take());
                std::mem::forget(self.streamline.process.take());
            }
            std::mem::forget(self.pipeline_cache.take());
            return;
        }
        #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
        {
            let released = self
                .streamline
                .runtime
                .as_mut()
                .map_or(Ok(()), |runtime| runtime.close());
            let released = released.and_then(|()| {
                if self.streamline.process.is_some() {
                    crate::streamline::shutdown()
                } else {
                    Ok(())
                }
            });
            if let Err(error) = released {
                log::error!("SDK 退役失败，保留设备和 SDK 所有者：{error}");
                self.native.retain_failed_shutdown();
                std::mem::forget(self.streamline.runtime.take());
                std::mem::forget(self.streamline.process.take());
                std::mem::forget(self.pipeline_cache.take());
                return;
            }
            drop(self.streamline.runtime.take());
            drop(self.streamline.process.take());
        }
        if let Some((cache, mut disk)) = self.pipeline_cache.take() {
            // 持有缓存的 2D owner 可以在上次快照后建立新管线。
            if idle.is_ok() && self.native.device.health.is_active() {
                disk.used();
                disk.flush(|| cache.get_data());
            }
        }
    }
}

/// 按顺序执行的混合命令；须交给 WgpuContext::submit 或 cancel，票据本身不拥有 native 资源。
#[must_use = "须通过 WgpuContext::submit 提交或 WgpuContext::cancel 取消命令"]
pub struct CommandBuffer {
    first: Segment,
    following: Vec<Segment>,
}

/// 已录制但未提交的 BLAS 批；Context 保有资源，命令须显式提交或取消。
#[must_use = "须提交或取消 commands，标识在 producer 取消后失效"]
pub struct PreparedBlasBatch {
    /// 与输入同序的新 BLAS 标识；GPU 构建尚未完成。
    pub blases: Vec<TriangleBlas>,
    /// 包含构建依赖和临时资源有效期的唯一待提交命令。
    pub commands: CommandBuffer,
    /// 仅 CPU 校验、上传、分配与录制耗时；无提交等待或设备 timestamp。
    pub timing: crate::BuildTiming,
}

/// 已录制但未提交的 TLAS；须在依赖的 BLAS 构建之后提交。
#[must_use = "须提交或取消 commands，标识在 producer 取消后失效"]
pub struct PreparedTlas {
    /// 可立即建立描述符的新 TLAS 标识；追踪提交会校验 producer 顺序。
    pub acceleration: AccelerationStructure,
    /// TLAS 构建及后续追踪 barrier；临时资源由 Context 保活。
    pub commands: CommandBuffer,
}

struct Segment {
    raw: wgpu::CommandBuffer,
    recorded: Option<RecordId>,
    preceding: Vec<wgpu::CommandBuffer>,
}

impl CommandBuffer {
    pub(crate) fn recorded(raw: wgpu::CommandBuffer, key: RecordId) -> Self {
        let mut result = Self::new(raw);
        result.first.recorded = Some(key);
        result
    }
    fn new(raw: wgpu::CommandBuffer) -> Self {
        Self {
            first: Segment {
                raw,
                recorded: None,
                preceding: Vec::new(),
            },
            following: Vec::new(),
        }
    }

    /// 在整组首段前按迭代顺序执行 wgpu 准备命令；资源转换仍由同一次 submit 排序。
    /// 重复调用追加到已有准备命令之后，不提交队列或等待设备。
    pub fn with_preceding(
        mut self,
        commands: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) -> Self {
        self.first.preceding.extend(commands);
        self
    }

    /// 在本组之后执行 next，保留两组每段的准备命令、tracker 和 native 资源所有权。
    ///
    /// 合并为平坦序列，不提交或等待；整组仍须交给同设备的 [`WgpuContext::submit`]。
    pub fn then(mut self, next: Self) -> Self {
        self.following.push(next.first);
        self.following.extend(next.following);
        self
    }
}

impl From<wgpu::CommandBuffer> for CommandBuffer {
    fn from(raw: wgpu::CommandBuffer) -> Self {
        Self::new(raw)
    }
}

pub(crate) fn record(
    owner: &Device,
    label: &str,
    f: impl FnOnce(vk::CommandBuffer) -> Result<(), Error>,
) -> Result<wgpu::CommandBuffer, Error> {
    owner.checked(|| {
        let device = owner._owner.as_ref().expect("checked已确认wgpu宿主");
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        unsafe {
            encoder.as_hal_mut::<Vulkan, _, _>(|hal| {
                let hal = hal.ok_or(Error::InvalidState("无法借入 Vulkan command encoder"))?;
                f(vk::CommandBuffer::from_raw(hal.raw_handle().as_raw()))
            })?;
        }
        Ok(encoder.finish())
    })
}

pub(crate) fn wait(owner: &Device, submission: Option<wgpu::SubmissionIndex>) -> Result<(), Error> {
    let device = owner
        ._owner
        .as_ref()
        .ok_or(Error::Unsupported("等待需要 wgpu 宿主"))?;
    if let Err(error) = device.poll(wgpu::PollType::Wait {
        submission_index: submission,
        timeout: None,
    }) {
        if !crate::context::execution_ended(owner.wait_idle()) {
            owner.quarantine();
        }
        return Err(error.into());
    }
    Ok(())
}

/// 受控 Slang 构建生成的单个光追阶段；接口契约由应用维护。
#[derive(Clone, Copy)]
pub struct RayShader<'a> {
    /// 直接交给 Vulkan 的 SPIR-V 字节。
    pub spirv: &'a [u8],
    /// SPIR-V 内的原始入口名称，不包含额外编码转换。
    pub entry: &'a CStr,
}

/// 已初始化、未映射的外部 wgpu storage handle；Clone 采用 wgpu 语义，不复制显存。
#[derive(Clone)]
pub struct SharedBuffer {
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) device: u64,
}

impl SharedBuffer {
    /// 不得在 native 命令录制或在途期间显式 `destroy`。
    pub const fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// 完整 buffer 的字节数，也作为 native storage 描述符范围。
    pub fn len(&self) -> u64 {
        self.buffer.size()
    }

    /// 始终为 false；构造入口拒绝零长度 buffer。
    pub const fn is_empty(&self) -> bool {
        false
    }
}

/// 单层 RGBA32F storage image；设备归属随资源保留。
#[derive(Clone)]
pub struct SharedTexture {
    pub(crate) texture: wgpu::Texture,
    pub(crate) device: u64,
}

/// 同设备浮点采样图像的外部 wgpu handles；Clone 不复制显存。
#[derive(Clone)]
pub struct SampledTexture {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) device: u64,
}

impl SampledTexture {
    /// 借给同设备的 wgpu 写入／采样；不得显式 destroy 在途图像。
    pub const fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
}

impl SharedTexture {
    /// 不得在 native 命令录制或在途期间显式 `destroy` 图像。
    pub const fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
}

/// raygen / miss / closest-hit 管线，可追加单层 callable；trace 深度为 1。
/// 三个基础 SBT 区各一条记录，callable 区按传入顺序索引。
/// SBT 位于 DEVICE_LOCAL；唯一 Context 管理描述符、程序、SBT 和 backing memory。
/// TLAS 和其余资源对 raygen、miss、hit 与已配置的 callable 可见。
/// 图像限单 mip/layer/sample 的 RGBA32F，须前置完整初始化并以 GENERAL 交接；shader 接口由应用维护。
/// <https://docs.vulkan.org/spec/latest/chapters/raytracing.html>
///
/// raygen 可迭代执行 TraceRay 与 CallShader；callable 可使用不发起 shader 调用的 RayQuery。
/// callable 和 hit/miss 不得执行 TraceRay 或 CallShader，保持调用图无环。
/// callable payload 的类型及访问范围由受控 shader 保证，本接口不反射或验证。
/// ray recursion depth 固定为1；保留 Vulkan 默认 stack，不按 callable 的 RayQuery 增加 shader 调用深度。
/// OpTraceRayKHR 不允许 Callable 阶段，RayQuery 不进入 hit/miss shader。
/// <https://github.khronos.org/SPIRV-Registry/extensions/KHR/SPV_KHR_ray_tracing.html>
/// <https://docs.vulkan.org/spec/latest/chapters/raytracing.html#ray-tracing-pipeline-stack>
/// 返回值只是 Copy ID；调用方将当前版本交给 Context::collect，已录制及在途版本由 Context 自动保护。
#[derive(Clone, Copy)]
pub struct RayTracingPipeline {
    device: u64,
    native: crate::native::RayTracingPipeline,
}

impl RayTracingPipeline {
    /// `shaders` 顺序为 raygen、miss、closest-hit；所有图像须来自 context 的设备。
    pub fn new(
        context: &mut WgpuContext,
        scene: AccelerationStructure,
        shaders: [RayShader<'_>; 3],
        textures: &[SharedTexture],
    ) -> Result<Self, Error> {
        Self::create(context, scene, shaders, &[], textures, &[], &[])
    }

    /// binding 0 为 TLAS，后续 binding 为 storage buffer。
    pub fn new_buffers(
        context: &mut WgpuContext,
        scene: AccelerationStructure,
        shaders: [RayShader<'_>; 3],
        buffers: &[SharedBuffer],
    ) -> Result<Self, Error> {
        Self::create(context, scene, shaders, &[], &[], buffers, &[])
    }

    /// storage buffers 后依次绑定 sampled images，末尾绑定一个归一化 linear-clamp sampler。
    ///
    /// binding0 为 TLAS，buffers 从1开始；只读图像使用 SHADER_READ_ONLY_OPTIMAL，
    /// 由提交 tracker 交接，资源对 raygen、miss、hit 和 callable 可见。
    /// # Errors
    /// 设备归属、原生资源或管线创建失败时保留来源。
    pub fn new_sampled_buffers(
        context: &mut WgpuContext,
        scene: AccelerationStructure,
        shaders: [RayShader<'_>; 3],
        buffers: &[SharedBuffer],
        sampled: &[SampledTexture],
    ) -> Result<Self, Error> {
        Self::new_sampled_buffers_with_callables(context, scene, shaders, &[], buffers, sampled)
    }

    /// 在 [`Self::new_sampled_buffers`] 的相同绑定后端加入可使用 RayQuery 的单层 callable。
    ///
    /// `shaders` 顺序为 raygen、miss、closest-hit；`callables[i]` 对应 Slang
    /// [`CallShader(i, payload)`]，索引须小于 `callables.len()`，payload 类型须一致。
    /// TLAS、storage buffers、sampled images 和 sampler 同时对 raygen/callable 可见。
    /// callable 可使用 RayQuery，不得执行 TraceRay 或 CallShader；hit/miss 也不得发起这两种调用。
    /// 空列表等价于原三阶段入口；不支持 any-hit、intersection 或 shader record 数据。
    /// SPIR-V/入口名称只在创建时借用；program/SBT 唯一保存于 Context，返回版本 ID。
    /// # Errors
    /// 数量或 SBT 范围溢出、设备归属错误或 native 创建失败时返回带来源的错误。
    ///
    /// [`CallShader(i, payload)`]: https://docs.shader-slang.org/en/latest/external/core-module-reference/global-decls/callshader-04.html
    pub fn new_sampled_buffers_with_callables(
        context: &mut WgpuContext,
        scene: AccelerationStructure,
        shaders: [RayShader<'_>; 3],
        callables: &[RayShader<'_>],
        buffers: &[SharedBuffer],
        sampled: &[SampledTexture],
    ) -> Result<Self, Error> {
        Self::create(context, scene, shaders, callables, &[], buffers, sampled)
    }

    fn create(
        context: &mut WgpuContext,
        scene: AccelerationStructure,
        shaders: [RayShader<'_>; 3],
        callables: &[RayShader<'_>],
        textures: &[SharedTexture],
        buffers: &[SharedBuffer],
        sampled: &[SampledTexture],
    ) -> Result<Self, Error> {
        use crate::native::{Binding, Bindings, Sampler};
        if textures.is_empty() && buffers.is_empty() && sampled.is_empty() {
            return Err(Error::InvalidArgument("光追管线缺少输出资源"));
        }
        let native = context.native();
        let mut resources = vec![Binding::Acceleration(scene)];
        resources.extend(textures.iter().cloned().map(Binding::SharedTexture));
        resources.extend(buffers.iter().cloned().map(Binding::SharedBuffer));
        resources.extend(sampled.iter().cloned().map(Binding::SampledTexture));
        if !sampled.is_empty() {
            resources.push(Binding::Sampler(Sampler::new(native, true)?));
        }
        let bindings = Bindings::new(native, &resources)?;
        let pipeline =
            crate::native::RayTracingPipeline::new(native, bindings, shaders, None, callables)?;
        Ok(Self {
            device: context.native.owner(),
            native: pipeline,
        })
    }

    /// 录制同设备光追命令；维度非零、总 invocation 数须在设备范围内。
    /// 返回命令须通过同一 context 提交/取消；Context 的记录与完成水位维护资源生命周期。
    pub fn encode(
        &self,
        context: &mut WgpuContext,
        size: [u32; 3],
    ) -> Result<CommandBuffer, Error> {
        if self.device != context.native.owner() {
            return Err(Error::InvalidArgument("光追管线设备无效"));
        }
        let mut commands = crate::native::Commands::new(context.native_ref())?;
        commands.trace_3d(self.native, size)?;
        let recorded = commands.finish(context.native())?;
        context.record_native(recorded)
    }
}

impl From<RayTracingPipeline> for crate::native::Resource {
    fn from(value: RayTracingPipeline) -> Self {
        value.native.into()
    }
}
