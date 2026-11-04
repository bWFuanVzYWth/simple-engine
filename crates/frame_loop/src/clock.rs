use std::time::{Duration, Instant};

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// 会话时间、期限或 tick 序号不能继续表示。
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ClockError {
    /// 频率不在 `1..=1_000_000_000` Hz，不能得到严格递增的纳秒期限。
    #[error("tick 频率须位于 1..=1000000000 Hz")]
    Frequency,
    /// 纳秒值或 tick 序号超出 `u64` 表示范围。
    #[error("会话时间或 tick 序号溢出")]
    Overflow,
    /// 目标时间超出平台 `Instant` 的表示范围。
    #[error("期限超出平台 Instant 范围")]
    InstantRange,
}

/// 以创建时刻为零点的只读单调时钟；复制保留同一 epoch 和会话域。
///
/// 所有纳秒值属于此实例的会话域，不是 Unix 时间，也不隐式映射暂停后的游戏时间。
#[derive(Clone, Copy, Debug)]
pub struct SessionClock {
    epoch: Instant,
}

impl Default for SessionClock {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionClock {
    /// 读取平台单调时钟建立会话零点，不分配。
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }

    /// 返回会话开始后的整数纳秒；约584年后超出 `u64` 时返回错误。
    pub fn now_ns(&self) -> Result<u64, ClockError> {
        u64::try_from(self.epoch.elapsed().as_nanos()).map_err(|_| ClockError::Overflow)
    }

    /// 将本会话绝对纳秒转换为平台等待期限；不可表示时返回错误。
    pub fn instant(&self, nanos: u64) -> Result<Instant, ClockError> {
        self.epoch
            .checked_add(Duration::from_nanos(nanos))
            .ok_or(ClockError::InstantRange)
    }
}

/// 固定频率的绝对期限序列；自身不读时钟、不等待、不调度。
///
/// 目标序号从1开始，期限为 `epoch_ns + floor(index * 1_000_000_000 / hz)`。
/// 每次由原始 epoch 和序号计算，不能整除的周期不会累计截断误差。
/// 调用方在完成当前周期后调用 [`Self::advance`]；落后时是否追赶由应用决定。
#[derive(Clone, Debug)]
pub struct TickClock {
    epoch_ns: u64,
    hz: u32,
    index: u64,
    deadline_ns: u64,
}

impl TickClock {
    /// 校验频率及第一个期限；epoch 属于调用方共享的会话纳秒域。
    ///
    /// 非法频率返回 [`ClockError::Frequency`]，第一个期限超出 `u64` 返回溢出。
    pub const fn new(epoch_ns: u64, hz: u32) -> Result<Self, ClockError> {
        if hz == 0 || hz as u64 > NANOS_PER_SECOND {
            return Err(ClockError::Frequency);
        }
        let deadline_ns = match deadline(epoch_ns, hz, 1) {
            Ok(value) => value,
            Err(error) => return Err(error),
        };
        Ok(Self {
            epoch_ns,
            hz,
            index: 1,
            deadline_ns,
        })
    }

    /// 当前等待目标的序号，从1开始，不代表已完成的模拟次数。
    #[must_use]
    pub const fn index(&self) -> u64 {
        self.index
    }

    /// 当前目标的绝对会话纳秒；调用方可用 [`SessionClock::instant`] 转换。
    #[must_use]
    pub const fn deadline_ns(&self) -> u64 {
        self.deadline_ns
    }

    /// 推进到下一个绝对期限；序号或时间溢出时保持当前目标不变。
    pub const fn advance(&mut self) -> Result<(), ClockError> {
        let Some(index) = self.index.checked_add(1) else {
            return Err(ClockError::Overflow);
        };
        let deadline_ns = match deadline(self.epoch_ns, self.hz, index) {
            Ok(value) => value,
            Err(error) => return Err(error),
        };
        self.index = index;
        self.deadline_ns = deadline_ns;
        Ok(())
    }
}

const fn deadline(epoch: u64, hz: u32, index: u64) -> Result<u64, ClockError> {
    let nanos = epoch as u128 + index as u128 * NANOS_PER_SECOND as u128 / hz as u128;
    if nanos > u64::MAX as u128 {
        Err(ClockError::Overflow)
    } else {
        Ok(nanos as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_divisible_frequency_returns_to_exact_seconds() {
        let mut ticks = TickClock::new(7, 30).unwrap();
        let mut previous = 7;
        for index in 1..=300 {
            assert_eq!(ticks.index(), index);
            assert_eq!(ticks.deadline_ns(), 7 + index * NANOS_PER_SECOND / 30);
            assert!(ticks.deadline_ns() > previous);
            previous = ticks.deadline_ns();
            ticks.advance().unwrap();
        }
    }

    #[test]
    fn large_index_keeps_integer_precision() {
        let index = (1_u64 << 53) + 1;
        assert_eq!(deadline(17, 1_000_000_000, index), Ok(17 + index));
    }

    #[test]
    fn overflow_preserves_current_target() {
        let mut ticks = TickClock::new(u64::MAX - 1, 1_000_000_000).unwrap();
        assert_eq!(ticks.advance(), Err(ClockError::Overflow));
        assert_eq!(ticks.index(), 1);
        assert_eq!(ticks.deadline_ns(), u64::MAX);
    }

    #[test]
    fn frequency_requires_strictly_increasing_nanosecond_targets() {
        for hz in [0, 1_000_000_001, u32::MAX] {
            assert!(matches!(TickClock::new(0, hz), Err(ClockError::Frequency)));
        }
    }

    #[test]
    fn session_conversion_uses_the_same_epoch() {
        let clock = SessionClock::new();
        let first = clock.now_ns().unwrap();
        let second = clock.now_ns().unwrap();
        assert!(second >= first);
        assert_eq!(
            clock.instant(first).unwrap() - clock.epoch,
            Duration::from_nanos(first)
        );
    }
}
