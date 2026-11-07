//! 来源接收、独立额度与失步恢复；批次存储和持续动作由各自所有者维护。

use crate::batch::StoredItem;
use crate::{
    CommandBatch, CommandRef, GapReason, Incoming, InputError, InputGap, MonoTime, Sequence,
    SourceId, Stamp,
};

#[derive(Clone, Copy, Debug)]
/// 一个 Hub 的共享额度；本地与可背压来源分别计费。
///
/// 本地命令、文本和来源预算必须非零；外部预算可以为零以禁用外部接收。
/// 事件上限另为每个来源预留一个缺口槽；两类文本预算之和不得超过 `u32::MAX`。
pub struct Limits {
    /// 无法背压的本地来源共享命令预留量。
    pub max_commands: usize,
    /// 本地来源共享的 UTF-8 字节预留量。
    pub max_text_bytes: usize,
    /// 可背压外部来源共享的命令额度，不消耗本地预留量。
    pub max_external_commands: usize,
    /// 外部来源共享的 UTF-8 字节额度。
    pub max_external_text_bytes: usize,
    /// 会话固定逻辑适配槽的总上限，不随连接重建增长。
    pub max_sources: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_commands: 4096,
            max_text_bytes: 64 * 1024,
            max_external_commands: 256,
            max_external_text_bytes: 16 * 1024,
            max_sources: 16,
        }
    }
}

struct SourceState {
    id: SourceId,
    external: bool,
    suspended: bool,
    gap: Option<InputGap>,
}

/// 顺序接收并校验命令，拥有来源状态与待交付的连续批次。
///
/// [`Self::register_source`] 适用于无法背压的物理适配器；
/// [`Self::register_external_source`] 适用于可重试的自动化连接。逻辑槽在 Hub 生命周期内
/// 不退役：重连应复用槽，断开先发送来源取消；当前接口不提供连接代际管理。
///
/// 本地溢出后锁存缺口并暂停来源。消费者先 [`Self::drain_into`] 处理取消，
/// 适配器同步硬件 baseline、抑制仍按住的输入直到释放，再 [`Self::resume_source`]。
pub struct InputHub {
    limits: Limits,
    sources: Vec<SourceState>,
    batch: CommandBatch,
    next_sequence: u64,
    last_received: MonoTime,
    local_commands: usize,
    local_text: usize,
    external_commands: usize,
    external_text: usize,
}

impl InputHub {
    /// 校验预算并预留连续缓冲；非法或相加溢出的预算返回 [`InputError::InvalidLimits`]。
    pub fn new(limits: Limits) -> Result<Self, InputError> {
        let commands = limits
            .max_commands
            .checked_add(limits.max_external_commands)
            .and_then(|value| value.checked_add(limits.max_sources))
            .ok_or(InputError::InvalidLimits)?;
        let text = limits
            .max_text_bytes
            .checked_add(limits.max_external_text_bytes)
            .ok_or(InputError::InvalidLimits)?;
        if limits.max_commands == 0
            || limits.max_text_bytes == 0
            || limits.max_sources == 0
            || text > u32::MAX as usize
        {
            return Err(InputError::InvalidLimits);
        }
        let mut sources = Vec::new();
        sources.try_reserve_exact(limits.max_sources)?;
        Ok(Self {
            limits,
            sources,
            batch: CommandBatch::with_capacity(commands, text)?,
            next_sequence: 1,
            last_received: MonoTime::new(0),
            local_commands: 0,
            local_text: 0,
            external_commands: 0,
            external_text: 0,
        })
    }

    /// 注册本地逻辑槽；重复身份或超过来源总预算时失败。
    pub fn register_source(&mut self, source: SourceId) -> Result<(), InputError> {
        self.register(source, false)
    }

    /// 注册共享外部额度的逻辑槽；重复身份或超过来源总预算时失败。
    /// 其请求满额时返回 [`InputError::Busy`]，不挂起来源或消耗本地预留量。
    pub fn register_external_source(&mut self, source: SourceId) -> Result<(), InputError> {
        self.register(source, true)
    }

