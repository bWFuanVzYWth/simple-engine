//! 应用无关的完整帧间隔统计与保峰抽样；不读取时钟或执行调度。
//!
//! 应用提供互斥阶段，数组最后一项接收未测量余量。CPU 完整间隔不混入并发 GPU 时长。

use std::{
    collections::{TryReserveError, VecDeque},
    time::Duration,
};

const REFRESH_INTERVAL_NS: u64 = 250_000_000;

/// 一个完整提交间隔的各阶段纳秒数；调用方须提供互斥计时。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameAttribution<const N: usize>(
    /// 按应用约定顺序排列；记录时裁剪，最后一段补齐未归因余量。
    pub [u64; N],
);

impl<const N: usize> Default for FrameAttribution<N> {
    fn default() -> Self {
        Self([0; N])
    }
}

impl<const N: usize> FrameAttribution<N> {
    /// 从应用固定顺序的纳秒数构造；不在此处裁剪。
    pub const fn new(values: [u64; N]) -> Self {
        Self(values)
    }

    /// 累计同一完整间隔内的阶段；包括未成功提交的尝试，溢出时饱和。
    /// `stage >= N` 时 panic。
    pub const fn add(&mut self, stage: usize, ns: u64) {
        self.0[stage] = self.0[stage].saturating_add(ns);
    }

    /// 读取单个阶段的纳秒数；`stage >= N` 时 panic。
    pub const fn get(self, stage: usize) -> u64 {
        self.0[stage]
    }

    fn fitted(self, elapsed_ns: u64) -> Self {
        let mut remaining = elapsed_ns;
        let mut result = Self::default();
        for (target, input) in result.0.iter_mut().zip(self.0) {
            let value = input.min(remaining);
            *target = value;
            remaining -= value;
        }
        if let Some(last) = result.0.last_mut() {
            *last += remaining;
        }
        result
    }
}

/// 一个有效完整提交间隔及其已裁剪归因，独立于滚动窗口保留。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameSample<const N: usize> {
    /// 完整间隔纳秒数，非零。
    pub elapsed_ns: u64,
    /// 各阶段之和等于完整间隔。
    pub attribution: FrameAttribution<N>,
}

/// 最近有效样本和会话最慢样本；暂停重置不会抹去最慢记录。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameDetail<const N: usize> {
    /// 最近一次有效记录；清空窗口后暂为空。
    pub latest: Option<FrameSample<N>>,
    /// 最近一次超过 `max(前一窗口平均值×2, 16.666667ms)` 的有效间隔。
    /// 这是显示用启发阈值，不是性能通过标准；窗口重置后仍保留。
    pub latest_stall: Option<FrameSample<N>>,
    /// 此对象会话内最慢的有效记录；相同时保留最早一次。
    pub slowest: Option<FrameSample<N>>,
}

/// 统计窗口容量增长失败；已有样本保持不变。
#[derive(Debug, thiserror::Error)]
pub enum FrameStatsError {
    /// 样本或暂存所需容量不足，保留底层分配来源。
    #[error("帧统计窗口分配失败")]
    Capacity(#[from] TryReserveError),
}

/// 最近窗口的帧统计；分位数采用 nearest rank，均为成功排定呈现/提交之间的完整间隔。
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameSnapshot {
    /// 当前快照参与计算的帧数。
    pub sample_count: usize,
    /// 最近 1s 内结束的完整间隔，帧数除以总时长；不是瞬时 FPS 的算术平均。
    pub average_fps: f64,
    /// 同一 1s 窗口的平均帧时长，单位 ms。
    pub mean_ms: f64,
    /// 帧时长 P90，单位 ms。
    pub p90_ms: f64,
    /// 帧时长 P99，单位 ms。
    pub p99_ms: f64,
}

/// 所有统计共用 1s 滚动窗口；本对象独占样本、排序、图表及其保留容量。
///
/// 保留结束时刻属于 `(now - 1s, now]` 的完整间隔，左边界恰好相等则删除。
/// 跨越左边界的帧不拆分，因此计算分母可略大于 1s；长帧也完整计入。初期按实际样本。
/// 仅在容量不足时尝试增长，不限制关闭限帧后的实际帧率。`N` 至少为 1。
#[derive(Default)]
pub struct FrameStats<const N: usize> {
    samples: VecDeque<FrameSample<N>>,
    scratch_ns: Vec<u64>,
    chart_ms: Vec<[f32; N]>,
    refresh_elapsed_ns: u64,
    skip_next_sample: bool,
    snapshot: FrameSnapshot,
    total_ns: u128,
    detail: FrameDetail<N>,
}

impl<const N: usize> FrameStats<N> {
    /// 清除滚动窗口并跳过跨暂停间隔，保留会话最慢的有效记录。
    pub fn reset_samples(&mut self) {
        self.samples.clear();
        self.scratch_ns.clear();
        self.chart_ms.clear();
        self.refresh_elapsed_ns = 0;
        self.skip_next_sample = true;
        self.snapshot = FrameSnapshot::default();
        self.total_ns = 0;
        self.detail.latest = None;
    }

