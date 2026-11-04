//! 同源几何核的数学形状和位集合对应；不引入外部假设。

use super::*;

verus! {

pub(crate) open spec fn side(lod: u8) -> u8 {
    if lod == 0 { 1 } else if lod == 1 { 2 } else if lod == 2 { 4 }
    else if lod == 3 { 8 } else if lod == 4 { 16 } else if lod == 5 { 32 } else { 64 }
}

pub(crate) open spec fn dyadic_side(size: u8) -> bool {
    size == 1 || size == 2 || size == 4 || size == 8 || size == 16 || size == 32 || size == 64
}

pub(crate) open spec fn shape_checked(u: u8, v: u8, lod: u8, result: Result<u8, ShapeError>) -> bool {
    let size = side(lod);
    let aligned = u % size == 0 && v % size == 0;
    let inside = u as nat + size as nat <= 64 && v as nat + size as nat <= 64;
    match result {
        Ok(actual) => lod <= 6 && aligned && inside && actual == size,
        Err(ShapeError::LodOutOfRange) => lod > 6,
        Err(ShapeError::Misaligned) => lod <= 6 && !aligned,
        Err(ShapeError::OutOfBounds) => lod <= 6 && aligned && !inside,
    }
}

pub(super) proof fn side_shift(lod: u8)
    requires lod <= 6,
    ensures (1u8 << lod) == side(lod), dyadic_side(1u8 << lod),
{
    assert((1u8 << lod) == if lod == 0 { 1u8 } else if lod == 1 { 2u8 }
        else if lod == 2 { 4u8 } else if lod == 3 { 8u8 }
        else if lod == 4 { 16u8 } else if lod == 5 { 32u8 } else { 64u8 }) by(bit_vector) requires lod <= 6;
}

pub(super) proof fn aligned_mask(value: u8, size: u8)
    requires dyadic_side(size),
    ensures ((value & ((size - 1) as u8)) == 0) == (value % size == 0),
{
    assert(size == 1 || size == 2 || size == 4 || size == 8 || size == 16 || size == 32 || size == 64);
    assert(((value & ((size - 1) as u8)) == 0) == (value % size == 0)) by(bit_vector)
        requires size == 1 || size == 2 || size == 4 || size == 8 || size == 16 || size == 32 || size == 64;
}

pub(crate) open spec fn cell_bits(start: u8, end: u8, mask: u64) -> bool {
    forall|bit: u8| #![auto] bit < 64 ==> ((mask >> bit) & 1) == if start <= bit && bit < end { 1u64 } else { 0u64 }
}

pub(super) proof fn cell_interval(start: u8, end: u8, mask: u64)
    requires
        start <= 64, end <= 64,
        mask == if start >= end { 0 } else {
            (u64::MAX << (start as u32)) & if end >= 64 { u64::MAX } else {
                ((1u64 << (end as u32)) - 1) as u64
            }
        },
    ensures cell_bits(start, end, mask),
{
    assert forall|bit: u8| #![auto] bit < 64 implies
        ((mask >> bit) & 1) == if start <= bit && bit < end { 1u64 } else { 0u64 } by {
        assert(((mask >> bit) & 1) == if start <= bit && bit < end { 1u64 } else { 0u64 }) by(bit_vector)
            requires start <= 64, end <= 64, bit < 64,
                mask == if start >= end { 0 } else {
                    (u64::MAX << (start as u32)) & if end >= 64 { u64::MAX } else {
                        ((1u64 << (end as u32)) - 1) as u64
                    }
                };
    }
}

pub(crate) open spec fn coord_bits(start: u8, end: u8, mask: u128) -> bool {
    forall|bit: u8| #![auto] bit < 128 ==> ((mask >> bit) & 1) == if start <= bit && bit <= end { 1u128 } else { 0u128 }
}

pub(super) proof fn coord_interval(start: u8, end: u8, mask: u128)
    requires
        start <= 64, end <= 64,
        mask == if start > end { 0 } else {
            (((1u128 << (((end - start) as u32 + 1) as u32)) - 1) as u128) << (start as u32)
        },
    ensures coord_bits(start, end, mask),
{
    assert forall|bit: u8| #![auto] bit < 128 implies
        ((mask >> bit) & 1) == if start <= bit && bit <= end { 1u128 } else { 0u128 } by {
        assert(((mask >> bit) & 1) == if start <= bit && bit <= end { 1u128 } else { 0u128 }) by(bit_vector)
            requires start <= 64, end <= 64, bit < 128,
                mask == if start > end { 0 } else {
                    (((1u128 << (((end - start) as u32 + 1) as u32)) - 1) as u128) << (start as u32)
                };
    }
}

pub(super) proof fn positive_u64_shift(shift: u32)
    by(bit_vector)
    requires shift < 64,
    ensures 1u64 << shift > 0,
{}

pub(super) proof fn positive_u128_shift(shift: u32)
    by(bit_vector)
    requires shift < 128,
    ensures 1u128 << shift > 0,
{}

}