    fn register(&mut self, source: SourceId, external: bool) -> Result<(), InputError> {
        if self.sources.iter().any(|state| state.id == source) {
            return Err(InputError::DuplicateSource(source));
        }
        if self.sources.len() == self.limits.max_sources {
            return Err(InputError::SourceLimit);
        }
        self.sources.push(SourceState {
            id: source,
            external,
            suspended: false,
            gap: None,
        });
        Ok(())
    }

    /// 同步校验并复制一条命令；接收时间非递减，采样时间不参与重排。
    ///
    /// 未注册、暂停、非法数值／文本、时间倒退和序号耗尽均拒绝接收。
    /// 外部满额返回 [`InputError::Busy`]，保持时间和总序不变，可稍后重试；本地满额返回
    /// [`InputError::Overflow`]，消耗本条总序并暂停来源，由缺口替代其未消费残批。
    pub fn push(&mut self, event: Incoming<'_>) -> Result<(), InputError> {
        let source = self
            .sources
            .iter()
            .position(|state| state.id == event.source)
            .ok_or(InputError::UnknownSource(event.source))?;
        if self.sources[source].suspended {
            return Err(InputError::Suspended(event.source));
        }
        event.kind.validate()?;
        if event.received < self.last_received {
            return Err(InputError::TimeOrder);
        }
        let next = self
            .next_sequence
            .checked_add(1)
            .ok_or(InputError::SequenceOverflow)?;
        let stamp = Stamp {
            sequence: Sequence::new(self.next_sequence),
            received: event.received,
            sampled: event.sampled,
        };
        let bytes = event.kind.text().map_or(0, str::len);
        let external = self.sources[source].external;
        let (count, text, max_count, max_text) = if external {
            (
                self.external_commands,
                self.external_text,
                self.limits.max_external_commands,
                self.limits.max_external_text_bytes,
            )
        } else {
            (
                self.local_commands,
                self.local_text,
                self.limits.max_commands,
                self.limits.max_text_bytes,
            )
        };
        let reason = if count >= max_count {
            Some(GapReason::CommandsFull)
        } else if bytes > max_text - text {
            Some(GapReason::TextFull)
        } else {
            None
        };
        if external && let Some(reason) = reason {
            return Err(InputError::Busy(reason));
        }
        self.next_sequence = next;
        self.last_received = event.received;
        if let Some(reason) = reason {
            self.sources[source].suspended = true;
            self.sources[source].gap = Some(InputGap {
                source: event.source,
                stamp,
                reason,
            });
            return Err(InputError::Overflow(reason));
        }
        self.batch.push(CommandRef {
            source: event.source,
            stamp,
            kind: event.kind,
        });
        if external {
            self.external_commands += 1;
            self.external_text += bytes;
        } else {
            self.local_commands += 1;
            self.local_text += bytes;
        }
        Ok(())
    }

    /// 清空旧输出并交换缓冲，按总序交付命令及缺口，保留两侧容量复用。
    ///
    /// 对有缺口的来源舍弃其全部未消费命令；其他来源保持原序。
    /// 文本缓冲不做压缩，未引用字节保留到下次清空；交付缺口后仍需显式恢复。
    /// 交换前为接收缓冲预留完整预算，`push` 无需逐条扩容。分配失败保持两侧事件及总序不变。
    pub fn drain_into(&mut self, batch: &mut CommandBatch) -> Result<(), InputError> {
        // 构造器已校验预算和缺口槽之和；预留以现有长度为基准，成功后才清空、交换。
        let commands =
            self.limits.max_commands + self.limits.max_external_commands + self.limits.max_sources;
        let text = self.limits.max_text_bytes + self.limits.max_external_text_bytes;
        batch
            .items
            .try_reserve(commands.saturating_sub(batch.items.len()))?;
        batch
            .text
            .try_reserve(text.saturating_sub(batch.text.len()))?;
        batch.clear();
        std::mem::swap(batch, &mut self.batch);
        self.local_commands = 0;
        self.local_text = 0;
        self.external_commands = 0;
        self.external_text = 0;
        if self.sources.iter().any(|state| state.gap.is_some()) {
            batch.items.retain(|item| {
                !self
                    .sources
                    .iter()
                    .any(|state| state.id == item.source() && state.gap.is_some())
            });
            for state in &mut self.sources {
                if let Some(gap) = state.gap.take() {
                    batch.items.push(StoredItem::Gap(gap));
                }
            }
            batch
                .items
                .sort_unstable_by_key(|item| item.stamp().sequence);
        }
        Ok(())
    }

