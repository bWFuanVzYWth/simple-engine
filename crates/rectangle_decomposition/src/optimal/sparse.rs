//! 固定 64x64 sparse quad 输入的矩形分解。
//!
//! 输入为非零 dyadic square 列表或连续标签切面，内部派生区间、chord 与 cut。
//! 少量叶子使用区间事件；大量叶子使用 SIMD 栅格扫描。

use core::num::NonZeroU16;

use crate::geometry::{ShapeError, cell_range_mask, coord_between_mask, validate_square};

use crate::optimal::fixed::FixedVec;
use crate::optimal::matching::{ChordBuffer, MatchingScratch};

use crate::optimal::types::{
    ActiveRect, EffectiveChord, Orientation, PackedRectangles64, RangeU8, Rectangle, Run,
    ValuedChord,
};
use crate::optimal::{get, slice};
#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "../proof/chord_extract.rs"]
mod chord_proof;
#[cfg(verus_only)]
#[path = "../proof/cut_geometry.rs"]
mod cut_proof;
#[cfg(verus_only)]
#[path = "../proof/history.rs"]
mod history_proof;
#[cfg(verus_only)]
#[path = "../proof/cut_masks.rs"]
mod mask_proof;
#[cfg(verus_only)]
#[path = "../proof/axis_owner.rs"]
mod proof;
#[cfg(verus_only)]
#[path = "../proof/scan_runs.rs"]
pub(super) mod scan_proof;
#[cfg(verus_only)]
#[path = "../proof/scratch.rs"]
mod scratch_proof;
#[cfg(verus_only)]
#[path = "../proof/selection_geometry.rs"]
mod selection_proof;

#[path = "dense.rs"]
mod dense;
#[path = "output.rs"]
mod output;
#[path = "scan.rs"]
mod scan;
#[path = "select.rs"]
mod select;
#[path = "sink.rs"]
mod sink;
pub use dense::DenseLabels64;

#[cfg_attr(verus_only, verus_verify)]
const EDGE: usize = 64;
#[cfg_attr(verus_only, verus_verify)]
const EDGE_U8: u8 = 64;
#[cfg_attr(verus_only, verus_verify)]
const MAX_AXIS_INTERVALS: usize = EDGE * EDGE;
#[cfg_attr(verus_only, verus_verify)]
const MAX_RECTANGLES: usize = EDGE * EDGE;
#[cfg_attr(verus_only, verus_verify)]
const EMPTY_INTERVAL: Interval = Interval {
    start: 0,
    end: 0,
    value: 0,
};
#[cfg_attr(verus_only, verus_verify)]
const EMPTY_CHORD: ValuedChord = ValuedChord {
    value: 0,
    chord: EffectiveChord {
        orientation: Orientation::Horizontal,
        x1: 0,
        y1: 0,
        x2: 0,
        y2: 0,
    },
};

/// 64x64 sparse quad image 的非零叶子。
///
/// `lod` 表示正方形边长的 log2，`size = 1 << lod`。`value` 为非零 label，
/// 0 始终表示隐式 background。
///
/// 规范输入域：`lod <= 6`，`u`、`v` 均为 `size` 的倍数，
/// `u + size <= 64`、`v + size <= 64`（按数学整数计算）。叶子覆盖的单元集合两两不交，
/// 相同标签也不能重叠；允许任意排列和空列表。覆盖处的标签为 `value`，其余单元为 0。
/// 公共字段不建立这些不变量，分解入口须在执行期校验。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
pub struct QuadLeaf64 {
    /// 正方形左边界；须按 `1 << lod` 对齐。
    pub u: u8,
    /// 正方形上边界；须按 `1 << lod` 对齐。
    pub v: u8,
    /// 边长的 log2，合法范围为 `0..=6`。
    pub lod: u8,
    /// 由调用方映射的非零合并标签。
    pub value: NonZeroU16,
}

pub use crate::error::SparseQuadError;

/// sparse optimal 分解的固定容量内联 scratch。
///
/// 可以放在 worker 栈上复用；对象较大，调用时传递 `&mut`，避免按值传递。
///
/// # 契约
///
/// 全部入口共用 [`crate::optimal`] 的 `Partition` 与 `Opt` 规范。
/// 任意 `DenseLabels64` 和满足 [`QuadLeaf64`] 输入域的列表，在 sink 均成功时成功，
/// 内核的 `CapacityOverflow` 对这些输入不可达；sink 可原样返回任意错误变体。
/// 非法列表返回对应输入错误，不调用 sink；同时违反多项约束时不约定错误优先级。
/// 输入不变，成功及任意错误返回后均可直接再次调用任何分解入口，无需 reset。
/// 借用结果只在当前 scratch 借用期内有效；不承诺保留失败前的内部结果。
/// 相同输入及相同 sink 行为在任意正常返回历史之后产生相同序列与错误，不依赖残留缓存。
/// 固定数组只允许读取本次已写入或由有效长度／占用位明确保留的内容。
#[cfg_attr(verus_only, verus_verify)]
pub struct SparseOptimalScratch64 {
    row_buckets: [IntervalBucket; EDGE],
    column_buckets: [IntervalBucket; EDGE],
    rows: AxisIntervals,
    columns: AxisIntervals,
    dense_labels: DenseLabels64,
    chord_groups: SparseChords,
    matching: MatchingScratch,
    horizontal_cuts: ChordBuffer<HorizontalCut>,
    vertical_cuts: ChordBuffer<VerticalCut>,
    horizontal_cut_masks: FixedVec<u64, EDGE>,
    vertical_cut_masks: FixedVec<u128, EDGE>,
    partition: PartitionScratch,
    rectangles: FixedVec<Rectangle, MAX_RECTANGLES>,
}

#[cfg_attr(verus_only, verus_verify)]
impl Default for SparseOptimalScratch64 {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.valid(), result.initialized()))]
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl SparseOptimalScratch64 {
    /// 自身固定容量的总字节数；不含调用方输入、输出、调用栈或分配器开销。
    #[cfg_attr(verus_only, verus_spec(result => ensures result as nat == vstd::layout::size_of::<Self>()))]
    pub const fn retained_bytes(&self) -> usize {
        size_of::<Self>()
    }

    #[allow(clippy::large_stack_arrays)] // 固定容量初值仍由 const 项求值。
    #[cfg_attr(verus_only, verifier::exec)]
    #[cfg_attr(verus_only, verus_spec(ensures Self::EMPTY.valid(), Self::EMPTY.initialized()))]
    const EMPTY: Self = {
        let output = Self {
            row_buckets: [EMPTY_BUCKET; EDGE],
            column_buckets: [EMPTY_BUCKET; EDGE],
            rows: AxisIntervals::new(),
            columns: AxisIntervals::new(),
            dense_labels: DenseLabels64::new(),
            chord_groups: SparseChords::new(),
            matching: MatchingScratch::new(),
            horizontal_cuts: FixedVec::new(HorizontalCut {
                y: 0,
                start: 0,
                end: 0,
            }),
            vertical_cuts: FixedVec::new(VerticalCut {
                x: 0,
                start: 0,
                end: 0,
            }),
            horizontal_cut_masks: FixedVec::new(0),
            vertical_cut_masks: FixedVec::new(0),
            partition: PartitionScratch::new(),
            rectangles: FixedVec::new(Rectangle {
                value: 0,
                x: RangeU8::new(0, 0),
                y: RangeU8::new(0, 0),
            }),
        };
        proof! { scratch_proof::new_owner(&output); }
        output
    };

    /// 创建可直接使用的内联缓存；初始化与借用分解均不分配堆内存。
    /// 在 worker 的任务循环外创建一次，并通过可变引用复用。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures result.valid(), result.initialized()))]
    pub const fn new() -> Self {
        Self::EMPTY
    }
}

impl SparseOptimalScratch64 {
    /// 对 sparse quad leaves 执行 64x64 最优分解，结果借用自 scratch。
    ///
    /// 资源契约：从 `new()` 创建后即可使用，本函数不调用堆分配器。
    ///
    /// # Errors
    ///
    /// leaf 越界、未按 lod 对齐、lod 超出范围或互相重叠时返回错误。
    /// 失败不返回旧结果；scratch 可继续复用，输入始终不变。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result => ensures final(self).sparse_borrowed(leaves@, result)))]
    pub fn decompose_borrowed(
        &mut self,
        leaves: &[QuadLeaf64],
    ) -> Result<&[Rectangle], SparseQuadError> {
        proof! { reveal(SparseOptimalScratch64::sparse_borrowed); reveal(SparseOptimalScratch64::produced); scratch_proof::establish(&*self); }
        build_axis_intervals_from_leaves(
            leaves,
            &mut self.dense_labels,
            &mut self.rows,
            &mut self.columns,
            &mut self.row_buckets,
            &mut self.column_buckets,
        )?;
        proof! { scratch_proof::represented_rows(&self.rows, input_proof::labels(leaves@)); }
        self.decompose_intervals_borrowed()
    }
}

impl SparseOptimalScratch64 {
    /// 对 sparse leaves 分解，每个完成的矩形直接交给调用方。
    ///
    /// 输出顺序与 `decompose_borrowed` 相同；不写入 scratch 的矩形数组。
    /// 库不分配堆内存，sink 的资源行为由调用方决定。
    ///
    /// # Errors
    ///
    /// 输入错误在调用 sink 前返回。sink 错误会立即终止分区并原样返回；
    /// 已交给 sink 的前缀及失败调用产生的副作用不回滚。后续调用可继续复用 scratch。
    ///
    /// 记同一输入的完整 borrowed 结果为 `R`。sink 全部成功时，
    /// 调用序列等于 `R`，返回值等于调用次数。
    /// 第 k 次调用（从 0 开始）失败时，仅调用 `R[0]..=R[k]`，其中此前 k 次成功；
    /// 原样返回该错误且不再调用 sink。失败调用的外部副作用由 sink 自身决定。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures final(self).sparse_delivery(leaves@, final(events)@, result),
    ))]
    pub fn decompose_into<F>(
        &mut self,
        leaves: &[QuadLeaf64],
        sink: F,
        #[cfg(verus_only)] events: &mut Ghost<Seq<sink::proof::Event>>,
    ) -> Result<usize, SparseQuadError>
    where
        F: FnMut(Rectangle) -> Result<(), SparseQuadError>,
    {
        proof! {
            reveal(SparseOptimalScratch64::sparse_delivery);
            events@ = Seq::empty(); scratch_proof::establish(self);
        }
        build_axis_intervals_from_leaves(
            leaves,
            &mut self.dense_labels,
            &mut self.rows,
            &mut self.columns,
            &mut self.row_buckets,
            &mut self.column_buckets,
        )?;
        proof! { scratch_proof::represented_rows(&self.rows, input_proof::labels(leaves@)); }
        extract_sparse_chords(&self.rows, &self.columns, &mut self.chord_groups)?;
        self.select_cuts()?;
        self.sparse_partition_into(
            sink,
            #[cfg(verus_only)]
            events,
        )
    }
}