    /// 记录成功排定呈现/提交之间的完整间隔；零间隔计作 1ns，超出 u64 的纳秒数饱和。
    ///
    /// 在改变样本前统一准备 ring、排序和图表容量；失败时保留样本。
    /// 已有容量在重置后复用，读取图表和排序不再分配；清理旧帧的成本为摊销 O(1)。
    ///
    /// `N == 0` 无法通过编译；至少需要一个未归因余量段。
    /// ```compile_fail
    /// let mut stats = frame_loop::FrameStats::<0>::default();
    /// stats.record(std::time::Duration::from_millis(1)).unwrap();
    /// ```
    pub fn record(&mut self, elapsed: Duration) -> Result<(), FrameStatsError> {
        self.record_attributed(elapsed, FrameAttribution::default())
    }

    /// 记录完整提交间隔及其主线程归因，后台 CPU/GPU 任务时间不得直接累加进来。
    ///
    /// 超量输入按数组顺序裁剪，剩余间隔补入最后一段，确保各段之和
    /// 等于完整间隔且不溢出。仅能限制错误输入的总量，不能自动识别重叠计时。
    /// 重置后的第一条记录整体跳过，不参与会话最慢帧；失败时保留原样本。
    ///
    /// `N == 0` 无法通过编译；至少需要一个未归因余量段。
    pub fn record_attributed(
        &mut self,
        elapsed: Duration,
        attribution: FrameAttribution<N>,
    ) -> Result<(), FrameStatsError> {
        const {
            assert!(N != 0, "帧归因至少需要一个余量段");
        }
        if self.skip_next_sample {
            self.skip_next_sample = false;
            return Ok(());
        }
        let elapsed_ns = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX).max(1);
        let required = self.samples.len() + 1;
        self.samples.try_reserve(1)?;
        self.scratch_ns
            .try_reserve(required.saturating_sub(self.scratch_ns.len()))?;
        self.chart_ms
            .try_reserve(required.saturating_sub(self.chart_ms.len()))?;
        let sample = FrameSample {
            elapsed_ns,
            attribution: attribution.fitted(elapsed_ns),
        };
        let previous_mean = if self.samples.is_empty() {
            0
        } else {
            self.total_ns / self.samples.len() as u128
        };
        let stall_threshold = previous_mean.saturating_mul(2).max(16_666_667);
        self.samples.push_back(sample);
        self.detail.latest = Some(sample);
        if u128::from(elapsed_ns) > stall_threshold {
            self.detail.latest_stall = Some(sample);
        }
        if self
            .detail
            .slowest
            .is_none_or(|old| sample.elapsed_ns > old.elapsed_ns)
        {
            self.detail.slowest = Some(sample);
        }
        self.total_ns += u128::from(elapsed_ns);
        while let Some(oldest) = self.samples.front() {
            if self.total_ns - u128::from(oldest.elapsed_ns) < 1_000_000_000 {
                break;
            }
            let oldest = self.samples.pop_front().expect("已检查首样本");
            self.total_ns -= u128::from(oldest.elapsed_ns);
        }
        self.snapshot.average_fps =
            self.samples.len() as f64 * 1_000_000_000.0 / self.total_ns as f64;
        self.snapshot.mean_ms = self.total_ns as f64 / self.samples.len() as f64 / 1_000_000.0;
        self.snapshot.sample_count = self.samples.len();
        self.refresh_elapsed_ns = self.refresh_elapsed_ns.saturating_add(elapsed_ns);
        if self.samples.len() == 1 || self.refresh_elapsed_ns >= REFRESH_INTERVAL_NS {
            self.refresh();
        }
        Ok(())
    }

    /// 平均值每帧更新；分位数首次样本和每累计 250ms 刷新。
    pub const fn snapshot(&self) -> FrameSnapshot {
        self.snapshot
    }

