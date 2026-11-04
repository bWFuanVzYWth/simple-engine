//! 批写借用输入；各入口校验跨字段约束，内部游标不复制数据。

use crate::ZOrder20x3;
#[cfg(verus_only)]
use vstd::prelude::*;

/// 单点数据修改条目。
#[derive(Clone, Copy, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct SetCell {
    /// 目标体素的root 内的 Z-order 坐标。
    pub path: ZOrder20x3,
    /// 写入的原始值，包含零值；同一坐标重复出现时采用最后一项。
    pub data: u32,
}

/// 均匀区域填充，lod 是边长的二进制指数 0..20；path 按该边长向下对齐。
#[derive(Clone, Copy, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct UniformFill {
    /// 目标区域内的坐标，写入时按 `lod` 向下对齐。
    pub path: ZOrder20x3,
    /// 区域边长的二进制指数，范围为 `0..=HashDag::AXIS_BITS`。
    pub lod: u8,
    /// 整个区域共享的原始值，包含零值。
    pub data: u32,
}

/// 借用完整 Z-order 稠密区域的写入条目。
///
/// 起点必须按 `lod` 对齐，值数量为 `2^(3 * lod)`；同批区域按起点升序且互不重叠。
/// 接收输入时校验上述跨字段约束，调用结束后不保留数据借用。
#[derive(Clone, Copy, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct DenseRegion<'a> {
    /// root 内按 `lod` 对齐的 Z-order 起点。
    pub path: ZOrder20x3,
    /// 区域边长的二进制指数，范围为 `0..=HashDag::AXIS_BITS`。
    pub lod: u8,
    /// 区域内按局部 Z-order 排列的全部体素；长度为 `2^(3 * lod)`。
    pub values: &'a [u32],
}
