//! 在生产命名空间装配冲突 CSR 与匹配核；未标记调用链仍不计覆盖。
#![allow(dead_code)]
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]

extern crate rectangle_std as std_adapter;

#[path = "../src/chord_graph.rs"]
mod chord_graph;
#[path = "../src/error.rs"]
mod error;
#[path = "../src/geometry.rs"]
mod geometry;
mod index {
    pub(crate) use crate::optimal::{copy, get_mut};
}
#[path = "../src/optimal"]
mod optimal {
    mod fixed;
    mod graph;
    mod greedy;
    mod hk;
    #[path = "../index.rs"]
    mod index;
    mod matching;
    mod search;
    mod types;
    pub(crate) use index::*;
}

fn main() {}
