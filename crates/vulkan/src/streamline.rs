//! Streamline 2.14.1 / DLSS RR 的可选 Windows x86-64 接入，所有档位使用 preset F。
//!
//! `dlss_rr` feature 从显式 `STREAMLINE_SDK` 构建桥接，按本 crate 的锁校验 SDK；
//! 初始化仅加载桥接模块同目录的 runtime。安装见仓库 `scripts/install_streamline.ps1`。
//! SDK 类型不跨 FFI，宿主提供应用身份并拥有设备、命令、图像与提交完成证明。
//!
//! 进程只允许一个 [`Runtime`]。early interposer 宿主先 [`bootstrap`]，再经同目录
//! `sl.interposer.dll` 创建 Vulkan；手动接入由 [`Runtime::new`] 初始化。bootstrap 成功
//! 后 proxy 保留到进程退出；[`shutdown`] 退役 plugin 后仍可销毁宿主设备。
//! Runtime 不提交或等待；configure、释放与 shutdown 前必须完成此前 evaluate。
//! SDK 异常使整个命令不可提交；普通失败允许宿主回退当前 noisy 图像。
//!
//! 宿主须实际启用 SDK 所需 extensions/features，至少包括 `synchronization2`、
//! `shaderStorageImageWriteWithoutFormat`；early interposer 另需 Vulkan 1.3 与 `privateData`。
//! 支持查询不等于已启用证明。相机、历史重置、输入内容和回退策略由应用维护。
//!
//! [`crate::interop::WgpuContext`] 提供安全的 configure_streamline/record_streamline 入口，
//! 将 Runtime、图像根、未提交记录与真实完成序号放在同一 owner。early 模式仅允许一个
//! Context 设备；其析构等待 GPU、释放 feature、关闭 plugin，再销毁设备。

use std::ffi::{CStr, c_char, c_void};

/// 宿主的 Streamline 身份；字符串仅在本次初始化调用期间借用。
#[derive(Clone, Copy, Debug)]
pub struct Application<'a> {
    /// 自定义引擎版本；非空，由 SDK 诊断使用。
    pub engine_version: &'a CStr,
    /// 宿主项目 UUID；非空，按 NVIDIA 项目身份要求提供。
    pub project_id: &'a CStr,
}

#[repr(C)]
struct RawApplication {
    engine_version: *const c_char,
    project_id: *const c_char,
}

impl Application<'_> {
    fn raw(self) -> RawApplication {
        RawApplication {
            engine_version: self.engine_version.as_ptr(),
            project_id: self.project_id.as_ptr(),
        }
    }
}

/// Bridge 或 SDK 错误，保留原始返回值与同步日志。
#[derive(Debug, thiserror::Error)]
#[error("Streamline ({code}): {detail}")]
pub struct Error {
    /// 原始 SDK 返回值；负值也可能来自 bridge。
    pub code: i32,
    /// 本线程 bridge 诊断的独立副本。
    pub detail: String,
}

/// RR 输入比例档位；最终尺寸以 SDK optimal settings 为准。
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::enum_variant_names, reason = "保留 DLSS 标准档位名称")]
pub enum Quality {
    /// 原生分辨率重建。
    Dlaa = 0,
    /// 质量档。
    Quality = 1,
    /// 平衡档。
    Balanced = 2,
    /// 性能档，输入宽高通常为输出的一半。
    #[default]
    Performance = 3,
    /// 超级性能档。
    UltraPerformance = 4,
}

impl TryFrom<u32> for Quality {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Dlaa),
            1 => Ok(Self::Quality),
            2 => Ok(Self::Balanced),
            3 => Ok(Self::Performance),
            4 => Ok(Self::UltraPerformance),
            _ => Err(Error {
                code: -1,
                detail: format!("无效 RR 档位：{value}"),
            }),
        }
    }
}

