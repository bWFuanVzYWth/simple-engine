//! 消费有序命令后维护各来源持有的动作，不参与接收额度或批次存储。

use crate::{
    ActionId, ActionPhase, CommandKind, CommandRef, InputError, InputItemRef, InputScope, SourceId,
};

/// 持续动作取各来源并集；Press 原始命令仍由消费者自行决定是否合并触发。
pub struct ActionState {
    held: Vec<(SourceId, ActionId)>,
    max_held: usize,
}

impl ActionState {
    /// 此容量也是上限，预算按来源与动作的组合数计算。
    /// 容量溢出或分配失败返回 [`InputError::Allocation`]。
    pub fn with_capacity(actions: usize) -> Result<Self, InputError> {
        let mut held = Vec::new();
        held.try_reserve_exact(actions)?;
        Ok(Self {
            held,
            max_held: actions,
        })
    }

    /// 按总序更新持续状态；新来源／动作组合满额时返回 [`InputError::ActionLimit`]。
    ///
    /// 重复按下和 Repeat 不增加状态；Release 只清除原来源。缺口、来源取消和全局取消
    /// 清理相应状态；view 取消由上层处理，不清除没有 view 的业务动作。
    pub fn observe(&mut self, item: InputItemRef<'_>) -> Result<(), InputError> {
        let source = item.source();
        match item {
            InputItemRef::Command(CommandRef {
                kind: CommandKind::Action { action, phase },
                ..
            }) => {
                let at = self
                    .held
                    .iter()
                    .position(|entry| *entry == (source, action));
                match (phase, at) {
                    (ActionPhase::Press, None) => {
                        if self.held.len() == self.max_held {
                            return Err(InputError::ActionLimit);
                        }
                        self.held.push((source, action));
                    }
                    (ActionPhase::Release, Some(at)) => {
                        self.held.swap_remove(at);
                    }
                    _ => {}
                }
            }
            InputItemRef::Command(CommandRef {
                kind:
                    CommandKind::Cancel {
                        scope: InputScope::All,
                        ..
                    },
                ..
            }) => self.held.clear(),
            InputItemRef::Command(CommandRef {
                kind:
                    CommandKind::Cancel {
                        scope: InputScope::Source,
                        ..
                    },
                ..
            })
            | InputItemRef::Gap(_) => self.held.retain(|entry| entry.0 != source),
            _ => {}
        }
        Ok(())
    }

    /// 任一来源仍持有该动作时为 `true`，不把多来源按下合并成触发事件。
    pub fn is_pressed(&self, action: ActionId) -> bool {
        self.held.iter().any(|entry| entry.1 == action)
    }
}