impl SparseOptimalScratch64 {
    /// 将矩形直接写入调用方的 bounds / labels `SoA` 输出。
    ///
    /// 调用开始时清空输出；整个调用不分配堆内存，也不经过 `AoS` 中间结果。
    ///
    /// # Errors
    ///
    /// 输入非法时返回错误，输出为空。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result => ensures final(self).sparse_packed(leaves@, final(output), result)))]
    pub fn decompose_packed(
        &mut self,
        leaves: &[QuadLeaf64],
        output: &mut PackedRectangles64,
    ) -> Result<usize, SparseQuadError> {
        proof! { reveal(SparseOptimalScratch64::sparse_packed); reveal(SparseOptimalScratch64::produced); scratch_proof::establish(&*self); }
        output.clear();
        build_axis_intervals_from_leaves(
            leaves,
            &mut self.dense_labels,
            &mut self.rows,
            &mut self.columns,
            &mut self.row_buckets,
            &mut self.column_buckets,
        )?;
        proof! { scratch_proof::represented_rows(&self.rows, input_proof::labels(leaves@)); }
        self.decompose_intervals_packed(output)
    }

    // 两种输入复用同一 packed 扫描核，避免重复单态化。
    #[inline(never)]
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).axes_ready(), old(output).valid(), old(output)@ == Seq::empty(),
        ensures final(self).valid(), final(output).valid(), final(self).rows == old(self).rows, final(self).columns == old(self).columns,
            match result {
                Ok(count) => count == final(output)@.len() && final(self).produced(old(self).rows.labels(), final(output)@),
                Err(_) => false,
            },
    ))]
    fn decompose_intervals_packed(
        &mut self,
        output: &mut PackedRectangles64,
    ) -> Result<usize, SparseQuadError> {
        proof! { reveal(SparseOptimalScratch64::produced); }
        extract_sparse_chords(&self.rows, &self.columns, &mut self.chord_groups)?;
        self.select_cuts()?;
        let cut_boundaries = build_cut_masks(
            &self.horizontal_cuts,
            &self.vertical_cuts,
            &mut self.horizontal_cut_masks,
            &mut self.vertical_cut_masks,
        )?;
        proof! { scratch_proof::prepare_output(self, cut_boundaries); }
        let count = {
            let mut scanner = scan::RectScan::new(
                &self.rows,
                &self.horizontal_cut_masks,
                &self.vertical_cut_masks,
                cut_boundaries,
                &mut self.partition,
            )?;
            output::write_packed(&mut scanner, output)?
        };
        proof! { scratch_proof::establish(&*self); }
        Ok(count)
    }

    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).axes_ready(),
        ensures final(self).valid(), final(self).rows == old(self).rows, final(self).columns == old(self).columns,
            match result {
                Ok(rectangles) => rectangles@ == final(self).rectangles@ && final(self).produced(old(self).rows.labels(), rectangles@),
                Err(_) => false,
            },
    ))]
    fn decompose_intervals_borrowed(&mut self) -> Result<&[Rectangle], SparseQuadError> {
        proof! { reveal(SparseOptimalScratch64::produced); }
        extract_sparse_chords(&self.rows, &self.columns, &mut self.chord_groups)?;
        self.select_cuts()?;
        self.sparse_partition()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl SparseOptimalScratch64 {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(), selection_proof::complete(old(self).rows.labels(),
            old(self).chord_groups.horizontal@, old(self).chord_groups.vertical@),
        ensures result.is_ok(), final(self).valid(), final(self).cuts_ready(),
            selection_proof::frame(old(self), final(self)),
    ))]
    fn select_cuts(&mut self) -> Result<(), SparseQuadError> {
        let groups = &self.chord_groups;
        let matching = &mut self.matching;
        let horizontal_cuts = &mut self.horizontal_cuts;
        let vertical_cuts = &mut self.vertical_cuts;

        horizontal_cuts.clear();
        vertical_cuts.clear();

        if groups.horizontal.is_empty() && groups.vertical.is_empty() {
            proof! {
                selection_proof::empty(self.rows.labels(),groups.horizontal@,groups.vertical@);
                selection_proof::selected_union(Set::empty(),horizontal_cuts@,vertical_cuts@);
            }
            return Ok(());
        }

        let horizontal_cap = groups.horizontal.len();
        let vertical_cap = groups.vertical.len();
        reserve_exact(horizontal_cuts, horizontal_cap)?;
        reserve_exact(vertical_cuts, vertical_cap)?;

        // chord 的闭段格点至少有三个同色邻像素，不同标签连端点也不能相交。
        // 全图是逐标签图的不相交并，匹配和独立集的大小可直接相加。
        {
            let horizontal = groups.horizontal.as_slice();
            let vertical = groups.vertical.as_slice();
            proof! {
                chord_proof::graph_domain(self.rows.labels(),false,horizontal@);
                chord_proof::graph_domain(self.rows.labels(),true,vertical@);
            }
            matching.select_maximum_independent_set(horizontal, vertical);
            select::fill_horizontal_cuts(
                horizontal,
                matching.selected_horizontal(),
                horizontal_cuts,
            )?;
            select::fill_vertical_cuts(vertical, matching.selected_vertical(), vertical_cuts)?;
            proof! {
                selection_proof::canonical(self.rows.labels(),horizontal@,vertical@,
                    matching.selected_horizontal_view(),matching.selected_vertical_view());
                selection_proof::raw_cuts(self.rows.labels(),horizontal@,vertical@,
                    matching.selected_horizontal_view(),matching.selected_vertical_view());
                selection_proof::raw_parts(horizontal_cuts@,vertical_cuts@);
                selection_proof::selected_union(selection_proof::selected(horizontal@,vertical@,
                    matching.selected_horizontal_view(),matching.selected_vertical_view()),horizontal_cuts@,vertical_cuts@);
            }
        }
        proof! { scratch_proof::establish(self); assert(self.cuts_ready()); }
        Ok(())
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl SparseOptimalScratch64 {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(), old(self).rows.lines@.len() == 64,
            old(self).rows.boundaries_complete(), old(self).cuts_ready(),
        ensures final(self).valid(), final(self).rows == old(self).rows, final(self).columns == old(self).columns,
            match result {
                Ok(rectangles) => rectangles@ == final(self).rectangles@ && rectangles@ == final(self).canonical()
                    && rectangles@ == crate::partition::optimality::result_for(old(self).rows.labels())
                    && crate::partition::optimal(old(self).rows.labels(), rectangles@),
                Err(_) => false,
            },
    ))]
    fn sparse_partition(&mut self) -> Result<&[Rectangle], SparseQuadError> {
        let cut_boundaries = build_cut_masks(
            &self.horizontal_cuts,
            &self.vertical_cuts,
            &mut self.horizontal_cut_masks,
            &mut self.vertical_cut_masks,
        )?;

        proof! { scratch_proof::prepare_output(self, cut_boundaries); }
        {
            let result = &mut self.rectangles;
            result.clear();
            reserve_exact(result, self.rows.intervals.len())?;
            let mut scanner = scan::RectScan::new(
                &self.rows,
                &self.horizontal_cut_masks,
                &self.vertical_cut_masks,
                cut_boundaries,
                &mut self.partition,
            )?;
            output::write_aos(&mut scanner, result)?;
        }
        proof! { scratch_proof::establish(&*self); }
        Ok(self.rectangles.as_slice())
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl SparseOptimalScratch64 {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(), old(self).rows.lines@.len() == 64,
            old(self).rows.boundaries_complete(), old(self).cuts_ready(), old(events)@ == Seq::empty(),
        ensures final(self).valid(), final(self).rows == old(self).rows, final(self).columns == old(self).columns,
            sink::proof::delivery(final(self).canonical(), final(events)@, result),
            crate::partition::optimal(old(self).rows.labels(), final(self).canonical()),
            final(self).canonical() == crate::partition::optimality::result_for(old(self).rows.labels()),
    ))]
    fn sparse_partition_into<F>(
        &mut self,
        sink: F,
        #[cfg(verus_only)] events: &mut Ghost<Seq<sink::proof::Event>>,
    ) -> Result<usize, SparseQuadError>
    where
        F: FnMut(Rectangle) -> Result<(), SparseQuadError>,
    {
        let cut_boundaries = build_cut_masks(
            &self.horizontal_cuts,
            &self.vertical_cuts,
            &mut self.horizontal_cut_masks,
            &mut self.vertical_cut_masks,
        )?;
        proof! { scratch_proof::prepare_output(self, cut_boundaries); }
        let result = partition_rows(
            &self.rows,
            &self.horizontal_cut_masks,
            &self.vertical_cut_masks,
            cut_boundaries,
            &mut self.partition,
            sink,
            #[cfg(verus_only)]
            events,
        );
        proof! { scratch_proof::establish(self); }
        result
    }
}

// Leaf 校验

#[cfg(verus_only)]
#[path = "../proof/events.rs"]
mod event_proof;
#[cfg(verus_only)]
#[path = "../proof/failure.rs"]
mod failure_proof;
#[cfg(verus_only)]
#[path = "../proof/input.rs"]
mod input_proof;

/// 是否全部为单位像素；只决定执行路径，不代替叶子形状或重叠校验。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == (forall|i: int| 0 <= i < leaves@.len() ==> #[trigger] leaves@[i].lod == 0),
))]
const fn all_unit_leaves(leaves: &[QuadLeaf64]) -> bool {
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= leaves@.len(),
            forall|i: int| 0 <= i < index ==> #[trigger] leaves@[i].lod == 0,
        decreases leaves@.len() - index,
    ))]
    while index < leaves.len() {
        if leaves[index].lod != 0 {
            return false;
        }
        index += 1;
    }
    true
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => ensures proof::leaf_checked(leaf, result)))]
fn validate_leaf_shape(leaf: QuadLeaf64) -> Result<u8, SparseQuadError> {
    match validate_square(leaf.u, leaf.v, leaf.lod) {
        Ok(size) => Ok(size),
        Err(ShapeError::LodOutOfRange) => Err(SparseQuadError::LodOutOfRange),
        Err(ShapeError::Misaligned) => Err(SparseQuadError::Misaligned),
        Err(ShapeError::OutOfBounds) => Err(SparseQuadError::OutOfBounds),
    }
}

