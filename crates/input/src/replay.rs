//! 整体驻内存的独立录制与按虚拟时间推进的有界重放。

use crate::batch::StoredItem;
use crate::{
    CommandBatch, CommandRef, InputError, MonoTime, ReplayTime, SampleTime, SessionId, Stamp,
};

/// 当前 JSON 录制格式版本；结构变化须递增版本，不能重新解释旧载荷。
pub const REPLAY_SCHEMA: u32 = 1;

#[derive(Debug, thiserror::Error)]
/// 录制导入、顺序、时钟映射与批次预算失败。
pub enum ReplayError {
    /// JSON 声明的 schema 不受支持，载荷未按当前结构解释。
    #[error("不支持的录制 schema：{0}")]
    UnsupportedSchema(u32),
    /// 分发总序为零或未严格递增。
    #[error("录制序号没有严格递增")]
    SequenceOrder,
    /// 接收时间倒退或早于声明的录制起点。
    #[error("录制接收时间倒退或早于录制起点")]
    TimeOrder,
    /// 回放控制者提供了早于此前截止点的虚拟时间。
    #[error("重放时间倒退")]
    PlaybackOrder,
    /// 截止时间或重映射时间无法用目标会话时钟表示。
    #[error("重放时间超出当前会话时钟范围")]
    TimeOverflow,
    /// 整体录制的 UTF-8 载荷超过 `u32` 可寻址范围。
    #[error("录制文本超出 u32 范围")]
    TextLimit,
    /// 批次额度为零或文本额度超过 `u32` 范围。
    #[error("重放批次预算必须非零，文本预算不能超过 u32::MAX")]
    InvalidLimits,
    /// 调用前的输出批次已经超过回放额度。
    #[error("输出批次已经超出重放预算")]
    OutputBudget,
    /// 单条文本超出批次额度，不能拆分交付。
    #[error("单条重放文本 {bytes} 字节超出批次预算 {limit}")]
    ItemTooLarge {
        /// 单条命令需要的 UTF-8 字节数。
        bytes: usize,
        /// 配置的单批文本额度。
        limit: usize,
    },
    /// 导入载荷违反规范化命令约束，保留原错误。
    #[error("录制命令非法：{0}")]
    Input(#[from] InputError),
    /// JSON 编解码失败，保留解析位置与原因。
    #[cfg(feature = "serde")]
    #[error("录制 JSON 无效：{0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
/// 独立拥有命令、缺口和 UTF-8 载荷的不可变录制；整体驻内存。
///
/// 应用确定性还依赖初始业务状态、资产、view 度量、随机源与媒体时钟映射，
/// 这些不属于命令录制。JSON 导入验证 schema、顺序、有限数值和文本边界。
pub struct Recording {
    schema: u32,
    session: SessionId,
    origin: MonoTime,
    batch: CommandBatch,
}

/// 同步复制已交付批次的录制所有者，不持有消费者的借用文本。
pub struct Recorder {
    recording: Recording,
}

impl Recorder {
    /// 建立应用命名的录制会话；后续接收时间不得早于 `origin`。
    pub const fn new(session: SessionId, origin: MonoTime) -> Self {
        Self {
            recording: Recording {
                schema: REPLAY_SCHEMA,
                session,
                origin,
                batch: CommandBatch::new(),
            },
        }
    }

    /// 同步复制连续事件与文本，调用者随后可立即回收批次。
    ///
    /// 总序、接收时间或总文本范围非法时整个追加失败，已存在录制保持不变。
    pub fn append(&mut self, batch: &CommandBatch) -> Result<(), ReplayError> {
        let target = &mut self.recording.batch;
        let mut previous = target.items.last().map(|item| item.stamp());
        for item in &batch.items {
            validate_order(item.stamp(), previous, self.recording.origin)?;
            previous = Some(item.stamp());
        }
        let total = target
            .text
            .len()
            .checked_add(batch.text.len())
            .ok_or(ReplayError::TextLimit)?;
        if total > u32::MAX as usize {
            return Err(ReplayError::TextLimit);
        }
        target
            .items
            .try_reserve(batch.items.len())
            .map_err(InputError::from)?;
        target
            .text
            .try_reserve(batch.text.len())
            .map_err(InputError::from)?;
        let base = target.text.len() as u32;
        target
            .items
            .extend(batch.items.iter().map(|item| item.shift_text(base)));
        target.text.push_str(&batch.text);
        Ok(())
    }

    /// 交出完整录制的所有权，不再复制载荷。
    pub fn finish(self) -> Recording {
        self.recording
    }
}

impl Recording {
    /// 录制使用的持久化格式版本。
    pub const fn schema(&self) -> u32 {
        self.schema
    }

    /// 原始会话身份，不随回放到目标会话而改写。
    pub const fn session(&self) -> SessionId {
        self.session
    }

    /// 原始单调时间域中的录制起点。
    pub const fn origin(&self) -> MonoTime {
        self.origin
    }

    /// 录制内命令与缺口的总条数。
    pub fn len(&self) -> usize {
        self.batch.len()
    }

    /// 是否没有录制事件。
    pub fn is_empty(&self) -> bool {
        self.batch.is_empty()
    }

    /// 用默认批次额度从头回放，将录制起点映射到 `destination_origin`。
    ///
    /// 回放独占业务输入会话；带外暂停／停止不改写录制总序，实际调度由调用方推进。
    pub fn replay(&self, destination_origin: MonoTime) -> Replay<'_> {
        Replay {
            recording: self,
            destination_origin,
            cursor: 0,
            last_time: ReplayTime::new(0),
            limits: ReplayLimits::default(),
        }
    }

    /// 使用显式批次额度创建回放；零额度或超出 `u32` 的文本额度失败。
    pub fn replay_with_limits(
        &self,
        destination_origin: MonoTime,
        limits: ReplayLimits,
    ) -> Result<Replay<'_>, ReplayError> {
        if limits.max_items == 0
            || limits.max_text_bytes == 0
            || limits.max_text_bytes > u32::MAX as usize
        {
            return Err(ReplayError::InvalidLimits);
        }
        Ok(Replay {
            limits,
            ..self.replay(destination_origin)
        })
    }

