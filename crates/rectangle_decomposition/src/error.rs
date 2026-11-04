//! 分解失败的公共类型；同源验证与普通构建共用同一个枚举。

#[cfg(verus_only)]
use vstd::prelude::*;

/// sparse quad 输入或分解失败。
#[derive(Debug)]
#[cfg_attr(not(verus_only), derive(thiserror::Error))]
#[cfg_attr(verus_only, verus_verify)]
pub enum SparseQuadError {
    /// 调用方 sink 无法保留输出容量；内核自身不申请堆内存。
    #[cfg_attr(not(verus_only), error("矩形输出分配失败"))]
    Allocation(#[cfg_attr(not(verus_only), from)] std::collections::TryReserveError),
    /// 输入或内部阶段超过固定网格推导的容量上界。
    #[cfg_attr(not(verus_only), error("矩形分解容量超限"))]
    CapacityOverflow,
    /// 正方形边长 log2 超出 `0..=6`。
    #[cfg_attr(not(verus_only), error("正方形 lod 超出 0..=6"))]
    LodOutOfRange,
    /// 正方形起点未按边长对齐。
    #[cfg_attr(not(verus_only), error("正方形起点未按边长对齐"))]
    Misaligned,
    /// 正方形超出 64×64 切面。
    #[cfg_attr(not(verus_only), error("正方形超出 64×64 切面"))]
    OutOfBounds,
    /// 两个输入正方形覆盖了同一像素，即使标签相同也拒绝。
    #[cfg_attr(not(verus_only), error("输入正方形相互重叠"))]
    Overlap,
}
