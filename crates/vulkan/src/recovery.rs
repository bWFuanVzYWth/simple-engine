//! SDK 恢复分为重配与待发送 reset；输入失败不消费 reset，成功录制后才恢复正常。
//! 取消的 SDK 命令可能包含尚未执行的私有图像初始化，必须先释放旧 feature，不能仅改常量。

#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "proof/recovery.rs"]
#[cfg(verus_only)]
mod proof;

/// 必须释放旧 feature 资源并重配的封闭原因。
#[derive(Clone, Copy, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum RecoveryCause {
    /// SDK 已录制的命令未提交而被取消。
    RecordingCancelled,
    /// configure 未成功建立可用配置。
    ConfigurationFailed,
    /// 录制容器失败，或 SDK 报告 command 污染。
    RecordingFailed,
    /// SDK 普通失败；返回的命令仍须提交或取消。
    EvaluationFailed,
}

/// 同一 Runtime 的恢复阶段；重配成功不等于 SDK 已录制或 GPU 已完成。
#[derive(Clone, Copy)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum Recovery {
    /// 当前没有待处理的恢复请求。
    Ready,
    /// 旧 GPU 工作结束后才能重配。
    NeedsReconfigure(RecoveryCause),
    /// 重配已完成，reset 尚未交给成功录制。
    ResetPending,
}

#[cfg_attr(verus_only, verus_verify)]
impl Recovery {
    /// 尚无恢复请求；Runtime 的存在和配置由 Session 另行维护。
    #[cfg_attr(verus_only, verus_spec(state => ensures state is Ready))]
    pub(crate) const fn new() -> Self {
        Self::Ready
    }

    /// 取消、配置或 SDK 失败使旧 feature 不能继续录制。
    #[cfg_attr(verus_only, verus_spec(
        ensures *final(self) == Self::NeedsReconfigure(cause),
    ))]
    pub(crate) const fn invalidate(&mut self, cause: RecoveryCause) {
        *self = Self::NeedsReconfigure(cause);
    }

    /// 仅返回尚未执行的重配请求；等待 reset 时不重复释放 feature。
    #[cfg_attr(verus_only, verifier::allow_in_spec)]
    #[cfg_attr(verus_only, verus_spec(reason =>
        returns proof::reconfigure_reason(*self),
    ))]
    pub(crate) const fn reconfigure_reason(&self) -> Option<RecoveryCause> {
        match *self {
            Self::NeedsReconfigure(cause) => Some(cause),
            _ => None,
        }
    }

    /// 外部 configure 已成功；reset 仍须交给下一次成功的 SDK 录制。
    #[cfg_attr(verus_only, verus_spec(
        requires *old(self) is NeedsReconfigure,
        ensures *final(self) is ResetPending,
    ))]
    pub(crate) const fn reconfigured(&mut self) {
        *self = Self::ResetPending;
    }

    /// 是否覆盖调用方常量，向 SDK 显式发送 reset=1。
    #[cfg_attr(verus_only, verifier::allow_in_spec)]
    #[cfg_attr(verus_only, verus_spec(required => returns *self is ResetPending))]
    pub(crate) const fn reset_required(&self) -> bool {
        matches!(self, Self::ResetPending)
    }

    /// SDK 已成功录制且可返回命令；之后若取消该命令，须再次 invalidate。
    #[cfg_attr(verus_only, verus_spec(
        requires !(*old(self) is NeedsReconfigure),
        ensures *final(self) is Ready,
    ))]
    pub(crate) const fn recorded(&mut self) {
        *self = Self::Ready;
    }
}

#[cfg(test)]
mod tests {
    use super::{Recovery, RecoveryCause};

    #[test]
    fn reset_remains_pending_until_successful_recording() {
        let mut recovery = Recovery::new();
        recovery.invalidate(RecoveryCause::RecordingCancelled);
        recovery.reconfigured();
        assert!(recovery.reconfigure_reason().is_none());
        assert!(recovery.reset_required());
        recovery.recorded();
        assert!(!recovery.reset_required());
    }
}
