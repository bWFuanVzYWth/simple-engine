//! 连续标签切面：按 512-bit 逻辑块比较相邻行／像素，直接构造最大同色区间。

#[cfg(verus_only)]
use super::failure_proof as failure;
#[cfg(verus_only)]
use super::input_proof as input;
#[cfg(verus_only)]
use super::proof as axis_owner;
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "../proof/axis.rs"]
#[cfg(verus_only)]
mod proof;
#[path = "../proof/raster.rs"]
#[cfg(verus_only)]
mod raster_proof;

use super::{
    AxisIntervals, EDGE, EDGE_U8, Interval, IntervalBucket, LineRange, QuadLeaf64,
    SparseOptimalScratch64, SparseQuadError, build_complete_axis_intervals_from_buckets,
    cell_range_mask, prepare_interval_buckets, reserve_exact, validate_leaf_shape,
};
use crate::optimal::types::{PackedRectangles64, Rectangle};
use crate::optimal::{copy, get, get_mut, slice_mut};

// 少量大叶子保留事件路径；大量叶子用紧凑网格减少两轴散写。
#[cfg_attr(verus_only, verus_verify)]
pub(super) const DENSE_LEAF_THRESHOLD: usize = 512;

/// 行主序的 64×64 标签切面。0 为背景，其余值为调用方的颜色／材质键。
/// 固定网格隐含边界与无重叠条件，无需构造单位正方形列表。
#[repr(C, align(64))]
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
pub struct DenseLabels64 {
    rows: [[u16; EDGE]; EDGE],
}

#[cfg_attr(verus_only, verus_verify)]
impl DenseLabels64 {
    /// 创建空切面，不调用堆分配器。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result@ == Seq::new(4096, |_: int| 0u16),
    ))]
    pub const fn new() -> Self {
        let output = Self {
            rows: [[0; EDGE]; EDGE],
        };
        #[cfg(verus_only)]
        proof! { proof::empty_labels(&output); }
        output
    }

    /// 取得行主序标签。每行含两个 512-bit 逻辑块。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures self@.len() == 4096,
            forall|p: int| 0 <= p < 4096 ==> #[trigger] self@[p] == result@[p / 64]@[p % 64],
            forall|y: int, x: int| 0 <= y < 64 && 0 <= x < 64
                ==> #[trigger] result@[y]@[x] == self@[y * 64 + x],
    ))]
    pub const fn rows(&self) -> &[[u16; EDGE]; EDGE] {
        #[cfg(verus_only)]
        proof! { proof::label_layout(self); }
        &self.rows
    }

    /// 上游可直接写入目标布局；任意 u16 标签组合均是合法像素图。
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures old(self)@.len() == 4096, final(self)@.len() == 4096,
            forall|p: int| 0 <= p < 4096 ==> #[trigger] old(self)@[p]
                == result@[p / 64]@[p % 64],
            forall|y: int, x: int| 0 <= y < 64 && 0 <= x < 64
                ==> #[trigger] result@[y]@[x] == old(self)@[y * 64 + x],
            forall|p: int| 0 <= p < 4096 ==> #[trigger] final(self)@[p]
                == final(result)@[p / 64]@[p % 64],
    ))]
    pub const fn rows_mut(&mut self) -> &mut [[u16; EDGE]; EDGE] {
        #[cfg(verus_only)]
        proof! { proof::label_layout(self); }
        &mut self.rows
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl DenseLabels64 {
    /// 清空标签，复用已有存储。
    #[cfg_attr(verus_only, verus_spec(
        ensures final(self)@ == Seq::new(4096, |_: int| 0u16),
    ))]
    pub fn clear(&mut self) {
        {
            let pixels = crate::std_adapter::flatten64(&mut self.rows);
            crate::std_adapter::fill_u16(pixels, 0);
        }
        #[cfg(verus_only)]
        proof! { proof::empty_labels(self); }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl Default for DenseLabels64 {
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result@ == Seq::new(4096, |_: int| 0u16),
    ))]
    fn default() -> Self {
        Self::new()
    }
}

