//! 装配真实输入、行比较与轴构建模块；覆盖以固定脚本的模块和契约为准。

#![allow(dead_code)]
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]

extern crate rectangle_std as std_adapter;

#[path = "../src/chord_graph.rs"]
mod chord_graph;
#[path = "../src/error.rs"]
mod error;
#[path = "../src/geometry.rs"]
mod geometry;
#[path = "../src/proof/partition.rs"]
mod partition;
mod index {
    pub(crate) use crate::optimal::{copy, get_mut};
}
#[path = "../src/optimal"]
pub mod optimal {
    mod fixed;
    mod graph;
    mod greedy;
    mod hk;
    mod matching;
    mod search;
    mod sparse;
    mod types;

    pub use sparse::{DenseLabels64, QuadLeaf64, SparseOptimalScratch64, SparseQuadError};
    pub use types::{PackedRectangles64, RangeU8, Rectangle};

    include!("../src/index.rs");
}

fn main() {}
