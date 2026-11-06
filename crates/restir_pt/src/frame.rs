use crate::Error;
use pt::history::{Histories, History, Invalidation, State};
use std::sync::atomic::{AtomicU64, Ordering};

/// 外部frame参数更新前读取的路径历史与帧序号快照。
/// 颜色连续性由[`FrameState::color_history_valid`]独立提供，不从路径有效位推导。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameParams {
    /// 已成功提交的帧数；本次 shader 使用此序号，不允许 u32 回绕。
    pub frame_index: u32,
    /// false 时跳过 temporal，仍完整执行 initial 与 spatial。
    pub history_valid: bool,
}

/// 本帧颜色估计的明确生产结果，与路径有效性及算法开关独立。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorOutput {
    /// 完整颜色估计已写入对应工作集；成功提交后可供下一帧读取。
    Produced,
    /// 本帧未生产颜色估计；成功提交后旧颜色不再连续，取消则保留原状态。
    Omitted,
}

/// 单个 owner 发行的帧事务票据；复制不复制状态，确认后所有副本失效。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameToken {
    owner: u64,
    serial: u64,
}

/// 唯一的已提交/待确认帧状态；GPU 完成水位仍由提交 owner 管理。
pub struct FrameState {
    owner: u64,
    serial: u64,
    frames: u32,
    history: State,
    pending: Option<FrameToken>,
}

impl FrameState {
    #[cfg(feature = "vulkan")]
    pub(crate) const fn owner(&self) -> u64 {
        self.owner
    }
    /// 创建独立视图身份；owner 序号耗尽时显式失败，不复用旧 token 身份。
    pub fn new() -> Result<Self, Error> {
        static OWNERS: AtomicU64 = AtomicU64::new(0);
        let owner = OWNERS
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Error::Overflow)?;
        Ok(Self {
            owner,
            serial: 0,
            frames: 0,
            history: State::new(),
            pending: None,
        })
    }

    /// 下次录制使用的帧快照；可先上传外部相机/参数，随后调用 begin 或 Renderer::record。
    pub const fn next_frame_params(&self) -> Result<FrameParams, Error> {
        if self.pending.is_some() {
            return Err(Error::PendingFrame);
        }
        if self.frames == u32::MAX || self.serial == u64::MAX {
            return Err(Error::Overflow);
        }
        Ok(FrameParams {
            frame_index: self.frames,
            history_valid: self.history.contains(History::Paths),
        })
    }

    /// 上一个已接受帧是否生产了可读的颜色估计；待确认帧不改变该状态。
    /// 与路径历史独立，消费者仍须满足其算法对路径、颜色和当前开关的联合条件。
    pub const fn color_history_valid(&self) -> bool {
        self.history.contains(History::Color)
    }

    /// 发行唯一待确认帧；失败不改变状态。
    pub fn begin(&mut self) -> Result<(FrameToken, FrameParams), Error> {
        let params = self.next_frame_params()?;
        self.serial += 1;
        let token = FrameToken {
            owner: self.owner,
            serial: self.serial,
        };
        self.pending = Some(token);
        Ok((token, params))
    }

    fn validate(&self, token: FrameToken) -> Result<(), Error> {
        if self.pending != Some(token) {
            return Err(Error::StaleFrame);
        }
        Ok(())
    }

    /// 兼容不生产颜色的消费者，等同于以 [`ColorOutput::Omitted`] 提交。
    /// 仅在宿主成功提交完整帧后确认；本方法不声称 GPU 已完成。
    pub fn commit(&mut self, token: FrameToken) -> Result<(), Error> {
        self.commit_with_color(token, ColorOutput::Omitted)
    }

    /// 原子接受路径和明确颜色输出；票据无效时不发布任一消费者或帧序号。
    /// 只有实际成功提交完整帧后才能调用；不证明 GPU 已完成。
    pub fn commit_with_color(
        &mut self,
        token: FrameToken,
        color: ColorOutput,
    ) -> Result<(), Error> {
        self.validate(token)?;
        self.pending = None;
        self.frames += 1;
        self.history.accept(Histories::one(History::Paths));
        match color {
            ColorOutput::Produced => self.history.accept(Histories::one(History::Color)),
            ColorOutput::Omitted => self.history.accept_omission(Histories::one(History::Color)),
        }
        Ok(())
    }

    /// 宿主已取消待执行命令后撤销事务，保留原帧数和历史身份。
    pub fn reject(&mut self, token: FrameToken) -> Result<(), Error> {
        self.validate(token)?;
        self.pending = None;
        Ok(())
    }

    /// 显式选择 Paths 和/或 Color 独立失效；帧序号和随机身份继续推进。
    /// 待确认帧存在时拒绝；其他消费者及 GPU 工作由其实际所有者处理。
    pub fn invalidate_history(&mut self, request: Invalidation) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(Error::PendingFrame);
        }
        self.history.invalidate(request);
        Ok(())
    }
}
