//! 三角形窗口示例的失败边界；可重试的临时取帧状态不进入错误枚举。

use simple_engine::{gpu_log, vulkan, wgpu};

#[derive(Debug, thiserror::Error)]
/// 参数、窗口、呈现或设备失败；绘制错误令该 renderer 停止提交。
pub enum Error {
    /// Vulkan/wgpu 共享设备或 native shader 创建失败。
    #[error(transparent)]
    Vulkan(#[from] vulkan::Error),
    /// 平台事件循环失败。
    #[error(transparent)]
    EventLoop(#[from] winit::error::EventLoopError),
    /// 窗口系统操作失败。
    #[error(transparent)]
    Window(#[from] winit::error::OsError),
    /// --frames 的整数解析失败。
    #[error(transparent)]
    FrameCount(#[from] std::num::ParseIntError),
    /// 无法为窗口创建 Surface。
    #[error("创建窗口表面失败：{0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    /// 无法找到兼容窗口的适配器。
    #[error("无法获取兼容窗口的适配器：{0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    /// 初始化 error scope 捕获的 wgpu 错误。
    #[error("GPU 验证失败：{0}")]
    Validation(#[from] wgpu::Error),
    /// 挂起或退出等待失败。
    #[error("等待 GPU 失败：{0}")]
    Poll(#[from] wgpu::PollError),
    /// Surface 无法提供默认呈现配置。
    #[error("窗口没有可用的呈现配置")]
    NoSurfaceConfiguration,
    /// 表面已丢失，必须重建 renderer。
    #[error("窗口表面已丢失，请重新创建渲染器")]
    SurfaceLost,
    /// 取帧时 Surface 报告验证失败。
    #[error("获取窗口纹理时发生验证错误")]
    SurfaceValidation,
    /// 首次锁存的异步设备错误，后续检查不清除。
    #[error("GPU 已停止：{0}")]
    GpuStopped(String),
    /// 全局日志安装或累计错误检查失败。
    #[error(transparent)]
    GpuLog(#[from] gpu_log::Error),
    /// 此前绘制已经失败，禁止继续提交。
    #[error("渲染器已停止，请重新创建")]
    RendererStopped,
    /// 缺少与 shader 线性输出匹配的 sRGB 格式／色彩空间。
    #[error("窗口表面没有支持 sRGB 输出编码的格式组合")]
    NoSrgbSurfaceFormat,
    /// --frames 缺少参数，或帧数不是正数。
    #[error("--frames 需要正整数帧数")]
    MissingFrameCount,
    /// 命令行包含未知参数或不符合示例用法。
    #[error("用法：hello_triangle [--frames N]")]
    Usage,
}
