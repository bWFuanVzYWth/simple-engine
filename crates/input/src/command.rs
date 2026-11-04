//! 命令载荷与版本约束；坐标采用逻辑像素，文本区间采用 UTF-8 字节。
//!
//! `T` 将载荷所有权与命令语义分开：适配器传入借用文本，批次和录制复制到自己的连续缓冲。
//! 构造值不代表已经通过边界校验；[`crate::InputHub::push`] 检查数值和文本范围。

use crate::{
    ActionId, InputError, PointerId, RequestId, SnapshotId, SpaceEpoch, TextRevision,
    TextSessionId, ViewId,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// 视图坐标空间中的逻辑像素点；有限性由输入边界校验。
pub struct Point {
    /// 向右增加的横坐标。
    pub x: f32,
    /// 向下增加的纵坐标。
    pub y: f32,
}

impl Point {
    /// 记录坐标，保留原值供 Hub 统一校验。
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn valid(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// 指针位置及其坐标空间证据，不隐式转换过期坐标。
pub struct PointerPosition {
    /// 坐标所属视图。
    pub view: ViewId,
    /// 产生坐标时使用的空间代次。
    pub space: SpaceEpoch,
    /// 指定时要求消费者仍发布该交互快照。
    pub expected_snapshot: Option<SnapshotId>,
    /// 视图内逻辑像素位置，可位于视图外以维持捕获。
    pub point: Point,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 单个指针的独立接触通道，不绑定具体硬件按钮。
pub enum Contact {
    /// 主要接触，如鼠标主键或触摸按下。
    Primary,
    /// 次要接触，如上下文操作。
    Secondary,
    /// 中间接触通道。
    Middle,
    /// 应用与适配器约定的额外通道编号。
    Auxiliary(u16),
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 来源内指针的边沿与移动；按下和释放不合并。
pub enum PointerCommand {
    /// 更新位置；移动不自行建立接触状态。
    Move {
        /// 来源内指针身份。
        id: PointerId,
        /// 当前坐标与空间代次。
        at: PointerPosition,
        /// 可选归一化压力，有限且位于 `[0, 1]`。
        pressure: Option<f32>,
    },
    /// 建立该接触通道；捕获由消费者决定。
    Down {
        /// 来源内指针身份。
        id: PointerId,
        /// 边沿发生位置。
        at: PointerPosition,
        /// 本次按下的接触通道。
        contact: Contact,
    },
    /// 释放接触通道，捕获消费者仍须接收视图外释放。
    Up {
        /// 来源内指针身份。
        id: PointerId,
        /// 释放位置；消费者先清理捕获，再处理坐标是否过期。
        at: PointerPosition,
        /// 本次释放的接触通道。
        contact: Contact,
    },
    /// 取消该指针的全部接触；不构成正常点击释放。
    Cancel {
        /// 来源内待取消指针。
        id: PointerId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 逻辑动作边沿；重复触发与持续按下状态分别消费。
pub enum ActionPhase {
    /// 从未按下进入按下状态。
    Press,
    /// 适配器生成的重复触发，不新增持续状态。
    Repeat,
    /// 结束该来源的持续状态。
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 逻辑焦点或值导航，不绑定键盘布局。
pub enum Navigation {
    /// 按语义顺序移到下一项。
    Next,
    /// 按语义顺序移到上一项。
    Previous,
    /// 向左导航或减小值。
    Left,
    /// 向右导航或增大值。
    Right,
    /// 向上导航或增大值。
    Up,
    /// 向下导航或减小值。
    Down,
    /// 到当前导航范围起点。
    Home,
    /// 到当前导航范围终点。
    End,
    /// 向上一页导航。
    PageUp,
    /// 向下一页导航。
    PageDown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 保留平台提供的滚动单位，行高策略由消费者决定。
pub enum ScrollDelta {
    /// 以逻辑行为单位的滚动量。
    Lines {
        /// 水平行数。
        x: f32,
        /// 垂直行数。
        y: f32,
    },
    /// 以逻辑像素为单位的滚动量。
    Pixels {
        /// 水平像素量。
        x: f32,
        /// 垂直像素量。
        y: f32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// 相对载荷的半开 UTF-8 字节区间，可表示空选区。
/// [`Self::new`] 校验给定文本；直接反序列化的区间仍须经过 Hub 输入校验。
pub struct TextRange {
    start: u32,
    end: u32,
}

impl TextRange {
    /// 校验顺序、边界和 `u32` 表示范围，失败返回 [`InputError::InvalidTextRange`]。
    pub fn new(text: &str, start: usize, end: usize) -> Result<Self, InputError> {
        let start = u32::try_from(start).map_err(|_| InputError::InvalidTextRange)?;
        let end = u32::try_from(end).map_err(|_| InputError::InvalidTextRange)?;
        let range = Self { start, end };
        range.validate(text)?;
        Ok(range)
    }

    /// 包含的起始字节。
    pub const fn start(self) -> usize {
        self.start as usize
    }

    /// 不包含的结束字节。
    pub const fn end(self) -> usize {
        self.end as usize
    }

    fn validate(self, text: &str) -> Result<(), InputError> {
        text.get(self.start()..self.end())
            .ok_or(InputError::InvalidTextRange)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 删除方向与单位；具体词边界由文本消费者解释。
pub enum EditUnit {
    /// 删除光标前一个 grapheme。
    GraphemeBackward,
    /// 删除光标后一个 grapheme。
    GraphemeForward,
    /// 删除到前一词边界。
    WordBackward,
    /// 删除到后一词边界。
    WordForward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 选择端点的移动目标；是否扩选由 [`TextOp::MoveSelection`] 指定。
pub enum TextMovement {
    /// 向前一个 grapheme。
    GraphemeBackward,
    /// 向后一个 grapheme。
    GraphemeForward,
    /// 到前一词边界。
    WordBackward,
    /// 到后一词边界。
    WordForward,
    /// 到当前行起点。
    LineStart,
    /// 到当前行终点。
    LineEnd,
    /// 到文档起点。
    DocumentStart,
    /// 到文档终点。
    DocumentEnd,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 对消费者拥有的文档执行编辑；Preedit 与正文提交分别表示。
/// `T` 表示文本载荷所有权，适配器通常使用借用别名 [`TextOpRef`]。
pub enum TextOp<T> {
    /// 用文本替换选区并结束组合输入。
    Commit(T),
    /// 显示组合输入，不修改正文；选区相对本条 `text`。
    Preedit {
        /// 当前组合显示文本。
        text: T,
        /// 平台提供的组合区选区，采用 UTF-8 字节区间。
        selection: Option<TextRange>,
    },
    /// 撤销尚未提交的组合显示，保留正文。
    EndComposition,
    /// 优先删除选区；无选区时按指定单位删除。
    Delete(EditUnit),
    /// 移动光标或选区活动端点。
    MoveSelection {
        /// 相对当前文档的移动目标。
        movement: TextMovement,
        /// 保留锚点以扩选；为 `false` 时收拢选区。
        extend: bool,
    },
}

/// 借用适配器或批次文本的编辑；不能越过载荷所有者的生命周期。
pub type TextOpRef<'a> = TextOp<&'a str>;

impl<T> TextOp<T> {
    pub(crate) fn map<U>(self, map: impl FnOnce(T) -> U) -> TextOp<U> {
        match self {
            Self::Commit(text) => TextOp::Commit(map(text)),
            Self::Preedit { text, selection } => TextOp::Preedit {
                text: map(text),
                selection,
            },
            Self::EndComposition => TextOp::EndComposition,
            Self::Delete(unit) => TextOp::Delete(unit),
            Self::MoveSelection { movement, extend } => TextOp::MoveSelection { movement, extend },
        }
    }
}

impl TextOpRef<'_> {
    fn validate(self) -> Result<(), InputError> {
        if let Self::Preedit {
            text,
            selection: Some(range),
        } = self
        {
            range.validate(text)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 外部语义请求允许消费的目标版本；不负责请求去重。
pub enum TargetVersion {
    /// 使用当前发布状态，允许请求期间界面更新。
    Current,
    /// 仅接受指定交互快照。
    ExactSnapshot(SnapshotId),
    /// 仅接受指定文本修订；应用还须校验未发布的当前模型版本。
    Document(TextRevision),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// 不依赖屏幕坐标的语义目标。
pub struct Target {
    /// 目标所属逻辑视图。
    pub view: ViewId,
    /// 由消费者定义的稳定业务键。
    pub widget: u128,
    /// 防止迟到请求误作用于新状态的版本约束。
    pub version: TargetVersion,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 控件级命令，可由 CLI、MCP 或外设适配器产生。
/// `T` 与其中的文本编辑共享所有权形式，借用入口为 [`SemanticOpRef`]。
pub enum SemanticOp<T> {
    /// 触发目标的主要操作。
    Activate,
    /// 设置目标值；范围与量化由目标解释。
    SetValue(f64),
    /// 沿目标步长增减指定次数。
    Step(i32),
    /// 编辑目标文本，仍受目标文档版本约束。
    Edit(TextOp<T>),
    /// 请求目标获得焦点，不模拟指针事件。
    Focus,
}

/// 载荷借用外部文本的语义命令。
pub type SemanticOpRef<'a> = SemanticOp<&'a str>;

impl<T> SemanticOp<T> {
    fn map<U>(self, map: impl FnOnce(T) -> U) -> SemanticOp<U> {
        match self {
            Self::Activate => SemanticOp::Activate,
            Self::SetValue(value) => SemanticOp::SetValue(value),
            Self::Step(steps) => SemanticOp::Step(steps),
            Self::Edit(edit) => SemanticOp::Edit(edit.map(map)),
            Self::Focus => SemanticOp::Focus,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 取消操作覆盖的持续状态范围。
pub enum InputScope {
    /// 仅取消发送此命令的来源。
    Source,
    /// 仅取消对应视图；不带 view 的持续动作由上层单独处理。
    View(ViewId),
    /// 取消全部来源与视图。
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 取消的来源证据，不隐式决定应用的恢复策略。
pub enum CancelReason {
    /// 平台输入焦点丢失。
    FocusLost,
    /// 设备或外部连接断开。
    Disconnected,
    /// 逻辑视图被移除。
    ViewRemoved,
    /// 坐标映射失效。
    SpaceChanged,
    /// 应用显式取消。
    Requested,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 规范化命令；硬件绑定和平台事件差异留在适配器。
/// `T` 仅承载文本，适配器借用的 [`CommandKindRef`] 在接收时复制到批次连续缓冲。
pub enum CommandKind<T> {
    /// 带来源内身份的指针操作。
    Pointer(PointerCommand),
    /// 离散动作边沿。
    Action {
        /// 业务逻辑动作。
        action: ActionId,
        /// 按下、重复或释放。
        phase: ActionPhase,
    },
    /// 更新连续动作的有限数值，不强制归一化范围。
    Axis {
        /// 轴值对应的业务动作。
        action: ActionId,
        /// 适配器与消费者约定单位的绝对值。
        value: f32,
    },
    /// 对视图焦点进行逻辑导航。
    Navigate {
        /// 导航所属视图。
        view: ViewId,
        /// 导航方向或端点。
        direction: Navigation,
    },
    /// 在位置处滚动；保留原始逻辑单位。
    Scroll {
        /// 决定滚动目标的位置。
        at: PointerPosition,
        /// 行数或逻辑像素增量。
        delta: ScrollDelta,
    },
    /// 平台文本编辑，消费者须拒绝旧 IME 会话。
    Text {
        /// 文本焦点所属视图。
        view: ViewId,
        /// 平台开始此次组合输入时绑定的会话代次。
        session: TextSessionId,
        /// 正文或组合输入操作。
        edit: TextOp<T>,
    },
    /// 按稳定语义身份操作控件，不要求物理设备存在。
    Invoke {
        /// 请求相关身份，由应用维护幂等语义。
        request: RequestId,
        /// 稳定目标及其版本约束。
        target: Target,
        /// 控件级操作。
        op: SemanticOp<T>,
    },
    /// 清理持续状态；不模拟正常释放产生的激活。
    Cancel {
        /// 取消覆盖的来源或视图。
        scope: InputScope,
        /// 触发取消的原因。
        reason: CancelReason,
    },
}

/// 载荷借用适配器或连续批次缓冲的规范化命令。
pub type CommandKindRef<'a> = CommandKind<&'a str>;

impl<T> CommandKind<T> {
    pub(crate) fn map<U>(self, map: impl FnOnce(T) -> U) -> CommandKind<U> {
        match self {
            Self::Pointer(pointer) => CommandKind::Pointer(pointer),
            Self::Action { action, phase } => CommandKind::Action { action, phase },
            Self::Axis { action, value } => CommandKind::Axis { action, value },
            Self::Navigate { view, direction } => CommandKind::Navigate { view, direction },
            Self::Scroll { at, delta } => CommandKind::Scroll { at, delta },
            Self::Text {
                view,
                session,
                edit,
            } => CommandKind::Text {
                view,
                session,
                edit: edit.map(map),
            },
            Self::Invoke {
                request,
                target,
                op,
            } => CommandKind::Invoke {
                request,
                target,
                op: op.map(map),
            },
            Self::Cancel { scope, reason } => CommandKind::Cancel { scope, reason },
        }
    }
}

impl<'a> CommandKindRef<'a> {
    pub(crate) fn text(self) -> Option<&'a str> {
        match self {
            Self::Text {
                edit: TextOp::Commit(text) | TextOp::Preedit { text, .. },
                ..
            }
            | Self::Invoke {
                op: SemanticOp::Edit(TextOp::Commit(text) | TextOp::Preedit { text, .. }),
                ..
            } => Some(text),
            _ => None,
        }
    }

    pub(crate) fn validate(self) -> Result<(), InputError> {
        let valid = match self {
            Self::Pointer(PointerCommand::Move { at, pressure, .. }) => {
                at.point.valid()
                    && pressure
                        .is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value))
            }
            Self::Pointer(PointerCommand::Down { at, .. } | PointerCommand::Up { at, .. }) => {
                at.point.valid()
            }
            Self::Axis { value, .. } => value.is_finite(),
            Self::Scroll {
                at,
                delta: ScrollDelta::Lines { x, y } | ScrollDelta::Pixels { x, y },
            } => at.point.valid() && x.is_finite() && y.is_finite(),
            Self::Invoke {
                op: SemanticOp::SetValue(value),
                ..
            } => value.is_finite(),
            Self::Text { edit, .. }
            | Self::Invoke {
                op: SemanticOp::Edit(edit),
                ..
            } => return edit.validate(),
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(InputError::InvalidNumber)
        }
    }
}
