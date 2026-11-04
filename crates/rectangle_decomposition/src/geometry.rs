//! 固定网格的叶子形状与位区间；不依赖标签、分区或匹配策略。

#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "proof/geometry.rs"]
#[cfg(verus_only)]
mod proof;
#[cfg(verus_only)]
pub(crate) use proof::{cell_bits, coord_bits, dyadic_side, shape_checked, side};

/// 单叶子的形状失败；重叠由持有整组叶子的上层检查。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum ShapeError {
    LodOutOfRange,
    Misaligned,
    OutOfBounds,
}

/// 检查 dyadic 正方形并返回边长；错误顺序为 LOD、对齐、边界。
/// 任意三个 `u8` 都是可校验输入，不改变输入或检查跨叶子的重叠。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures proof::shape_checked(u, v, lod, result),
))]
pub(crate) fn validate_square(u: u8, v: u8, lod: u8) -> Result<u8, ShapeError> {
    if lod > 6 {
        return Err(ShapeError::LodOutOfRange);
    }

    let size = quad_size(lod);
    let mask = size - 1;
    #[cfg(verus_only)]
    proof! { proof::aligned_mask(u, size); proof::aligned_mask(v, size); }
    if (u & mask) != 0 || (v & mask) != 0 {
        return Err(ShapeError::Misaligned);
    }

    if u16::from(u) + u16::from(size) > 64 || u16::from(v) + u16::from(size) > 64 {
        return Err(ShapeError::OutOfBounds);
    }

    Ok(size)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(size =>
    requires lod <= 6,
    ensures size == proof::side(lod), proof::dyadic_side(size),
))]
const fn quad_size(lod: u8) -> u8 {
    #[cfg(verus_only)]
    proof! { proof::side_shift(lod); }
    1u8 << lod
}

/// 坐标 `start..=end` 对应的 128-bit 位集；倒置范围为空，合法坐标为 `0..=64`。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(mask =>
    requires start <= 64, end <= 64,
    ensures proof::coord_bits(start, end, mask),
))]
pub(crate) const fn coord_between_mask(start: u8, end: u8) -> u128 {
    if start > end {
        #[cfg(verus_only)]
        proof! { proof::coord_interval(start, end, 0); }
        return 0;
    }

    let len = (end - start) as u32 + 1;
    #[cfg(verus_only)]
    proof! { proof::positive_u128_shift(len); }
    let mask = ((1u128 << len) - 1) << (start as u32);
    #[cfg(verus_only)]
    proof! { proof::coord_interval(start, end, mask); }
    mask
}

/// 单元 `start..end` 对应的 64-bit 位集；空或倒置范围为空，合法边界为 `0..=64`。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(mask =>
    requires start <= 64, end <= 64,
    ensures proof::cell_bits(start, end, mask),
))]
pub(crate) const fn cell_range_mask(start: u8, end: u8) -> u64 {
    if start >= end {
        #[cfg(verus_only)]
        proof! { proof::cell_interval(start, end, 0); }
        return 0;
    }

    let start_mask = u64::MAX << (start as u32);
    let end_mask = if end >= 64 {
        u64::MAX
    } else {
        #[cfg(verus_only)]
        proof! { proof::positive_u64_shift(end as u32); }
        (1u64 << (end as u32)) - 1
    };
    let mask = start_mask & end_mask;
    #[cfg(verus_only)]
    proof! { proof::cell_interval(start, end, mask); }
    mask
}