// 几何类型

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(verus_only, derive(StructuralEq))]
#[cfg_attr(verus_only, verus_verify)]
struct Interval {
    start: u8,
    end: u8,
    value: u16,
}

/// 区间直接存入起点对应的槽；位图记录占用，清空只重置位图。
#[derive(Clone, Copy)]
#[cfg_attr(verus_only, verus_verify)]
struct IntervalBucket {
    items: [Interval; EDGE],
    starts: u64,
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::exec)]
#[cfg_attr(verus_only, verus_spec(ensures EMPTY_BUCKET.valid(), EMPTY_BUCKET.starts == 0))]
const EMPTY_BUCKET: IntervalBucket = IntervalBucket::new();

#[cfg_attr(verus_only, verus_verify)]
impl IntervalBucket {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.valid(), result.starts == 0))]
    const fn new() -> Self {
        let output = Self {
            items: [Interval {
                start: 0,
                end: 0,
                value: 0,
            }; EDGE],
            starts: 0,
        };
        proof! { proof::bucket_empty(&output); }
        output
    }

    #[cfg_attr(verus_only, verus_spec(
        ensures final(self).valid(), final(self).starts == 0,
            final(self).items@ == old(self).items@,
    ))]
    const fn clear(&mut self) {
        self.starts = 0;
        proof! { proof::bucket_empty(self); }
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(), proof::interval_valid(interval),
        ensures final(self).valid(),
            result.is_ok() <==> old(self).starts & (1u64 << interval.start as u64) == 0,
            result.is_ok() ==> final(self).starts == old(self).starts | (1u64 << interval.start as u64)
                && final(self).items@ == old(self).items@.update(interval.start as int, interval),
            result.is_err() ==> result is Err && result->Err_0 is Overlap
                && final(self).starts == old(self).starts && final(self).items@ == old(self).items@,
    ))]
    fn try_push(&mut self, interval: Interval) -> Result<(), SparseQuadError> {
        proof_decl! { let ghost before = self.items@; let ghost starts = self.starts; }
        let slot = self
            .items
            .get_mut(usize::from(interval.start))
            .ok_or(SparseQuadError::CapacityOverflow)?;
        let bit = 1u64 << u32::from(interval.start);
        if self.starts & bit != 0 {
            return Err(SparseQuadError::Overlap);
        }
        *slot = interval;
        self.starts |= bit;
        proof! { proof::bucket_insert(before, starts, interval, self); }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(verus_only, derive(StructuralEq))]
#[cfg_attr(verus_only, verus_verify)]
struct LineRange {
    start: usize,
    end: usize,
}

/// 验证目标：构建成功后，每条线引用其最大同标签非零闭开区间，按起点严格递增且互不相交。
/// 相邻同标签区间必须合并；区间展开恰等于对应像素行／列，不改变标签或背景。
/// `boundaries` 可保守多标，但相邻线内容不同时必须置位；第 0 线由分区入口单独处理。
/// 行间可复用同一只读区间片段，全部引用范围有效；每轴存储区间数至多 4096。
#[cfg_attr(verus_only, verus_verify)]
struct AxisIntervals {
    lines: FixedVec<LineRange, EDGE>,
    intervals: FixedVec<Interval, MAX_AXIS_INTERVALS>,
    boundaries: u64,
}

#[cfg_attr(verus_only, verus_verify)]
impl AxisIntervals {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.valid(), result.lines@.len() == 0,
        result.intervals@.len() == 0, result.boundaries == 0))]
    const fn new() -> Self {
        let output = Self {
            lines: FixedVec::new(LineRange { start: 0, end: 0 }),
            intervals: FixedVec::new(EMPTY_INTERVAL),
            boundaries: 0,
        };
        proof! { proof::empty_axis(&output); }
        output
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures result.is_ok(), final(self).valid(), final(self).lines@.len() == 64,
            final(self).intervals@.len() == 0, final(self).boundaries == 0,
    ))]
    fn clear_for_build(&mut self) -> Result<(), SparseQuadError> {
        self.lines.clear();
        self.intervals.clear();
        self.boundaries = 0;
        reserve_exact(&self.lines, EDGE)?;
        self.lines.resize(EDGE, LineRange { start: 0, end: 0 });
        proof! { proof::cleared_axis(self); }
        Ok(())
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires self.valid(), index < self.lines@.len(),
        ensures result@ == self.line_spec(index as int), proof::line_valid(result@),
    ))]
    fn line(&self, index: usize) -> &[Interval] {
        let range = get(&self.lines, index);
        slice(&self.intervals, range.start..range.end)
    }

    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(), index < old(self).lines@.len(), old(self).range_valid(range),
        ensures final(self).valid(), final(self).lines@ == old(self).lines@.update(index as int, range),
            final(self).intervals@ == old(self).intervals@,
            final(self).boundaries == old(self).boundaries,
    ))]
    const fn publish_line(&mut self, index: usize, range: LineRange) {
        proof! { proof::publication_preserves_owner(self, index, range); }
        self.lines.set(index, range);
    }
}

// 从 leaves 构造轴区间

/// 三种输入策略共用的边界：任意叶子列表均执行真实校验；合法图必成功。
/// 错误仅来自叶子形状或覆盖重叠，失败后两轴仍是有效所有者，供下次清空复用。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(rows).valid(), old(columns).valid(),
    ensures final(rows).valid(), final(columns).valid(), input_proof::checked(leaves@, result),
        result.is_ok() == input_proof::valid(leaves@),
        match result { Ok(()) => failure_proof::failure_for(leaves@).is_none(),
            Err(error) => failure_proof::failure_for(leaves@) == Some(error) },
        result.is_ok() ==> final(rows).represents(input_proof::labels(leaves@), false)
            && final(columns).represents(input_proof::labels(leaves@), true)
            && final(rows).boundaries_complete() && final(columns).boundaries_complete(),
))]
fn build_axis_intervals_from_leaves(
    leaves: &[QuadLeaf64],
    dense_labels: &mut DenseLabels64,
    rows: &mut AxisIntervals,
    columns: &mut AxisIntervals,
    row_buckets: &mut [IntervalBucket; EDGE],
    column_buckets: &mut [IntervalBucket; EDGE],
) -> Result<(), SparseQuadError> {
    let result = if leaves.len() >= dense::DENSE_LEAF_THRESHOLD {
        match dense::rasterize_leaves(leaves, dense_labels, row_buckets) {
            Ok(()) => dense::build_axes(dense_labels, rows, columns, column_buckets),
            Err(error) => {
                proof! { failure_proof::raster_result(leaves@, error); }
                Err(error)
            }
        }
    } else {
        match seed_leaf_axis_intervals(leaves, row_buckets, column_buckets) {
            Ok(true) => build_unit_leaf_axes(
                #[cfg(verus_only)]
                leaves,
                row_buckets,
                column_buckets,
                rows,
                columns,
            ),
            Ok(false) => {
                let result = build_event_leaf_axes(
                    #[cfg(verus_only)]
                    leaves,
                    row_buckets,
                    column_buckets,
                    rows,
                    columns,
                );
                proof! { if result is Err {
                    input_proof::checked_status(leaves@, result);
                    failure_proof::sweep_result(leaves@);
                } }
                result
            }
            Err(error) => {
                proof! { failure_proof::seed_result(leaves@, error); }
                Err(error)
            }
        }
    };
    proof! { input_proof::checked_status(leaves@, result); if result is Ok { failure_proof::valid_result(leaves@); } }
    result
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(rows).valid(), old(columns).valid(),
        input_proof::seeded(leaves@, row_buckets@, false), input_proof::seeded(leaves@, column_buckets@, true),
    ensures final(rows).valid(), final(columns).valid(), input_proof::checked(leaves@, result),
        result.is_ok() == input_proof::valid(leaves@),
        result is Err ==> result->Err_0 is Overlap,
        result.is_ok() ==> final(rows).represents(input_proof::labels(leaves@), false)
            && final(columns).represents(input_proof::labels(leaves@), true)
            && final(rows).boundaries_complete() && final(columns).boundaries_complete(),
))]
fn build_event_leaf_axes(
    #[cfg(verus_only)] leaves: &[QuadLeaf64],
    row_buckets: &[IntervalBucket; EDGE],
    column_buckets: &[IntervalBucket; EDGE],
    rows: &mut AxisIntervals,
    columns: &mut AxisIntervals,
) -> Result<(), SparseQuadError> {
    proof! { input_proof::seeded_domain(leaves@, row_buckets@, false); input_proof::seeded_domain(leaves@, column_buckets@, true); }
    let result = build_axis_intervals_from_buckets(row_buckets, rows, true);
    proof! { if result is Err { input_proof::seeded_validity(leaves@, row_buckets@, false); input_proof::checked_status(leaves@, Err(result->Err_0)); } }
    result?;
    proof! {
        input_proof::event_axis_completed(leaves@, row_buckets@, rows, false);
        input_proof::seeded_validity(leaves@, column_buckets@, true);
    }
    build_axis_intervals_from_buckets(column_buckets, columns, false)?;
    proof! { input_proof::event_axis_completed(leaves@, column_buckets@, columns, true); }
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(rows).valid(), old(columns).valid(),
        input_proof::seeded(leaves@, row_buckets@, false), input_proof::seeded(leaves@, column_buckets@, true),
        forall|i: int| 0 <= i < leaves@.len() ==> #[trigger] leaves@[i].lod == 0,
    ensures result.is_ok(), input_proof::valid(leaves@),
        final(rows).represents(input_proof::labels(leaves@), false), final(columns).represents(input_proof::labels(leaves@), true),
        final(rows).boundaries_complete(), final(columns).boundaries_complete(),
))]
fn build_unit_leaf_axes(
    #[cfg(verus_only)] leaves: &[QuadLeaf64],
    row_buckets: &[IntervalBucket; EDGE],
    column_buckets: &[IntervalBucket; EDGE],
    rows: &mut AxisIntervals,
    columns: &mut AxisIntervals,
) -> Result<(), SparseQuadError> {
    proof! { input_proof::seeded_units(leaves@, row_buckets@, false); input_proof::seeded_units(leaves@, column_buckets@, true); }
    build_complete_axis_intervals_from_buckets(row_buckets, rows)?;
    build_complete_axis_intervals_from_buckets(column_buckets, columns)?;
    proof! { input_proof::unit_axis_completed(leaves@, row_buckets@, rows, false); input_proof::unit_axis_completed(leaves@, column_buckets@, columns, true); }
    Ok(())
}

