//! NVIDIA NRD 4.18 的 Vulkan/SPIR-V 集成。会话拥有算法历史和描述符，宿主拥有输入、输出及提交。
use std::{ffi::c_void, ptr::NonNull};

/// 每个 NRD 像素的双样本暂存字节数。与 Slang 的两个六组 RGBA16F SignalLane 匹配。
pub const SIGNAL_BYTES_PER_PIXEL: u64 = 2 * 6 * 8;

/// 官方编码输入与解码输出的 Slang compute 入口 `pack`、`resolve`。
pub const ADAPTERS: [&[u8]; 2] = [
    include_bytes!(concat!(env!("OUT_DIR"), "/nrd_pack.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/nrd_resolve.spv")),
];

/// 传入 Vulkan 的单层 GENERAL 图像，资源必须具有 sampled 与 storage 用途。
#[repr(C)]
#[derive(Copy, Clone)]
pub struct Image {
    /// 同设备 VkImage。
    pub handle: u64,
    /// 原生 VkFormat。
    pub format: u32,
    reserved: u32,
}
impl Image {
    /// 包装与视图等大的图像。资源和设备的有效期在创建会话时借入。
    pub const fn new(handle: u64, format: u32) -> Self {
        Self {
            handle,
            format,
            reserved: 0,
        }
    }
}
/// 去抖动矩阵使用列主序。运动的 XY 为输入像素 previous−current，Z 为前后观察深度之差，抖动以采样像素计。
#[repr(C)]
pub struct Frame {
    /// 当前投影。
    pub projection: [f32; 16],
    /// 历史投影。
    pub previous_projection: [f32; 16],
    /// 当前观察变换。
    pub view: [f32; 16],
    /// 同一世界原点下的历史观察变换。
    pub previous_view: [f32; 16],
    /// 当前采样偏移。
    pub jitter: [f32; 2],
    /// 上一帧采样偏移。
    pub previous_jitter: [f32; 2],
    /// 表面指向太阳的单位方向。
    pub sun: [f32; 3],
    /// 当前帧间隔，毫秒。
    pub milliseconds: f32,
    /// 连续渲染帧号。
    pub index: u32,
    /// 非零时清空所有历史。
    pub reset: u32,
}
unsafe extern "C" {
    fn engine_nrd_create(
        handles: *const u64,
        family: u32,
        queue_index: u32,
        w: u16,
        h: u16,
        images: *const Image,
        out: *mut *mut c_void,
    ) -> i32;
    fn engine_nrd_record(session: *mut c_void, cmd: u64, frame: *const Frame) -> i32;
    fn engine_nrd_destroy(session: *mut c_void);
}
/// 原生 SDK 适配失败阶段。
#[derive(Debug, thiserror::Error)]
#[error("NRD Vulkan/SPIR-V 阶段 {0} 失败")]
pub struct Error(i32);
/// 一次分辨率与资源世代独占的 REBLUR SH、SIGMA 会话。
pub struct Runtime(NonNull<c_void>);
// SAFETY: 独占移动，录制与销毁均在宿主渲染会话的串行提交边界执行。
unsafe impl Send for Runtime {}
impl Runtime {
    /// 创建同设备同队列的视图会话。输入依次为 MV、打包法线、viewZ、四张 SH、半影、反遮挡因子、四张输出 SH 和阴影输出，再附高光独立的 MV、打包法线、viewZ 与反遮挡因子。
    ///
    /// # Safety
    /// instance/physical/device 必须属于同一 Vulkan 1.3 宿主。逻辑设备需开启 push descriptor、
    /// descriptorBindingPartiallyBound、shaderFloat16、shaderInt16 与 shaderStorageImageWriteWithoutFormat。
    /// 图像须为同尺寸、同设备的非别名单采样单层图像，宿主应提供 sampled 和 storage 用途及 GENERAL 布局。
    /// 2.5D 运动使用 RGBA16F，法线使用 A2B10G10R10_UNORM，SH 使用 RGBA16F，其余标量使用 R32F。
    /// 宿主应将图像和设备保活到会话析构及 GPU 完成。每次录制前上一帧必须完成。
    pub unsafe fn new(
        handles: [u64; 3],
        family: u32,
        queue_index: u32,
        extent: [u32; 2],
        images: [Image; 18],
    ) -> Result<Self, Error> {
        let [w, h] = extent.map(u16::try_from);
        let (w, h) = (w.map_err(|_| Error(-1))?, h.map_err(|_| Error(-1))?);
        if handles.contains(&0)
            || w == 0
            || h == 0
            || images.iter().any(|i| i.handle == 0)
            || queue_index == u32::MAX
        {
            return Err(Error(-1));
        }
        let mut session = std::ptr::null_mut();
        // SAFETY: 所有 C 输入在调用期间有效，句柄借用由上层所有者维持。
        let code = unsafe {
            engine_nrd_create(
                handles.as_ptr(),
                family,
                queue_index,
                w,
                h,
                images.as_ptr(),
                &mut session,
            )
        };
        if code != 0 {
            return Err(Error(code));
        }
        Ok(Self(NonNull::new(session).expect("SDK 成功创建会话")))
    }
    /// 分别以进入材质和界面反射的几何导引记录 REBLUR SH，并以主导引记录 SIGMA，恢复 GENERAL 布局。取消录制后下次必须清历史。
    ///
    /// # Safety
    /// command 须属于创建队列且正在 render pass 外独占录制。宿主保证此会话及图像存活到执行完成。
    pub unsafe fn record(&mut self, command: u64, frame: &Frame) -> Result<(), Error> {
        if command == 0 || !frame.milliseconds.is_finite() || frame.milliseconds <= 0. {
            return Err(Error(-1));
        }
        // SAFETY: 调用者持有录制与完成边界。
        let code = unsafe { engine_nrd_record(self.0.as_ptr(), command, frame) };
        if code == 0 { Ok(()) } else { Err(Error(code)) }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: 视图独占此会话，创建契约保证已完成使用它的录制。
        unsafe { engine_nrd_destroy(self.0.as_ptr()) };
    }
}