    /// 读取持久归因；不借用图表缓存，可在 [`Self::chart`] 前复制给 HUD。
    pub const fn detail(&self) -> FrameDetail<N> {
        self.detail
    }

    /// 立即刷新快照；显示统计开关开启时可调用，排序暂存由本对象复用。
    pub fn refresh(&mut self) {
        self.refresh_elapsed_ns = 0;
        if self.samples.is_empty() {
            return;
        }
        self.scratch_ns.clear();
        self.scratch_ns
            .extend(self.samples.iter().map(|sample| sample.elapsed_ns));
        self.scratch_ns.sort_unstable();
        let len = self.samples.len();
        self.snapshot.sample_count = len;
        self.snapshot.p90_ms = self.scratch_ns[percentile_index(len, 90)] as f64 / 1_000_000.0;
        self.snapshot.p99_ms = self.scratch_ns[percentile_index(len, 99)] as f64 / 1_000_000.0;
    }

    /// 将最新窗口按时间递增展开为 ms 分阶段堆叠柱。
    ///
    /// 返回借用在下一次可变操作前有效；图表转换成本为 O(窗口长度)，无分配。
    pub fn chart(&mut self) -> &[[f32; N]] {
        self.chart_ms.clear();
        self.chart_ms.extend(
            self.samples
                .iter()
                .map(|sample| sample.attribution.0.map(|ns| ns as f32 / 1_000_000.0)),
        );
        &self.chart_ms
    }

    /// 按图表的时间递增顺序读取原始样本；越界返回 None，成本 O(1)。
    ///
    /// [`compact_chart`] 的赢家索引可用于取得未经过 f32 毫秒转换的精确归因。
    pub fn sample(&self, index: usize) -> Option<FrameSample<N>> {
        self.samples.get(index).copied()
    }

    /// 按连续时间桶保留整帧纳秒时长最大的样本索引；同峰值保留最早样本。
    ///
    /// 输出数取窗口、列数和输出容量的最小值。成本 O(样本数)，不转换图表、分配或遍历
    /// 每帧的归因段；结果可用 [`Self::sample`] 读取，下一次修改窗口前有效。
    pub fn compact_sample_indices(&self, columns: usize, winners: &mut [usize]) -> usize {
        let count = self.samples.len().min(columns).min(winners.len());
        if count == 0 {
            return 0;
        }
        let base = self.samples.len() / count;
        let extra = self.samples.len() % count;
        let mut start = 0;
        for (column, output) in winners[..count].iter_mut().enumerate() {
            let end = start + base + usize::from(column < extra);
            let mut winner = start;
            for index in start + 1..end {
                if self.samples[index].elapsed_ns > self.samples[winner].elapsed_ns {
                    winner = index;
                }
            }
            *output = winner;
            start = end;
        }
        count
    }

    /// 样本、排序和图表的保留容量字节数，不含对象字段和分配器元数据。
    pub fn retained_bytes(&self) -> usize {
        self.samples.capacity() * std::mem::size_of::<FrameSample<N>>()
            + self.scratch_ns.capacity() * std::mem::size_of::<u64>()
            + self.chart_ms.capacity() * std::mem::size_of::<[f32; N]>()
    }
}

const fn percentile_index(len: usize, percentile: usize) -> usize {
    let rank = (len / 100 * percentile + (len % 100 * percentile).div_ceil(100)).saturating_sub(1);
    let last = len.saturating_sub(1);
    if rank < last { rank } else { last }
}

