//! 检查输出规范的推论与违规反例；不装配分区算法，不计为分区实现证明。
#![allow(dead_code)]
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]

#[path = "../src/error.rs"]
mod error;
#[path = "../src/index.rs"]
mod index;
#[path = "../src/optimal/types.rs"]
mod types;
mod optimal {
    pub(crate) use crate::types::{RangeU8, Rectangle};
}
#[path = "../src/proof/partition.rs"]
mod partition;

use types::{RangeU8, Rectangle};
use vstd::prelude::*;

verus! {

#[cfg(reject_partition_overlap)]
proof fn reject_partition_overlap(rectangle: Rectangle)
    requires partition::valid(rectangle),
{
    let labels = Seq::<u16>::new(4096, |p: int| if partition::covers(rectangle, p) { rectangle.value } else { 0 });
    partition::origin_covered(rectangle);
    assert(partition::partition(labels, seq![rectangle, rectangle]));
}

#[cfg(reject_partition_background)]
proof fn reject_partition_background() {
    let labels = Seq::<u16>::new(4096, |i: int| 0);
    let rectangle = Rectangle { value: 7, x: RangeU8 { start: 0, end: 1 }, y: RangeU8 { start: 0, end: 1 } };
    assert(partition::partition(labels, seq![rectangle]));
}

fn main() {}

}