    #[cfg(feature = "serde")]
    /// 将完整录制编码为当前 schema 的 JSON；编解码来源保留在错误中。
    pub fn to_json(&self) -> Result<String, ReplayError> {
        Ok(serde_json::to_string(self)?)
    }

    #[cfg(feature = "serde")]
    /// 导入独立录制，先拒绝未知 schema，再校验总序、时间、数值和 UTF-8 范围。
    ///
    /// 不接受未知结构字段；失败不产生可消费的部分录制。
    pub fn from_json(json: &str) -> Result<Self, ReplayError> {
        #[derive(serde::Deserialize)]
        struct Schema {
            schema: u32,
        }
        let schema: Schema = serde_json::from_str(json)?;
        if schema.schema != REPLAY_SCHEMA {
            return Err(ReplayError::UnsupportedSchema(schema.schema));
        }
        let encoded: EncodedRecording = serde_json::from_str(json)?;
        let recording = Self {
            schema: encoded.schema,
            session: encoded.session,
            origin: encoded.origin,
            batch: CommandBatch {
                items: encoded.batch.items,
                text: encoded.batch.text,
            },
        };
        recording.validate()?;
        Ok(recording)
    }

    #[cfg(feature = "serde")]
    fn validate(&self) -> Result<(), ReplayError> {
        if self.batch.text.len() > u32::MAX as usize {
            return Err(ReplayError::TextLimit);
        }
        let mut previous = None;
        for item in &self.batch.items {
            validate_order(item.stamp(), previous, self.origin)?;
            previous = Some(item.stamp());
            if let StoredItem::Command(command) = *item {
                let mut valid_span = true;
                let kind = command.kind.map(|span| {
                    match self.batch.text.get(span.start as usize..span.end as usize) {
                        Some(text) => text,
                        None => {
                            valid_span = false;
                            ""
                        }
                    }
                });
                if !valid_span {
                    return Err(InputError::InvalidTextRange.into());
                }
                kind.validate()?;
            }
        }
        Ok(())
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EncodedRecording {
    schema: u32,
    session: SessionId,
    origin: MonoTime,
    batch: EncodedBatch,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EncodedBatch {
    items: Vec<StoredItem>,
    text: String,
}

/// 借用不可变录制的单向游标；同一录制可建立独立回放，但不能倒退此游标。
///
/// 时间戳相对起点重映射，保留原序号、采样时间差、缺口与取消。调用者消费并清空
/// 输出后可用同一截止时间继续交付积压；不能因一批满额而跨过未交付边沿。
pub struct Replay<'a> {
    recording: &'a Recording,
    destination_origin: MonoTime,
    cursor: usize,
    last_time: ReplayTime,
    limits: ReplayLimits,
}

#[derive(Clone, Copy, Debug)]
/// 每次输出批次可容纳的事件和文本额度；不是整个录制的大小上限。
pub struct ReplayLimits {
    /// 命令与缺口的合计上限，必须非零。
    pub max_items: usize,
    /// UTF-8 字节上限，必须位于 `1..=u32::MAX`。
    pub max_text_bytes: usize,
}

impl Default for ReplayLimits {
    fn default() -> Self {
        Self {
            max_items: 4096,
            max_text_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// 本次读取的进展；零交付也可能只是在等待后续时间。
pub struct ReplayProgress {
    /// 本次追加到输出的事件数，不包括原有内容。
    pub delivered: usize,
    /// 全部录制事件是否已交付；不只表示当前截止点处理完毕。
    pub finished: bool,
}

impl Replay<'_> {
    /// 追加接收时间不晚于相对截止点的事件，直到批次额度或截止时间到达。
    ///
    /// 截止时间必须非递减。输出已有内容也计入额度；消费并清空后可使用同一截止时间继续。
    /// 时间映射、总序、输出额度或单条载荷超限时失败；已交付前缀保留，游标停在首条
    /// 未交付事件。零交付不代表结束，必须检查 [`ReplayProgress::finished`]。
    pub fn read_until(
        &mut self,
        time: ReplayTime,
        out: &mut CommandBatch,
    ) -> Result<ReplayProgress, ReplayError> {
        if time < self.last_time {
            return Err(ReplayError::PlaybackOrder);
        }
        if out.len() > self.limits.max_items || out.text_bytes() > self.limits.max_text_bytes {
            return Err(ReplayError::OutputBudget);
        }
        let cutoff = self
            .recording
            .origin
            .checked_add(time.get())
            .ok_or(ReplayError::TimeOverflow)?;
        let start = self.cursor;
        for item in &self.recording.batch.items[self.cursor..] {
            if item.stamp().received > cutoff {
                break;
            }
            if out.len() == self.limits.max_items {
                break;
            }
            let mut stamp = item.stamp();
            stamp.received = rebase(
                stamp.received,
                self.recording.origin,
                self.destination_origin,
            )?;
            stamp.sampled = stamp
                .sampled
                .map(|sample| {
                    rebase(sample.at, self.recording.origin, self.destination_origin)
                        .map(|at| SampleTime { at, ..sample })
                })
                .transpose()?;
            validate_order(
                stamp,
                out.items.last().map(|item| item.stamp()),
                self.destination_origin,
            )?;
            match item.map_stamp(stamp) {
                StoredItem::Command(command) => {
                    let kind = command.kind.map(|span| {
                        &self.recording.batch.text[span.start as usize..span.end as usize]
                    });
                    let bytes = kind.text().map_or(0, str::len);
                    if bytes > self.limits.max_text_bytes {
                        return Err(ReplayError::ItemTooLarge {
                            bytes,
                            limit: self.limits.max_text_bytes,
                        });
                    }
                    if bytes > self.limits.max_text_bytes - out.text_bytes() {
                        break;
                    }
                    if out
                        .text
                        .len()
                        .checked_add(bytes)
                        .is_none_or(|total| total > u32::MAX as usize)
                    {
                        return Err(ReplayError::TextLimit);
                    }
                    out.items.try_reserve(1).map_err(InputError::from)?;
                    out.text.try_reserve(bytes).map_err(InputError::from)?;
                    out.push(CommandRef {
                        source: command.source,
                        stamp,
                        kind,
                    });
                }
                gap => {
                    out.items.try_reserve(1).map_err(InputError::from)?;
                    out.items.push(gap);
                }
            }
            self.cursor += 1;
        }
        self.last_time = time;
        Ok(ReplayProgress {
            delivered: self.cursor - start,
            finished: self.cursor == self.recording.batch.items.len(),
        })
    }
}

fn validate_order(
    stamp: Stamp,
    previous: Option<Stamp>,
    origin: MonoTime,
) -> Result<(), ReplayError> {
    if stamp.sequence.get() == 0
        || previous.is_some_and(|previous| stamp.sequence <= previous.sequence)
    {
        return Err(ReplayError::SequenceOrder);
    }
    if stamp.received < origin
        || previous.is_some_and(|previous| stamp.received < previous.received)
    {
        return Err(ReplayError::TimeOrder);
    }
    Ok(())
}

fn rebase(time: MonoTime, from: MonoTime, to: MonoTime) -> Result<MonoTime, ReplayError> {
    let value = i128::from(to.get()) + i128::from(time.get()) - i128::from(from.get());
    Ok(MonoTime::new(
        u64::try_from(value).map_err(|_| ReplayError::TimeOverflow)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionId, ActionPhase, CommandKind, GapReason, Incoming, InputHub, InputItemRef, Limits,
        Sequence, SourceId,
    };

    fn action(time: u64, phase: ActionPhase) -> Incoming<'static> {
        Incoming {
            source: SourceId::new(1),
            received: MonoTime::new(time),
            sampled: Some(SampleTime {
                at: MonoTime::new(time - 2),
                uncertainty_ns: 1,
            }),
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
        hub
    }

    #[test]
    fn replay_preserves_relative_timing() {
        let mut hub = hub(2);
        hub.push(action(110, ActionPhase::Press)).unwrap();
        hub.push(action(115, ActionPhase::Release)).unwrap();
        let mut batch = CommandBatch::new();
        hub.drain_into(&mut batch).unwrap();
        let mut recorder = Recorder::new(SessionId::new(1), MonoTime::new(100));
        recorder.append(&batch).unwrap();
        let recording = recorder.finish();
        let mut replay = recording.replay(MonoTime::new(1000));
        batch.clear();
        let progress = replay.read_until(ReplayTime::new(10), &mut batch).unwrap();
        assert_eq!(progress.delivered, 1);
        assert_eq!(
            batch.iter().next().unwrap().stamp(),
            Stamp {
                sequence: Sequence::new(1),
                received: MonoTime::new(1010),
                sampled: Some(SampleTime {
                    at: MonoTime::new(1008),
                    uncertainty_ns: 1
                }),
            }
        );
        assert!(
            replay
                .read_until(ReplayTime::new(15), &mut batch)
                .unwrap()
                .finished
        );
    }

    #[test]
    fn replay_keeps_input_gap() {
        let mut hub = hub(1);
        hub.push(action(10, ActionPhase::Press)).unwrap();
        assert!(hub.push(action(11, ActionPhase::Release)).is_err());
        let mut batch = CommandBatch::new();
        hub.drain_into(&mut batch).unwrap();
        let mut recorder = Recorder::new(SessionId::new(1), MonoTime::new(0));
        recorder.append(&batch).unwrap();
        let recording = recorder.finish();
        let mut replay = recording.replay(MonoTime::new(0));
        batch.clear();
        replay.read_until(ReplayTime::new(11), &mut batch).unwrap();
        assert!(
            matches!(batch.iter().next(), Some(InputItemRef::Gap(gap)) if gap.reason == GapReason::CommandsFull)
        );
    }

    #[test]
    fn replay_catchup_obeys_batch_limit() {
        let mut hub = hub(2);
        hub.push(action(10, ActionPhase::Press)).unwrap();
        hub.push(action(11, ActionPhase::Release)).unwrap();
        let mut batch = CommandBatch::new();
        hub.drain_into(&mut batch).unwrap();
        let mut recorder = Recorder::new(SessionId::new(1), MonoTime::new(0));
        recorder.append(&batch).unwrap();
        let recording = recorder.finish();
        let mut replay = recording
            .replay_with_limits(
                MonoTime::new(0),
                ReplayLimits {
                    max_items: 1,
                    max_text_bytes: 32,
                },
            )
            .unwrap();
        batch.clear();
        assert_eq!(
            replay.read_until(ReplayTime::new(11), &mut batch).unwrap(),
            ReplayProgress {
                delivered: 1,
                finished: false
            }
        );
        batch.clear();
        assert_eq!(
            replay.read_until(ReplayTime::new(11), &mut batch).unwrap(),
            ReplayProgress {
                delivered: 1,
                finished: true
            }
        );
        assert_eq!(
            batch.iter().next().unwrap().stamp().sequence,
            Sequence::new(2)
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn recording_rejects_invalid_utf8_span() {
        let json = r#"{"schema":1,"session":9,"origin":0,"batch":{"items":[{"command":{"source":1,"stamp":{"sequence":1,"received":12,"sampled":null},"kind":{"text":{"view":1,"session":1,"edit":{"commit":{"start":1,"end":3}}}}}}],"text":"中"}}"#;
        assert!(matches!(
            Recording::from_json(json),
            Err(ReplayError::Input(InputError::InvalidTextRange))
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn schema_one_fixture_is_readable() {
        let json = r#"{"schema":1,"session":9,"origin":0,"batch":{"items":[{"command":{"source":1,"stamp":{"sequence":1,"received":12,"sampled":null},"kind":{"action":{"action":7,"phase":"press"}}}}],"text":""}}"#;
        let recording = Recording::from_json(json).unwrap();
        let reparsed = Recording::from_json(&recording.to_json().unwrap()).unwrap();
        assert_eq!(reparsed, recording);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn unknown_schema_is_rejected_before_payload() {
        assert!(matches!(
            Recording::from_json(r#"{"schema":2}"#),
            Err(ReplayError::UnsupportedSchema(2))
        ));
    }
}
