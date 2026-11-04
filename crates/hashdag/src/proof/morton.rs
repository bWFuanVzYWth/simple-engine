//! Morton 路径的真实整数视图与位语义。
use super::*;
use vstd::prelude::*;
verus! {
pub open(crate) spec fn value(path: ZOrder20x3) -> u64 { path.0 }
impl ZOrder20x3 {
    #[verifier::type_invariant]
    pub(crate) open spec fn well_formed(self) -> bool { self.0 < 0x1000_0000_0000_0000u64 }
}
pub open(crate) spec fn spread(value: u32) -> u64 {
    let x = (value & 0xfffff) as u64;
    let x = (x | (x << 32)) & MASK_5;
    let x = (x | (x << 16)) & MASK_4;
    let x = (x | (x << 8)) & MASK_3;
    let x = (x | (x << 4)) & MASK_2;
    (x | (x << 2)) & MASK_1
}
pub open(crate) spec fn interleave(x: u32, y: u32, z: u32) -> u64 {
    spread(x) | (spread(y) << 1) | (spread(z) << 2)
}
pub open(crate) spec fn compact(value: u64) -> u32 {
    let x = value & MASK_1;
    let x = (x | (x >> 2)) & MASK_2;
    let x = (x | (x >> 4)) & MASK_3;
    let x = (x | (x >> 8)) & MASK_4;
    let x = (x | (x >> 16)) & MASK_5;
    (x | (x >> 32)) as u32
}
pub open(crate) spec fn deinterleave(value: u64) -> (u32, u32, u32) {
    (compact(value), compact(value >> 1), compact(value >> 2))
}
pub proof fn range_constant()
    ensures (1u64 << 60) == 0x1000_0000_0000_0000u64,
{
    assert((1u64 << 60) == 0x1000_0000_0000_0000u64) by(bit_vector);
}
pub proof fn interleave_range(x: u32, y: u32, z: u32)
    ensures interleave(x, y, z) < 0x1000_0000_0000_0000u64,
{
    assert(interleave(x, y, z) < 0x1000_0000_0000_0000u64) by(bit_vector);
}
pub proof fn deinterleave_range(code: u64)
    requires code < 0x1000_0000_0000_0000u64,
    ensures deinterleave(code).0 < 1_048_576, deinterleave(code).1 < 1_048_576, deinterleave(code).2 < 1_048_576,
{
    assert(code < 0x1000_0000_0000_0000u64 ==> compact(code) < 1_048_576u32 && compact(code >> 1) < 1_048_576u32 && compact(code >> 2) < 1_048_576u32) by(bit_vector);
}
pub proof fn decode_encode(code: u64)
    requires code < 0x1000_0000_0000_0000u64,
    ensures interleave(deinterleave(code).0, deinterleave(code).1, deinterleave(code).2) == code,
{
    assert(code < 0x1000_0000_0000_0000u64 ==> interleave(compact(code), compact(code >> 1), compact(code >> 2)) == code) by(bit_vector);
}
pub proof fn encode_decode(x: u32, y: u32, z: u32)
    ensures deinterleave(interleave(x, y, z)) == (x & 0xfffff, y & 0xfffff, z & 0xfffff),
{
    assert(compact(interleave(x, y, z)) == x & 0xfffff) by(bit_vector);
    assert(compact(interleave(x, y, z) >> 1) == y & 0xfffff) by(bit_vector);
    assert(compact(interleave(x, y, z) >> 2) == z & 0xfffff) by(bit_vector);
}
pub proof fn input_mask(x: u32, y: u32, z: u32)
    requires x < 1_048_576, y < 1_048_576, z < 1_048_576,
    ensures x & 0xfffff == x, y & 0xfffff == y, z & 0xfffff == z,
{
    assert(x < 1_048_576 ==> x & 0xfffff == x) by(bit_vector);
    assert(y < 1_048_576 ==> y & 0xfffff == y) by(bit_vector);
    assert(z < 1_048_576 ==> z & 0xfffff == z) by(bit_vector);
}
/// 与位扩散步骤无关的 Morton 位定义。
pub open(crate) spec fn bit_encoding(code: u64, x: u32, y: u32, z: u32) -> bool {
    code < 0x1000_0000_0000_0000u64 && forall|bit: u32| bit < 20 ==>
        ((code >> (3 * bit)) & 1u64) == ((x >> bit) & 1u32) as u64
        && ((code >> (3 * bit + 1)) & 1u64) == ((y >> bit) & 1u32) as u64
        && ((code >> (3 * bit + 2)) & 1u64) == ((z >> bit) & 1u32) as u64
}
pub proof fn interleave_bit(x: u32, y: u32, z: u32, bit: u32)
    requires bit < 20,
    ensures ((interleave(x, y, z) >> (3 * bit)) & 1u64) == ((x >> bit) & 1u32) as u64,
        ((interleave(x, y, z) >> (3 * bit + 1)) & 1u64) == ((y >> bit) & 1u32) as u64,
        ((interleave(x, y, z) >> (3 * bit + 2)) & 1u64) == ((z >> bit) & 1u32) as u64,
{
    assert(bit < 20 ==> ((interleave(x, y, z) >> (3 * bit)) & 1u64) == ((x >> bit) & 1u32) as u64) by(bit_vector);
    assert(bit < 20 ==> ((interleave(x, y, z) >> (3 * bit + 1)) & 1u64) == ((y >> bit) & 1u32) as u64) by(bit_vector);
    assert(bit < 20 ==> ((interleave(x, y, z) >> (3 * bit + 2)) & 1u64) == ((z >> bit) & 1u32) as u64) by(bit_vector);
}
pub proof fn interleave_bits(x: u32, y: u32, z: u32)
    ensures bit_encoding(interleave(x, y, z), x, y, z),
{
    interleave_range(x, y, z);
    assert forall|bit: u32| bit < 20 implies
        ((interleave(x, y, z) >> (3 * bit)) & 1u64) == ((x >> bit) & 1u32) as u64
        && ((interleave(x, y, z) >> (3 * bit + 1)) & 1u64) == ((y >> bit) & 1u32) as u64
        && ((interleave(x, y, z) >> (3 * bit + 2)) & 1u64) == ((z >> bit) & 1u32) as u64 by {
        interleave_bit(x, y, z, bit);
    }
}
}