impl SparseOptimalScratch64 {
    /// 直接分解原生连续标签切面，返回借用结果，不分配堆内存。
    /// 输入无需叶子形状、排序或重叠校验；0 表示背景。
    ///
    /// # Errors
    /// 任意标签图均成功。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result => ensures final(self).dense_borrowed(labels@, result)))]
    pub fn decompose_labels_borrowed(
        &mut self,
        labels: &DenseLabels64,
    ) -> Result<&[Rectangle], SparseQuadError> {
        #[cfg(verus_only)]
        proof! { reveal(SparseOptimalScratch64::dense_borrowed); reveal(SparseOptimalScratch64::produced); super::scratch_proof::establish(&*self); }
        build_axes(
            labels,
            &mut self.rows,
            &mut self.columns,
            &mut self.column_buckets,
        )?;
        #[cfg(verus_only)]
        proof! { super::scratch_proof::represented_rows(&self.rows, labels@); }
        self.decompose_intervals_borrowed()
    }
}

impl SparseOptimalScratch64 {
    /// 连续标签切面的直接 sink 输出，库不分配堆内存或写入中间矩形数组。
    /// 成功次数、失败调用与前缀语义同 [`Self::decompose_into`]。
    ///
    /// # Errors
    /// sink 错误原样返回；已输出前缀不回滚。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures final(self).dense_delivery(labels@, final(events)@, result),
    ))]
    pub fn decompose_labels_into<F>(
        &mut self,
        labels: &DenseLabels64,
        sink: F,
        #[cfg(verus_only)] events: &mut Ghost<Seq<super::sink::proof::Event>>,
    ) -> Result<usize, SparseQuadError>
    where
        F: FnMut(Rectangle) -> Result<(), SparseQuadError>,
    {
        #[cfg(verus_only)]
        proof! {
            reveal(SparseOptimalScratch64::dense_delivery);
            events@ = Seq::empty(); super::scratch_proof::establish(self);
        }
        build_axes(
            labels,
            &mut self.rows,
            &mut self.columns,
            &mut self.column_buckets,
        )?;
        #[cfg(verus_only)]
        proof! { super::scratch_proof::represented_rows(&self.rows, labels@); }
        super::extract_sparse_chords(&self.rows, &self.columns, &mut self.chord_groups)?;
        self.select_cuts()?;
        self.sparse_partition_into(
            sink,
            #[cfg(verus_only)]
            events,
        )
    }
}

impl SparseOptimalScratch64 {
    /// 将连续标签切面的矩形直接写入调用方的 packed bounds / labels。
    /// 入口先清空旧输出；成功时返回值、有效长度及 sink 成功次数相同。
    ///
    /// # Errors
    /// 任意标签图均成功。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result => ensures final(self).dense_packed(labels@, final(output), result)))]
    pub fn decompose_labels_packed(
        &mut self,
        labels: &DenseLabels64,
        output: &mut PackedRectangles64,
    ) -> Result<usize, SparseQuadError> {
        #[cfg(verus_only)]
        proof! { reveal(SparseOptimalScratch64::dense_packed); reveal(SparseOptimalScratch64::produced); super::scratch_proof::establish(&*self); }
        output.clear();
        build_axes(
            labels,
            &mut self.rows,
            &mut self.columns,
            &mut self.column_buckets,
        )?;
        #[cfg(verus_only)]
        proof! { super::scratch_proof::represented_rows(&self.rows, labels@); }
        self.decompose_intervals_packed(output)
    }
}

