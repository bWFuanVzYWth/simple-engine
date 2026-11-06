//! 场景无关的 point ReSTIR PT Enhanced 资源布局与阶段录制。
//!
//! 本 profile 对应 Falcor 9.0 `759aad03` 的 point、Hybrid、paired K=3、
//! 一轮 spatial；不启用 reservoir compression、duplication map、decoupled shading 或 area reservoir。
//! 初始候选、重放、shift/MIS 的 shader 由上层受控程序注入，本 crate 不解释材质或场景身份。
//! reservoir/reconnection/precomputed 保持未压缩 80/44/20B scalar ABI。
//! 参考调度使用Compact；融合可消除reconnection与队列，保留相同采样和跨像素paired缓存。
//!
//! GPU 资源的真实所有者是唯一 Vulkan Context；本 crate 保存资源标识与唯一帧进度，
//! 只录制命令，不在 record 中提交或等待。宿主成功提交后 commit，失败时先取消命令再 reject。
//! 当前/历史reservoir角色固定：initial/temporal写A，spatial从A写B；融合merge直接输出贡献。
//! 一轮 spatial 与帧末逻辑交换抵消，下一帧仍以 A 接新样本、B 保存历史，不维护第二份可见状态。
//! [`Layout::with_color_estimates`]可选分配同角色的两份12N字节RGB估计，用于Control Variates。
//! 颜色估计属于像素，不是赢家路径身份；[`FrameState`]集中维护路径与颜色的已接受状态。
//! 宿主用[`ColorOutput`]明确声明本帧产出，成功提交后共同接受；取消保留两者，
//! 失效请求可以分别选择路径或颜色。算法开关及实际生产者由宿主提供，不据此隐式全域重置。

#![forbid(unsafe_code)]

mod frame;
mod layout;
mod settings;

#[cfg(feature = "vulkan")]
pub mod gpu;

pub use frame::{ColorOutput, FrameParams, FrameState, FrameToken};
pub use layout::{
    Dispatch, Fusion, HitInfo, Layout, Memory, NeighborMask, PathReservoir, PrecomputedShift,
    Reconnection, SpatialFusion, Stage, WorkItem,
};
pub use settings::Settings;

/// 输入、容量或帧事务违反本 profile 的支持范围。
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    /// 工作项保留 NV 的 12+12+8 编码，单轴只支持 1..=4096。
    #[error("ReSTIR PT 视图尺寸须属于 1..=4096")]
    Extent,
    /// 字节、索引、任务或帧序号计算溢出。
    #[error("ReSTIR PT 容量或计数溢出")]
    Overflow,
    /// temporal整体融合与方向拆分不能同时启用。
    #[error("ReSTIR PT temporal融合模式互斥")]
    TemporalFusion,
    /// 本 profile 的 1..=64 surface bounce 上限。
    #[error("ReSTIR PT surface bounce 上限须属于 1..=64")]
    Bounces,
    /// temporal 历史上限必须非零。
    #[error("ReSTIR PT 历史上限必须大于零")]
    HistoryLength,
    /// shift 阈值须有限，roughness 属于 0..=1，distance 为正，sigma 非负。
    #[error("ReSTIR PT shift 阈值无效")]
    ShiftThreshold,
    /// 仅允许本 profile 的三邻居六个有向工作项。
    #[error("ReSTIR PT 工作项坐标或方向越界")]
    WorkItem,
    /// 已录制帧尚未由宿主提交确认或取消。
    #[error("ReSTIR PT 已有待确认帧")]
    PendingFrame,
    /// token 来自其他 owner、旧事务或已确认事务。
    #[error("ReSTIR PT 帧票据已失效")]
    StaleFrame,
    /// 声明产出颜色时，实际工作集必须配置颜色估计缓冲。
    #[error("ReSTIR PT 颜色输出需要颜色估计工作集")]
    ColorStorage,
}
