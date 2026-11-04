//! 标准库 popcount 补充规格所用的有限位和；不信任算法容量或循环。

use vstd::prelude::*;

verus! {

pub open spec fn prefix(mask: u128, len: nat) -> nat
    decreases len,
{
    if len == 0 { 0 }
    else { prefix(mask, (len - 1) as nat)
        + if (mask >> ((len - 1) as u32)) & 1 == 1 { 1nat } else { 0nat } }
}

pub proof fn prefix_bound(mask: u128, len: nat)
    ensures prefix(mask, len) <= len,
    decreases len,
{
    if len > 0 { prefix_bound(mask, (len - 1) as nat); }
}

pub proof fn zero_prefix(len: nat)
    ensures prefix(0, len) == 0,
    decreases len,
{
    if len > 0 {
        zero_prefix((len - 1) as nat);
        let shift = (len - 1) as u32;
        assert((0u128 >> shift) & 1 == 0) by(bit_vector);
    }
}

pub open spec fn u64_ones(mask: u64) -> u32 { prefix(mask as u128, 64) as u32 }
pub open spec fn u128_ones(mask: u128) -> u32 { prefix(mask, 128) as u32 }

pub proof fn bounds(mask64: u64, mask128: u128)
    ensures u64_ones(mask64) <= 64, u128_ones(mask128) <= 128,
        u64_ones(0) == 0, u128_ones(0) == 0,
{
    prefix_bound(mask64 as u128, 64);
    prefix_bound(mask128, 128);
    zero_prefix(64);
    zero_prefix(128);
}

}
