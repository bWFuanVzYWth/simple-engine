use crate::vk;

/// native 互操作失败；底层 Vulkan、wgpu 与 IO 错误保留来源。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 无法加载宿主使用的 Vulkan loader。
    #[error("加载 Vulkan 失败：{0}")]
    Loader(#[from] ash::LoadingError),
    /// 无法加载 wgpu HAL 使用的 Vulkan loader。
    #[error("加载 wgpu Vulkan 失败：{0}")]
    WgpuLoader(#[from] ash_wgpu::LoadingError),
    /// wgpu 宿主实例的 Vulkan 调用失败。
    #[error("wgpu Vulkan 调用失败：{0}")]
    WgpuVulkan(#[from] ash_wgpu::vk::Result),
    /// 初始化 wgpu Vulkan 实例失败。
    #[error("创建 wgpu Vulkan 实例失败：{0}")]
    WgpuInstance(#[from] wgpu::hal::InstanceError),
    /// CPU 资源描述或上传暂存无法申请容量。
    #[error("Vulkan 资源暂存分配失败")]
    Allocation(#[from] std::collections::TryReserveError),
    /// 无法为地形显存列创建驱动回收线程。
    #[error("创建 Vulkan 显存回收线程失败")]
    RetirementWorker(#[source] std::io::Error),
    /// Vulkan 驱动调用返回失败状态。
    #[error("Vulkan 调用失败：{0}")]
    Vulkan(#[from] vk::Result),
    /// 字节流不是可解码的 SPIR-V；不表示已验证 shader 语义。
    #[error("解码 SPIR-V 失败：{0}")]
    ShaderParse(#[from] std::io::Error),
    /// Vulkan 适配器枚举中不存在指定索引。
    #[error("没有索引为 {0} 的 Vulkan 适配器")]
    AdapterIndex(usize),
    /// 将 HAL 设备接入 wgpu 时失败。
    #[error("创建 wgpu 设备失败：{0}")]
    WgpuDevice(#[from] wgpu::RequestDeviceError),
    /// 创建底层 Vulkan HAL 设备失败。
    #[error("创建 wgpu HAL 设备失败：{0}")]
    WgpuHal(#[from] wgpu::hal::DeviceError),
    /// 等待或轮询提交失败。
    #[error("等待 wgpu 提交失败：{0}")]
    WgpuPoll(#[from] wgpu::PollError),
    /// 资源或命令验证失败；原始 wgpu 来源保留，所有者转为仅可清理。
    #[error("wgpu 操作失败：{0}")]
    WgpuOperation(#[source] wgpu::Error),
    /// 首次收取的设备丢失通知；之后的调用返回 DeviceFault。
    #[error("wgpu 设备丢失（{reason:?}）：{message}")]
    DeviceLost {
        /// wgpu 给出的丢失原因。
        reason: wgpu::DeviceLostReason,
        /// 原始诊断文本。
        message: String,
    },
    /// 提交所有者已失效，禁止继续构建、发布或提交；首个错误由触发操作返回。
    #[error("设备所有者已失效：{0:?}")]
    DeviceFault(crate::DeviceFailure),
    /// 访问共享资源的映射范围失败。
    #[error("访问 wgpu 映射范围失败：{0}")]
    MapRange(#[from] wgpu::MapRangeError),
    /// 设备或后端缺少请求的能力，携带具体原因。
    #[error("{0}")]
    Unsupported(&'static str),
    /// 输入参数违反该接口的大小、归属或布局约束。
    #[error("{0}")]
    InvalidArgument(&'static str),
    /// 资源或提交所有者已失效，携带具体原因。
    #[error("{0}")]
    InvalidState(&'static str),
}

impl From<crate::lifecycle::Error> for Error {
    fn from(error: crate::lifecycle::Error) -> Self {
        use crate::lifecycle::Error as LifeError;
        match error {
            LifeError::ForeignOwner => Self::InvalidArgument("对象属于其他设备"),
            LifeError::InvalidKey => Self::InvalidState("对象版本已失效"),
            LifeError::Retired => Self::InvalidState("资源已退休，不能创建新的使用"),
            LifeError::CancelledProducer => Self::InvalidState("AS 构建已取消"),
            LifeError::InvalidState => Self::InvalidState("记录阶段不允许此操作"),
            LifeError::SerialExhausted => Self::InvalidState("提交序号已耗尽"),
            LifeError::Full => Self::InvalidState("对象槽已耗尽"),
            LifeError::Allocation(error) => Self::Allocation(error),
        }
    }
}