/// SDK evaluate 失败；异常后宿主必须丢弃整个命令缓冲。
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct EvaluationFailure {
    /// 原始 bridge 或 SDK 错误。
    #[from]
    pub source: Error,
}

impl EvaluationFailure {
    /// SDK 抛出异常，命令缓冲可能仅部分录制，禁止提交。
    pub const fn unsafe_recording(&self) -> bool {
        self.source.code == -100
    }
}

/// 借用同一活动宿主设备及 graphics/compute queue 的原始句柄。
#[repr(C)]
pub struct Init {
    /// `VkInstance`。
    pub instance: u64,
    /// 实际选用的 `VkPhysicalDevice`。
    pub physical_device: u64,
    /// `VkDevice`。
    pub device: u64,
    /// 同时支持 graphics/compute 的 queue family。
    pub queue_family: u32,
    /// 上述 family 内的 queue index。
    pub queue_index: u32,
}

#[repr(C)]
#[derive(Default)]
struct Size {
    width: u32,
    height: u32,
}

/// 单 mip/layer、单样本 Vulkan image；GENERAL，usage 含 STORAGE|SAMPLED。
///
/// 只描述借用，不保活对象。全零值只可用于未提交给 SDK 的 specular distance。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Resource {
    /// `VkImage`。
    pub image: u64,
    /// 对应 `VkImageView`。
    pub view: u64,
    /// backing `VkDeviceMemory`；可由 SDK 按图像句柄处理。
    pub memory: u64,
    /// 图像宽度。
    pub width: u32,
    /// 图像高度。
    pub height: u32,
    /// `VkFormat` 原始值。
    pub format: u32,
    /// `VkImageLayout` 原始值，当前仅接受 GENERAL。
    pub layout: u32,
    /// 实际创建 usage 的位集。
    pub usage: u32,
    /// 必须为零。
    pub reserved: u32,
}

/// 一次 RR 录制；无 jitter 行主序矩阵乘行向量。
///
/// 图像顺序：noisy/depth/motion/normal+roughness/diffuse/specular/output/distance/specularMotion。
/// output 使用输出尺寸，其余使用 SDK optimal 输入尺寸。颜色、albedo、normal+roughness
/// 为 RGBA16F，depth/distance 为 R32F，motion 为 RG16F；depth 是正 view-Z。
/// 运动为左上原点、输入像素 previous-current，不含 jitter；显式 specular motion 必需，
/// distance 可全零，当前 bridge 永不同时 tag distance 与 specular motion。
#[repr(C)]
#[derive(Clone)]
pub struct Frame {
    /// 正在录制的 `VkCommandBuffer`。
    pub command: u64,
    /// 宿主帧编号；SDK frame token 仍由进程统一获取。
    pub index: u32,
    /// 非零重置历史；仅接受 0 或 1。
    pub reset: u32,
    /// 负的实际主射线偏移，单位为输入像素。
    pub jitter: [f32; 2],
    /// 正的近裁剪距离。
    pub near_plane: f32,
    /// 大于近面的远裁剪距离。
    pub far_plane: f32,
    /// 垂直 FOV，弧度，属于 `(0, PI)`。
    pub fov: f32,
    /// 正的相机宽高比。
    pub aspect: f32,
    /// 与矩阵使用同一世界锚点的相机位置。
    pub position: [f32; 3],
    /// 相机世界上向量。
    pub up: [f32; 3],
    /// 相机世界右向量。
    pub right: [f32; 3],
    /// 相机世界前向量。
    pub forward: [f32; 3],
    /// 世界到 view 的无 jitter 变换。
    pub world_to_view: [f32; 16],
    /// view 到世界变换。
    pub view_to_world: [f32; 16],
    /// view 到 clip 的无 jitter 投影。
    pub view_to_clip: [f32; 16],
    /// clip 到 view 的逆投影。
    pub clip_to_view: [f32; 16],
    /// 当前 clip 到前帧 clip 的变换。
    pub clip_to_previous: [f32; 16],
    /// 前帧 clip 到当前 clip 的逆变换。
    pub previous_to_clip: [f32; 16],
    /// 按本类型约定排序的图像描述；对象保留至 GPU 完成。
    pub images: [Resource; 9],
}

