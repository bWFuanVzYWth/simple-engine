//! SDK 的设备协商、HAL 适配和提交所有权；仅 Windows dlss_rr 编译。

use crate::{
    Error, PhysicalDevice,
    handles::RecordId,
    interop::{CommandBuffer, WgpuContext},
    native::{Commands, Image},
    recovery::{Recovery, RecoveryCause},
    streamline::{Application, Frame, Init, Quality, RecordError, Recording, Resource, Runtime},
    vk,
};
use ash::vk::{Handle, TaggedStructure};
use std::sync::atomic::{AtomicBool, Ordering};

static EARLY_DEVICE: AtomicBool = AtomicBool::new(false);

/// early interposer 只允许一个设备 owner；feature 退役不放弃进程生命周期责任。
pub(crate) struct ProcessOwner;

impl ProcessOwner {
    pub fn claim() -> Result<Option<Self>, Error> {
        if crate::streamline::interposer_path().is_none() {
            return Ok(None);
        }
        EARLY_DEVICE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                Error::InvalidState("early Streamline 仅允许一个 WgpuContext 设备 owner")
            })?;
        Ok(Some(Self))
    }
}

impl Drop for ProcessOwner {
    fn drop(&mut self) {
        if let Err(error) = crate::streamline::shutdown() {
            eprintln!("无法在 VkDevice 退役前关闭 Streamline：{error}");
            std::process::abort();
        }
        EARLY_DEVICE.store(false, Ordering::Release);
    }
}

pub(crate) struct Session {
    pub capable: bool,
    pub runtime: Option<Runtime>,
    pub pending: Vec<RecordId>,
    pub last_serial: u64,
    pub process: Option<ProcessOwner>,
    configuration: Option<([u32; 2], Quality)>,
    recovery: Recovery,
}

impl Session {
    pub fn new(capable: bool, process: Option<ProcessOwner>) -> Self {
        Self {
            capable,
            runtime: None,
            pending: Vec::new(),
            last_serial: 0,
            process,
            configuration: None,
            recovery: Recovery::new(),
        }
    }
    pub fn cancel(&mut self, key: RecordId) {
        if self.pending.contains(&key) {
            self.recovery.invalidate(RecoveryCause::RecordingCancelled);
        }
        self.pending.retain(|&pending| pending != key);
    }
    pub fn submitted(&mut self, keys: &[RecordId], serial: u64) {
        if keys.iter().any(|key| self.pending.contains(key)) {
            self.last_serial = serial;
            self.pending.retain(|key| !keys.contains(key));
        }
    }
}

pub(crate) const EXTENSIONS: [&std::ffi::CStr; 5] = [
    ash::nvx::binary_import::NAME,
    ash::nvx::image_view_handle::NAME,
    ash::khr::push_descriptor::NAME,
    ash::khr::buffer_device_address::NAME,
    ash::khr::synchronization2::NAME,
];

pub(crate) struct DeviceFeatures {
    pub capable: bool,
    pub private_data: vk::PhysicalDevicePrivateDataFeatures<'static>,
    pub indexing: vk::PhysicalDeviceDescriptorIndexingFeatures<'static>,
    pub extended_formats: bool,
}

impl DeviceFeatures {
    pub fn query(physical: &PhysicalDevice) -> Result<Self, Error> {
        let mut private_data = vk::PhysicalDevicePrivateDataFeatures::default();
        let mut core12 = vk::PhysicalDeviceVulkan12Features::default();
        let mut indexing = vk::PhysicalDeviceDescriptorIndexingFeatures::default();
        // 聚合12与其别名不能放同一链，分两次查询；两次都不创建设备或改变状态。
        let mut features = vk::PhysicalDeviceFeatures2::default()
            .push(&mut private_data)
            .push(&mut core12);
        unsafe {
            physical
                .instance
                .raw
                .get_physical_device_features2(physical.raw, &mut features);
        }
        let extended_formats = features.features.shader_storage_image_extended_formats != 0;
        let write_without_format = features.features.shader_storage_image_write_without_format != 0;
        let mut separate = vk::PhysicalDeviceFeatures2::default().push(&mut indexing);
        unsafe {
            physical
                .instance
                .raw
                .get_physical_device_features2(physical.raw, &mut separate);
        }
        let capable = physical.properties().vendor_id == 0x10de
            && physical.supports_extensions(&EXTENSIONS)?
            && private_data.private_data != 0
            && core12.timeline_semaphore != 0
            && core12.descriptor_indexing != 0
            && extended_formats
            && write_without_format;
        // 查询链指向本地栈；出边界前解除指针，创建链由调用者重新挂接。
        private_data.p_next = std::ptr::null_mut();
        indexing.p_next = std::ptr::null_mut();
        Ok(Self {
            capable,
            private_data,
            indexing,
            extended_formats,
        })
    }
}

