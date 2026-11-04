//! 固定 vstd 尚未提供的标准库绑定；这是显式可信边界，不是执行算法证明。
//!
//! `count_ones` 返回整数二进制表示中的 1 位数量，依据 Rust 标准库接口：
//! <https://doc.rust-lang.org/std/primitive.u64.html#method.count_ones>。
//! 有限位和及其界限在 `bit_count` 中验证；实际 intrinsic、编译器和硬件属于标准库信任。
//! `fill_u16` 仅补充 `[u16]` 的全片段赋值；`flatten64` 仅补充 64×64 `u16` 的
//! 行主序借用及借用结束后的逐像素对应，不假设叶子合法性、重叠检测或分解性质。
//! `invoke` 只委托一次 Rust `FnMut` 调用，不假设闭包的返回值或状态；实际调用
//! 序列、首次失败停止及计数在生产消费循环中验证。panic／退出不在正常返回域。
//! `compare32` 只绑定 32-lane `u16` 的标准库 SIMD 不等位掩码；切片及跨块拼接另证。
//! 来源：<https://doc.rust-lang.org/std/primitive.slice.html#method.fill>、
//! <https://doc.rust-lang.org/std/primitive.slice.html#method.as_flattened_mut>。
//! 仅此入口允许补充规格，生产核及其证明仍使用 `--no-cheating`。

#![feature(portable_simd)]

#[path = "../src/proof/bit_count.rs"]
pub mod bit_count;
#[path = "../src/std_adapter.rs"]
pub mod memory;
pub use memory::{compare32, fill_u16, flatten64, invoke};

use vstd::prelude::*;

verus! {

pub open spec fn u64_ones(mask: u64) -> u32 { bit_count::u64_ones(mask) }
pub open spec fn u128_ones(mask: u128) -> u32 { bit_count::u128_ones(mask) }

#[verifier::when_used_as_spec(u64_ones)]
pub assume_specification[u64::count_ones](mask: u64) -> (result: u32)
    ensures result == bit_count::u64_ones(mask), result <= 64,
;

#[verifier::when_used_as_spec(u128_ones)]
pub assume_specification[u128::count_ones](mask: u128) -> (result: u32)
    ensures result == bit_count::u128_ones(mask), result <= 128,
;

}
