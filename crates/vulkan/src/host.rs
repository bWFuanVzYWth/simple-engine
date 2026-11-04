//! Vulkan 1.4 宿主实例；旧 ash 只负责 wgpu HAL 交接。

use crate::Error;
use ash_wgpu::{ext, khr, vk};
use std::ffi::{CStr, c_void};

/// 实例和调试 messenger 的唯一所有者，由 HAL 的退出回调收回。
struct Owner {
    raw: ash_wgpu::Instance,
    debug: Option<(ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    _entry: ash_wgpu::Entry,
}

impl Drop for Owner {
    fn drop(&mut self) {
        unsafe {
            if let Some((loader, messenger)) = self.debug.take() {
                loader.destroy_debug_utils_messenger(messenger, None);
            }
            self.raw.destroy_instance(None);
        }
    }
}

unsafe extern "system" fn debug_message(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut c_void,
) -> vk::Bool32 {
    if data.is_null() || std::thread::panicking() {
        return vk::FALSE;
    }
    // Vulkan 保证回调参数在调用期间有效；日志实现的 panic 不得跨越 FFI。
    let _ = std::panic::catch_unwind(|| {
        let data = unsafe { &*data };
        let level = match severity {
            vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE => log::Level::Trace,
            vk::DebugUtilsMessageSeverityFlagsEXT::INFO => log::Level::Debug,
            vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => log::Level::Warn,
            _ => log::Level::Error,
        };
        let name = unsafe { data.message_id_name_as_c_str() }
            .unwrap_or(c"")
            .to_string_lossy();
        let message = unsafe { data.message_as_c_str() }
            .unwrap_or(c"")
            .to_string_lossy();
        log::log!(target: "wgpu_hal::vulkan", level,
            "{kind:?} [{name} (0x{:x})] {message}", data.message_id_number);
    });
    vk::FALSE
}

fn debug_info() -> vk::DebugUtilsMessengerCreateInfoEXT<'static> {
    let mut severity = vk::DebugUtilsMessageSeverityFlagsEXT::ERROR;
    for (level, flag) in [
        (
            log::LevelFilter::Debug,
            vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE,
        ),
        (
            log::LevelFilter::Info,
            vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
        ),
        (
            log::LevelFilter::Warn,
            vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
        ),
    ] {
        if log::max_level() >= level {
            severity |= flag;
        }
    }
    vk::DebugUtilsMessengerCreateInfoEXT::default()
        .message_severity(severity)
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .pfn_user_callback(Some(debug_message))
}

#[cfg(target_os = "android")]
fn android_sdk() -> u32 {
    android_system_properties::AndroidSystemProperties::new()
        .get("ro.build.version.sdk")
        .and_then(|version| version.parse().ok())
        .unwrap_or(0)
}

#[cfg(not(target_os = "android"))]
const fn android_sdk() -> u32 {
    0
}

