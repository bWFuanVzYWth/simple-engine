use crate::Error;
use std::sync::atomic::{AtomicU64, Ordering};

/// 外部 frame 参数更新前可读取的唯一历史快照。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameParams {
    /// 已成功提交的帧数；本次 shader 使用此序号，不允许 u32 回绕。
    pub frame_index: u32,
    /// false 时跳过 temporal，仍完整执行 initial 与 spatial。
    pub history_valid: bool,
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
    history_valid: bool,
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
            history_valid: false,
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
            history_valid: self.history_valid,
        })
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

    /// 仅在宿主成功提交完整帧后确认；本方法不声称 GPU 已完成。
    pub fn commit(&mut self, token: FrameToken) -> Result<(), Error> {
        self.validate(token)?;
        self.pending = None;
        self.frames += 1;
        self.history_valid = true;
        Ok(())
    }

    /// 宿主已取消待执行命令后撤销事务，保留原帧数和历史身份。
    pub fn reject(&mut self, token: FrameToken) -> Result<(), Error> {
        self.validate(token)?;
        self.pending = None;
        Ok(())
    }

    /// 令下一帧跳过 temporal，从帧零开始；待确认帧存在时拒绝。
    /// 宿主同时重置相机历史和外部累积，不能取消已提交的 GPU 工作。
    pub fn reset_history(&mut self) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(Error::PendingFrame);
        }
        self.frames = 0;
        self.history_valid = false;
        Ok(())
    }
}