/// 检查并写入一个单位叶子；空像素写入外保留整幅输入视图。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires leaf.lod == 0,
    ensures raster_proof::unit_write(old(labels)@, final(labels)@, *leaf, result),
        raster_proof::leaf_context(old(labels)@, final(labels)@, *leaf, result),
))]
fn rasterize_unit_leaf(
    leaf: &QuadLeaf64,
    labels: &mut DenseLabels64,
) -> Result<(), SparseQuadError> {
    #[cfg(verus_only)]
    proof! { input::unit_bounds(*leaf); proof::label_layout(labels); }
    let result = if (leaf.u | leaf.v) >= EDGE_U8 {
        Err(SparseQuadError::OutOfBounds)
    } else {
        let index = usize::from(leaf.v) * EDGE + usize::from(leaf.u);
        let occupied = {
            let pixel = get_mut(crate::std_adapter::flatten64(&mut labels.rows), index);
            if *pixel != 0 {
                true
            } else {
                *pixel = leaf.value.get();
                false
            }
        };
        #[cfg(verus_only)]
        proof! {
            assert(index as int == input::origin(*leaf));
            proof::label_layout(labels);
            if occupied {
                assert(old(labels)@[index as int] != 0);
                assert(labels@ =~= old(labels)@);
            } else {
                assert(old(labels)@[index as int] == 0);
                assert(labels@ =~= old(labels)@.update(index as int, leaf.value@));
            }
        }
        if occupied {
            Err(SparseQuadError::Overlap)
        } else {
            Ok(())
        }
    };
    #[cfg(verus_only)]
    proof! { raster_proof::unit_frontiers(old(labels)@, labels@, *leaf, result); }
    result
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(labels)@ == Seq::new(4096, |_: int| 0u16),
        forall|i: int| 0 <= i < leaves@.len() ==> #[trigger] leaves@[i].lod == 0,
    ensures input::checked(leaves@, result), result.is_ok() == input::valid(leaves@),
        result.is_ok() ==> final(labels)@ == input::labels(leaves@),
        result is Err ==> failure::raster_error(leaves@, result->Err_0),
))]
fn rasterize_unit_leaves(
    leaves: &[QuadLeaf64],
    labels: &mut DenseLabels64,
) -> Result<(), SparseQuadError> {
    #[cfg(verus_only)]
    proof! { input::empty_labels(); }
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= leaves@.len(), input::valid(leaves@.take(index as int)),
            labels@ == input::labels(leaves@.take(index as int)),
            forall|i: int| 0 <= i < leaves@.len() ==> #[trigger] leaves@[i].lod == 0,
        decreases leaves@.len() - index,
    ))]
    while index < leaves.len() {
        let leaf = get(leaves, index);
        let result = rasterize_unit_leaf(leaf, labels);
        #[cfg(verus_only)]
        proof! { input::prefix_step(leaves@, index as int); }
        #[cfg(verus_only)]
        proof! { if result is Err {
            failure::raster_failed(leaves@, index as int, result->Err_0);
            input::error_prefix(leaves@, index as int + 1, result->Err_0);
            input::checked_status(leaves@, result);
        } }
        result?;
        index += 1;
    }
    #[cfg(verus_only)]
    proof! { assert(leaves@.take(index as int) =~= leaves@); }
    Ok(())
}

/// 先按占用位拒绝重叠，再提交同一行的位与标签；不把占用位解释为事件 bucket。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires start < end <= 64, value != 0, crate::geometry::cell_bits(start, end, mask),
        raster_proof::row_occupancy(old(row)@, *old(occupied)),
    ensures raster_proof::row_step(old(row)@, final(row)@, *old(occupied), *final(occupied), start, end, value, result),
))]
fn rasterize_row(
    row: &mut [u16; EDGE],
    occupied: &mut u64,
    start: u8,
    end: u8,
    mask: u64,
    value: u16,
) -> Result<(), SparseQuadError> {
    let result = if *occupied & mask != 0 {
        Err(SparseQuadError::Overlap)
    } else {
        *occupied |= mask;
        crate::std_adapter::fill_u16(slice_mut(row, usize::from(start)..usize::from(end)), value);
        #[cfg(verus_only)]
        proof! { assert(row@ =~= raster_proof::replaced(old(row)@, start, end, value)); }
        Ok(())
    };
    #[cfg(verus_only)]
    proof! { raster_proof::row_effect(old(row)@, row@, *old(occupied), *occupied, start, end, mask, value, result); }
    result
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires y < 64, start < end <= 64, value != 0, crate::geometry::cell_bits(start, end, mask),
        raster_proof::grid_occupancy(old(labels)@, old(occupied)@),
    ensures raster_proof::row_at_step(old(labels)@, final(labels)@, y, start, end, value, result),
        raster_proof::grid_occupancy(final(labels)@, final(occupied)@),
        forall|line: int| 0 <= line < 64 ==> #[trigger] final(occupied)@[line].items@ == old(occupied)@[line].items@,
))]
fn rasterize_row_at(
    labels: &mut DenseLabels64,
    occupied: &mut [IntervalBucket; EDGE],
    y: usize,
    start: u8,
    end: u8,
    mask: u64,
    value: u16,
) -> Result<(), SparseQuadError> {
    #[cfg(verus_only)]
    proof! { raster_proof::selected_occupancy(labels, occupied@, y); }
    let result = {
        let row_occupied = &mut get_mut(occupied, y).starts;
        let row = get_mut(&mut labels.rows, y);
        rasterize_row(row, row_occupied, start, end, mask, value)
    };
    #[cfg(verus_only)]
    proof! { raster_proof::row_at_effect(old(labels), labels, old(occupied)@, occupied@, y, start, end, value, result); }
    result
}