/// HAL 默认入口将应用版本封顶 1.3；此处显式创建 1.4 并保留其平台扩展及诊断策略。
pub(crate) fn instance() -> Result<wgpu::Instance, Error> {
    let flags = wgpu::InstanceFlags::from_env_or_default();
    #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
    let interposed = crate::streamline::interposer_path().is_some();
    #[cfg(not(all(feature = "dlss_rr", target_os = "windows")))]
    let interposed = false;
    if interposed && flags.contains(wgpu::InstanceFlags::GPU_BASED_VALIDATION) {
        return Err(Error::Unsupported(
            "early Streamline interposer 不支持 GPU assisted validation 的 layer 扩展",
        ));
    }
    #[cfg(target_env = "ohos")]
    let entry = unsafe { ash_wgpu::Entry::load_from("libvulkan.so")? };
    #[cfg(not(target_env = "ohos"))]
    let entry = {
        #[cfg(all(feature = "dlss_rr", target_os = "windows"))]
        if let Some(path) = crate::streamline::interposer_path() {
            unsafe { ash_wgpu::Entry::load_from(path)? }
        } else {
            unsafe { ash_wgpu::Entry::load()? }
        }
        #[cfg(not(all(feature = "dlss_rr", target_os = "windows")))]
        unsafe {
            ash_wgpu::Entry::load()?
        }
    };
    let api_version = vk::make_api_version(0, 1, 4, 0);
    if unsafe { entry.try_enumerate_instance_version()? }.unwrap_or(vk::API_VERSION_1_0)
        < api_version
    {
        return Err(Error::Unsupported("Vulkan loader 需要 1.4"));
    }
    let mut extensions =
        wgpu::hal::vulkan::Instance::desired_extensions(&entry, api_version, flags)?;
    let layers = unsafe { entry.enumerate_instance_layer_properties()? };
    let has_layer = |name: &CStr| {
        layers
            .iter()
            .any(|layer| layer.layer_name_as_c_str() == Ok(name))
    };
    let validation = c"VK_LAYER_KHRONOS_validation";
    let validation_available = has_layer(validation);
    let validation_features = validation_available
        && unsafe { entry.enumerate_instance_extension_properties(Some(validation))? }
            .iter()
            .any(|extension| {
                extension.extension_name_as_c_str() == Ok(ext::validation_features::NAME)
            });
    let gpu_validation =
        flags.contains(wgpu::InstanceFlags::GPU_BASED_VALIDATION) && validation_features;
    let request_validation = flags.contains(wgpu::InstanceFlags::VALIDATION) || gpu_validation;
    let enabled_layers: Vec<_> = if request_validation && validation_available {
        vec![validation.as_ptr()]
    } else {
        if request_validation {
            log::warn!("未安装 VK_LAYER_KHRONOS_validation");
        }
        Vec::new()
    };
    // early interposer 拒绝 layer-only 扩展；bootstrap 已通过 VK_LAYER_VALIDATE_SYNC 开启同一检查。
    let enable_validation_features =
        !interposed && !enabled_layers.is_empty() && validation_features;
    if enable_validation_features {
        extensions.push(ext::validation_features::NAME);
    }
    let extension_names: Vec<_> = extensions.iter().map(|name| name.as_ptr()).collect();
    let app = vk::ApplicationInfo::default()
        .application_name(c"simple_engine")
        .engine_name(c"simple_engine")
        .api_version(api_version);
    let mut create = vk::InstanceCreateInfo::default()
        .application_info(&app)
        .enabled_layer_names(&enabled_layers)
        .enabled_extension_names(&extension_names);
    if extensions.contains(&khr::portability_enumeration::NAME) {
        create = create.flags(vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR);
    }
    let mut debug = debug_info();
    let enable_debug = extensions.contains(&ext::debug_utils::NAME);
    if enable_debug {
        create = create.push_next(&mut debug);
    }
    let mut checks = vec![vk::ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION];
    if gpu_validation {
        checks.extend([
            vk::ValidationFeatureEnableEXT::GPU_ASSISTED,
            vk::ValidationFeatureEnableEXT::GPU_ASSISTED_RESERVE_BINDING_SLOT,
        ]);
    }
    let mut validation_info =
        vk::ValidationFeaturesEXT::default().enabled_validation_features(&checks);
    if enable_validation_features {
        create = create.push_next(&mut validation_info);
    }
    let raw = unsafe { entry.create_instance(&create, None)? };
    let mut owner = Owner {
        raw,
        debug: None,
        _entry: entry,
    };
    if enable_debug {
        let loader = ext::debug_utils::Instance::new(&owner._entry, &owner.raw);
        let messenger = unsafe { loader.create_debug_utils_messenger(&debug_info(), None)? };
        owner.debug = Some((loader, messenger));
    }
    // HAL 的 DebugUtilsCreateInfo 无公开构造器；唯一 owner 经退出回调保活及释放。
    // extensions 是 desired_extensions 的超集；HAL 和实例使用完全相同的版本与 flags。
    let hal = unsafe {
        wgpu::hal::vulkan::Instance::from_raw(
            owner._entry.clone(),
            owner.raw.clone(),
            api_version,
            android_sdk(),
            None,
            extensions,
            flags,
            Default::default(),
            has_layer(c"VK_LAYER_NV_optimus"),
            Some(Box::new(move || drop(owner))),
        )?
    };
    Ok(unsafe { wgpu::Instance::from_hal::<wgpu::hal::api::Vulkan>(hal) })
}
