//! 直接装配生产几何核；不加载 SIMD、匹配或分区实现。

#![allow(dead_code)]
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]

#[path = "../src/geometry.rs"]
mod geometry;

use geometry::{ShapeError, cell_range_mask, coord_between_mask, validate_square};
use vstd::prelude::*;

verus! {

fn boundary_cases() {
    let root = validate_square(0, 0, 6);
    assert(root == Ok(64));
    let wrong_lod = validate_square(1, 255, 7);
    assert(wrong_lod == Err(ShapeError::LodOutOfRange));
    let wrong_alignment = validate_square(65, 0, 1);
    assert(wrong_alignment == Err(ShapeError::Misaligned));
    let outside = validate_square(64, 0, 0);
    assert(outside == Err(ShapeError::OutOfBounds));
    let whole = cell_range_mask(0, 64);
    assert(whole & (1u64 << 63) != 0) by(bit_vector)
        requires (whole >> 63u8) & 1 == 1;
    let empty = cell_range_mask(64, 0);
    assert(empty & (1u64 << 63) == 0) by(bit_vector)
        requires (empty >> 63u8) & 1 == 0;
    let end = coord_between_mask(64, 64);
    assert(end & (1u128 << 64) != 0) by(bit_vector)
        requires (end >> 64u8) & 1 == 1;
}

#[cfg(reject_shape_lod)]
fn reject_shape_lod() {
    let result = validate_square(0, 0, 7);
    assert(result is Ok);
}

#[cfg(reject_shape_alignment)]
fn reject_shape_alignment() {
    let result = validate_square(1, 0, 1);
    assert(result is Ok);
}

#[cfg(reject_shape_bounds)]
fn reject_shape_bounds() {
    let result = validate_square(64, 0, 0);
    assert(result is Ok);
}

#[cfg(reject_cell_end)]
fn reject_cell_end() {
    let mask = cell_range_mask(0, 63);
    assert(mask & (1u64 << 63) != 0);
}

#[cfg(reject_coord_end)]
fn reject_coord_end() {
    let mask = coord_between_mask(63, 64);
    assert(mask & (1u128 << 64) == 0);
}

}

fn main() {}