/// 在叶子的一个横截面提交像素；仅占用位属于该阶段，事件槽保持原值。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires axis_owner::leaf_valid(*leaf), leaf.v <= y < leaf.v as int + crate::geometry::side(leaf.lod),
        end_x == leaf.u as int + crate::geometry::side(leaf.lod),
        crate::geometry::cell_bits(leaf.u, end_x, mask),
        raster_proof::grid_occupancy(old(labels)@, old(occupied)@),
    ensures raster_proof::grid_occupancy(final(labels)@, final(occupied)@),
        raster_proof::leaf_row_context(old(labels)@, final(labels)@, *leaf, y, result),
))]
fn rasterize_leaf_row(
    leaf: &QuadLeaf64,
    labels: &mut DenseLabels64,
    occupied: &mut [IntervalBucket; EDGE],
    y: u8,
    end_x: u8,
    mask: u64,
) -> Result<(), SparseQuadError> {
    let result = rasterize_row_at(
        labels,
        occupied,
        usize::from(y),
        leaf.u,
        end_x,
        mask,
        leaf.value.get(),
    );
    #[cfg(verus_only)]
    proof! { raster_proof::leaf_row_effect(old(labels)@, labels@, *leaf, y, end_x, result); }
    result
}

/// 校验一个任意叶子后逐行提交；重叠时允许保留此前已经写入的行。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires raster_proof::grid_occupancy(old(labels)@, old(occupied)@),
    ensures raster_proof::grid_occupancy(final(labels)@, final(occupied)@),
        raster_proof::leaf_write(old(labels)@, final(labels)@, *leaf, result),
        raster_proof::leaf_context(old(labels)@, final(labels)@, *leaf, result),
))]
fn rasterize_quad_leaf(
    leaf: &QuadLeaf64,
    labels: &mut DenseLabels64,
    occupied: &mut [IntervalBucket; EDGE],
) -> Result<(), SparseQuadError> {
    let side = match validate_leaf_shape(*leaf) {
        Ok(side) => side,
        Err(error) => {
            #[cfg(verus_only)]
            proof! { raster_proof::leaf_frontiers(old(labels)@, labels@, *leaf, Err(error)); }
            return Err(error);
        }
    };
    let end_x = leaf.u + side;
    let end_y = leaf.v + side;
    let mask = cell_range_mask(leaf.u, end_x);
    let mut y = leaf.v;
    #[cfg(verus_only)]
    proof! { raster_proof::leaf_started(labels@, *leaf); }
    #[cfg_attr(verus_only, verus_spec(
        invariant raster_proof::grid_occupancy(labels@, occupied@),
            raster_proof::leaf_frontier(old(labels)@, labels@, *leaf, y),
            end_y == leaf.v as int + crate::geometry::side(leaf.lod),
            end_x == leaf.u as int + crate::geometry::side(leaf.lod),
            crate::geometry::cell_bits(leaf.u, end_x, mask),
        decreases end_y - y,
    ))]
    while y < end_y {
        let result = rasterize_leaf_row(leaf, labels, occupied, y, end_x, mask);
        #[cfg(verus_only)]
        proof! { if result is Err { raster_proof::leaf_frontiers(old(labels)@, labels@, *leaf, result); } }
        result?;
        y += 1;
    }
    #[cfg(verus_only)]
    proof! {
        raster_proof::leaf_completed(old(labels)@, labels@, *leaf, end_y);
        raster_proof::leaf_frontiers(old(labels)@, labels@, *leaf, Ok(()));
    }
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(labels)@ == Seq::new(4096, |_: int| 0u16),
        raster_proof::grid_occupancy(old(labels)@, old(occupied)@),
    ensures raster_proof::grid_occupancy(final(labels)@, final(occupied)@),
        input::checked(leaves@, result), result.is_ok() == input::valid(leaves@),
        result.is_ok() ==> final(labels)@ == input::labels(leaves@),
        result is Err ==> failure::raster_error(leaves@, result->Err_0),
))]
fn rasterize_mixed_leaves(
    leaves: &[QuadLeaf64],
    labels: &mut DenseLabels64,
    occupied: &mut [IntervalBucket; EDGE],
) -> Result<(), SparseQuadError> {
    #[cfg(verus_only)]
    proof! { input::empty_labels(); }
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= leaves@.len(), input::valid(leaves@.take(index as int)),
            labels@ == input::labels(leaves@.take(index as int)),
            raster_proof::grid_occupancy(labels@, occupied@),
        decreases leaves@.len() - index,
    ))]
    while index < leaves.len() {
        let result = rasterize_quad_leaf(get(leaves, index), labels, occupied);
        #[cfg(verus_only)]
        proof! { input::prefix_step(leaves@, index as int); }
        #[cfg(verus_only)]
        proof! { if result is Err {
            failure::raster_failed(leaves@, index as int, result->Err_0);
            input::error_prefix(leaves@, index as int + 1, result->Err_0);
            input::checked_status(leaves@, result);
        } }
        result?;
        index += 1;
    }
    #[cfg(verus_only)]
    proof! { assert(leaves@.take(index as int) =~= leaves@); }
    Ok(())
}

