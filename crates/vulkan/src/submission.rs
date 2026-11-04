//! 提交序号、记录阶段与设备失效的同源状态核；不把完成通知解释成计算结果正确。

#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "proof/submission.rs"]
#[cfg(verus_only)]
mod proof;

/// 使提交所有者停止正常使用的首个故障类别；原始错误由触发操作返回。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(verus_only, verus_verify)]
pub enum FailureKind {
    /// wgpu 拒绝命令或资源描述。
    Validation,
    /// wgpu 内存申请失败。
    OutOfMemory,
    /// wgpu 后端内部错误。
    Internal,
    /// 设备丢失或被销毁。
    Lost,
}

#[derive(Clone, Copy)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Active {}

#[derive(Clone, Copy)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Failed {
    kind: FailureKind,
}

/// 失效为终态；不能经由普通操作重新取得 Active。
#[derive(Clone, Copy)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum DeviceState {
    Active(Active),
    Failed(Failed),
}

#[cfg_attr(verus_only, verus_verify)]
impl DeviceState {
    /// 创建尚未记录故障的设备阶段；失效后由所有者禁止重新初始化。
    #[cfg_attr(verus_only, verus_spec(state =>
        ensures state is Active, state.failure_view().is_none(),
    ))]
    pub(crate) const fn new() -> Self {
        Self::Active(Active {})
    }

    /// 返回第一个故障类别；不能由完成通知恢复为正常阶段。
    #[cfg_attr(verus_only, verifier::allow_in_spec)]
    #[cfg_attr(verus_only, verus_spec(kind =>
        returns self.failure_view(),
    ))]
    pub(crate) const fn failure(self) -> Option<FailureKind> {
        match self {
            Self::Active(_) => None,
            Self::Failed(failed) => Some(failed.kind),
        }
    }

    /// 首个故障使状态失效；后续故障不覆盖原类别。
    #[cfg_attr(verus_only, verus_spec(
        ensures proof::failed(*old(self), *final(self), kind),
    ))]
    pub(crate) const fn fail(&mut self, kind: FailureKind) {
        if let Self::Active(_) = self {
            *self = Self::Failed(Failed { kind });
        }
    }
}

/// Synchronous 需真实 host wait；Submitting 尚未取得验证结果，Uncertain 仅供故障清理。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum RecordState {
    Pending,
    External,
    Synchronous,
    Submitting(u64),
    Submitted(u64),
    Uncertain,
}

#[cfg_attr(verus_only, verus_verify)]
impl RecordState {
    /// 仅已接受的提交可由完成水位关闭；待验证和不确定记录须保守保留。
    #[cfg_attr(verus_only, verus_spec(done =>
        ensures done == proof::record_completed(self, serial),
    ))]
    pub(crate) const fn completed(self, serial: u64) -> bool {
        match self {
            Self::Submitted(n) => n <= serial,
            _ => false,
        }
    }

    /// Pending 进入正序号的提交预检阶段；其他阶段或序号零返回 None。
    #[cfg_attr(verus_only, verus_spec(next =>
        ensures next == proof::begin(self, serial),
    ))]
    pub(crate) const fn begin(self, serial: u64) -> Option<Self> {
        if matches!(self, Self::Pending) && serial > 0 {
            Some(Self::Submitting(serial))
        } else {
            None
        }
    }

    /// Pending 开始不占用管理序号的同步执行；完成事实由实际 host wait 提供。
    #[cfg_attr(verus_only, verus_spec(next =>
        ensures next == proof::begin_sync(self),
    ))]
    pub(crate) const fn begin_sync(self) -> Option<Self> {
        if matches!(self, Self::Pending) {
            Some(Self::Synchronous)
        } else {
            None
        }
    }

    /// 仅接受同一序号的 Submitting；不能把其他阶段或批次变成已提交。
    #[cfg_attr(verus_only, verus_spec(next =>
        ensures next == proof::accept_record(self, serial),
    ))]
    pub(crate) const fn accept(self, serial: u64) -> Option<Self> {
        match self {
            Self::Submitting(n) => {
                if n == serial {
                    Some(Self::Submitted(serial))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// 故障后隔离记录；普通完成水位不能释放它，由失效所有者受控清理。
    #[cfg_attr(verus_only, verus_spec(next =>
        returns Self::Uncertain,
    ))]
    pub(crate) const fn quarantine(self) -> Self {
        Self::Uncertain
    }
}

/// 序号仅统计本所有者接受的批次；底层队列可另有初始化/外部提交。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Timeline {
    submitted: u64,
    completed: u64,
}

#[cfg_attr(verus_only, verus_verify)]
impl Timeline {
    /// 初始无接受提交或完成水位。
    #[cfg_attr(verus_only, verus_spec(value => ensures proof::empty(&value), value.valid(),
        value.submitted_view() == 0, value.completed_view() == 0))]
    pub(crate) const fn new() -> Self {
        Self {
            submitted: 0,
            completed: 0,
        }
    }

    /// 提议下一个管理序号；用尽后返回 None，不回绕。
    #[cfg_attr(verus_only, verus_spec(serial =>
        ensures serial == proof::next_serial(self),
    ))]
    pub(crate) const fn next(&self) -> Option<u64> {
        self.submitted.checked_add(1)
    }

    /// 仅接受紧邻序号；失败不改变已接受提交和完成水位。
    #[cfg_attr(verus_only, verus_spec(accepted =>
        requires old(self).valid(),
        ensures proof::accepted_timeline(old(self), final(self), serial, accepted),
    ))]
    pub(crate) const fn accept(&mut self, serial: u64) -> bool {
        if self.submitted < u64::MAX && serial == self.submitted + 1 {
            self.submitted = serial;
            true
        } else {
            false
        }
    }

    /// 收取已接受范围内的完成事件；乱序或重复事件不降低水位，超前事件被拒绝。
    #[cfg_attr(verus_only, verus_spec(accepted =>
        requires old(self).valid(),
        ensures proof::observed(old(self), final(self), serial, accepted),
    ))]
    pub(crate) const fn observe(&mut self, serial: u64) -> bool {
        if serial > self.submitted {
            return false;
        }
        if serial > self.completed {
            self.completed = serial;
        }
        true
    }

    /// 最后接受的提交序号。
    #[cfg_attr(verus_only, verifier::allow_in_spec)]
    #[cfg_attr(verus_only, verus_spec(serial =>
        returns self.submitted_view(),
    ))]
    pub(crate) const fn submitted(&self) -> u64 {
        self.submitted
    }

    /// 已收取的单调完成水位，不表示计算结果正确。
    #[cfg_attr(verus_only, verifier::allow_in_spec)]
    #[cfg_attr(verus_only, verus_spec(serial =>
        returns self.completed_view(),
    ))]
    pub(crate) const fn completed(&self) -> u64 {
        self.completed
    }
}
