//! 时间状态的独立失效契约；输入比较、存储替换和失效策略归应用所有者。
//!
//! 四个消费者分别接受历史。失效只清除指定消费者的有效位，不推进或回退随机样本、
//! jitter、帧序号及GPU完成水位。录制失败或取消不能调用[`State::accept`]或
//! [`State::accept_omission`]；
//! 宿主提交事务成功后才发布新历史。路径、颜色、静态均值和重建之间没有隐式依赖。

/// 确认旧语义域不可继续使用的封闭原因；常规帧更新不能隐式产生这些事件。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalReset {
    /// 切换世界，旧历史不属于当前世界。
    WorldReplaced,
    /// 场景身份命名空间整体替换。
    SceneDomainReplaced,
    /// 实时、离线或积分器的渲染域改变。
    RenderDomainChanged,
    /// 调用者明确请求失效，不由相机或照明变化阈值推导。
    ExplicitRequested,
}

impl GlobalReset {
    /// 日志使用的固定事件名。
    pub const fn name(self) -> &'static str {
        match self {
            Self::WorldReplaced => "world_replaced",
            Self::SceneDomainReplaced => "scene_domain_replaced",
            Self::RenderDomainChanged => "render_domain_changed",
            Self::ExplicitRequested => "explicit_request",
        }
    }
}

/// 可以独立失效的时间消费者；曝光适应不属于这些域。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum History {
    /// 固定输入下的普通 PT 线性颜色均值。
    Accumulation = 1,
    /// RR 等重建器的输入与相机历史。
    Reconstruction = 2,
    /// ReSTIR 路径 reservoir 及相应前帧表面。
    Paths = 4,
    /// ReSTCV 像素颜色估计；不等同于路径赢家。
    Color = 8,
}

/// 时间消费者集合，只能由有效消费者构造，不接受任意位模式。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Histories(u8);

impl Histories {
    /// 没有消费者。
    pub const NONE: Self = Self(0);
    /// 全部四个消费者；不包括随机身份、jitter 和曝光适应。
    pub const ALL: Self = Self(15);

    /// 单个消费者。
    pub const fn one(history: History) -> Self {
        Self(history as u8)
    }

    /// 合并两个显式选择的集合。
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// 是否包含该消费者。
    pub const fn contains(self, history: History) -> bool {
        self.0 & history as u8 != 0
    }
}

/// 所有者显式选择的失效范围与原因；不包含任何输入比较或自动扩大范围的逻辑。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Invalidation {
    histories: Histories,
    reason: GlobalReset,
}

impl Invalidation {
    /// 失效指定消费者；空集合是无操作。
    pub const fn new(histories: Histories, reason: GlobalReset) -> Self {
        Self { histories, reason }
    }

    /// 整个语义域失效的兼容入口。
    pub const fn all(reason: GlobalReset) -> Self {
        Self::new(Histories::ALL, reason)
    }

    /// 已明确选择的范围。
    pub const fn histories(self) -> Histories {
        self.histories
    }

    /// 已明确给出的原因。
    pub const fn reason(self) -> GlobalReset {
        self.reason
    }
}

/// 单个所有者维护的有效位；存储与提交事务由实际资源所有者维护。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct State {
    valid: Histories,
}

impl State {
    /// 新存储没有可接受的历史。
    pub const fn new() -> Self {
        Self {
            valid: Histories::NONE,
        }
    }

    /// 查询已接受历史，不推导其他消费者是否有效。
    pub const fn contains(self, history: History) -> bool {
        self.valid.contains(history)
    }

    /// 仅在宿主成功提交相应输出后接受；不表示 GPU 已完成。
    pub fn accept(&mut self, histories: Histories) {
        self.valid = self.valid.union(histories);
    }

    /// 生产者明确声明本次已接受帧没有这些输出；对应旧输出不再具有连续性。
    /// 这属于提交结果，不是根据输入推导的失效请求，不修改其他消费者或任何序号。
    pub fn accept_omission(&mut self, histories: Histories) {
        self.valid.0 &= !histories.0;
    }

    /// 只清除请求范围，返回此前有效且本次失效的消费者，不改变任何序号。
    pub fn invalidate(&mut self, request: Invalidation) -> Histories {
        let removed = Histories(self.valid.0 & request.histories.0);
        self.valid.0 &= !request.histories.0;
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidating_one_consumer_preserves_all_other_consumers() {
        for selected in [
            History::Accumulation,
            History::Reconstruction,
            History::Paths,
            History::Color,
        ] {
            let mut state = State::new();
            state.accept(Histories::ALL);
            state.invalidate(Invalidation::new(
                Histories::one(selected),
                GlobalReset::ExplicitRequested,
            ));
            for other in [
                History::Accumulation,
                History::Reconstruction,
                History::Paths,
                History::Color,
            ] {
                assert_eq!(state.contains(other), other != selected);
            }
        }
    }
}
