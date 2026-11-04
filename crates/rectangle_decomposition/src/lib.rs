//! 64×64 标签区域的最少同标签矩形分解。
//! [`optimal`] 提供稀疏正方形与连续 `u16` 标签输入、验证规范及当前证明范围。

#![forbid(unsafe_code)]
#![feature(portable_simd)]

mod chord_graph;
mod error;
mod geometry;
#[cfg(not(verus_only))]
mod std_adapter;
#[cfg(verus_only)]
extern crate rectangle_std as std_adapter;
mod index {
    pub(crate) use crate::optimal::{copy, get_mut};
}
pub mod optimal;
#[cfg(verus_only)]
#[path = "proof/partition.rs"]
mod partition;

/// 切面的固定边长。
pub const SIDE: usize = 64;
/// 完整切面的标签数量。
pub const AREA: usize = SIDE * SIDE;
