//! 独占的连续命令与 UTF-8 存储，按借用交付已接收事件。

use crate::{CommandKind, CommandKindRef, InputError, MonoTime, SampleTime, SourceId, Stamp};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
/// 某来源无法保持完整边沿流的容量原因。
pub enum GapReason {
    /// 同类来源共享的命令额度耗尽。
    CommandsFull,
    /// 同类来源共享的 UTF-8 字节额度耗尽。
    TextFull,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// 不完整来源流的替代标记；消费者须取消该来源的持续动作与捕获。
///
/// 标记替代该来源尚未消费的整批命令，且不依赖普通队列的剩余容量。
pub struct InputGap {
    /// 已暂停并等待适配器同步基线的来源。
    pub source: SourceId,
    /// 首次无法接收命令的时间与总序。
    pub stamp: Stamp,
    /// 触发缺口的容量约束。
    pub reason: GapReason,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// 适配器提交的借用命令；成功接收时 Hub 同步复制文本。
pub struct Incoming<'a> {
    /// 已注册的逻辑适配槽。
    pub source: SourceId,
    /// 适配器最早接收时间，必须在整个 Hub 内非递减。
    pub received: MonoTime,
    /// 校准到会话时钟的可选硬件采样证据。
    pub sampled: Option<SampleTime>,
    /// 规范化语义与借用载荷。
    pub kind: CommandKindRef<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// 借用批次或录制文本的已接收命令。
pub struct CommandRef<'a> {
    /// 原始逻辑适配槽。
    pub source: SourceId,
    /// Hub 接收时分配的时间与总序。
    pub stamp: Stamp,
    /// 文本只在其批次或录制所有者仍被借用时有效。
    pub kind: CommandKindRef<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// 总序流中的完整命令或失步标记。
pub enum InputItemRef<'a> {
    /// 已通过 Hub 边界校验的命令。
    Command(CommandRef<'a>),
    /// 必须清理该来源持续状态的缺口。
    Gap(InputGap),
}

impl InputItemRef<'_> {
    /// 命令或缺口所属的逻辑适配槽。
    pub const fn source(self) -> SourceId {
        match self {
            Self::Command(command) => command.source,
            Self::Gap(gap) => gap.source,
        }
    }

    /// 命令接收或首次失步的时间与总序。
    pub const fn stamp(self) -> Stamp {
        match self {
            Self::Command(command) => command.stamp,
            Self::Gap(gap) => gap.stamp,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct TextSpan {
    pub start: u32,
    pub end: u32,
}

impl TextSpan {
    pub fn shifted(self, base: u32) -> Self {
        Self {
            start: self.start + base,
            end: self.end + base,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct StoredCommand {
    pub source: SourceId,
    pub stamp: Stamp,
    pub kind: CommandKind<TextSpan>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub(crate) enum StoredItem {
    Command(StoredCommand),
    Gap(InputGap),
}

impl StoredItem {
    pub const fn source(self) -> SourceId {
        match self {
            Self::Command(command) => command.source,
            Self::Gap(gap) => gap.source,
        }
    }

    pub const fn stamp(self) -> Stamp {
        match self {
            Self::Command(command) => command.stamp,
            Self::Gap(gap) => gap.stamp,
        }
    }

    pub fn map_stamp(self, stamp: Stamp) -> Self {
        match self {
            Self::Command(command) => Self::Command(StoredCommand { stamp, ..command }),
            Self::Gap(gap) => Self::Gap(InputGap { stamp, ..gap }),
        }
    }

    pub fn shift_text(self, base: u32) -> Self {
        match self {
            Self::Command(command) => Self::Command(StoredCommand {
                kind: command.kind.map(|span| span.shifted(base)),
                ..command
            }),
            gap => gap,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
/// 独占连续事件与 UTF-8 缓冲的可复用批次。
///
/// [`Self::iter`] 的命令借用此批次，无法逃逸到 [`Self::clear`] 或下一次 Hub 交换之后。
/// 容量保留供后续更新复用；单个批次本身不设硬预算，预算由 Hub 或回放所有者执行。
pub struct CommandBatch {
    pub(crate) items: Vec<StoredItem>,
    pub(crate) text: String,
}

impl CommandBatch {
    /// 创建空批次，不预分配容量。
    pub const fn new() -> Self {
        Self {
            items: Vec::new(),
            text: String::new(),
        }
    }

    /// 为事件槽与 UTF-8 字节预留容量；参数是预留量，不是容量上限。
    /// 容量溢出或分配失败返回 [`InputError::Allocation`]。
    pub fn with_capacity(commands: usize, text_bytes: usize) -> Result<Self, InputError> {
        let mut batch = Self::new();
        batch.items.try_reserve_exact(commands)?;
        batch.text.try_reserve_exact(text_bytes)?;
        Ok(batch)
    }

    /// 清空事件和文本，保留两者容量。
    pub fn clear(&mut self) {
        self.items.clear();
        self.text.clear();
    }

    /// 命令和缺口的总条数。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 是否没有命令或缺口。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 批次拥有的 UTF-8 字节数，不含保留容量；缺口丢弃命令后可能保留未引用文本。
    pub fn text_bytes(&self) -> usize {
        self.text.len()
    }

    /// 按分发总序借用事件；不合并相邻边沿，也不复制文本。
    pub fn iter(&self) -> impl ExactSizeIterator<Item = InputItemRef<'_>> {
        self.items.iter().map(|item| match *item {
            StoredItem::Command(command) => InputItemRef::Command(CommandRef {
                source: command.source,
                stamp: command.stamp,
                kind: command
                    .kind
                    .map(|span| &self.text[span.start as usize..span.end as usize]),
            }),
            StoredItem::Gap(gap) => InputItemRef::Gap(gap),
        })
    }

    pub(crate) fn push(&mut self, command: CommandRef<'_>) {
        let kind = command.kind.map(|text| {
            let start = self.text.len() as u32;
            self.text.push_str(text);
            TextSpan {
                start,
                end: self.text.len() as u32,
            }
        });
        self.items.push(StoredItem::Command(StoredCommand {
            source: command.source,
            stamp: command.stamp,
            kind,
        }));
    }
}