unsafe extern "C" {
    fn se_sl_abi_version() -> u32;
    fn se_sl_interposer_path() -> *const u16;
    fn se_sl_bootstrap(application: *const RawApplication) -> i32;
    fn se_sl_shutdown() -> i32;
    fn se_sl_create(
        init: *const Init,
        application: *const RawApplication,
        output: *mut *mut c_void,
    ) -> i32;
    fn se_sl_configure(
        context: *mut c_void,
        width: u32,
        height: u32,
        quality: u32,
        size: *mut Size,
    ) -> i32;
    fn se_sl_evaluate(context: *mut c_void, frame: *const Frame) -> i32;
    fn se_sl_destroy(context: *mut c_void) -> i32;
    fn se_sl_last_error() -> *const c_char;
}

fn check(code: i32) -> Result<(), Error> {
    if code == 0 {
        return Ok(());
    }
    // Bridge 诊断为同线程有效的 NUL 结尾存储，在下一次调用前复制。
    let detail = unsafe { CStr::from_ptr(se_sl_last_error()) }
        .to_string_lossy()
        .into_owned();
    Err(Error { code, detail })
}

/// 已成功 bootstrap 的 interposer 绝对路径；未初始化时返回 None。
///
/// C++ owner 在进程退出前保留字符串，复制后不借用 SDK 存储。
pub fn interposer_path() -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let pointer = unsafe { se_sl_interposer_path() };
    if pointer.is_null() {
        return None;
    }
    let mut len = 0;
    // C++ 返回稳定、NUL 结尾的 GetModuleFileNameW 路径，最大32767个UTF-16单元。
    while len < 32768 && unsafe { *pointer.add(len) } != 0 {
        len += 1;
    }
    let path = unsafe { std::slice::from_raw_parts(pointer, len) };
    Some(std::ffi::OsString::from_wide(path).into())
}

/// 在线程启动及宿主创建 Vulkan instance/device 前初始化 early interposer。
///
/// 随后须从桥接模块同目录 interposer 加载 Vulkan。相同身份重复调用保持原初始化；
/// shutdown 后不重新初始化。此处不检测宿主是否已创建 Vulkan。
/// early interposer 拒绝 layer-only 实例扩展，故在创建实例前对未设置的 VK_LAYER_VALIDATE_SYNC 写入1，
/// 保留 Khronos 层的同步验证；此设置不自动启用 validation layer。
/// # Errors
/// runtime 不可加载、身份不符或 SDK 初始化失败时保留原始来源。
pub fn bootstrap(application: Application<'_>) -> Result<(), Error> {
    check(unsafe { se_sl_bootstrap(&application.raw()) })
}

/// 所有 RR owner 与 GPU 工作退役后，在宿主销毁 VkDevice 前关闭 plugin；重复关闭保持成功。
///
/// early interposer 模块保留到进程退出，以便宿主继续销毁设备和 instance。
/// WgpuContext 在设备析构前自动关闭；此接口也为尚未完成 Context 构造的 bootstrap 收尾。
/// # Errors
/// 仍有活动 Runtime 或 SDK 关闭失败时返回原始错误。
pub fn shutdown() -> Result<(), Error> {
    check(unsafe { se_sl_shutdown() })
}

/// 单 renderer 的 SDK owner；独占录制，不拥有宿主 Vulkan 设备。
///
/// 每次 configure 与析构前须完成此前 GPU evaluate。释放 SDK 资源失败时终止进程，
/// 避免继续销毁无法证明安全的借用设备。early interposer 的进程状态由 shutdown 退役。
pub struct Runtime {
    handle: *mut c_void,
}