/// 只清一个 bucket 的占用位，所有槽位内容保持不变。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires index < 64,
    ensures final(buckets)@[index as int].valid(), final(buckets)@[index as int].starts == 0,
        forall|i: int| 0 <= i < 64 ==> #[trigger] final(buckets)@[i].items@ == old(buckets)@[i].items@,
        forall|i: int| 0 <= i < 64 && i != index ==> #[trigger] final(buckets)@[i] == old(buckets)@[i],
))]
fn clear_interval_bucket(buckets: &mut [IntervalBucket; EDGE], index: usize) {
    {
        let bucket = crate::optimal::get_mut(buckets, index);
        bucket.clear();
    }
    proof! { proof::bucket_items_frame(old(buckets)@, buckets@, index); }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    ensures forall|i: int| 0 <= i < 64 ==> #[trigger] final(buckets)@[i].valid(),
        forall|i: int| 0 <= i < 64 ==> #[trigger] final(buckets)@[i].starts == 0,
        forall|i: int| 0 <= i < 64 ==> #[trigger] final(buckets)@[i].items@ == old(buckets)@[i].items@,
))]
fn prepare_interval_buckets(buckets: &mut [IntervalBucket; EDGE]) {
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= 64,
            forall|i: int| 0 <= i < index ==> #[trigger] buckets@[i].valid(),
            forall|i: int| 0 <= i < index ==> #[trigger] buckets@[i].starts == 0,
            forall|i: int| 0 <= i < 64 ==> #[trigger] buckets@[i].items@ == old(buckets)@[i].items@,
        decreases 64 - index,
    ))]
    while index < EDGE {
        clear_interval_bucket(buckets, index);
        index += 1;
    }
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::leaf_valid(leaf), size == crate::geometry::side(leaf.lod),
        forall|line: int| 0 <= line < 64 ==> #[trigger] old(buckets)@[line].valid(),
    ensures input_proof::origin_write(old(buckets)@, final(buckets)@, leaf, COLUMNS, result),
        forall|line: int| 0 <= line < 64 ==> #[trigger] final(buckets)@[line].valid(),
))]
fn push_leaf_origin<const COLUMNS: bool>(
    leaf: QuadLeaf64,
    size: u8,
    buckets: &mut [IntervalBucket; EDGE],
) -> Result<(), SparseQuadError> {
    let line = if COLUMNS { leaf.u } else { leaf.v };
    let start = if COLUMNS { leaf.v } else { leaf.u };
    let interval = Interval {
        start,
        end: start + size,
        value: leaf.value.get(),
    };
    proof! { input_proof::bit_test(buckets@[line as int].starts, start); }
    let result = {
        let bucket = crate::optimal::get_mut(buckets, usize::from(line));
        bucket.try_push(interval)
    };
    proof! { input_proof::origin_inserted(old(buckets)@, buckets@, leaf, COLUMNS, result); }
    result
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires forall|line: int| 0 <= line < 64 ==> #[trigger] old(row_buckets)@[line].valid(),
        forall|line: int| 0 <= line < 64 ==> #[trigger] old(column_buckets)@[line].valid(),
    ensures input_proof::seed_write(old(row_buckets)@, final(row_buckets)@, old(column_buckets)@, final(column_buckets)@, leaf, result),
        input_proof::seed_context(old(row_buckets)@, final(row_buckets)@, old(column_buckets)@, final(column_buckets)@, leaf, result),
        forall|line: int| 0 <= line < 64 ==> #[trigger] final(row_buckets)@[line].valid(),
        forall|line: int| 0 <= line < 64 ==> #[trigger] final(column_buckets)@[line].valid(),
))]
fn push_leaf_axis_intervals_one(
    leaf: QuadLeaf64,
    row_buckets: &mut [IntervalBucket; EDGE],
    column_buckets: &mut [IntervalBucket; EDGE],
) -> Result<(), SparseQuadError> {
    let result = match validate_leaf_shape(leaf) {
        Ok(size) => match push_leaf_origin::<false>(leaf, size, row_buckets) {
            Ok(()) => push_leaf_origin::<true>(leaf, size, column_buckets),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    proof! { input_proof::seed_frontiers(old(row_buckets)@, row_buckets@, old(column_buckets)@, column_buckets@, leaf, result); }
    result
}

/// 原点事件表不要求输入排序；相同原点立即拒绝，其余覆盖重叠由行扫描检查。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures forall|line: int| 0 <= line < 64 ==> #[trigger] final(row_buckets)@[line].valid(),
        forall|line: int| 0 <= line < 64 ==> #[trigger] final(column_buckets)@[line].valid(),
        match result {
            Ok(unit) => input_proof::seeded(leaves@, final(row_buckets)@, false)
                && input_proof::seeded(leaves@, final(column_buckets)@, true)
                && unit == (forall|i: int| 0 <= i < leaves@.len() ==> #[trigger] leaves@[i].lod == 0),
            Err(error) => input_proof::checked(leaves@, Err(error)) && failure_proof::seed_error(leaves@, error),
        },
))]
fn seed_leaf_axis_intervals(
    leaves: &[QuadLeaf64],
    row_buckets: &mut [IntervalBucket; EDGE],
    column_buckets: &mut [IntervalBucket; EDGE],
) -> Result<bool, SparseQuadError> {
    prepare_interval_buckets(row_buckets);
    prepare_interval_buckets(column_buckets);
    proof! { input_proof::seeded_empty(row_buckets@, false); input_proof::seeded_empty(column_buckets@, true); }
    let mut all_lod_zero = true;
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= leaves@.len(),
            input_proof::seeded(leaves@.take(index as int), row_buckets@, false),
            input_proof::seeded(leaves@.take(index as int), column_buckets@, true),
            all_lod_zero == (forall|i: int| 0 <= i < index ==> #[trigger] leaves@[i].lod == 0),
        decreases leaves@.len() - index,
    ))]
    while index < leaves.len() {
        let leaf = crate::optimal::copy(leaves, index);
        let result = push_leaf_axis_intervals_one(leaf, row_buckets, column_buckets);
        proof! { input_proof::prefix_step(leaves@, index as int); }
        proof! { if result is Err {
            failure_proof::seed_failed(leaves@, index as int, result->Err_0);
            input_proof::error_prefix(leaves@, index as int + 1, result->Err_0);
        } }
        result?;
        all_lod_zero &= leaf.lod == 0;
        index += 1;
    }
    proof! { assert(leaves@.take(index as int) =~= leaves@); }
    Ok(all_lod_zero)
}

// 轴区间构建

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires bucket.disjoint(), old(output).valid(), old(output).intervals@.len() <= MAX_AXIS_INTERVALS - EDGE,
    ensures result.is_ok(), final(output).valid(),
        final(output).lines == old(output).lines, final(output).boundaries == old(output).boundaries,
        result->Ok_0.start == old(output).intervals@.len(), result->Ok_0.end == final(output).intervals@.len(),
        old(output).intervals@.len() <= final(output).intervals@.len() <= old(output).intervals@.len() + EDGE,
        final(output).intervals@.take(old(output).intervals@.len() as int) == old(output).intervals@,
        proof::line_valid(final(output).intervals@.skip(old(output).intervals@.len() as int)),
        forall|p: int| 0 <= p < 64 ==>
            #[trigger] proof::line_value(final(output).intervals@.skip(old(output).intervals@.len() as int), p)
                == proof::bucket_value(bucket, p),
))]
fn append_bucket_line(
    bucket: &IntervalBucket,
    output: &mut AxisIntervals,
) -> Result<LineRange, SparseQuadError> {
    let start = output.intervals.len();
    let mut starts = bucket.starts;
    reserve_exact(&output.intervals, starts.count_ones() as usize)?;
    proof! { proof::bucket_started(bucket); assert(output.intervals@.skip(start as int) =~= Seq::<Interval>::empty()); }
    #[cfg_attr(verus_only, verus_spec(
        invariant output.valid(), start == old(output).intervals@.len(), start <= MAX_AXIS_INTERVALS - EDGE,
            output.lines == old(output).lines, output.boundaries == old(output).boundaries,
            forall|i: int| 0 <= i < output.lines@.len() ==> #[trigger] output.lines@[i].end <= start,
            output.intervals@.take(start as int) == old(output).intervals@,
            proof::bucket_progress(bucket, output.intervals@.skip(start as int), starts),
            output.intervals@.len() == start + output.intervals@.skip(start as int).len(),
        decreases starts,
    ))]
    while starts != 0 {
        proof_decl! { let ghost previous = output.intervals@.skip(start as int); }
        proof! { proof::lowest(starts); }
        let slot = starts.trailing_zeros() as usize;
        let interval = *get(&bucket.items, slot);
        proof! { proof::bucket_next(bucket, output.intervals@.skip(start as int), starts,
        slot as u64, starts & ((starts - 1) as u64)); }
        push_merged_interval(output, start, interval)?;
        proof! { assert(output.intervals@.skip(start as int) =~= proof::merged(previous, interval)); }
        starts &= starts - 1;
    }
    proof! { let mask = bucket.starts; assert(mask & !0u64 == mask) by(bit_vector); }
    Ok(LineRange {
        start,
        end: output.intervals.len(),
    })
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires line < 64, proof::complete_progress(buckets@, old(output), line as int),
        buckets@[line as int].disjoint(), old(output).range_valid(previous),
        line == 0 ==> previous.start == 0 && previous.end == 0,
        line > 0 ==> previous == old(output).lines@[line as int - 1],
    ensures result.is_ok(), proof::complete_progress(buckets@, final(output), line as int + 1),
        final(output).range_valid(result->Ok_0), final(output).lines@[line as int] == result->Ok_0,
))]
fn publish_bucket_line(
    buckets: &[IntervalBucket; EDGE],
    output: &mut AxisIntervals,
    line: usize,
    previous: LineRange,
) -> Result<LineRange, SparseQuadError> {
    let bucket = get(buckets, line);
    let appended = append_bucket_line(bucket, output)?;
    let start = appended.start;
    let end = appended.end;
    proof! {
        assert(output.intervals@.take(old(output).intervals@.len() as int) =~= old(output).intervals@);
        proof::keep_published(output, old(output));
    }
    proof_decl! { let ghost new_line = output.intervals@.subrange(start as int, end as int); }
    proof! { assert(new_line =~= output.intervals@.skip(start as int)); }
    let reused = end - start == previous.end - previous.start
        && slice(&output.intervals, start..end)
            == slice(&output.intervals, previous.start..previous.end);
    let range = if reused {
        output.intervals.resize(start, EMPTY_INTERVAL);
        proof! { assert(output.intervals@ == old(output).intervals@); }
        previous
    } else {
        output.boundaries |= 1u64 << line;
        LineRange { start, end }
    };
    proof! {
        assert(output.intervals@.take(old(output).intervals@.len() as int) =~= old(output).intervals@);
        proof::keep_published(output, old(output));
        assert(output.range_valid(range));
        assert(output.intervals@.subrange(range.start as int, range.end as int) == new_line);
    }
    output.publish_line(line, range);
    proof! {
        proof::boundary_kept(old(output).boundaries, output.boundaries, line as u64);
        proof::complete_step_finished(buckets@, old(output), output, line, range, previous, new_line);
    }
    Ok(range)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(output).valid(), forall|i: int| 0 <= i < 64 ==> #[trigger] buckets@[i].disjoint(),
    ensures result.is_ok(), proof::complete_progress(buckets@, final(output), 64),
))]
fn build_complete_axis_intervals_from_buckets(
    buckets: &[IntervalBucket; EDGE],
    output: &mut AxisIntervals,
) -> Result<(), SparseQuadError> {
    output.clear_for_build()?;
    let mut previous = LineRange { start: 0, end: 0 };
    let mut line = 0;
    proof! { proof::complete_started(buckets@, output); }
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::complete_progress(buckets@, output, line as int), output.range_valid(previous),
            line == 0 ==> previous.start == 0 && previous.end == 0,
            line > 0 ==> previous == output.lines@[line as int - 1],
            forall|i: int| 0 <= i < 64 ==> #[trigger] buckets@[i].disjoint(),
        decreases 64 - line,
    ))]
    while line < EDGE {
        previous = publish_bucket_line(buckets, output, line, previous)?;
        line += 1;
    }
    Ok(())
}