/// HAL 30 无 core setter；仅用其公开 API 获取字段位置，再以独占来源写入。
/// 返回 HAL 是否已持有 descriptor-indexing 节点，避免追加重复 sType。
pub(crate) fn enable_hal_features(
    features: &mut wgpu::hal::vulkan::PhysicalDeviceFeatures,
    extended: bool,
    write_without_format: bool,
    indexing: Option<vk::PhysicalDeviceDescriptorIndexingFeatures<'static>>,
) -> bool {
    const {
        assert!(
            size_of::<vk::PhysicalDeviceFeatures>()
                == size_of::<ash_wgpu::vk::PhysicalDeviceFeatures>()
        );
        assert!(
            align_of::<vk::PhysicalDeviceFeatures>()
                == align_of::<ash_wgpu::vk::PhysicalDeviceFeatures>()
        );
        assert!(
            size_of::<vk::PhysicalDeviceDescriptorIndexingFeatures>()
                == size_of::<ash_wgpu::vk::PhysicalDeviceDescriptorIndexingFeatures>()
        );
        assert!(
            align_of::<vk::PhysicalDeviceDescriptorIndexingFeatures>()
                == align_of::<ash_wgpu::vk::PhysicalDeviceDescriptorIndexingFeatures>()
        );
    }
    let base = (features as *mut wgpu::hal::vulkan::PhysicalDeviceFeatures).cast::<u8>();
    let bytes = size_of_val(features);
    let (core, descriptor, links) = {
        let info = features.add_to_device_create(ash_wgpu::vk::DeviceCreateInfo::default());
        let core = info
            .p_enabled_features
            .addr()
            .checked_sub(base.addr())
            .filter(|&at| {
                at.checked_add(size_of::<vk::PhysicalDeviceFeatures>())
                    .is_some_and(|end| end <= bytes)
            })
            .expect("HAL core features must belong to the exclusive owner");
        assert!(
            info.p_enabled_features
                .addr()
                .is_multiple_of(align_of::<vk::PhysicalDeviceFeatures>())
        );
        let mut descriptor = None;
        let mut links = Vec::new();
        let mut node = info.p_next.cast::<ash_wgpu::vk::BaseOutStructure>();
        while !node.is_null() {
            // 该链由本次 add_to_device_create 构造，所有节点在独占 HAL owner 内活动。
            let at = node
                .addr()
                .checked_sub(base.addr())
                .filter(|&at| {
                    at.checked_add(size_of::<vk::BaseOutStructure>())
                        .is_some_and(|end| end <= bytes)
                })
                .expect("HAL feature node must belong to the exclusive owner");
            assert!(
                node.addr()
                    .is_multiple_of(align_of::<vk::BaseOutStructure>())
            );
            links.push(at);
            let value = unsafe { &*node };
            if value.s_type
                == ash_wgpu::vk::StructureType::PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_FEATURES
            {
                descriptor = Some(
                    node.addr()
                        .checked_sub(base.addr())
                        .filter(|&at| {
                            at.checked_add(size_of::<
                                    vk::PhysicalDeviceDescriptorIndexingFeatures,
                                >())
                                .is_some_and(|end| end <= bytes)
                        })
                        .expect("HAL indexing features must belong to the exclusive owner"),
                );
                assert!(
                    node.addr()
                        .is_multiple_of(align_of::<vk::PhysicalDeviceDescriptorIndexingFeatures>())
                );
            }
            node = value.p_next;
        }
        (core, descriptor, links)
    };
    // 临时 info/共享派生指针不再使用。base 源于原始独占 &mut，offset 由实际公开字段指针
    // 校验取得，不猜 HAL 私有布局。Vk1.0 C ABI 固定；HAL 随后必须重新构造提交用 info。
    unsafe {
        // add_to_device_create 会写回 pNext；清除探针链，避免 HAL 再次 push_next 形成重复节点环。
        for at in links {
            (*base.add(at).cast::<vk::BaseOutStructure>()).p_next = std::ptr::null_mut();
        }
        let core = &mut *base.add(core).cast::<vk::PhysicalDeviceFeatures>();
        if extended {
            core.shader_storage_image_extended_formats = vk::TRUE;
        }
        if write_without_format {
            core.shader_storage_image_write_without_format = vk::TRUE;
        }
        if let (Some(at), Some(mut enabled)) = (descriptor, indexing) {
            let existing = &mut *base
                .add(at)
                .cast::<vk::PhysicalDeviceDescriptorIndexingFeatures>();
            enabled.p_next = existing.p_next;
            *existing = enabled;
        }
    }
    descriptor.is_some()
}

