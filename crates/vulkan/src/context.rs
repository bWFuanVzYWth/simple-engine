use crate::vk;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
    pub(crate) uncertain_use: AtomicBool,
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
    pub(crate) pipeline_statistics: bool,
}

impl Device {
    pub(crate) fn quarantine(&self) {
        self.uncertain_use.store(true, Ordering::Relaxed);
        self.health.fail(crate::DeviceFailure::Internal);
    }

    pub(crate) fn is_quarantined(&self) -> bool {
        self.uncertain_use.load(Ordering::Relaxed)
    }

    /// 同一队列的宿主访问由 Context 串行化；DEVICE_LOST 也终止 pending 使用。
    pub(crate) fn wait_idle(&self) -> Result<(), vk::Result> {
        let result = unsafe { self.raw.queue_wait_idle(self.queue) };
        if let Err(error) = result {
            self.health.fail(if error == vk::Result::ERROR_DEVICE_LOST {
                crate::DeviceFailure::Lost
            } else {
                crate::DeviceFailure::Internal
            });
        }
        result
    }

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

    /// 提交错误可能发生在队列已接收部分工作之后；返回 Err 前确认完成，否则保留在途资源。
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
                if !execution_ended(self.wait_idle()) {
                    self.quarantine();
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

/// Vulkan 的 Lost Device 规则仅使 DEVICE_LOST 等价于完成，不适用于 OOM 等其他等待失败。
/// https://docs.vulkan.org/spec/latest/chapters/devsandqueues.html#devsandqueues-lost-device
pub(crate) fn execution_ended(result: Result<(), vk::Result>) -> bool {
    matches!(result, Ok(()) | Err(vk::Result::ERROR_DEVICE_LOST))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ash::vk::Handle as _;
    use std::cell::Cell;

    unsafe extern "system" fn wait_result(queue: vk::Queue) -> vk::Result {
        vk::Result::from_raw(queue.as_raw() as i32)
    }

    fn device(result: vk::Result) -> Device {
        // 测试函数表只调用 queue_wait_idle；假句柄不交给驱动，其余入口不得执行。
        let raw = unsafe {
            ash::Device::load_with(
                |name| {
                    if name == c"vkQueueWaitIdle" {
                        wait_result as *const () as _
                    } else {
                        std::ptr::null()
                    }
                },
                vk::Device::null(),
            )
        };
        let instance =
            unsafe { ash::Instance::load_with(|_| std::ptr::null(), vk::Instance::null()) };
        Device {
            health: crate::health::Health::new(),
            uncertain_use: AtomicBool::new(false),
            raw,
            physical: PhysicalDevice {
                raw: vk::PhysicalDevice::null(),
                instance: Instance {
                    raw: instance,
                    _owner: None,
                },
            },
            buffer_bytes: AtomicU64::new(0),
            ray_pipeline: false,
            pipeline_robustness: false,
            subgroup_size_control: false,
            _owner: None,
            wgpu_queue: None,
            _loader: None,
            queue: vk::Queue::from_raw(result.as_raw() as u64),
            queue_family: 0,
            position_fetch: false,
            opacity_micromap: false,
            invocation_reorder: false,
            pipeline_statistics: false,
        }
    }

    struct Allocation<'a>(&'a Cell<(u32, u32)>);
    impl crate::owner::Destroy for Allocation<'_> {
        fn destroy(&mut self, _: &Device) {
            let (destroyed, dropped) = self.0.get();
            self.0.set((destroyed + 1, dropped));
        }
    }
    impl Drop for Allocation<'_> {
        fn drop(&mut self) {
            let (destroyed, dropped) = self.0.get();
            self.0.set((destroyed, dropped + 1));
        }
    }

    #[test]
    fn device_loss_retires_guarded_allocation_once() {
        let device = device(vk::Result::ERROR_DEVICE_LOST);
        let count = Cell::new((0, 0));
        let allocation = crate::owner::Guard::new(&device, Allocation(&count));
        assert!(execution_ended(device.wait_idle()));
        assert!(matches!(
            device.ensure_active(),
            Err(crate::Error::DeviceFault(crate::DeviceFailure::Lost))
        ));
        drop(allocation);
        assert_eq!(count.get(), (1, 1));
    }

    #[test]
    fn unconfirmed_completion_retains_guarded_allocation() {
        let device = device(vk::Result::ERROR_OUT_OF_HOST_MEMORY);
        let count = Cell::new((0, 0));
        let allocation = crate::owner::Guard::new(&device, Allocation(&count));
        assert!(!execution_ended(device.wait_idle()));
        device.quarantine();
        drop(allocation);
        assert_eq!(count.get(), (0, 0));
    }

    #[test]
    fn retirement_requires_completion_or_device_loss() {
        assert!(execution_ended(Ok(())));
        assert!(execution_ended(Err(vk::Result::ERROR_DEVICE_LOST)));
        for failure in [
            vk::Result::ERROR_OUT_OF_HOST_MEMORY,
            vk::Result::ERROR_OUT_OF_DEVICE_MEMORY,
            vk::Result::ERROR_UNKNOWN,
            vk::Result::TIMEOUT,
        ] {
            assert!(!execution_ended(Err(failure)));
        }
    }
}