// bridge 全局互斥，方法独占 &mut self；宿主仍须独占 Vulkan command/queue。
unsafe impl Send for Runtime {}

impl Runtime {
    /// 借入活动宿主设备；无 early bootstrap 时使用 manual hooking。
    /// # Safety
    /// init 来自同一活动设备，宿主保活至本 owner 释放；所需 SDK extensions/features 已启用。
    /// interposed 生产路径必须在 Vulkan 创建前 bootstrap；身份须与 bootstrap 一致。
    /// 宿主须保证每次 configure/drop 前完成此前 GPU evaluate，且设备不被并发销毁。
    /// # Errors
    /// ABI 不符、重复 owner、设备/身份不符或 SDK 初始化失败时返回原始来源。
    pub unsafe fn new(init: Init, application: Application<'_>) -> Result<Self, Error> {
        if unsafe { se_sl_abi_version() } != 3 {
            return Err(Error {
                code: -1,
                detail: "bridge ABI 不匹配".into(),
            });
        }
        let mut handle = std::ptr::null_mut();
        // POD 和身份在同步调用期间有效，bridge 复制身份并只保留借用设备句柄。
        check(unsafe { se_sl_create(&init, &application.raw(), &mut handle) })?;
        if handle.is_null() {
            return Err(Error {
                code: -1,
                detail: "bridge 返回空 owner".into(),
            });
        }
        Ok(Self { handle })
    }

    /// 配置最终尺寸与档位，返回 SDK optimal 输入尺寸。
    ///
    /// 遵守 new 的安全契约：调用前完成此前 GPU 工作。配置释放既有 feature 资源。
    /// # Errors
    /// 尺寸非法、SDK 查询或配置失败时返回原始来源。
    pub fn configure(&mut self, output: [u32; 2], quality: Quality) -> Result<[u32; 2], Error> {
        let mut size = Size::default();
        check(unsafe {
            se_sl_configure(self.handle, output[0], output[1], quality as u32, &mut size)
        })?;
        Ok([size.width, size.height])
    }

