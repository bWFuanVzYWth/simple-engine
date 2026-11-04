//! 外设无关的有序命令、持续动作与有界批次重放。
//!
//! 外设适配器负责物理绑定与时钟校准，所有输入经 [`InputHub`] 校验后交付。
//! [`Sequence`] 给出 Hub 内总序；接收时间属于会话单调时钟，晚到的采样不重排事件。
//! 同一更新内的按下与释放分别保留，消费者按命令顺序更新自己的唯一业务模型。
//!
//! # 所有权与失步恢复
//!
//! Hub 与消费者交换 [`CommandBatch`] 的连续事件和 UTF-8 缓冲；文本引用不能越过批次清空。
//! 本地与外部来源共享总序、使用独立额度：外部满额可重试，本地满额产生不依赖队列容量的
//! [`InputGap`]。缺口取消该来源的持续状态，适配器同步硬件基线后显式恢复来源。
//!
//! # 录制边界
//!
//! [`Recorder`] 复制并独立拥有命令与文本，录制整体驻内存；`serde` feature 提供 JSON。
//! 命令录制不等于应用确定性重放：初始模型、资产、字体、view 度量、随机性和媒体时钟
//! 由应用保存。本 crate 不拥有窗口、GUI、业务模型或音频设备。
//!
//! # 批次消费
//!
//! ```
//! use input::{
//!     ActionId, ActionPhase, ActionState, CommandBatch, CommandKind, Incoming,
//!     InputHub, Limits, MonoTime, SourceId,
//! };
//!
//! let source = SourceId::new(1);
//! let action = ActionId::new(7);
//! let mut hub = InputHub::new(Limits::default())?;
//! hub.register_source(source)?;
//! hub.push(Incoming {
//!     source,
//!     received: MonoTime::new(10),
//!     sampled: None,
//!     kind: CommandKind::Action { action, phase: ActionPhase::Press },
//! })?;
//! let mut batch = CommandBatch::new();
//! hub.drain_into(&mut batch);
//! let mut held = ActionState::with_capacity(16);
//! for item in batch.iter() {
//!     held.observe(item)?;
//! }
//! assert!(held.is_pressed(action));
//! # Ok::<(), input::InputError>(())
//! ```
#![forbid(unsafe_code)]

mod action;
mod batch;
mod command;
mod hub;
mod replay;

pub use action::*;
pub use batch::*;
pub use command::*;
pub use hub::*;
pub use replay::*;

macro_rules! id {
    ($(#[doc = $doc:literal] $name:ident($raw:ty);)*) => {$ (
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name($raw);

        impl $name {
            /// 保留原值构造，不校验所属会话或时钟域。
            pub const fn new(value: $raw) -> Self { Self(value) }
            /// 交给适配器或持久化边界的原始表示。
            pub const fn get(self) -> $raw { self.0 }
        }
    )*};
}

id! {
    /// 当前输入会话单调时钟中的纳秒时间。
    MonoTime(u64);
    /// 相对录制起点的纳秒时间，由回放控制者推进。
    ReplayTime(u64);
    /// Hub 分配的非零分发总序；同一会话内严格递增。
    Sequence(u64);
    /// 应用分配的录制会话身份；不负责建立时钟映射。
    SessionId(u128);
    /// 会话内固定的逻辑适配槽；重连复用槽，断开先发送来源取消。
    SourceId(u64);
    /// 来源内的指针身份；完整身份为 `(SourceId, PointerId)`。
    PointerId(u64);
    /// 应用分配的逻辑视图身份，不等同于窗口或渲染目标。
    ViewId(u64);
    /// 视图坐标空间代次；尺寸、密度或坐标映射改变时由应用递增。
    SpaceEpoch(u64);
    /// 消费者发布的交互快照身份，用于拒绝过期操作。
    SnapshotId(u64);
    /// 业务定义的逻辑动作，独立于物理按键与控件身份。
    ActionId(u64);
    /// 外部语义请求的相关身份；去重与重试策略由应用维护。
    RequestId(u128);
    /// 文本焦点的 IME 会话代次，用于拒绝迟到的组合输入。
    TextSessionId(u64);
    /// 文档代次，用于拒绝旧布局的光标命中或外部编辑。
    TextRevision(u64);
}

impl MonoTime {
    /// 增加纳秒；超出 `u64` 时钟范围时返回 `None`。
    pub const fn checked_add(self, nanos: u64) -> Option<Self> {
        match self.0.checked_add(nanos) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// 校准到当前会话单调域的采样时间，不改变分发顺序。
pub struct SampleTime {
    /// 样本发生时间，可以早于此前接收的样本。
    pub at: MonoTime,
    /// 时钟映射的不确定性上界，单位纳秒。
    pub uncertainty_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// 已接收命令的总序与时间证据。
pub struct Stamp {
    /// Hub 分配的分发总序。
    pub sequence: Sequence,
    /// 适配器接收到命令的会话时间；全 Hub 非递减。
    pub received: MonoTime,
    /// 经适配器校准的可选采样时间。
    pub sampled: Option<SampleTime>,
}

#[derive(Debug, thiserror::Error)]
/// 输入校验、预算与失步恢复失败。
pub enum InputError {
    /// 本地预算为零、总容量溢出或文本超出可寻址范围。
    #[error("输入预算必须非零，文本预算不能超过 u32::MAX")]
    InvalidLimits,
    /// 逻辑适配槽已经注册。
    #[error("输入来源已存在：{0:?}")]
    DuplicateSource(SourceId),
    /// 来源未向该 Hub 注册。
    #[error("未知输入来源：{0:?}")]
    UnknownSource(SourceId),
    /// 已达到固定逻辑适配槽上限。
    #[error("输入来源预算已满")]
    SourceLimit,
    /// 本地来源已失步，须先交付缺口并恢复。
    #[error("输入来源存在缺口，尚未恢复：{0:?}")]
    Suspended(SourceId),
    /// 缺口尚未排出，不能提前恢复来源。
    #[error("输入缺口尚未交付：{0:?}")]
    GapPending(SourceId),
    /// 本地满额；该来源锁存缺口并暂停。
    #[error("输入容量已满：{0:?}")]
    Overflow(GapReason),
    /// 外部满额；请求未接收，可待消费者排空后重试。
    #[error("外部输入配额已满：{0:?}")]
    Busy(GapReason),
    /// 新的来源／动作组合超过持续状态预算。
    #[error("持续动作数量超出预算")]
    ActionLimit,
    /// Hub 的分发序号耗尽，命令未接收。
    #[error("输入序号溢出")]
    SequenceOverflow,
    /// 接收时间倒退，命令未接收。
    #[error("接收时间早于已接收命令")]
    TimeOrder,
    /// 坐标或数值非有限，或压力不在 `[0, 1]`。
    #[error("输入包含非有限数值或非法压力值")]
    InvalidNumber,
    /// 文本区间超界、逆序或不在 UTF-8 字节边界。
    #[error("文本范围超界或不位于 UTF-8 边界")]
    InvalidTextRange,
}