impl WgpuContext {
    fn wait_streamline_reconfiguration(&mut self) -> Result<(), RecordError> {
        self.drain_completed();
        self.streamline
            .pending
            .retain(|&key| self.native.record_pending(key));
        if !self.streamline.pending.is_empty() {
            return Err(Error::InvalidState("SDK 命令须先提交或取消，再重配").into());
        }
        if self.streamline.last_serial > self.completed_serial() {
            crate::interop::wait(&self.device, None)?;
            self.drain_completed();
        }
        Ok(())
    }

    /// 该设备已实际启用 SDK 所需功能；不表示 SDK 已初始化或求值成功。
    pub const fn streamline_capable(&self) -> bool {
        self.streamline.capable
    }

    /// 初始化或重配 RR，返回 SDK optimal 输入尺寸；重配等待此前 SDK 提交实际完成。
    ///
    /// 必须先提交或取消本 owner 的未提交 SDK 命令。设备能力不可用时不会请求 SDK。
    /// 成功配置后下一次成功 SDK 录制显式 reset；此前输入校验失败不消费该请求。
    /// # Errors
    /// 返回原始 Vulkan/SDK 错误，重复进程 owner 或应用身份不符也会失败。
    pub fn configure_streamline(
        &mut self,
        application: Application<'_>,
        output: [u32; 2],
        quality: Quality,
    ) -> Result<[u32; 2], RecordError> {
        self.native.device.ensure_active()?;
        if !self.streamline.capable {
            return Err(Error::Unsupported("设备未启用 Streamline RR 能力").into());
        }
        self.wait_streamline_reconfiguration()?;
        self.native.device.ensure_active()?;
        if self.streamline.runtime.is_none() {
            let device = &self.native.device;
            let init = Init {
                instance: device.physical.instance.raw.handle().as_raw(),
                physical_device: device.physical.raw.as_raw(),
                device: device.raw.handle().as_raw(),
                queue_family: device.queue_family,
                queue_index: 0,
            };
            // 此 Context 唯一拥有/保活同一设备；能力由创建回调证明。Drop 先 wait 再释放 SDK。
            self.streamline.runtime = Some(unsafe { Runtime::new(init, application)? });
        }
        self.streamline
            .recovery
            .invalidate(RecoveryCause::ConfigurationFailed);
        let input = self
            .streamline
            .runtime
            .as_mut()
            .unwrap()
            .configure(output, quality)?;
        self.native.device.ensure_active()?;
        self.streamline.configuration = Some((output, quality));
        self.streamline.recovery.reconfigured();
        Ok(input)
    }

    /// 退役 SDK feature owner，等待此前 SDK 提交；普通图像仍由 native collect 管理。
    ///
    /// 未提交的 SDK 命令必须先提交或取消。之后可重新 configure_streamline。
    pub fn release_streamline(&mut self) -> Result<(), RecordError> {
        self.wait_streamline_reconfiguration()?;
        drop(self.streamline.runtime.take());
        self.streamline.configuration = None;
        self.streamline.recovery = Recovery::new();
        Ok(())
    }

