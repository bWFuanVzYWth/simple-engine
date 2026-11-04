//! 装配真实 packed 存储、借用迭代器、公共错误和索引；不验证分区生产者。

#![allow(dead_code)]
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]

#[path = "../src/error.rs"]
mod error;
#[path = "../src/index.rs"]
mod index;
#[path = "../src/optimal/types.rs"]
mod types;

use types::{PackedRectangles64, RangeU8, Rectangle};
use vstd::{prelude::*, std_specs::iter::IteratorSpec};

verus! {

fn reuse_and_accessors(rectangle: Rectangle) {
    let mut output = PackedRectangles64::new();
    let empty = output.is_empty();
    assert(empty);
    let result = output.push(rectangle);
    assert(result is Ok);
    let len = output.len();
    assert(len == 1);
    let bounds = output.bounds();
    assert(bounds.len() == 1);
    let labels = output.labels();
    assert(labels[0] == rectangle.value);
    output.clear();
    let empty = output.is_empty();
    assert(empty);
    let bounds = output.bounds();
    let labels = output.labels();
    assert(bounds.len() == 0 && labels.len() == 0);
}

fn iteration_order(first: Rectangle, second: Rectangle) {
    let mut output = PackedRectangles64::default();
    let pushed = output.push(first);
    assert(pushed is Ok);
    let pushed = output.push(second);
    assert(pushed is Ok);
    let mut iter = output.iter();
    assert(iter.remaining() == seq![first, second]);
    let len = iter.len();
    assert(len == 2);
    let value = iter.next();
    assert(value == Some(first));
    let len = iter.len();
    assert(len == 1);
    let value = iter.next();
    assert(value == Some(second));
    let len = iter.len();
    assert(len == 0);
    let value = iter.next();
    assert(value is None);
}

#[cfg(reject_packed_iter_order)]
fn reject_packed_iter_order() {
    let mut output = PackedRectangles64::new();
    let first = Rectangle { value: 7, x: RangeU8::new(0, 64), y: RangeU8::new(0, 64) };
    let second = Rectangle { value: 8, ..first };
    let result = output.push(first);
    assert(result is Ok);
    let result = output.push(second);
    assert(result is Ok);
    let mut iter = output.iter();
    let value = iter.next();
    assert(value == Some(second));
}

#[cfg(reject_packed_iter_length)]
fn reject_packed_iter_length() {
    let output = PackedRectangles64::new();
    let iter = output.iter();
    let len = iter.len();
    assert(len == 1);
}

#[cfg(reject_packed_clear_contents)]
fn reject_packed_clear_contents() {
    let mut output = PackedRectangles64::new();
    let result = output.push(Rectangle { value: 7, x: RangeU8::new(0, 64), y: RangeU8::new(0, 64) });
    assert(result is Ok);
    output.clear();
    let len = output.len();
    assert(len == 1);
}

#[cfg(reject_packed_full_push)]
fn reject_packed_full_push(mut output: PackedRectangles64)
    requires output.valid(), output@.len() == 4096,
{
    let result = output.push(Rectangle { value: 7, x: RangeU8::new(0, 64), y: RangeU8::new(0, 64) });
    assert(result is Ok);
}

}

fn main() {}
