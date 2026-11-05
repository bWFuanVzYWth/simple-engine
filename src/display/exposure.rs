// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

//! 场景线性 RGB 的对数直方图计量与时域曝光，不持有 GPU 或应用状态。
//!
//! 测光使用未乘材质反照率的线性颜色，所有有效像素等权，黑色也参与计量；
//! 因而明暗材质占比会影响结果。BT.709 输入先转 Rec.2020，再裁剪负通道并计亮度。
//! 两端各丢弃 0.5% 样本，按桶中心求平均 log2 亮度并保留场景明暗倾向。
//! 自动曝光 EV 只在显示阶段转换为 `2^EV`，与手动曝光倍率相乘，不改变线性累积。
//! `compensation` 表示自动调节强度，0 为无自动调节，1 为全部采用计量结果。
//! GPU 对应模块为 `display.exposure`；资源清零、阶段同步与冻结策略由调用方维护。

/// native Vulkan 的设备内直方图与时域状态；不回读曝光、不额外提交。
#[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
#[path = "exposure_gpu.rs"]
pub mod gpu;

/// 自动曝光默认适应强度。
pub const DEFAULT_COMPENSATION: f32 = 0.65;

/// 自动曝光帧参数的输入错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// 图像为空或像素总数超过 u32，无法使用 GPU 直方图计数。
    #[error("自动曝光图像须非空且像素总数不超过 u32::MAX")]
    Extent,
    /// 帧间隔不是有限非负秒数。
    #[error("自动曝光帧间隔须为有限非负秒数")]
    DeltaSeconds,
    /// 自动调节强度不在有限范围 `0..=1`。
    #[error("自动曝光调节强度须在 0..=1")]
    Compensation,
    /// 宿主时间早于最后成功提交的曝光帧。
    #[error("曝光帧时间不能早于最后成功提交的帧")]
    TimeOrder,
}

/// 曝光的提交时序；宿主提供同一单调时钟的时间，不在引擎内读取系统时钟。
///
/// 每次录制先调用 [`Self::prepare`]，实际提交或外部命令编码成功后调用
/// [`Self::submitted`]；录制或提交取消时调用 [`Self::cancel`]。未提交帧不推进适应时间。
/// 尺寸、相机或 PT 历史变化不重置本状态。强度在正值间变化时保留适应历史，
/// 启用或停用测光时使下一帧重新建立曝光。
/// 克隆只用于准备替换视图，发布后仍由单个宿主所有者顺序维护。
#[derive(Clone, Debug)]
pub struct Timeline {
    last: Option<std::time::Duration>,
    pending: Option<std::time::Duration>,
    compensation: f32,
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Timeline {
    /// 创建尚无提交历史的时序，开启采用完整测光强度，关闭使用恒等曝光倍率。
    pub const fn new(enabled: bool) -> Self {
        Self {
            last: None,
            pending: None,
            compensation: if enabled { 1.0 } else { 0.0 },
        }
    }

    /// 是否执行自动测光；手动 EV 不属于本开关。
    pub const fn enabled(&self) -> bool {
        self.compensation > 0.0
    }

    /// 当前自动调节强度；供宿主在冻结画面时构造独立的即时测光参数。
    pub const fn compensation(&self) -> f32 {
        self.compensation
    }

    /// 选择完整或零自动调节强度。启用或停用测光时重置适应历史。
    pub fn set_enabled(&mut self, enabled: bool) {
        self.set_compensation(if enabled { 1.0 } else { 0.0 })
            .expect("开关只使用有效强度");
    }

    /// 将下一帧自动曝光目标的 EV 按 `compensation` 缩放，保留手动 EV 的独立性。
    /// 正值之间的修改保留适应历史，0 禁用测光。
    /// # Errors
    /// 强度不是 `0..=1` 内的有限值时返回 [`Error::Compensation`]，保持先前状态。
    pub fn set_compensation(&mut self, compensation: f32) -> Result<(), Error> {
        if !(0.0..=1.0).contains(&compensation) {
            return Err(Error::Compensation);
        }
        if self.enabled() != (compensation > 0.0) {
            self.last = None;
            self.pending = None;
        }
        self.compensation = compensation;
        Ok(())
    }

    /// 准备一次录制的测光参数；失败保留先前状态，重新准备替换未提交帧。
    ///
    /// `now` 来自宿主固定原点的单调时间。首次提交立即采用测光目标，后续帧
    /// 从最后成功提交时间计算适应间隔；取消帧不缩短下一帧间隔。
    /// # Errors
    /// 时间倒退返回 [`Error::TimeOrder`]，图像范围错误沿用 [`Frame::new`]。
    pub fn prepare(
        &mut self,
        extent: [u32; 2],
        linear709: bool,
        now: std::time::Duration,
    ) -> Result<Frame, Error> {
        let delta = match self.last {
            Some(last) => now.checked_sub(last).ok_or(Error::TimeOrder)?,
            None => std::time::Duration::ZERO,
        };
        let frame = Frame::new(
            extent,
            linear709,
            delta.as_secs_f32(),
            self.last.is_none(),
            !self.enabled(),
            self.compensation,
        )?;
        self.pending = Some(now);
        Ok(frame)
    }

    /// 接受已成功提交或交给外部宿主的帧；没有待提交帧时保持历史。
    pub fn submitted(&mut self) {
        if let Some(now) = self.pending.take() {
            self.last = Some(now);
        }
    }

    /// 丢弃未提交帧的时间，不改变最后成功提交的历史。
    pub fn cancel(&mut self) {
        self.pending = None;
    }
}

/// 已校验的单帧计量与更新参数；不规定应用何时重置或冻结曝光。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    words: [u32; 8],
}