/// 将按时间排序的非负有限毫秒数分为连续桶，每桶保留总时长最大的完整样本。
///
/// 输出数取样本、列数和两个输出切片容量的最小值；相同峰值保留最早样本。
/// `winners` 返回输入索引供悬停定位，不独立取各段最大值。成本 O(样本数×N)，无分配。
pub fn compact_chart<const N: usize>(
    samples: &[[f32; N]],
    columns: usize,
    bars: &mut [[f32; N]],
    winners: &mut [usize],
) -> usize {
    let count = samples
        .len()
        .min(columns)
        .min(bars.len())
        .min(winners.len());
    if count == 0 {
        return 0;
    }
    let base = samples.len() / count;
    let extra = samples.len() % count;
    let mut start = 0;
    for column in 0..count {
        let end = start + base + usize::from(column < extra);
        let mut winner = start;
        let mut maximum = samples[start].iter().sum::<f32>();
        for (offset, sample) in samples[start + 1..end].iter().enumerate() {
            let total = sample.iter().sum();
            if total > maximum {
                maximum = total;
                winner = start + offset + 1;
            }
        }
        bars[column] = samples[winner];
        winners[column] = winner;
        start = end;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    type FrameStats = super::FrameStats<10>;
    type FrameAttribution = super::FrameAttribution<10>;
    #[test]
    fn raw_peak_selection_retains_nanosecond_order_and_first_ties() {
        let mut stats = FrameStats::default();
        for ns in [100_000_000, 100_000_001, 100_000_001] {
            stats.record(Duration::from_nanos(ns)).unwrap();
        }
        let mut winners = [99; 1];
        assert_eq!(stats.compact_sample_indices(8, &mut winners), 1);
        assert_eq!(winners, [1]);
        assert_eq!(stats.sample(winners[0]).unwrap().elapsed_ns, 100_000_001);
        assert_eq!(stats.compact_sample_indices(0, &mut winners), 0);
    }
    #[test]
    fn peak_buckets_keep_whole_samples_and_original_indices() {
        let samples = [[3.0, 0.0], [6.0, 4.0], [0.0, 5.0], [7.0, 0.0], [0.0, 2.0]];
        let mut bars = [[0.0; 2]; 2];
        let mut winners = [0; 2];
        assert_eq!(compact_chart(&samples, 2, &mut bars, &mut winners), 2);
        assert_eq!(bars, [samples[1], samples[3]]);
        assert_eq!(winners, [1, 3]);
    }

    #[test]
    fn equal_peaks_keep_the_first_sample_and_short_output_is_bounded() {
        let samples = [[4.0, 0.0], [0.0, 4.0]];
        let mut bars = [[0.0; 2]; 3];
        let mut winners = [99; 1];
        assert_eq!(compact_chart(&samples, 3, &mut bars, &mut winners), 1);
        assert_eq!(bars[0], samples[0]);
        assert_eq!(winners[0], 0);
        assert_eq!(compact_chart(&samples, 0, &mut bars, &mut winners), 0);
    }

    #[test]
    fn retained_sample_matches_chart_order_after_window_eviction() {
        let mut stats = super::FrameStats::<1>::default();
        for ms in [400, 300, 200, 500] {
            stats.record(Duration::from_millis(ms)).unwrap();
        }
        assert_eq!(stats.sample(0).unwrap().elapsed_ns, 300_000_000);
        assert_eq!(stats.sample(2).unwrap().attribution.0, [500_000_000]);
        assert!(stats.sample(3).is_none());
    }

    #[test]
    fn nearest_rank_percentiles_measure_frame_times() {
        let mut stats = FrameStats::default();
        for ms in 1..=100 {
            stats.record(Duration::from_micros(ms * 100)).unwrap();
        }
        stats.refresh();
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.p90_ms, 9.0);
        assert_eq!(snapshot.p99_ms, 9.9);
    }

    #[test]
    fn chart_keeps_frames_ending_in_the_last_second_in_order() {
        let mut stats = FrameStats::default();
        stats.record(Duration::from_millis(400)).unwrap();
        stats.record(Duration::from_millis(300)).unwrap();
        stats.record(Duration::from_millis(200)).unwrap();
        stats.record(Duration::from_millis(500)).unwrap();
        let chart = stats.chart();
        assert_eq!(
            chart
                .iter()
                .map(|sample| sample.iter().sum::<f32>())
                .collect::<Vec<_>>(),
            [300.0, 200.0, 500.0]
        );
    }

    #[test]
    fn all_statistics_use_one_second_without_a_frame_count_cap() {
        let mut stats = FrameStats::default();
        for _ in 0..2500 {
            stats.record(Duration::from_micros(500)).unwrap();
        }
        stats.refresh();
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.sample_count, 2000);
        assert_eq!(snapshot.average_fps, 2000.0);
        assert_eq!(snapshot.mean_ms, 0.5);
        assert_eq!(snapshot.p90_ms, 0.5);
        assert_eq!(snapshot.p99_ms, 0.5);
        assert_eq!(stats.chart().len(), 2000);
    }

    #[test]
    fn average_is_count_over_elapsed_time() {
        let mut stats = FrameStats::default();
        stats.record(Duration::from_millis(10)).unwrap();
        stats.record(Duration::from_millis(30)).unwrap();
        assert_eq!(stats.snapshot().average_fps, 50.0);
        assert_eq!(stats.snapshot().mean_ms, 20.0);
    }

    #[test]
    fn long_frame_is_counted_whole() {
        let mut stats = FrameStats::default();
        stats.record(Duration::from_millis(10)).unwrap();
        stats.record(Duration::from_secs(2)).unwrap();
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.average_fps, 0.5);
        assert_eq!(snapshot.p99_ms, 2000.0);
        assert_eq!(
            stats
                .chart()
                .iter()
                .map(|sample| sample.iter().sum::<f32>())
                .collect::<Vec<_>>(),
            [2000.0]
        );
    }

    #[test]
    fn reset_excludes_the_cross_pause_interval() {
        let mut stats = FrameStats::default();
        stats.record(Duration::from_millis(10)).unwrap();
        stats.reset_samples();
        stats.record(Duration::from_secs(20)).unwrap();
        stats.record(Duration::from_millis(20)).unwrap();
        assert_eq!(stats.snapshot().sample_count, 1);
        assert_eq!(stats.snapshot().mean_ms, 20.0);
    }

    #[test]
    fn excessive_attribution_is_clipped_in_stage_order_without_overflow() {
        let mut stats = FrameStats::default();
        stats
            .record_attributed(
                Duration::from_nanos(10),
                FrameAttribution::new([u64::MAX; 10]),
            )
            .unwrap();
        let sample = stats.detail().latest.unwrap();
        assert_eq!(sample.attribution.get(0), 10);
        assert_eq!(sample.attribution.0.iter().sum::<u64>(), 10);
        assert!(sample.attribution.0[1..].iter().all(|&value| value == 0));
    }

    #[test]
    fn chart_keeps_attribution_with_its_frame_and_fills_unmeasured_time() {
        let mut stats = FrameStats::default();
        let mut first = FrameAttribution::default();
        first.add(0, 2_000_000);
        first.add(4, 3_000_000);
        stats
            .record_attributed(Duration::from_millis(10), first)
            .unwrap();
        let mut second = FrameAttribution::default();
        second.add(1, 15_000_000);
        stats
            .record_attributed(Duration::from_millis(20), second)
            .unwrap();
        let chart = stats.chart();
        assert_eq!(chart[0][0], 2.0);
        assert_eq!(chart[0][4], 3.0);
        assert_eq!(chart[1][1], 15.0);
        assert_eq!(chart[0][9], 5.0);
        assert_eq!(chart[1][9], 5.0);
    }

    #[test]
    fn slowest_survives_window_expiry_and_pause_is_never_recorded_as_a_stall() {
        let mut stats = FrameStats::default();
        let mut attribution = FrameAttribution::default();
        attribution.add(4, 400_000_000);
        stats
            .record_attributed(Duration::from_millis(400), attribution)
            .unwrap();
        for _ in 0..20 {
            stats.record(Duration::from_millis(100)).unwrap();
        }
        assert!(stats.chart().iter().all(|sample| sample[4] == 0.0));
        stats.reset_samples();
        stats.record(Duration::from_secs(20)).unwrap();
        assert!(stats.detail().latest.is_none());
        stats.record(Duration::from_millis(10)).unwrap();
        let detail = stats.detail();
        assert_eq!(detail.latest.unwrap().elapsed_ns, 10_000_000);
        assert_eq!(detail.slowest.unwrap().attribution.get(4), 400_000_000);
        assert_eq!(detail.latest_stall.unwrap().attribution.get(4), 400_000_000);
    }

    #[test]
    fn latest_stall_uses_previous_mean_and_replaces_an_older_longer_stall() {
        let mut stats = FrameStats::default();
        stats.record(Duration::from_millis(200)).unwrap();
        for _ in 0..200 {
            stats.record(Duration::from_millis(10)).unwrap();
        }
        stats.record(Duration::from_millis(20)).unwrap();
        assert_eq!(stats.detail().latest_stall.unwrap().elapsed_ns, 200_000_000);
        let mut attribution = FrameAttribution::default();
        attribution.add(1, 25_000_000);
        stats
            .record_attributed(Duration::from_millis(25), attribution)
            .unwrap();
        assert_eq!(
            stats.detail().latest_stall.unwrap().attribution.get(1),
            25_000_000
        );
        assert_eq!(stats.detail().slowest.unwrap().elapsed_ns, 200_000_000);
    }
}