/// 事件扫描阶段独占的活跃槽与到期位；复用原先两个局部缓冲。
#[cfg_attr(verus_only, verus_verify)]
struct AxisEventSweep {
    active: IntervalBucket,
    end_masks: [u64; EDGE],
}

#[cfg_attr(verus_only, verus_verify)]
impl AxisEventSweep {
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result.active.valid(), result.active.starts == 0,
            forall|line: int| 0 <= line < 64 ==> #[trigger] result.end_masks@[line] == 0,
    ))]
    const fn new() -> Self {
        Self {
            active: IntervalBucket::new(),
            end_masks: [0u64; EDGE],
        }
    }

    /// 先删除本行到期的槽；没有起止事件时不写入活跃状态。
    #[inline]
    #[cfg_attr(verus_only, verus_spec(changed =>
        requires line < 64, event_proof::model(buckets@, old(self), line as int - 1, 0),
        ensures event_proof::model(buckets@, final(self), line as int, buckets@[line as int].starts),
            changed == (buckets@[line as int].starts | old(self).end_masks@[line as int] != 0),
            final(self).active.starts == old(self).active.starts & !old(self).end_masks@[line as int],
            final(self).active.items@ == old(self).active.items@, final(self).end_masks@ == old(self).end_masks@,
            !changed ==> buckets@[line as int].starts == 0 && final(self).active.starts == old(self).active.starts
                && final(self).active.items@ == old(self).active.items@,
    ))]
    fn expire_line(&mut self, buckets: &[IntervalBucket; EDGE], line: usize) -> bool {
        let expired = *get(&self.end_masks, line);
        let changed = get(buckets, line).starts | expired != 0;
        proof! { event_proof::union_empty(buckets@[line as int].starts, expired); }
        if changed {
            self.active.starts &= !expired;
        }
        proof! {
            if !changed {
                let mask = old(self).active.starts;
                assert(mask & !expired == mask) by(bit_vector) requires expired == 0;
            }
            event_proof::expired(buckets@, old(self), self, line);
        }
        changed
    }

    /// 插入当前最低原点事件并登记到期位；相同活跃起点的碰撞已经构成覆盖重叠。
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires line < 64, *old(remaining) != 0,
            event_proof::model(buckets@, old(self), line as int, *old(remaining)),
        ensures match result {
            Ok(()) => event_proof::model(buckets@, final(self), line as int, *final(remaining))
                && *final(remaining) < *old(remaining),
            Err(error) => error is Overlap && event_proof::overlap(buckets@),
        },
    ))]
    fn insert_event(
        &mut self,
        buckets: &[IntervalBucket; EDGE],
        line: usize,
        remaining: &mut u64,
    ) -> Result<(), SparseQuadError> {
        proof! { proof::lowest(*remaining); }
        let slot = remaining.trailing_zeros() as usize;
        proof! { event_proof::selected(buckets@, self, line, *remaining, slot as u64); input_proof::bit_test(self.active.starts, slot as u8); }
        let interval = *get(&get(buckets, line).items, slot);
        let result = self.active.try_push(interval);
        proof! { if result is Err { event_proof::collision(buckets@, old(self), line, *old(remaining), slot as u64); } }
        result?;
        let end_line = line + usize::from(interval.end - interval.start);
        if end_line < EDGE {
            *crate::optimal::get_mut(&mut self.end_masks, end_line) |= 1u64 << interval.start;
        }
        *remaining &= *remaining - 1;
        proof! { event_proof::inserted(buckets@, old(self), self, line, *old(remaining), *remaining, slot as u64); }
        Ok(())
    }

    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires line < 64, event_proof::model(buckets@, old(self), line as int, buckets@[line as int].starts),
        ensures match result {
            Ok(()) => event_proof::model(buckets@, final(self), line as int, 0),
            Err(error) => error is Overlap && event_proof::overlap(buckets@),
        },
    ))]
    fn insert_line_events(
        &mut self,
        buckets: &[IntervalBucket; EDGE],
        line: usize,
    ) -> Result<(), SparseQuadError> {
        let mut remaining = get(buckets, line).starts;
        #[cfg_attr(verus_only, verus_spec(
            invariant event_proof::model(buckets@, self, line as int, remaining),
            decreases remaining,
        ))]
        while remaining != 0 {
            self.insert_event(buckets, line, &mut remaining)?;
        }
        Ok(())
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires line < 64, event_proof::progress(buckets@, old(state), old(output), line as int, previous),
        !validate_overlap ==> !event_proof::overlap(buckets@),
    ensures final(output).valid(),
        match result {
            Ok(range) => event_proof::progress(buckets@, final(state), final(output), line as int + 1, range),
            Err(error) => error is Overlap && event_proof::overlap(buckets@),
        },
))]
fn publish_event_line(
    buckets: &[IntervalBucket; EDGE],
    state: &mut AxisEventSweep,
    output: &mut AxisIntervals,
    line: usize,
    previous: LineRange,
    validate_overlap: bool,
) -> Result<LineRange, SparseQuadError> {
    let changed = state.expire_line(buckets, line);
    let range = if changed {
        output.boundaries |= 1u64 << line;
        state.insert_line_events(buckets, line)?;
        proof! { if !validate_overlap { event_proof::active_disjoint(buckets@, state, line as int); } }
        let result = append_active_line(&state.active, output, validate_overlap);
        proof! {
            assert(output.intervals@.take(old(output).intervals@.len() as int) =~= old(output).intervals@);
            proof::keep_published(output, old(output));
        }
        match result {
            Ok(range) => range,
            Err(error) => {
                proof! { event_proof::active_overlap(buckets@, state, line as int); }
                return Err(error);
            }
        }
    } else {
        previous
    };
    proof! {
        assert(output.intervals@.take(old(output).intervals@.len() as int) =~= old(output).intervals@);
        assert(output.range_valid(range));
        proof::boundary_kept(old(output).boundaries, output.boundaries, line as u64);
    }
    output.publish_line(line, range);
    proof! { event_proof::step_finished(buckets@, old(state), state, old(output), output, line, previous, range); }
    Ok(range)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(output).valid(), event_proof::domain(buckets@),
        !validate_overlap ==> !event_proof::overlap(buckets@),
    ensures final(output).valid(),
        match result {
            Ok(()) => !event_proof::overlap(buckets@) && final(output).lines@.len() == 64 && final(output).boundaries_complete()
                && (forall|line: int, pos: int| 0 <= line < 64 && 0 <= pos < 64 ==> #[trigger]
                    proof::line_value(final(output).line_spec(line), pos) == event_proof::value_at(buckets@, line, pos)),
            Err(error) => error is Overlap && event_proof::overlap(buckets@),
        },
))]
fn build_axis_intervals_from_buckets(
    buckets: &[IntervalBucket; EDGE],
    output: &mut AxisIntervals,
    validate_overlap: bool,
) -> Result<(), SparseQuadError> {
    output.clear_for_build()?;
    let mut state = AxisEventSweep::new();
    let mut range = LineRange { start: 0, end: 0 };
    let mut line = 0;
    proof! { event_proof::started(buckets@, &state); event_proof::progress_started(buckets@, &state, output); }
    #[cfg_attr(verus_only, verus_spec(
        invariant event_proof::progress(buckets@, &state, output, line as int, range),
            !validate_overlap ==> !event_proof::overlap(buckets@),
        decreases 64 - line,
    ))]
    while line < EDGE {
        range = publish_event_line(buckets, &mut state, output, line, range, validate_overlap)?;
        line += 1;
    }
    proof! { event_proof::progress_completed(buckets@, &state, output, range); }
    Ok(())
}