/// 任意叶子列表均先校验再形成像素图；成功恰好对应无重叠的合法输入。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures input::checked(leaves@, result), result.is_ok() == input::valid(leaves@),
        result.is_ok() ==> final(labels)@ == input::labels(leaves@),
        result is Err ==> failure::raster_error(leaves@, result->Err_0),
))]
pub(super) fn rasterize_leaves(
    leaves: &[QuadLeaf64],
    labels: &mut DenseLabels64,
    occupied: &mut [IntervalBucket; EDGE],
) -> Result<(), SparseQuadError> {
    labels.clear();
    if super::all_unit_leaves(leaves) {
        // 单位像素直接查目标格；跳过 side、行跨度和占用区间的重复计算。
        return rasterize_unit_leaves(leaves, labels);
    }
    prepare_interval_buckets(occupied);
    #[cfg(verus_only)]
    proof! { raster_proof::empty_occupancy(labels@, occupied@); }
    rasterize_mixed_leaves(leaves, labels, occupied)
}

/// 比较一个 512-bit 逻辑块；低 32 位逐项表示不等，其余位为零。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(mask =>
    requires a@.len() == 32, b@.len() == 32,
    ensures proof::block_bits(a@, b@, mask),
))]
fn compare32(a: &[u16], b: &[u16]) -> u64 {
    crate::std_adapter::compare32(a, b)
}

/// 位 x 精确表示两行第 x 个标签不同；接受任意完整的 u16 标签行。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(mask =>
    ensures proof::between_rows(current@, previous@, mask),
))]
fn row_changes(current: &[u16; EDGE], previous: &[u16; EDGE]) -> u64 {
    let (current_low, current_high) = current.split_at(32);
    let (previous_low, previous_high) = previous.split_at(32);
    let low = compare32(current_low, previous_low);
    let high = compare32(current_high, previous_high);
    let mask = low | (high << 32);
    #[cfg(verus_only)]
    proof! { proof::join_rows(current@, previous@, low, high, mask); }
    mask
}

/// 位 x 表示行内 x-1 与 x 标签不同；位 0 恒为零，重叠比较的位 32 只保留一次。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(mask =>
    ensures proof::within_row(row@, mask),
))]
fn row_boundaries(row: &[u16; EDGE]) -> u64 {
    let (low_left, high_right) = row.split_at(32);
    let (_, low_tail) = row.split_at(1);
    let (low_right, _) = low_tail.split_at(32);
    let (_, high_tail) = row.split_at(31);
    let (high_left, _) = high_tail.split_at(32);
    let low = compare32(low_right, low_left);
    let high = compare32(high_right, high_left);
    let mask = (low << 1) | (high << 32);
    #[cfg(verus_only)]
    proof! { proof::join_boundaries(row@, low, high, mask); }
    mask
}

/// 返回最低变化位；空位集返回 64，全部 u64 输入均可表示为 u8。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(position =>
    ensures proof::first_bit(mask, position),
        position == vstd::std_specs::bits::u64_trailing_zeros(mask),
))]
const fn first_boundary(mask: u64) -> u8 {
    #[cfg(verus_only)]
    proof! { vstd::std_specs::bits::axiom_u64_trailing_zeros(mask); }
    mask.trailing_zeros() as u8
}

