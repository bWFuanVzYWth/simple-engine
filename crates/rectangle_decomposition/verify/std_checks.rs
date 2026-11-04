//! 检查导入的标准库绑定及其错误命题；不验证 intrinsic 或 callback 的实现。

use vstd::prelude::*;

verus! {

fn bounds(a: u64, b: u128) {
    let count64 = a.count_ones();
    let count128 = b.count_ones();
    assert(count64 <= 64 && count128 <= 128);
    #[cfg(reject_std_count_width)]
    assert(count64 == 65);
}

fn fill(values: &mut [u16], value: u16)
    ensures final(values)@ == Seq::new(old(values)@.len(), |_: int| value),
{
    rectangle_std::fill_u16(values, value);
    #[cfg(reject_std_fill_value)]
    if values.len() > 0 { assert(values@[0] != value); }
}

fn flatten(rows: &mut [[u16; 64]; 64], index: usize, value: u16)
    requires index < 4096,
    ensures final(rows)@[index as int / 64]@[index as int % 64] == value,
        forall|p: int| 0 <= p < 4096 && p != index ==>
            #[trigger] final(rows)@[p / 64]@[p % 64] == old(rows)@[p / 64]@[p % 64],
{
    rectangle_std::flatten64(rows)[index] = value;
    #[cfg(reject_std_flatten_value)]
    assert(rows@[index as int / 64]@[index as int % 64] != value);
}

fn simd_lanes(a: &[u16], b: &[u16], lane: u64)
    requires a@.len() == 32, b@.len() == 32, lane < 32,
{
    let mask = rectangle_std::compare32(a, b);
    assert(mask >> 32u64 == 0);
    assert(((mask >> lane) & 1) == if a@[lane as int] != b@[lane as int] { 1u64 } else { 0u64 });
    #[cfg(reject_std_simd_lane)]
    assert(((mask >> lane) & 1) == if a@[lane as int] == b@[lane as int] { 1u64 } else { 0u64 });
    #[cfg(reject_std_simd_high)]
    assert(mask >> 32u64 != 0);
}

fn main() {
    let empty64 = 0u64.count_ones();
    let empty128 = 0u128.count_ones();
    proof { rectangle_std::bit_count::bounds(0, 0); }
    assert(empty64 == 0 && empty128 == 0);
    #[cfg(reject_std_empty_count)]
    assert(empty64 == 1);
}

}