/// 以起点顺序检查并合并活跃区间；重叠检查仍在对应区间的追加之前。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires bucket.valid(), old(output).valid(), old(output).intervals@.len() <= MAX_AXIS_INTERVALS - EDGE,
        !validate_overlap ==> bucket.disjoint(),
    ensures final(output).valid(),
        final(output).lines == old(output).lines, final(output).boundaries == old(output).boundaries,
        old(output).intervals@.len() <= final(output).intervals@.len() <= old(output).intervals@.len() + EDGE,
        final(output).intervals@.take(old(output).intervals@.len() as int) == old(output).intervals@,
        proof::line_valid(final(output).intervals@.skip(old(output).intervals@.len() as int)),
        match result {
            Ok(range) => range.start == old(output).intervals@.len() && range.end == final(output).intervals@.len() && bucket.disjoint()
                && (forall|p: int| 0 <= p < 64 ==> #[trigger] proof::line_value(final(output).intervals@.skip(old(output).intervals@.len() as int), p)
                    == proof::bucket_value(bucket, p)),
            Err(error) => error is Overlap && !bucket.disjoint(),
        },
))]
fn append_active_line(
    bucket: &IntervalBucket,
    output: &mut AxisIntervals,
    validate_overlap: bool,
) -> Result<LineRange, SparseQuadError> {
    let start = output.intervals.len();
    let mut remaining = bucket.starts;
    reserve_exact(&output.intervals, remaining.count_ones() as usize)?;
    let mut previous_end = 0u8;
    proof! { event_proof::ordered_started(bucket); assert(output.intervals@.skip(start as int) =~= Seq::<Interval>::empty()); }
    #[cfg_attr(verus_only, verus_spec(
        invariant output.valid(), start == old(output).intervals@.len(), start <= MAX_AXIS_INTERVALS - EDGE,
            output.lines == old(output).lines, output.boundaries == old(output).boundaries,
            forall|i: int| 0 <= i < output.lines@.len() ==> #[trigger] output.lines@[i].end <= start,
            output.intervals@.take(start as int) == old(output).intervals@,
            output.intervals@.len() == start + output.intervals@.skip(start as int).len(),
            event_proof::ordered_prefix(bucket, output.intervals@.skip(start as int), remaining, previous_end),
            !validate_overlap ==> bucket.disjoint(),
        decreases remaining,
    ))]
    while remaining != 0 {
        proof_decl! { let ghost before = output.intervals@.skip(start as int); let ghost old_end = previous_end; let ghost old_remaining = remaining; }
        proof! { proof::lowest(remaining); }
        let slot = remaining.trailing_zeros() as usize;
        proof! { event_proof::ordered_selected(bucket, before, remaining, previous_end, slot as u64); }
        let interval = *get(&bucket.items, slot);
        if validate_overlap && previous_end > interval.start {
            return Err(SparseQuadError::Overlap);
        }
        previous_end = interval.end;
        push_merged_interval(output, start, interval)?;
        remaining &= remaining - 1;
        proof! {
            assert(output.intervals@.skip(start as int) =~= proof::merged(before, interval));
            event_proof::ordered_advanced(bucket, before, output.intervals@.skip(start as int), old_remaining, remaining, old_end, slot as u64);
        }
    }
    proof! { event_proof::ordered_complete(bucket, output.intervals@.skip(start as int), previous_end); }
    Ok(LineRange {
        start,
        end: output.intervals.len(),
    })
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(output).valid(), line_start <= old(output).intervals@.len() < MAX_AXIS_INTERVALS,
        proof::appendable(old(output).intervals@.skip(line_start as int), interval),
        forall|i: int| 0 <= i < old(output).lines@.len() ==> #[trigger] old(output).lines@[i].end <= line_start,
    ensures result.is_ok(), final(output).valid(),
        final(output).lines == old(output).lines, final(output).boundaries == old(output).boundaries,
        final(output).intervals@ == old(output).intervals@.take(line_start as int)
            + proof::merged(old(output).intervals@.skip(line_start as int), interval),
))]
fn push_merged_interval(
    output: &mut AxisIntervals,
    line_start: usize,
    interval: Interval,
) -> Result<(), SparseQuadError> {
    proof! { proof::prefix_preserves_owner(output, line_start); }
    if line_start < output.intervals.len() {
        let last_index = output.intervals.len() - 1;
        proof! { proof::slot_preserves_owner(output, last_index); }
        let joined = {
            let last = output.intervals.slot_mut(last_index);
            if last.value == interval.value && last.end == interval.start {
                last.end = interval.end;
                true
            } else {
                false
            }
        };
        if joined {
            proof! {
                assert(output.intervals@ =~= old(output).intervals@.take(line_start as int)
                    + proof::merged(old(output).intervals@.skip(line_start as int), interval));
                assert(output.intervals@.take(line_start as int) =~= old(output).intervals@.take(line_start as int));
                proof::keep_prefix(output, old(output), line_start);
            }
            return Ok(());
        }
    }
    reserve_one(&output.intervals)?;
    proof! { proof::append_preserves_owner(output, interval); }
    output.intervals.push(interval);
    proof! {
        assert(output.intervals@ =~= old(output).intervals@.take(line_start as int)
            + proof::merged(old(output).intervals@.skip(line_start as int), interval));
        assert(output.intervals@.take(line_start as int) =~= old(output).intervals@.take(line_start as int));
        proof::keep_prefix(output, old(output), line_start);
    }
    Ok(())
}

// Chord 提取

#[cfg_attr(verus_only, verus_verify)]
struct SparseChords {
    horizontal: ChordBuffer<ValuedChord>,
    vertical: ChordBuffer<ValuedChord>,
}

#[cfg_attr(verus_only, verus_verify)]
impl SparseChords {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.horizontal.valid(), result.vertical.valid(),
        result.horizontal@.len() == 0, result.vertical@.len() == 0,
    ))]
    const fn new() -> Self {
        Self {
            horizontal: FixedVec::new(EMPTY_CHORD),
            vertical: FixedVec::new(EMPTY_CHORD),
        }
    }

    #[cfg_attr(verus_only, verus_spec(
        requires old(self).horizontal.valid(), old(self).vertical.valid(),
        ensures final(self).horizontal.valid(), final(self).vertical.valid(),
            final(self).horizontal@.len() == 0, final(self).vertical@.len() == 0,
    ))]
    const fn clear(&mut self) {
        self.horizontal.clear();
        self.vertical.clear();
    }
}

/// 验证目标：完整且无重复地提取每条内部格线两侧最大同标签区间 a、b 的非空交集，
/// 条件为 `a.start != b.start ∧ a.end != b.end`。交集端点各有恰三个该标签邻单元，
/// 内部格点有四个；这与同标签凹角间内部 chord 的几何定义须证明等价。
/// 同一直线的 chord 闭段互不相交，不同标签的 chord 连端点也不相交。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires rows.valid(), rows.lines@.len() == 64, rows.boundaries_complete(),
        columns.represents(rows.labels(), true), columns.boundaries_complete(),
        old(groups).horizontal.valid(), old(groups).vertical.valid(),
    ensures result.is_ok(), final(groups).horizontal.valid(), final(groups).vertical.valid(),
        chord_proof::axis_collected(rows.labels(), false, final(groups).horizontal@, 64),
        chord_proof::axis_collected(rows.labels(), true, final(groups).vertical@, 64),
        chord_proof::ids(final(groups).horizontal@).to_set() == crate::partition::good::chords(rows.labels(), false),
        chord_proof::ids(final(groups).vertical@).to_set() == crate::partition::good::chords(rows.labels(), true),
        crate::optimal::graph::proof::chords_valid(final(groups).horizontal@, false),
        crate::optimal::graph::proof::chords_valid(final(groups).vertical@, true),
))]
fn extract_sparse_chords(
    rows: &AxisIntervals,
    columns: &AxisIntervals,
    groups: &mut SparseChords,
) -> Result<(), SparseQuadError> {
    groups.clear();
    proof! { chord_proof::rows_self(rows); }
    let mut y = 1u8;
    #[cfg_attr(verus_only, verus_spec(
        invariant 1 <= y <= 64, rows.represents(rows.labels(), false), rows.boundaries_complete(),
            groups.horizontal.valid(), groups.vertical.valid(), groups.vertical@.len() == 0,
            chord_proof::axis_collected(rows.labels(), false, groups.horizontal@, y as int),
        decreases 64 - y,
    ))]
    while y < EDGE_U8 {
        proof! {
            chord_proof::axis_line(rows, rows.labels(), false, y as int - 1);
            chord_proof::axis_line(rows, rows.labels(), false, y as int);
            chord_proof::boundary_bit(rows.boundaries, y);
        }
        if rows.boundaries & (1u64 << y) != 0 {
            collect_chords_for_boundary::<true>(
                rows.line(usize::from(y - 1)),
                rows.line(usize::from(y)),
                y,
                &mut groups.horizontal,
                #[cfg(verus_only)]
                verus_exec_expr! { Ghost(rows.labels()) },
            )?;
        }
        proof! {
            if rows.boundaries & (1u64 << y) == 0 { chord_proof::same_lines_empty(rows.labels(), false, y, rows.line_spec(y as int)); }
        }
        y += 1;
    }
    let mut x = 1u8;
    #[cfg_attr(verus_only, verus_spec(
        invariant 1 <= x <= 64, columns.represents(rows.labels(), true), columns.boundaries_complete(),
            groups.horizontal.valid(), groups.vertical.valid(),
            chord_proof::axis_collected(rows.labels(), false, groups.horizontal@, 64),
            chord_proof::axis_collected(rows.labels(), true, groups.vertical@, x as int),
        decreases 64 - x,
    ))]
    while x < EDGE_U8 {
        proof! {
            chord_proof::axis_line(columns, rows.labels(), true, x as int - 1);
            chord_proof::axis_line(columns, rows.labels(), true, x as int);
            chord_proof::boundary_bit(columns.boundaries, x);
        }
        if columns.boundaries & (1u64 << x) != 0 {
            collect_chords_for_boundary::<false>(
                columns.line(usize::from(x - 1)),
                columns.line(usize::from(x)),
                x,
                &mut groups.vertical,
                #[cfg(verus_only)]
                verus_exec_expr! { Ghost(rows.labels()) },
            )?;
        }
        proof! {
            if columns.boundaries & (1u64 << x) == 0 { chord_proof::same_lines_empty(rows.labels(), true, x, columns.line_spec(x as int)); }
        }
        x += 1;
    }
    proof! {
        chord_proof::extraction_complete(rows.labels(), false, groups.horizontal@);
        chord_proof::extraction_complete(rows.labels(), true, groups.vertical@);
        chord_proof::graph_domain(rows.labels(), false, groups.horizontal@);
        chord_proof::graph_domain(rows.labels(), true, groups.vertical@);
    }
    Ok(())
}