/// 追加一行最大非零 runs，最多使用 64 个槽并保持已发布的线范围。
/// 内部剩余预算由构建器按已处理行数建立，不限制公开标签图的输入域。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(output).valid(), old(output).intervals@.len() <= 4032,
    ensures final(output).valid(), final(output).lines == old(output).lines,
        final(output).boundaries == old(output).boundaries,
        final(output).intervals@.take(old(output).intervals@.len() as int) == old(output).intervals@,
        match result {
            Ok(range) => range.start == old(output).intervals@.len()
                && range.end == final(output).intervals@.len()
                && range.end <= range.start + 64
                && final(output).range_valid(range)
                && (forall|p: int| 0 <= p < 64 ==> #[trigger] axis_owner::line_value(
                    final(output).intervals@.subrange(range.start as int, range.end as int), p) == row@[p]),
            Err(_) => false,
        },
))]
fn append_row(row: &[u16; EDGE], output: &mut AxisIntervals) -> Result<LineRange, SparseQuadError> {
    let mut boundaries = row_boundaries(row);
    let start = output.intervals.len();
    let mut left = 0u8;
    #[cfg(verus_only)]
    proof! {
        assert(output.intervals@.skip(start as int) =~= Seq::<Interval>::empty());
        proof::empty_row(row@);
        assert(proof::remaining(row@, left, boundaries));
    }
    #[cfg_attr(verus_only, verus_spec(
        invariant output.valid(), old(output).valid(), start == old(output).intervals@.len(), start <= 4032,
            output.lines == old(output).lines, output.boundaries == old(output).boundaries,
            start <= output.intervals@.len(),
            output.intervals@.take(start as int) == old(output).intervals@,
        invariant_except_break proof::remaining(row@, left, boundaries),
            proof::row_prefix(row@, output.intervals@.skip(start as int), left),
        ensures proof::row_prefix(row@, output.intervals@.skip(start as int), 64),
        decreases 64 - left,
    ))]
    loop {
        #[cfg(verus_only)]
        proof_decl! { let ghost previous_runs = output.intervals@.skip(start as int); }
        let right = if boundaries == 0 {
            first_boundary(0)
        } else {
            first_boundary(boundaries)
        };
        #[cfg(verus_only)]
        proof! { proof::next_segment(row@, left, boundaries, right); }
        let value = copy(row, usize::from(left));
        if value != 0 {
            reserve_exact(&output.intervals, 1)?;
            #[cfg(verus_only)]
            proof! { axis_owner::append_preserves_owner(output, Interval { start: left, end: right, value }); }
            output.intervals.push(Interval {
                start: left,
                end: right,
                value,
            });
        }
        #[cfg(verus_only)]
        proof! {
            assert(output.intervals@.skip(start as int) =~= if value == 0 { previous_runs } else {
                previous_runs.push(Interval { start: left, end: right, value })
            });
            proof::extend_row(row@, previous_runs, left, right, output.intervals@.skip(start as int));
            assert(output.intervals@.take(start as int) =~= old(output).intervals@);
            axis_owner::keep_published(output, old(output));
        }
        if boundaries == 0 {
            break;
        }
        #[cfg(verus_only)]
        proof_decl! { let ghost before_boundaries = boundaries; }
        boundaries &= boundaries - 1;
        #[cfg(verus_only)]
        proof! { proof::advance_boundary(row@, left, before_boundaries, right, boundaries); }
        left = right;
    }
    Ok(LineRange {
        start,
        end: output.intervals.len(),
    })
}

/// 关闭一段非零纵向 run；既有段均在起点之前结束，因此起点槽必为空。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::closed_before(old(bucket), start), start < end <= 64, value != 0,
    ensures result.is_ok(), proof::closed_before(final(bucket), end),
        final(bucket).starts == old(bucket).starts | (1u64 << start as u64),
        final(bucket).items@ == old(bucket).items@.update(start as int, Interval { start, end, value }),
        forall|p: int| 0 <= p < 64 ==> #[trigger] axis_owner::bucket_value(final(bucket), p)
            == if start <= p < end { value } else { axis_owner::bucket_value(old(bucket), p) },
))]
fn close_column_run(
    bucket: &mut IntervalBucket,
    start: u8,
    end: u8,
    value: u16,
) -> Result<(), SparseQuadError> {
    #[cfg(verus_only)]
    proof! { proof::free_column_start(bucket, start); }
    let result = bucket.try_push(Interval { start, end, value });
    #[cfg(verus_only)]
    proof! {
        proof::closed_column_insert(bucket, old(bucket), start, end, value);
        proof::column_insert_values(bucket, old(bucket), start, end, value);
    }
    result
}