    /// 交付缺口并同步硬件 baseline 后恢复来源，不合成任何按下或释放。
    ///
    /// 适配器须抑制仍按住的输入直到释放；未注册或缺口尚未交付时返回错误。
    pub fn resume_source(&mut self, source: SourceId) -> Result<(), InputError> {
        let state = self
            .sources
            .iter_mut()
            .find(|state| state.id == source)
            .ok_or(InputError::UnknownSource(source))?;
        if state.gap.is_some() {
            return Err(InputError::GapPending(source));
        }
        state.suspended = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionId, ActionPhase, ActionState, CommandKind, InputItemRef, InputScope, SampleTime,
        TextOp, TextSessionId, ViewId,
    };

    #[test]
    fn unrepresentable_storage_is_an_allocation_error() {
        let result = InputHub::new(Limits {
            max_commands: isize::MAX as usize,
            max_sources: 1,
            max_external_commands: 0,
            ..Limits::default()
        });
        assert!(matches!(result, Err(InputError::Allocation(_))));
    }

    fn action(source: u64, time: u64, phase: ActionPhase) -> Incoming<'static> {
        Incoming {
            source: SourceId::new(source),
            received: MonoTime::new(time),
            sampled: None,
            kind: CommandKind::Action {
                action: ActionId::new(7),
                phase,
            },
        }
    }

    fn hub(max_commands: usize) -> InputHub {
        let mut hub = InputHub::new(Limits {
            max_commands,
            ..Limits::default()
        })
        .unwrap();
        hub.register_source(SourceId::new(1)).unwrap();
        hub.register_source(SourceId::new(2)).unwrap();
        hub
    }

    #[test]
    fn short_press_keeps_both_edges() {
        let mut hub = hub(4);
        hub.push(action(1, 10, ActionPhase::Press)).unwrap();
        hub.push(action(1, 10, ActionPhase::Release)).unwrap();
        let mut batch = CommandBatch::new();
        hub.drain_into(&mut batch).unwrap();
        let phases: Vec<_> = batch
            .iter()
            .filter_map(|item| match item {
                InputItemRef::Command(CommandRef {
                    kind: CommandKind::Action { phase, .. },
                    ..
                }) => Some(phase),
                _ => None,
            })
            .collect();
        assert_eq!(phases, [ActionPhase::Press, ActionPhase::Release]);
    }

    #[test]
    fn late_sample_keeps_dispatch_order() {
        let mut hub = hub(4);
        let mut first = action(1, 10, ActionPhase::Press);
        first.sampled = Some(SampleTime {
            at: MonoTime::new(9),
            uncertainty_ns: 1,
        });
        let mut second = action(2, 11, ActionPhase::Press);
        second.sampled = Some(SampleTime {
            at: MonoTime::new(4),
            uncertainty_ns: 1,
        });
        hub.push(first).unwrap();
        hub.push(second).unwrap();
        let mut batch = CommandBatch::new();
        hub.drain_into(&mut batch).unwrap();
        let stamps: Vec<_> = batch.iter().map(InputItemRef::stamp).collect();
        assert!(stamps[0].sequence < stamps[1].sequence);
        assert_eq!(stamps[1].sampled.unwrap().at, MonoTime::new(4));
    }

    #[test]
    fn overflow_delivers_gap_without_queue_space() {
        let mut hub = hub(2);
        hub.push(action(1, 1, ActionPhase::Press)).unwrap();
        hub.push(action(2, 2, ActionPhase::Press)).unwrap();
        assert!(matches!(
            hub.push(action(1, 3, ActionPhase::Release)),
            Err(InputError::Overflow(_))
        ));
        let mut batch = CommandBatch::new();
        hub.drain_into(&mut batch).unwrap();
        let items: Vec<_> = batch.iter().collect();
        assert!(
            matches!(items.as_slice(), [InputItemRef::Command(command), InputItemRef::Gap(gap)] if command.source == SourceId::new(2) && gap.source == SourceId::new(1))
        );
        assert!(matches!(
            hub.push(action(1, 4, ActionPhase::Press)),
            Err(InputError::Suspended(_))
        ));
    }

    #[test]
    fn external_quota_preserves_local_release_capacity() {
        let mut hub = InputHub::new(Limits {
            max_commands: 2,
            max_external_commands: 1,
            ..Limits::default()
        })
        .unwrap();
        hub.register_source(SourceId::new(1)).unwrap();
        hub.register_external_source(SourceId::new(2)).unwrap();
        hub.push(action(1, 1, ActionPhase::Press)).unwrap();
        hub.push(action(2, 2, ActionPhase::Press)).unwrap();
        assert!(matches!(
            hub.push(action(2, 3, ActionPhase::Repeat)),
            Err(InputError::Busy(_))
        ));
        hub.push(action(1, 4, ActionPhase::Release)).unwrap();
        let mut batch = CommandBatch::new();
        hub.drain_into(&mut batch).unwrap();
        assert_eq!(batch.len(), 3);
        assert!(
            batch
                .iter()
                .all(|item| matches!(item, InputItemRef::Command(_)))
        );
        hub.push(action(2, 5, ActionPhase::Release)).unwrap();
    }

    #[test]
    fn action_state_has_a_hard_capacity() {
        let mut state = ActionState::with_capacity(1).unwrap();
        let first = CommandRef {
            source: SourceId::new(1),
            stamp: Stamp {
                sequence: Sequence::new(1),
                received: MonoTime::new(1),
                sampled: None,
            },
            kind: action(1, 1, ActionPhase::Press).kind,
        };
        state.observe(InputItemRef::Command(first)).unwrap();
        assert!(matches!(
            state.observe(InputItemRef::Command(CommandRef {
                source: SourceId::new(2),
                ..first
            })),
            Err(InputError::ActionLimit)
        ));
    }

    #[test]
    fn source_cancel_keeps_other_source_pressed() {
        let mut hub = hub(4);
        hub.push(action(1, 1, ActionPhase::Press)).unwrap();
        hub.push(action(2, 2, ActionPhase::Press)).unwrap();
        hub.push(Incoming {
            kind: CommandKind::Cancel {
                scope: InputScope::Source,
                reason: crate::CancelReason::Disconnected,
            },
            ..action(1, 3, ActionPhase::Release)
        })
        .unwrap();
        let mut batch = CommandBatch::new();
        hub.drain_into(&mut batch).unwrap();
        let mut state = ActionState::with_capacity(2).unwrap();
        for item in batch.iter() {
            state.observe(item).unwrap();
        }
        assert!(state.is_pressed(ActionId::new(7)));
    }

    #[test]
    fn releasing_one_source_keeps_other_source_pressed() {
        let mut hub = hub(4);
        hub.push(action(1, 1, ActionPhase::Press)).unwrap();
        hub.push(action(2, 2, ActionPhase::Press)).unwrap();
        hub.push(action(1, 3, ActionPhase::Release)).unwrap();
        let mut batch = CommandBatch::new();
        hub.drain_into(&mut batch).unwrap();
        let mut state = ActionState::with_capacity(2).unwrap();
        for item in batch.iter() {
            state.observe(item).unwrap();
        }
        assert!(state.is_pressed(ActionId::new(7)));
    }

    #[test]
    fn text_ref_borrows_batch_arena() {
        let mut hub = hub(2);
        let text = String::from("输入🌈");
        hub.push(Incoming {
            source: SourceId::new(1),
            received: MonoTime::new(1),
            sampled: None,
            kind: CommandKind::Text {
                view: ViewId::new(1),
                session: TextSessionId::new(3),
                edit: TextOp::Commit(&text),
            },
        })
        .unwrap();
        drop(text);
        let mut batch = CommandBatch::new();
        hub.drain_into(&mut batch).unwrap();
        assert!(matches!(
            batch.iter().next(),
            Some(InputItemRef::Command(CommandRef {
                kind: CommandKind::Text {
                    edit: TextOp::Commit("输入🌈"),
                    ..
                },
                ..
            }))
        ));
    }
}
