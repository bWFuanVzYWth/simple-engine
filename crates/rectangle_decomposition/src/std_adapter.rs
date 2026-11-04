//! 固定 vstd 缺失的标准库与单次 callback 绑定；执行体只委托原方法。
//!
//! 验证入口单独导出这些可信绑定，生产证明导入；不包含输入或分解算法。

#[cfg(verus_only)]
use vstd::prelude::*;

/// 把整个 `u16` 片段赋为同一值，保留长度；不约束所属缓冲之外的存储。
#[inline]
#[cfg_attr(verus_only, verus_verify(external_body))]
#[cfg_attr(verus_only, verus_spec(
    ensures final(values)@ == Seq::new(old(values)@.len(), |_: int| value),
))]
pub fn fill_u16(values: &mut [u16], value: u16) {
    values.fill(value);
}

/// 比较 32 个 `u16`；低位按 lane 次序标记不等，其余位为零。
///
/// 可信范围仅为 `from_slice`、`simd_ne` 和 `to_bitmask` 的标准库语义；
/// 两个逻辑块的切片、移位与边界拼接由调用方证明。
/// 来源：<https://doc.rust-lang.org/std/simd/struct.Mask.html#method.to_bitmask>。
#[inline]
#[cfg_attr(verus_only, verus_verify(external_body))]
#[cfg_attr(verus_only, verus_spec(result =>
    requires a@.len() == 32, b@.len() == 32,
    ensures result >> 32u64 == 0,
        forall|bit: u64| bit < 64 ==> #[trigger] ((result >> bit) & 1)
            == if bit < 32 && a@[bit as int] != b@[bit as int] { 1u64 } else { 0u64 },
))]
pub fn compare32(a: &[u16], b: &[u16]) -> u64 {
    use std::simd::{cmp::SimdPartialEq, u16x32};
    u16x32::from_slice(a)
        .simd_ne(u16x32::from_slice(b))
        .to_bitmask()
}

/// 原样取得 64×64 `u16` 的行主序可变视图；借用结束后写入逐像素对应原数组。
#[inline]
#[cfg_attr(verus_only, verus_verify(external_body))]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result@.len() == 4096,
        forall|p: int| 0 <= p < 4096 ==> #[trigger] result@[p] == old(rows)@[p / 64]@[p % 64],
        forall|p: int| 0 <= p < 4096 ==> #[trigger] final(rows)@[p / 64]@[p % 64] == final(result)@[p],
))]
pub const fn flatten64(rows: &mut [[u16; 64]; 64]) -> &mut [u16] {
    rows.as_flattened_mut()
}

/// 按 Rust `FnMut` 语义调用一次；不约束返回值、闭包状态或外部副作用。
///
/// 当前 Verus 不支持任意可变捕获的逐次状态；只信任这一处 dispatch。
/// 调用方须正常返回，panic／进程退出不在正常返回契约内。
#[inline]
#[cfg_attr(verus_only, verus_verify(external_body))]
pub fn invoke<A, R, F: FnMut(A) -> R>(callback: &mut F, argument: A) -> R {
    callback(argument)
}

#[cfg(test)]
mod tests {
    #[test]
    fn simd_mask_follows_lane_order() {
        let a = std::array::from_fn::<_, 32, _>(|lane| (lane as u16).wrapping_mul(2113));
        assert_eq!(super::compare32(&a, &a), 0);
        for lane in 0..32 {
            let mut b = a;
            b[lane] ^= u16::MAX;
            assert_eq!(super::compare32(&a, &b), 1u64 << lane);
        }
        assert_eq!(
            super::compare32(&a, &a.map(|value| !value)),
            u32::MAX as u64
        );
    }
}