    /// 最终关闭本 Context 的 SDK，再由调用者释放 Context 和其底层 VkDevice。
    ///
    /// early interposer 要求 plugin 早于 VkDevice 退役；代理模块保留至进程退出。
    /// 此后进程不能重新初始化 SDK。未提交命令须先提交或取消。
    /// # Errors
    /// 保留等待、未提交工作或 SDK 关闭失败的来源。
    pub fn shutdown_streamline(&mut self) -> Result<(), RecordError> {
        self.release_streamline()?;
        crate::streamline::shutdown()?;
        drop(self.streamline.process.take());
        Ok(())
    }

    /// 录制借用九张本 Context 图像的 RR，自动登记资源根和同队列完成边界。
    ///
    /// 使用 Frame 的相机/历史常量，但覆盖 command/images；资源格式和尺寸由 bridge 校验。
    /// 同时最多一个未提交 SDK 记录；返回的 commands 必须提交或取消。取消后首次重录会等待
    /// 此前 SDK 提交完成并重建 feature、强制 reset，恢复被取消的 SDK 私有图像初始化。
    /// 重配后的输入失败保留待发送 reset，重试不重复释放 feature。
    /// 普通 SDK 错误可追加 fallback；异常时返回错误并
    /// 丢弃整个 SDK command。前序包不在本函数所有权内，调用者须一并取消未提交工作。
    /// # Errors
    /// 未初始化、图像归属、录制失败或 SDK 异常时保留原始来源；SDK 求值失败也要求下次重建 feature。
    pub fn record_streamline(
        &mut self,
        images: [Image; 9],
        mut frame: Frame,
    ) -> Result<Recording, RecordError> {
        self.drain_completed();
        self.native.device.ensure_active()?;
        if self.streamline.runtime.is_none() {
            return Err(Error::InvalidState("SDK 尚未配置").into());
        }
        if !self.streamline.pending.is_empty() {
            return Err(Error::InvalidState("前一 SDK 记录须先提交或取消").into());
        }
        if let Some(reason) = self.streamline.recovery.reconfigure_reason() {
            self.wait_streamline_reconfiguration()?;
            self.native.device.ensure_active()?;
            let (output, quality) = self
                .streamline
                .configuration
                .ok_or(Error::InvalidState("SDK 尚未配置"))?;
            self.streamline
                .runtime
                .as_mut()
                .unwrap()
                .configure(output, quality)?;
            self.native.device.ensure_active()?;
            self.streamline.recovery.reconfigured();
            eprintln!("[vulkan] rr.history event={reason:?} action=feature_recreate");
        }
        if self.streamline.recovery.reset_required() {
            frame.reset = 1;
        }
        let resources = images.map(|image| image.raw(&self.native));
        let mut described = [Resource::default(); 9];
        for (slot, raw) in described.iter_mut().zip(resources) {
            let raw = raw?;
            *slot = Resource {
                image: raw.image,
                view: raw.view,
                memory: raw.memory,
                width: raw.extent[0],
                height: raw.extent[1],
                format: raw.format,
                layout: raw.layout,
                usage: raw.usage,
                reserved: 0,
            };
        }
        frame.images = described;
        let mut commands = Commands::new(&self.native)?;
        for image in images {
            commands.access(image.into())?;
        }
        commands.barrier();
        let recorded = commands.finish(&mut self.native)?;
        let key = recorded.key;
        let mut evaluation = Ok(());
        let runtime = self
            .streamline
            .runtime
            .as_mut()
            .ok_or(Error::InvalidState("SDK 尚未配置"))?;
        let result = crate::interop::record(&self.native.device, "Streamline RR", |command| {
            self.native.encode(key, command)?;
            frame.command = command.as_raw();
            // HAL 正在独占同队列 command；所有图像已登记，提交层保活至实际完成。
            evaluation = unsafe { runtime.evaluate(&frame) };
            if evaluation
                .as_ref()
                .is_err_and(|error| error.unsafe_recording())
            {
                return Err(Error::InvalidState(
                    "SDK 异常，丢弃未完成的 command encoder",
                ));
            }
            // SDK 在 GENERAL 中读写；后续 shader 访问需覆盖内部 compute/transfer 写入。
            unsafe {
                self.native.device.raw.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::DependencyFlags::empty(),
                    &[vk::MemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                        .dst_access_mask(
                            vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
                        )],
                    &[],
                    &[],
                );
            }
            Ok(())
        });
        let raw = match result {
            Ok(raw) => raw,
            Err(error) => {
                self.native.cancel(recorded)?;
                self.streamline
                    .recovery
                    .invalidate(RecoveryCause::RecordingFailed);
                if let Err(evaluation) = evaluation {
                    return Err(evaluation.into());
                }
                return Err(error.into());
            }
        };
        if evaluation.is_err() {
            self.streamline
                .recovery
                .invalidate(RecoveryCause::EvaluationFailed);
        } else {
            self.streamline.recovery.recorded();
        }
        self.streamline.pending.push(key);
        Ok(Recording {
            commands: CommandBuffer::recorded(raw, key),
            evaluation,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::native::{Image, ImageFormat};
    use crate::streamline::interop_tests::{check_validation, frame, images, setup};

    #[test]
    #[ignore = "需要真实 Vulkan；不初始化 SDK，验证故障 owner 拒绝新工作"]
    fn failed_owner_rejects_sdk_configuration_and_recording()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::{DeviceFailure, Error, interop::WgpuContext, streamline};

        crate::streamline::interop_tests::install_logger()?;
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let image = Image::new(context.native(), [1, 1, 1], ImageFormat::Rgba16Float)?;
        context.native.device.health.fail(DeviceFailure::Validation);
        let application = streamline::Application {
            engine_version: c"fault test",
            project_id: c"00000000-0000-0000-0000-000000000000",
        };
        assert!(matches!(
            context.configure_streamline(application, [1, 1], streamline::Quality::Dlaa),
            Err(streamline::RecordError::Vulkan(Error::DeviceFault(
                DeviceFailure::Validation
            )))
        ));
        assert!(matches!(
            context.record_streamline([image; 9], frame()),
            Err(streamline::RecordError::Vulkan(Error::DeviceFault(
                DeviceFailure::Validation
            )))
        ));
        assert!(context.streamline.runtime.is_none());
        context.release_streamline()?;
        drop(context);
        check_validation();
        Ok(())
    }

    #[test]
    #[ignore = "独立进程：需要 Streamline RR，验证恢复后输入失败不消费 reset"]
    fn cancelled_record_preserves_reset_across_invalid_image()
    -> Result<(), Box<dyn std::error::Error>> {
        let (application, mut context) = setup()?;
        let size = [1920, 1080];
        context.configure_streamline(application, size, crate::streamline::Quality::Dlaa)?;
        let images = images(&mut context, size)?;
        let stale = Image::new(context.native(), [1, 1, 1], ImageFormat::Rgba16Float)?;
        let roots: Vec<_> = images.iter().copied().map(Into::into).collect();
        context.native().collect(&roots)?;

        let cancelled = context.record_streamline(images, frame())?;
        context.cancel(cancelled.commands)?;
        cancelled.evaluation?;
        let mut invalid_images = images;
        invalid_images[0] = stale;
        let mut retry = frame();
        retry.reset = 0;
        assert!(matches!(
            context.record_streamline(invalid_images, retry.clone()),
            Err(crate::streamline::RecordError::Vulkan(
                crate::Error::InvalidState(_)
            ))
        ));
        assert!(context.streamline.recovery.reset_required());
        assert!(context.streamline.recovery.reconfigure_reason().is_none());

        let recorded = context.record_streamline(images, retry)?;
        assert!(!context.streamline.recovery.reset_required());
        context.submit([recorded.commands])?;
        crate::interop::wait(context.device(), None)?;
        context.completed_serial();
        recorded.evaluation?;
        context.release_streamline()?;
        drop(context);
        crate::streamline::shutdown()?;
        check_validation();
        Ok(())
    }
}
