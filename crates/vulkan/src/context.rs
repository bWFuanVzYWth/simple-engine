use crate::vk;
use std::sync::atomic::AtomicU64;

pub(crate) struct Instance {
    pub(crate) raw: ash::Instance,
    pub(crate) _owner: Option<wgpu::Instance>,
}

pub(crate) struct PhysicalDevice {
    pub(crate) raw: vk::PhysicalDevice,
    pub(crate) instance: Instance,
}

impl PhysicalDevice {
    pub(crate) fn properties(&self) -> vk::PhysicalDeviceProperties {
        unsafe { self.instance.raw.get_physical_device_properties(self.raw) }
    }

    pub(crate) fn queue_families(&self) -> Vec<vk::QueueFamilyProperties> {
        unsafe {
            self.instance
                .raw
                .get_physical_device_queue_family_properties(self.raw)
        }
    }

    pub(crate) fn supports_extensions(
        &self,
        required: &[&std::ffi::CStr],
    ) -> Result<bool, crate::Error> {
        let extensions = unsafe {
            self.instance
                .raw
                .enumerate_device_extension_properties(self.raw)?
        };
        Ok(required.iter().all(|name| {
            extensions
                .iter()
                .any(|extension| extension.extension_name_as_c_str() == Ok(*name))
        }))
    }
}

/// 借入宿主 Vulkan device；wgpu 适配持原生 handle，raw 适配遵守宿主有效期契约。
/// 只由 Context 唯一保存，不销毁宿主 instance/device。
pub(crate) struct Device {
    pub(crate) health: crate::health::Health,
    pub(crate) raw: ash::Device,
    pub(crate) physical: PhysicalDevice,
    pub(crate) buffer_bytes: AtomicU64,
    pub(crate) ray_pipeline: bool,
    pub(crate) pipeline_robustness: bool,
    pub(crate) subgroup_size_control: bool,
    pub(crate) _owner: Option<wgpu::Device>,
    pub(crate) wgpu_queue: Option<wgpu::Queue>,
    pub(crate) _loader: Option<ash::Entry>,
    pub(crate) queue: vk::Queue,
    pub(crate) queue_family: u32,
    pub(crate) position_fetch: bool,
    pub(crate) opacity_micromap: bool,
    pub(crate) invocation_reorder: bool,
}

impl Device {
    pub(crate) fn ensure_active(&self) -> Result<(), crate::Error> {
        self.health.check()
    }

    pub(crate) fn checked<T>(
        &self,
        f: impl FnOnce() -> Result<T, crate::Error>,
    ) -> Result<T, crate::Error> {
        self.ensure_active()?;
        let device = self
            ._owner
            .as_ref()
            .ok_or(crate::Error::Unsupported("操作需要 wgpu 宿主"))?;
        crate::health::checked(device, &self.health, f)
    }

    /// 提交错误可能发生在队列已接收部分工作之后；返回 Err 前确认可安全清理。
    pub(crate) fn submit(
        &self,
        commands: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) -> Result<wgpu::SubmissionIndex, crate::Error> {
        let queue = self
            .wgpu_queue
            .as_ref()
            .ok_or(crate::Error::Unsupported("提交需要 wgpu 宿主"))?;
        match self.checked(|| Ok(queue.submit(commands))) {
            Ok(index) => Ok(index),
            Err(error) => {
                if let Some(device) = &self._owner {
                    // wait 自身在不能确认全队列结束时 abort，错误时不释放在途候选。
                    let _ = crate::interop::wait(device, None);
                }
                Err(error)
            }
        }
    }

    pub(crate) fn format_properties(&self, format: vk::Format) -> vk::FormatProperties {
        unsafe {
            self.physical
                .instance
                .raw
                .get_physical_device_format_properties(self.physical.raw, format)
        }
    }
}
