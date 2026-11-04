//! 装配生产闭段掩码及最终标记转换；不导入未验证的图构造和 HK 调度。

#![allow(dead_code)]
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]

#[path = "../src/chord_graph.rs"]
mod chord_graph;
#[path = "../src/index.rs"]
mod index;

use vstd::prelude::*;

verus! {

#[cfg(reject_chord_endpoint)]
fn reject_chord_endpoint() {
    let mask = chord_graph::internal_mask(7, 7);
    assert(mask == 0);
}

#[cfg(reject_mark_matched_index)]
fn reject_mark_matched_index() {
    let pairs = [1u16];
    let distances = [0u16];
    let mut left = [false];
    let mut right = [false];
    chord_graph::mark_independent_set(&pairs, &distances, false, &mut left, &mut right);
}

#[cfg(reject_uncovered_matching)]
proof fn reject_uncovered_matching() {
    let pairs = Set::empty().insert((0int, 0int));
    chord_graph::proof::matching_cover_bound(pairs, Set::empty(), Set::empty());
}

#[cfg(reject_csr_degree_loss)]
fn reject_csr_degree_loss() {
    let edges = [chord_graph::SparseEdge { left: 0u16, right: 0u16 }];
    let mut offsets = [0u16; 2];
    let mut starts = [0u32; 1];
    chord_graph::build_csr_layout(&edges, &mut offsets, &mut starts);
    assert(offsets[1] == 0);
}

fn zero_degree_right_keeps_zero_start() {
    let edges = [chord_graph::SparseEdge { left: 0u16, right: 0u16 }];
    let mut offsets = [0u16; 2];
    let mut starts = [0u32; 2];
    chord_graph::build_csr_layout(&edges, &mut offsets, &mut starts);
    assert(starts[1] == 0);
    #[cfg(reject_csr_unused_start)]
    assert(starts[1] == 1);
}

#[cfg(reject_scatter_capacity)]
fn reject_scatter_capacity() {
    let edges = [chord_graph::SparseEdge { left: 0u16, right: 0u16 }; 2];
    let offsets = [0u16, 1u16];
    let mut cursors = [0u16];
    let mut output = [0u16];
    chord_graph::scatter_group(&edges, 0, &offsets, &mut cursors, &mut output);
}

#[cfg(reject_scatter_prefix)]
fn reject_scatter_prefix() {
    let edges = [chord_graph::SparseEdge { left: 0u16, right: 3u16 }];
    let offsets = [0u16, 2u16];
    let mut cursors = [1u16];
    let mut output = [9u16; 2];
    proof { reveal_with_fuel(chord_graph::proof::count_below, 3); }
    chord_graph::scatter_group(&edges, 3, &offsets, &mut cursors, &mut output);
    assert(output[0] == 3);
}

}

fn main() {}
