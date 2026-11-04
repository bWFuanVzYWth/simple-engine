//! 全局几何归约的数学证明入口；不单独计为生产分解证明。
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

use vstd::prelude::*;

verus! {

#[cfg(reject_corner_diagonal_owner)]
proof fn reject_diagonal_owner() {
    assert(partition::corners::legal(0,-1,0,-1));
}

#[cfg(reject_corner_cross_weight)]
proof fn reject_cross_weight() {
    use partition::corners::*;
    local_identity(0,1,2,3);
    assert(weight(true,true,true,true)==local_corners(0,1,2,3)-2*endpoints(0,1,2,3));
}

#[cfg(reject_corner_label_touch)]
proof fn reject_label_touch() {
    assert(partition::corners::weight(true,false,true,false)==0);
}

#[cfg(reject_corner_double_count)]
proof fn reject_double_count(rectangle: types::Rectangle)
    requires partition::valid(rectangle),
{
    partition::corners::rectangle_four_corners(rectangle);
    assert(partition::corners::all_corner_count(rectangle)==3);
}

#[cfg(reject_good_closed_endpoint)]
proof fn reject_closed_endpoint() {
    use partition::good::*;
    let h=encode(false,1,1,2);let v=encode(true,1,1,2);
    assert(!intersects(h,v));
}

#[cfg(reject_vertical_closed_count)]
proof fn reject_closed_count(labels: Seq<u16>,verticals: Set<int>)
    requires partition::split_geometry::selected(labels,verticals),verticals.len()>0,
{
    partition::vertical_count::closed_minus_open(labels,verticals);
    assert(partition::vertical_count::total(verticals,true)==partition::vertical_count::total(verticals,false));
}

#[cfg(reject_lower_witness)]
proof fn reject_lower_witness(labels: Seq<u16>,rectangles: Seq<types::Rectangle>)
    requires partition::partition(labels,rectangles),
{
    partition::lower_bound::competitor_witness(labels,rectangles);
    assert(partition::colored::b4(labels)>4*rectangles.len()+4*partition::lower_bound::chosen(labels,rectangles).len());
}

#[cfg(reject_scan_close_order)]
proof fn reject_scan_order(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,i: int,j: int)
    requires 0<=i<j<partition::scan_model::canonical(labels,verticals,blocked).len(),
{
    partition::scan_model::canonical_order(labels,verticals,blocked,i,j);
    let rectangles=partition::scan_model::canonical(labels,verticals,blocked);
    assert(rectangles[j].y.end<rectangles[i].y.end);
}

fn main() {}

}