/// 输入是已合并的最大同色区间。
/// 两段同色且相交时，起点不等意味着交集左侧恰有一个同色像素，即凹角；
/// 终点不等给出另一端的凹角。无需重新查询相邻像素。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(output).valid(), 1 <= coordinate <= 63,
        chord_proof::line_matches(labels@, !HORIZONTAL, coordinate as int - 1, first@),
        chord_proof::line_matches(labels@, !HORIZONTAL, coordinate as int, second@),
        chord_proof::all_good(labels@, !HORIZONTAL, old(output)@), chord_proof::ids(old(output)@).no_duplicates(),
        chord_proof::ordered(old(output)@),
        forall|p: int| 0 <= p < old(output)@.len() ==> #[trigger] old(output)@[p].line() < coordinate,
    ensures result.is_ok(), final(output).valid(),
        chord_proof::all_good(labels@, !HORIZONTAL, final(output)@), chord_proof::ids(final(output)@).no_duplicates(),
        chord_proof::ordered(final(output)@),
        old(output)@.len() <= final(output)@.len(), final(output)@.take(old(output)@.len() as int) == old(output)@,
        forall|p: int| old(output)@.len() <= p < final(output)@.len() ==> #[trigger] final(output)@[p].line() == coordinate,
        forall|p: int| 0 <= p < final(output)@.len() ==> #[trigger] final(output)@[p].line() <= coordinate,
        forall|id: int| chord_proof::ids(old(output)@).contains(id) ==> #[trigger] chord_proof::ids(final(output)@).contains(id),
        forall|id: int| crate::partition::good::good(labels@, id) && crate::partition::good::vertical(id) == !HORIZONTAL
            && crate::partition::good::line(id) == coordinate ==> #[trigger] chord_proof::ids(final(output)@).contains(id),
))]
fn collect_chords_for_boundary<const HORIZONTAL: bool>(
    first: &[Interval],
    second: &[Interval],
    coordinate: u8,
    output: &mut ChordBuffer<ValuedChord>,
    #[cfg(verus_only)] labels: Ghost<Seq<u16>>,
) -> Result<(), SparseQuadError> {
    let (mut i, mut j) = (0, 0);
    proof! { chord_proof::collector_started(labels@, !HORIZONTAL, coordinate, first@, second@, output@); }
    #[cfg_attr(verus_only, verus_spec(
        invariant output.valid(), chord_proof::collector_progress(labels@, !HORIZONTAL, coordinate, first@, second@, output@, i as int, j as int),
            old(output)@.len() <= output@.len(), output@.take(old(output)@.len() as int) == old(output)@,
            chord_proof::suffix_on_line(output@, old(output)@.len() as int, coordinate),
        decreases (first@.len() - i) + (second@.len() - j),
    ))]
    while i < first.len() && j < second.len() {
        collect_chord_pair::<HORIZONTAL>(
            first,
            second,
            coordinate,
            &mut i,
            &mut j,
            output,
            #[cfg(verus_only)]
            verus_exec_expr! { Ghost(labels@) },
        )?;
    }
    proof! {
        chord_proof::collector_complete(labels@, !HORIZONTAL, coordinate, first@, second@, output@, i as int, j as int);
        chord_proof::prefix_ids(old(output)@, output@);
    }
    Ok(())
}

/// 合并游标的一步；写入当前交集后，耗尽的两侧同时前进。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(output).valid(),
        chord_proof::collector_progress(labels@, !HORIZONTAL, coordinate, first@, second@, old(output)@, *old(i) as int, *old(j) as int),
        *old(i) < first@.len(), *old(j) < second@.len(),
    ensures result.is_ok(), final(output).valid(),
        chord_proof::collector_progress(labels@, !HORIZONTAL, coordinate, first@, second@, final(output)@, *final(i) as int, *final(j) as int),
        *final(i) as int == *old(i) as int + if first@[*old(i) as int].end <= second@[*old(j) as int].end { 1int } else { 0int },
        *final(j) as int == *old(j) as int + if second@[*old(j) as int].end <= first@[*old(i) as int].end { 1int } else { 0int },
        old(output)@.len() <= final(output)@.len(), final(output)@.take(old(output)@.len() as int) == old(output)@,
        forall|p: int| old(output)@.len() <= p < final(output)@.len() ==> #[trigger] final(output)@[p].line() == coordinate,
        forall|start: int| 0 <= start <= old(output)@.len() && chord_proof::suffix_on_line(old(output)@, start, coordinate)
            ==> #[trigger] chord_proof::suffix_on_line(final(output)@, start, coordinate),
))]
fn collect_chord_pair<const HORIZONTAL: bool>(
    first: &[Interval],
    second: &[Interval],
    coordinate: u8,
    i: &mut usize,
    j: &mut usize,
    output: &mut ChordBuffer<ValuedChord>,
    #[cfg(verus_only)] labels: Ghost<Seq<u16>>,
) -> Result<(), SparseQuadError> {
    proof! { chord_proof::line_count(first@); chord_proof::line_count(second@); }
    let a = crate::optimal::copy(first, *i);
    let b = crate::optimal::copy(second, *j);
    let start = a.start.max(b.start);
    let end = a.end.min(b.end);
    if a.value == b.value && a.start != b.start && a.end != b.end && start < end {
        proof! { chord_proof::candidate_ready(labels@, !HORIZONTAL, coordinate, first@, second@, output@, *i as int, *j as int); }
        reserve_one(output)?;
        let chord = ValuedChord::on_axis::<HORIZONTAL>(coordinate, start, end, a.value);
        proof! { assert(chord == chord_proof::converted(!HORIZONTAL, coordinate, a, b)); }
        output.push(chord);
    }
    if a.end <= b.end {
        *i += 1;
    }
    if b.end <= a.end {
        *j += 1;
    }
    proof! { chord_proof::collector_step(labels@, !HORIZONTAL, coordinate, first@, second@,
    old(output)@, output@, *old(i) as int, *old(j) as int, *i as int, *j as int); }
    Ok(())
}

// Cut / 分区类型

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
struct HorizontalCut {
    y: u8,
    start: u8,
    end: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
struct VerticalCut {
    x: u8,
    start: u8,
    end: u8,
}

// 扫描线分区

#[cfg_attr(verus_only, verus_verify)]
struct PartitionScratch {
    runs: FixedVec<Run, EDGE>,
    active: FixedVec<ActiveRect, EDGE>,
    next_active: FixedVec<ActiveRect, EDGE>,
}

#[cfg_attr(verus_only, verus_verify)]
impl PartitionScratch {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.runs.valid(), result.active.valid(),
        result.next_active.valid(), result.runs@.len() == 0, result.active@.len() == 0, result.next_active@.len() == 0))]
    const fn new() -> Self {
        Self {
            runs: FixedVec::new(Run {
                value: 0,
                x_start: 0,
                x_end: 0,
            }),
            active: FixedVec::new(ActiveRect::new(0, 0, 0, 0)),
            next_active: FixedVec::new(ActiveRect::new(0, 0, 0, 0)),
        }
    }
}

/// 验证目标：有效轴区间及非冲突选线产生满足 `Partition(L,R)` 的完成序列。
/// 每个扫描边界之前，已交付矩形与活跃矩形在已扫描单元上构成精确、不重叠覆盖；
/// 活跃矩形仅在标签、x 范围相同且没有水平切线穿过时延长；活跃项的 x 范围非空，
/// y 起点不晚于当前扫描行；交付矩形的宽高均为正。
/// 未标记的行边界不能有标签区间或竖直切线变化，跳行与逐行扫描等价。
/// 对最大独立集选线，还须证明最终数量达到几何下界，不能由覆盖不变量代替。
/// 每行 runs／活跃项至多 64，最终交付至多 4096 项；计数只包含 sink 成功调用。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires scan::proof::ready(rows, horizontal_cut_masks@, vertical_cut_masks@, cut_boundaries),
        old(scratch).runs.valid(), old(scratch).active.valid(), old(scratch).next_active.valid(),
        old(events)@ == Seq::empty(),
    ensures final(scratch).runs.valid(), final(scratch).active.valid(), final(scratch).next_active.valid(),
        sink::proof::delivery(crate::partition::scan_model::canonical(rows.labels(),
            scan::proof::selection(rows.labels(), vertical_cut_masks@), scan::proof::block_fn(horizontal_cut_masks@)),
            final(events)@, result),
))]
fn partition_rows<F>(
    rows: &AxisIntervals,
    horizontal_cut_masks: &FixedVec<u64, EDGE>,
    vertical_cut_masks: &FixedVec<u128, EDGE>,
    cut_boundaries: u64,
    scratch: &mut PartitionScratch,
    sink: F,
    #[cfg(verus_only)] events: &mut Ghost<Seq<sink::proof::Event>>,
) -> Result<usize, SparseQuadError>
where
    F: FnMut(Rectangle) -> Result<(), SparseQuadError>,
{
    let mut scanner = scan::RectScan::new(
        rows,
        horizontal_cut_masks,
        vertical_cut_masks,
        cut_boundaries,
        scratch,
    )?;
    let result = sink::write(
        &mut scanner,
        sink,
        #[cfg(verus_only)]
        events,
    );
    proof! {
        FixedVec::establish(&scratch.runs);
        FixedVec::establish(&scratch.active);
        FixedVec::establish(&scratch.next_active);
    }
    result
}