    /// 在宿主命令缓冲中录制 RR，不提交、不等待。
    /// # Safety
    /// command 正在同设备上录制，宿主独占 command/queue。图像满足 Frame 契约，在先前写入后
    /// 已建立可见性，并保留至实际 GPU 完成。所有录制及在途工作完成前不得 configure/drop；
    /// unsafe_recording 为 true 后必须丢弃整个 command，禁止提交。
    /// # Errors
    /// 输入、SDK 或同步错误日志导致失败；异常和可回退失败由 EvaluationFailure 区分。
    pub unsafe fn evaluate(&mut self, frame: &Frame) -> Result<(), EvaluationFailure> {
        check(unsafe { se_sl_evaluate(self.handle, frame) }).map_err(Into::into)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // new/evaluate 的调用方保证已完成所有 GPU 工作；bridge 只释放本 owner 的资源。
        if let Err(error) = check(unsafe { se_sl_destroy(self.handle) }) {
            eprintln!("{error}");
            std::process::abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_layout_matches_cpp() {
        assert_eq!(size_of::<Init>(), 32);
        assert_eq!(size_of::<Resource>(), 48);
        assert_eq!(size_of::<Frame>(), 904);
        assert_eq!(std::mem::offset_of!(Frame, world_to_view), 88);
        assert_eq!(std::mem::offset_of!(Frame, images), 472);
        assert_eq!(unsafe { se_sl_abi_version() }, 3);
    }

    #[test]
    fn quality_rejects_unknown_ffi_value() {
        let error = Quality::try_from(5).unwrap_err();
        assert_eq!(error.code, -1);
        assert!(error.detail.contains('5'));
    }
}

/// 在 wgpu command 中录制 SDK 失败；资源和命令的错误保留原始来源。
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// Vulkan、HAL 或提交所有权失败。
    #[error(transparent)]
    Vulkan(#[from] crate::Error),
    /// SDK 失败；异常时整个返回命令不可提交。
    #[error(transparent)]
    Evaluation(#[from] EvaluationFailure),
    /// 初始化或配置失败。
    #[error(transparent)]
    Sdk(#[from] Error),
}

/// 已保活九张图像的 SDK 命令；仍须由同一 WgpuContext 提交或取消。
pub struct Recording {
    /// SDK 命令及其 native 资源根。
    pub commands: crate::interop::CommandBuffer,
    /// 普通求值失败仍返回命令；调用者可追加 fallback，异常不会返回此对象。
    pub evaluation: Result<(), EvaluationFailure>,
}

#[cfg(test)]
pub(crate) mod interop_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ERRORS: AtomicUsize = AtomicUsize::new(0);
    struct Logger;
    impl log::Log for Logger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Warn
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                if record.level() == log::Level::Error {
                    ERRORS.fetch_add(1, Ordering::Relaxed);
                }
                eprintln!("{}: {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }

    pub(crate) fn install_logger() -> Result<(), Box<dyn std::error::Error>> {
        log::set_logger(&Logger).map_err(|_| "测试logger已被占用")?;
        log::set_max_level(log::LevelFilter::Warn);
        Ok(())
    }

    pub(crate) fn setup() -> Result<
        (super::Application<'static>, crate::interop::WgpuContext),
        Box<dyn std::error::Error>,
    > {
        install_logger()?;
        let application = super::Application {
            engine_version: c"simple_engine wgpu fixture",
            project_id: c"7d79286f-4a68-405d-a8ed-4acba40da614",
        };
        let executable = std::env::current_exe()?;
        let directory = executable.parent().ok_or("测试程序目录缺失")?;
        let profile = directory
            .ancestors()
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name == "debug" || name == "release")
            })
            .ok_or("测试程序不在 Cargo profile 目录内")?;
        if profile != directory {
            for name in [
                "sl.interposer.dll",
                "sl.common.dll",
                "sl.dlss_d.dll",
                "nvngx_dlssd.dll",
            ] {
                std::fs::copy(profile.join(name), directory.join(name))?;
            }
        }
        super::bootstrap(application)?;
        let context = pollster::block_on(crate::interop::WgpuContext::with_capabilities(
            0,
            crate::native::Capabilities {
                ray_tracing: true,
                ..Default::default()
            },
            wgpu::Features::empty(),
        ))?;
        Ok((application, context))
    }

    #[test]
    #[ignore = "独立进程：验证 early interposer、实际设备功能链、RR提交回读和析构"]
    fn wgpu_rr_device_and_optimal_extent() -> Result<(), Box<dyn std::error::Error>> {
        let (application, mut context) = setup()?;
        assert!(context.streamline_capable());
        let output = [1920, 1080];
        let mut previous = output;
        for quality in [
            super::Quality::Dlaa,
            super::Quality::Quality,
            super::Quality::Balanced,
            super::Quality::Performance,
            super::Quality::UltraPerformance,
        ] {
            let input = context.configure_streamline(application, output, quality)?;
            println!("quality={quality:?} output={output:?} optimal={input:?}");
            if quality == super::Quality::Dlaa {
                assert_eq!(input, output);
            } else {
                for ((value, bound), prior) in input.into_iter().zip(output).zip(previous) {
                    assert!(value > 0 && value < bound);
                    assert!(value < prior);
                }
            }
            previous = input;
        }
        assert_eq!(
            context.configure_streamline(application, output, super::Quality::Dlaa)?,
            output
        );
        let images = images(&mut context, output)?;
        let canceled = context.record_streamline(images, frame())?;
        context.cancel(canceled.commands)?;
        canceled.evaluation?;
        let mut retry = frame();
        retry.reset = 0; // 取消后的安全适配必须自行重建并强制 reset。
        let recorded = context.record_streamline(images, retry)?;
        context.submit([recorded.commands])?;
        crate::interop::wait(context.device(), None)?;
        context.completed_serial();
        recorded.evaluation?;
        let output = images[6].download(context.native())?;
        let mut nonzero = 0;
        for pixel in output.as_chunks::<8>().0 {
            for channel in pixel[..6].as_chunks::<2>().0 {
                let half = u16::from_le_bytes([channel[0], channel[1]]);
                assert_ne!(half & 0x7c00, 0x7c00, "SDK须完整覆盖NaN输出哨兵");
                nonzero += usize::from(half & 0x7fff != 0);
            }
        }
        assert_eq!(nonzero, 1920 * 1080 * 3);
        context.release_streamline()?;
        drop(context);
        super::shutdown()?;
        check_validation();
        Ok(())
    }

    pub(crate) fn check_validation() {
        assert_eq!(
            ERRORS.load(Ordering::Relaxed),
            0,
            "validation error不可忽略"
        );
    }

    pub(crate) fn images(
        context: &mut crate::interop::WgpuContext,
        size: [u32; 2],
    ) -> Result<[crate::native::Image; 9], crate::Error> {
        use crate::native::{Image, ImageFormat};
        let formats = [
            ImageFormat::Rgba16Float,
            ImageFormat::R32Float,
            ImageFormat::Rg16Float,
            ImageFormat::Rgba16Float,
            ImageFormat::Rgba16Float,
            ImageFormat::Rgba16Float,
            ImageFormat::Rgba16Float,
            ImageFormat::R32Float,
            ImageFormat::Rg16Float,
        ];
        let mut images = Vec::with_capacity(9);
        for (index, format) in formats.into_iter().enumerate() {
            let image = Image::new(context.native(), [size[0], size[1], 1], format)?;
            let words: &[u16] = match index {
                0 => &[0x3400, 0x34cd, 0x3666, 0x3c00],
                3 => &[0, 0, 0xbc00, 0x3c00],
                4 => &[0x3800; 4],
                5 => &[0x291f, 0x291f, 0x291f, 0x3c00],
                6 => &[0x7e00, 0x7e00, 0x7e00, 0],
                _ => &[0, 0],
            };
            let pixel = if index == 1 {
                10f32.to_le_bytes().to_vec()
            } else {
                words.iter().flat_map(|word| word.to_le_bytes()).collect()
            };
            image.upload(
                context.native(),
                &pixel.repeat(size[0] as usize * size[1] as usize),
            )?;
            images.push(image);
        }
        Ok(images.try_into().expect("固定九个测试图像"))
    }

    pub(crate) fn frame() -> super::Frame {
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        let near = 0.1;
        let far = 1000.;
        let fov: f32 = 1.2;
        let aspect = 16. / 9.;
        let sx = 1. / ((fov * 0.5).tan() * aspect);
        let sy = 1. / (fov * 0.5).tan();
        let a = far / (far - near);
        let b = -near * a;
        super::Frame {
            command: 0,
            index: 0,
            reset: 1,
            jitter: [0.; 2],
            near_plane: near,
            far_plane: far,
            fov,
            aspect,
            position: [0.; 3],
            up: [0., 1., 0.],
            right: [1., 0., 0.],
            forward: [0., 0., 1.],
            world_to_view: identity,
            view_to_world: identity,
            view_to_clip: [sx, 0., 0., 0., 0., sy, 0., 0., 0., 0., a, 1., 0., 0., b, 0.],
            clip_to_view: [
                1. / sx,
                0.,
                0.,
                0.,
                0.,
                1. / sy,
                0.,
                0.,
                0.,
                0.,
                0.,
                1. / b,
                0.,
                0.,
                1.,
                -a / b,
            ],
            clip_to_previous: identity,
            previous_to_clip: identity,
            images: [super::Resource::default(); 9],
        }
    }
}