/// 按最低变化位关闭上一行的纵段；其它列的 bucket 与起点保持原值。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires *old(changes) != 0, proof::pending_columns(old(buckets)@, old(starts)@, y, *old(changes)),
    ensures result.is_ok(), *final(changes) < *old(changes),
        proof::pending_columns(final(buckets)@, final(starts)@, y, *final(changes)),
        proof::column_step_context(old(buckets)@, old(starts)@, proof::frontier_values(current@, previous@, *old(changes)), *old(changes),
            final(buckets)@, final(starts)@, proof::frontier_values(current@, previous@, *final(changes)), *final(changes), current@, y),
))]
fn step_column_changes(
    buckets: &mut [IntervalBucket; EDGE],
    starts: &mut [u8; EDGE],
    previous: &[u16; EDGE],
    #[cfg(verus_only)] current: &[u16; EDGE],
    changes: &mut u64,
    y: u8,
) -> Result<(), SparseQuadError> {
    let x = usize::from(first_boundary(*changes));
    #[cfg(verus_only)]
    proof! { proof::selected_column(buckets@, starts@, y, *changes, x); }
    let value = copy(previous, x);
    if value != 0 {
        close_column_run(get_mut(buckets, x), copy(starts, x), y, value)?;
    }
    *get_mut(starts, x) = y;
    #[cfg(verus_only)]
    proof! {
        proof::restart_frontiers(&old(buckets)@[x as int], old(starts)@[x as int], previous@[x as int],
            &buckets@[x as int], y, current@[x as int]);
    }
    *changes &= *changes - 1;
    #[cfg(verus_only)]
    proof! {
        axis_owner::cleared_lowest(*old(changes), x as u64, *changes);
        proof::frontier_values_step(current@, previous@, *old(changes), *changes, x);
        proof::column_step_frontiers(old(buckets)@, old(starts)@, proof::frontier_values(current@, previous@, *old(changes)), *old(changes),
            buckets@, starts@, proof::frontier_values(current@, previous@, *changes), *changes, current@, y);
    }
    Ok(())
}

/// 发布下一行；变化行追加区间，否则复用上一行范围。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(rows).valid(), old(rows).lines@.len() == 64, 0 < y < 64,
        old(rows).intervals@.len() <= y as int * 64,
        previous == old(rows).lines@[y as int - 1],
    ensures final(rows).valid(), final(rows).lines@.len() == 64, final(rows).intervals@.len() <= (y as int + 1) * 64,
        match result { Ok(range) => final(rows).lines@[y as int] == range, Err(_) => false },
        proof::row_step_context(old(rows), final(rows), y, current@, changes != 0),
))]
fn advance_dense_row(
    rows: &mut AxisIntervals,
    y: u8,
    previous: LineRange,
    current: &[u16; EDGE],
    changes: u64,
) -> Result<LineRange, SparseQuadError> {
    let range = if changes != 0 {
        rows.boundaries |= 1u64 << y;
        append_row(current, rows)?
    } else {
        previous
    };
    rows.publish_line(usize::from(y), range);
    #[cfg(verus_only)]
    proof! {
        assert(rows.intervals@.take(old(rows).intervals@.len() as int) =~= old(rows).intervals@);
        proof::row_step_frontiers(old(rows), rows, y, current@, changes != 0);
    }
    Ok(range)
}

/// 关闭指定列的最后纵段；其余列保持原值。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires x < 64, proof::closed_before(&old(buckets)@[x as int], starts@[x as int]), starts@[x as int] < 64,
    ensures result.is_ok(), proof::finish_context(old(buckets)@, final(buckets)@, starts@, values@, x),
))]
fn finish_column_at(
    buckets: &mut [IntervalBucket; EDGE],
    starts: &[u8; EDGE],
    values: &[u16; EDGE],
    x: usize,
) -> Result<(), SparseQuadError> {
    let value = copy(values, x);
    if value != 0 {
        close_column_run(get_mut(buckets, x), copy(starts, x), EDGE_U8, value)?;
    }
    #[cfg(verus_only)]
    proof! { proof::finish_frontiers(old(buckets)@, buckets@, starts@, values@, x); }
    Ok(())
}