/// 验证目标：水平第 y 行的第 x 位恰表示选中水平线覆盖单元边 `[x,x+1]`；
/// 竖直第 y 行的第 x 位恰表示选中竖线覆盖 `[y,y+1]`。竖线端点事件的 XOR 前缀
/// 必须等于覆盖集合的 OR，此结论依赖同线 chord 不重叠；变化行必须进入返回的 boundaries。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(horizontal_masks).valid(), old(vertical_masks).valid(),
        mask_proof::horizontal_input(horizontal_cuts@), mask_proof::vertical_input(vertical_cuts@),
    ensures result.is_ok(), final(horizontal_masks).valid(), final(vertical_masks).valid(),
        mask_proof::built(horizontal_cuts@, vertical_cuts@, final(horizontal_masks)@, final(vertical_masks)@, result->Ok_0),
))]
fn build_cut_masks(
    horizontal_cuts: &[HorizontalCut],
    vertical_cuts: &[VerticalCut],
    horizontal_masks: &mut FixedVec<u64, EDGE>,
    vertical_masks: &mut FixedVec<u128, EDGE>,
) -> Result<u64, SparseQuadError> {
    // 位图用 OR 合并，切线顺序无关；无需在收集后排序。
    horizontal_masks.clear();
    horizontal_masks.resize(EDGE, 0);
    vertical_masks.clear();
    vertical_masks.resize(EDGE, 0);

    let mut boundaries = 0u64;

    let mut index = 0;
    proof! { mask_proof::vertical_started(vertical_cuts@); }
    #[cfg_attr(verus_only, verus_spec(
        invariant vertical_masks.valid(), mask_proof::vertical_events(vertical_cuts@, index as int, vertical_masks@, boundaries),
        decreases vertical_cuts@.len() - index,
    ))]
    while index < vertical_cuts.len() {
        insert_vertical_cut(
            crate::optimal::copy(vertical_cuts, index),
            vertical_masks.as_mut_slice(),
            &mut boundaries,
            #[cfg(verus_only)]
            verus_exec_expr! { Ghost((vertical_cuts@, index as int)) },
        )?;
        index += 1;
    }
    // 同向 chord 在相同 x 上连端点都互不重叠，因此每个 x 的覆盖数仅为 0/1。
    // 起止端点 XOR 后做前缀即可重建 OR 覆盖；end=64 在最终关闭时隐式处理。
    let mut active = 0u128;
    let mut row = 0;
    proof! { mask_proof::prefix_started(vertical_cuts@, vertical_masks@, boundaries); }
    #[cfg_attr(verus_only, verus_spec(
        invariant vertical_masks.valid(), mask_proof::vertical_prefix(vertical_cuts@, vertical_masks@, row as int, active),
        decreases 64 - row,
    ))]
    while row < EDGE {
        prefix_vertical_row(
            row,
            vertical_masks.as_mut_slice(),
            &mut active,
            #[cfg(verus_only)]
            verus_exec_expr! { Ghost(vertical_cuts@) },
        );
        row += 1;
    }

    let mut index = 0;
    proof! { mask_proof::horizontal_started(horizontal_cuts@, boundaries); }
    #[cfg_attr(verus_only, verus_spec(
        invariant horizontal_masks.valid(), mask_proof::horizontal_progress(horizontal_cuts@, index as int, horizontal_masks@, boundaries),
            mask_proof::vertical_marked(vertical_cuts@, vertical_cuts@.len() as int, boundaries),
        decreases horizontal_cuts@.len() - index,
    ))]
    while index < horizontal_cuts.len() {
        proof_decl! { let ghost previous = boundaries; }
        insert_horizontal_cut(
            crate::optimal::copy(horizontal_cuts, index),
            horizontal_masks.as_mut_slice(),
            &mut boundaries,
            #[cfg(verus_only)]
            verus_exec_expr! { Ghost((horizontal_cuts@, index as int)) },
        )?;
        proof! { mask_proof::vertical_marks_preserved(vertical_cuts@, vertical_cuts@.len() as int, previous, boundaries); }
        index += 1;
    }

    Ok(boundaries)
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires mask_proof::vertical_events(context@.0, context@.1, old(masks)@, *old(boundaries)),
        context@.1 < context@.0.len(), cut == context@.0[context@.1],
    ensures result.is_ok(), mask_proof::vertical_events(context@.0, context@.1 + 1, final(masks)@, *final(boundaries)),
))]
fn insert_vertical_cut(
    cut: VerticalCut,
    masks: &mut [u128],
    boundaries: &mut u64,
    #[cfg(verus_only)] context: Ghost<(Seq<VerticalCut>, int)>,
) -> Result<(), SparseQuadError> {
    proof! { assert(mask_proof::vertical_valid(cut)); }
    if cut.start >= cut.end {
        return Ok(());
    }
    let bit = 1u128 << u32::from(cut.x);
    {
        let Some(mask) = masks.get_mut(usize::from(cut.start)) else {
            return Err(SparseQuadError::CapacityOverflow);
        };
        *mask ^= bit;
    }
    *boundaries |= 1u64 << cut.start;
    if cut.end < EDGE_U8 {
        *crate::optimal::get_mut(masks, usize::from(cut.end)) ^= bit;
        *boundaries |= 1u64 << cut.end;
    }
    proof! { mask_proof::vertical_inserted(context@.0, context@.1, old(masks)@, masks@, *old(boundaries), *boundaries); }
    Ok(())
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires mask_proof::vertical_prefix(cuts@, old(masks)@, row as int, *old(active)), row < 64,
    ensures mask_proof::vertical_prefix(cuts@, final(masks)@, row as int + 1, *final(active)),
))]
fn prefix_vertical_row(
    row: usize,
    masks: &mut [u128],
    active: &mut u128,
    #[cfg(verus_only)] cuts: Ghost<Seq<VerticalCut>>,
) {
    *active ^= crate::optimal::copy(masks, row);
    *crate::optimal::get_mut(masks, row) = *active;
    proof! { mask_proof::prefix_step(cuts@, old(masks)@, masks@, row as int, *old(active), *active); }
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires mask_proof::horizontal_progress(context@.0, context@.1, old(masks)@, *old(boundaries)),
        context@.1 < context@.0.len(), cut == context@.0[context@.1],
    ensures result.is_ok(), mask_proof::horizontal_progress(context@.0, context@.1 + 1, final(masks)@, *final(boundaries)),
        forall|p: u8| scan_proof::bit64(*old(boundaries), p) ==> #[trigger] scan_proof::bit64(*final(boundaries), p),
))]
fn insert_horizontal_cut(
    cut: HorizontalCut,
    masks: &mut [u64],
    boundaries: &mut u64,
    #[cfg(verus_only)] context: Ghost<(Seq<HorizontalCut>, int)>,
) -> Result<(), SparseQuadError> {
    proof! { assert(mask_proof::horizontal_valid(cut)); }
    let range = cell_range_mask(cut.start, cut.end);
    {
        let Some(mask) = masks.get_mut(usize::from(cut.y)) else {
            return Err(SparseQuadError::CapacityOverflow);
        };
        *mask |= range;
    }
    *boundaries |= 1u64 << cut.y;
    proof! { mask_proof::horizontal_step(context@.0, context@.1, old(masks)@, masks@, *old(boundaries), *boundaries, range); }
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(runs).valid(), proof::line_valid(intervals@),
    ensures result.is_ok(), final(runs).valid(), scan_proof::progress(intervals@,vertical_cut_mask,final(runs)@,64),
))]
fn build_runs_for_row(
    intervals: &[Interval],
    vertical_cut_mask: u128,
    runs: &mut FixedVec<Run, EDGE>,
) -> Result<(), SparseQuadError> {
    if vertical_cut_mask == 0 {
        runs.clear();
        proof! { scan_proof::line_count_bound(intervals@, intervals@.len()); scan_proof::started(intervals@, 0); }
        reserve_exact(runs, intervals.len())?;
        let mut index = 0;
        #[cfg_attr(verus_only, verus_spec(
            invariant index <= intervals@.len(), proof::line_valid(intervals@), runs.valid(),
                scan_proof::progress(intervals@, 0, runs@, if index == 0 { 0 } else { intervals@[index as int - 1].end as int }),
            decreases intervals@.len() - index,
        ))]
        while index < intervals.len() {
            let item = crate::optimal::copy(intervals, index);
            proof! { assert(proof::interval_valid(item)); scan_proof::advance_gap(intervals@, 0, runs@, index as int); }
            proof_decl! { let ghost before = runs@; }
            runs.push(Run {
                value: item.value,
                x_start: item.start,
                x_end: item.end,
            });
            proof! { scan_proof::append_unsplit(intervals@, before, index as int); }
            index += 1;
        }
        proof! { scan_proof::finished(intervals@, 0, runs@); }
        return Ok(());
    }
    runs.clear();
    let mut index = 0;
    proof! { scan_proof::started(intervals@,vertical_cut_mask); }
    #[cfg_attr(verus_only, verus_spec(
        invariant index<=intervals@.len(), proof::line_valid(intervals@), runs.valid(),
            scan_proof::progress(intervals@,vertical_cut_mask,runs@,if index==0{0}else{intervals@[index as int-1].end as int}),
        decreases intervals@.len()-index,
    ))]
    while index < intervals.len() {
        let interval = crate::optimal::copy(intervals, index);
        proof! { assert(proof::interval_valid(intervals@[index as int]));
        scan_proof::advance_gap(intervals@,vertical_cut_mask,runs@,index as int); }
        let mut start = interval.start;
        let coordinates = coord_between_mask(
            interval.start.saturating_add(1),
            interval.end.saturating_sub(1),
        );
        // 内部分割点仅在 1..63，收窄后再枚举。
        let mut split_mask = (vertical_cut_mask & coordinates) as u64;
        proof! { scan_proof::mask_started(vertical_cut_mask,coordinates,start,interval.end,split_mask);
        scan_proof::split_capacity(vertical_cut_mask,split_mask,start as int,interval.end as int); }
        reserve_exact(runs, split_mask.count_ones() as usize + 1)?;
        #[cfg_attr(verus_only, verus_spec(
            invariant index<intervals@.len(),proof::line_valid(intervals@),interval==intervals@[index as int],
                runs.valid(),interval.start<=start<interval.end<=64,
                start==interval.start || scan_proof::bit128(vertical_cut_mask,start as u32),
                scan_proof::progress(intervals@,vertical_cut_mask,runs@,start as int),
                scan_proof::remaining(vertical_cut_mask,split_mask,start as int,interval.end as int),
            decreases split_mask,
        ))]
        while split_mask != 0 {
            proof! { scan_proof::next_split(vertical_cut_mask,split_mask,start as int,interval.end as int); }
            let split = u8::try_from(split_mask.trailing_zeros())
                .map_err(|_| SparseQuadError::CapacityOverflow)?;
            proof! { scan_proof::piece(intervals@,vertical_cut_mask,index as int,start,split); }
            proof_decl! { let ghost before=runs@; }
            push_run(runs, interval.value, start, split)?;
            proof! { scan_proof::append_piece(intervals@,vertical_cut_mask,before,index as int,start,split); }
            start = split;
            split_mask &= split_mask - 1;
        }
        proof! { scan_proof::no_split(vertical_cut_mask,start as int,interval.end as int);
        scan_proof::piece(intervals@,vertical_cut_mask,index as int,start,interval.end); }
        proof_decl! { let ghost before=runs@; }
        push_run(runs, interval.value, start, interval.end)?;
        proof! { scan_proof::append_piece(intervals@,vertical_cut_mask,before,index as int,start,interval.end); }
        index += 1;
    }
    proof! { scan_proof::finished(intervals@,vertical_cut_mask,runs@); }
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(runs).valid(),start<end ==> old(runs)@.len()<64,
    ensures result.is_ok(),final(runs).valid(),final(runs)@==if start<end {
        old(runs)@.push(Run{value,x_start:start,x_end:end}) }else{old(runs)@},
))]
fn push_run(
    runs: &mut FixedVec<Run, EDGE>,
    value: u16,
    start: u8,
    end: u8,
) -> Result<(), SparseQuadError> {
    if start >= end {
        return Ok(());
    }
    reserve_one(runs)?;
    runs.push(Run {
        value,
        x_start: start,
        x_end: end,
    });
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => requires start<=64,end<=64,
    ensures result==scan_proof::overlaps(mask,start,end)))]
fn horizontal_cut_overlaps(mask: u64, start: u8, end: u8) -> bool {
    let range = cell_range_mask(start, end);
    proof! { scan_proof::overlap_mask(mask,start,end,range); }
    mask & range != 0
}

// 工具函数

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires items.valid(),
    ensures result.is_ok() <==> items@.len() + additional <= N,
))]
const fn reserve_exact<T: Copy, const N: usize>(
    items: &FixedVec<T, N>,
    additional: usize,
) -> Result<(), SparseQuadError> {
    if additional <= items.capacity() - items.len() {
        return Ok(());
    }
    Err(SparseQuadError::CapacityOverflow)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires items.valid(),
    ensures result.is_ok() <==> items@.len() < N,
))]
const fn reserve_one<T: Copy, const N: usize>(
    items: &FixedVec<T, N>,
) -> Result<(), SparseQuadError> {
    if items.len() < items.capacity() {
        return Ok(());
    }
    Err(SparseQuadError::CapacityOverflow)
}

// 测试