impl Frame {
    /// 校验图像、帧间隔与自动调节强度，不裁剪输入。
    ///
    /// `linear709` 为真表示输入为 D65 线性 BT.709，否则为线性 Rec.2020。
    /// `reset`、`instant` 或未初始化状态会直接采用本帧目标，不做时域平滑。
    /// # Errors
    /// 空图像、像素数溢出、非有限或负间隔、超出 `0..=1` 的强度返回对应 [`Error`]。
    pub const fn new(
        extent: [u32; 2],
        linear709: bool,
        delta_seconds: f32,
        reset: bool,
        instant: bool,
        compensation: f32,
    ) -> Result<Self, Error> {
        if extent[0] == 0 || extent[1] == 0 || extent[0] as u64 * extent[1] as u64 > u32::MAX as u64
        {
            return Err(Error::Extent);
        }
        if !delta_seconds.is_finite() || delta_seconds < 0.0 {
            return Err(Error::DeltaSeconds);
        }
        if !compensation.is_finite() || compensation < 0.0 || compensation > 1.0 {
            return Err(Error::Compensation);
        }
        Ok(Self {
            words: [
                extent[0],
                extent[1],
                linear709 as u32,
                reset as u32,
                delta_seconds.to_bits(),
                compensation.to_bits(),
                instant as u32,
                0,
            ],
        })
    }

    /// 返回 32B shader 参数：宽、高、BT.709、重置、间隔 bits、强度 bits、立即更新、零。
    ///
    /// 仅数组内容定义 GPU ABI，Rust 类型布局不作为上传契约。
    pub const fn to_words(self) -> [u32; 8] {
        self.words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_exposure_frame_does_not_advance_adaptation() {
        use std::time::Duration;
        let mut timeline = Timeline::default();
        timeline
            .prepare([4, 4], true, Duration::from_secs(1))
            .unwrap();
        timeline.submitted();
        timeline
            .prepare([4, 4], true, Duration::from_secs(2))
            .unwrap();
        timeline.cancel();
        let next = timeline
            .prepare([4, 4], true, Duration::from_secs(4))
            .unwrap();
        assert_eq!(f32::from_bits(next.to_words()[4]), 3.0);
        assert_eq!(next.to_words()[3], 0);
    }

    #[test]
    fn exposure_toggle_reestablishes_target() {
        use std::time::Duration;
        let mut timeline = Timeline::default();
        timeline.prepare([4, 4], true, Duration::ZERO).unwrap();
        timeline.submitted();
        timeline.set_enabled(false);
        let manual = timeline
            .prepare([4, 4], true, Duration::from_secs(1))
            .unwrap();
        assert_eq!(manual.to_words()[5], 0.0f32.to_bits());
        timeline.submitted();
        timeline.set_enabled(true);
        let automatic = timeline
            .prepare([4, 4], true, Duration::from_secs(2))
            .unwrap();
        assert_eq!(automatic.to_words()[3], 1);
        assert_eq!(automatic.to_words()[5], 1.0f32.to_bits());
    }

    #[test]
    fn positive_strength_changes_target_with_existing_adaptation_time() -> Result<(), Error> {
        use std::time::Duration;
        let mut timeline = Timeline::default();
        timeline.prepare([4, 4], true, Duration::from_secs(1))?;
        timeline.submitted();
        timeline.set_compensation(0.65)?;
        assert_eq!(
            timeline.set_compensation(f32::NAN),
            Err(Error::Compensation)
        );
        let next = timeline.prepare([4, 4], true, Duration::from_secs(3))?;
        assert_eq!(
            next.to_words()[3..6],
            [0, 2.0f32.to_bits(), 0.65f32.to_bits()]
        );
        Ok(())
    }

    #[test]
    fn frame_rejects_invalid_inputs() {
        assert_eq!(
            Frame::new([0, 1], false, 0.0, false, false, 1.0),
            Err(Error::Extent)
        );
        assert_eq!(
            Frame::new([65536, 65536], false, 0.0, false, false, 1.0),
            Err(Error::Extent)
        );
        assert_eq!(
            Frame::new([1, 1], false, f32::NAN, false, false, 1.0),
            Err(Error::DeltaSeconds)
        );
        assert_eq!(
            Frame::new([1, 1], false, -0.1, false, false, 1.0),
            Err(Error::DeltaSeconds)
        );
        assert_eq!(
            Frame::new([1, 1], false, 0.0, false, false, f32::INFINITY),
            Err(Error::Compensation)
        );
        assert_eq!(
            Frame::new([1, 1], false, 0.0, false, false, 1.1),
            Err(Error::Compensation)
        );
    }

    #[test]
    fn frame_words_preserve_upload_values() {
        let frame = Frame::new([u32::MAX, 1], true, 0.125, true, false, 0.75).unwrap();
        assert_eq!(
            frame.to_words(),
            [
                u32::MAX,
                1,
                1,
                1,
                0.125f32.to_bits(),
                0.75f32.to_bits(),
                0,
                0
            ]
        );
    }
}