/// 一遍行扫描同时发布两轴区间；仅缓存列起点，标签直接借用相邻输入行。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(rows).valid(),
    ensures result.is_ok(), proof::rows_prefix(labels@, final(rows), 64),
        forall|x: int| 0 <= x < 64 ==> #[trigger] proof::complete_column(labels@, x, &final(column_buckets)@[x]),
))]
fn scan_dense_labels(
    labels: &DenseLabels64,
    rows: &mut AxisIntervals,
    column_buckets: &mut [IntervalBucket; EDGE],
) -> Result<(), SparseQuadError> {
    rows.clear_for_build()?;
    prepare_interval_buckets(column_buckets);
    let first = get(&labels.rows, 0);
    let mut column_starts = [0u8; EDGE];
    let mut range = append_row(first, rows)?;
    rows.publish_line(0, range);
    rows.boundaries = 1;
    #[cfg(verus_only)]
    proof! {
        proof::matched_label_row(labels, 0, first@);
        proof::initial_rows(labels@, rows, first@, range);
        proof::initial_frontiers(labels@, column_buckets@, column_starts@, first@);
    }
    let mut y = 1u8;
    #[cfg_attr(verus_only, verus_spec(
        invariant 1 <= y <= 64, proof::rows_prefix(labels@, rows, y),
            proof::columns_frontier(labels@, y, column_buckets@, column_starts@, labels.rows@[y as int - 1]@),
            range == rows.lines@[y as int - 1],
        decreases 64 - y,
    ))]
    while y < EDGE_U8 {
        let current = get(&labels.rows, usize::from(y));
        let previous = get(&labels.rows, usize::from(y - 1));
        let mut changes = row_changes(current, previous);
        #[cfg(verus_only)]
        proof! {
            proof::matched_label_row(labels, y, current@);
            proof::matched_label_row(labels, (y - 1) as u8, previous@);
            proof::begin_changed_row(labels@, y, column_buckets@, column_starts@, previous@, current@, previous@, changes);
            proof::initial_values(current@, previous@, changes);
        }
        range = advance_dense_row(rows, y, range, current, changes)?;
        #[cfg_attr(verus_only, verus_spec(
            invariant proof::mixed_frontiers(labels@, y, column_buckets@, column_starts@, proof::frontier_values(current@, previous@, changes), changes),
                proof::row_matches(labels@, y, current@),
            decreases changes,
        ))]
        while changes != 0 {
            #[cfg(verus_only)]
            proof! { proof::pending_from_mixed(labels@, y, column_buckets@, column_starts@, proof::frontier_values(current@, previous@, changes), changes); }
            step_column_changes(
                column_buckets,
                &mut column_starts,
                previous,
                #[cfg(verus_only)]
                current,
                &mut changes,
                y,
            )?;
        }
        #[cfg(verus_only)]
        proof! {
            proof::finished_values(current@, previous@);
            proof::changed_row_finished(labels@, y, column_buckets@, column_starts@, current@);
        }
        y += 1;
    }
    let last = get(&labels.rows, EDGE - 1);
    #[cfg(verus_only)]
    proof! { proof::finishing_started(labels@, column_buckets@, column_starts@, last@); }
    let mut x = 0usize;
    #[cfg_attr(verus_only, verus_spec(
        invariant x <= 64, proof::finishing_columns(labels@, column_buckets@, column_starts@, last@, x),
        decreases 64 - x,
    ))]
    while x < EDGE {
        #[cfg(verus_only)]
        proof! { assert(proof::column_frontier(labels@, x as int, 64, &column_buckets@[x as int], column_starts@[x as int], last@[x as int])); }
        finish_column_at(column_buckets, &column_starts, last, x)?;
        x += 1;
    }
    Ok(())
}

/// 由完整标签图构造两轴最大非零区间；未标变化线与前线完全相同。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(rows).valid(), old(columns).valid(),
    ensures result.is_ok(), final(rows).represents(labels@, false), final(columns).represents(labels@, true),
        final(rows).boundaries_complete(), final(columns).boundaries_complete(),
))]
pub(super) fn build_axes(
    labels: &DenseLabels64,
    rows: &mut AxisIntervals,
    columns: &mut AxisIntervals,
    column_buckets: &mut [IntervalBucket; EDGE],
) -> Result<(), SparseQuadError> {
    scan_dense_labels(labels, rows, column_buckets)?;
    #[cfg(verus_only)]
    proof! { proof::completed_buckets(labels@, column_buckets@); }
    build_complete_axis_intervals_from_buckets(column_buckets, columns)?;
    #[cfg(verus_only)]
    proof! { proof::dense_axes_finished(labels@, rows, columns, column_buckets@); }
    Ok(())
}
