//! 实际 split run 开始事件及 V 闭点计数。

use super::{continuation_count, corners, good, row_count, split_geometry, vertical_count};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn start(labels: Seq<u16>,verticals: Set<int>,y: int,x: int) -> bool {
    0<=y<64 && 0<=x<64 && corners::at(labels,x,y)!=0
        && (corners::at(labels,x-1,y)!=corners::at(labels,x,y) || split_geometry::active(verticals,x,y))
}

pub(crate) open spec fn row_runs(labels: Seq<u16>,verticals: Set<int>,y: int) -> int {
    sum(64,|x: int|bit(start(labels,verticals,y,x)))
}

pub(crate) open spec fn total_runs(labels: Seq<u16>,verticals: Set<int>) -> int {
    sum(64,|y: int|row_runs(labels,verticals,y))
}

pub(crate) proof fn row_run_count(labels: Seq<u16>,verticals: Set<int>,y: int)
    requires split_geometry::selected(labels,verticals),0<=y<64,
    ensures row_runs(labels,verticals,y)==row_count::rows_begin(labels,y)+vertical_count::row_count(verticals,y,false),
{
    let a=|x: int|bit(start(labels,verticals,y,x));
    let b=|x: int|super::chains::begin(corners::at(labels,x-1,y),corners::at(labels,x,y));
    let c=|x: int|bit(split_geometry::active(verticals,x,y));
    assert forall|x: int| 0<=x<64 implies #[trigger] a(x)==#[trigger] b(x)+#[trigger] c(x) by {
        if split_geometry::active(verticals,x,y) {split_geometry::active_interior(labels,verticals,x,y);}
    }
    corners::counting::sum_equal(64,a,|x: int|b(x)+c(x));
    corners::counting::sum_add(64,b,c);
    if split_geometry::active(verticals,64,y) {split_geometry::active_interior(labels,verticals,64,y);}
    assert(sum(65,b)==sum(64,b));assert(sum(65,c)==sum(64,c));
    corners::counting::sum_equal(65,c,|x: int|bit(vertical_count::coordinate(verticals,x,y,false)));
}

pub(crate) proof fn run_count(labels: Seq<u16>,verticals: Set<int>)
    requires split_geometry::selected(labels,verticals),
    ensures total_runs(labels,verticals)==row_count::total_runs(labels)+vertical_count::total(verticals,false),
{
    let a=|y: int|row_runs(labels,verticals,y);
    let b=|y: int|row_count::rows_begin(labels,y);
    let c=|y: int|vertical_count::row_count(verticals,y,false);
    assert forall|y: int| 0<=y<64 implies #[trigger] a(y)==#[trigger] b(y)+#[trigger] c(y) by {row_run_count(labels,verticals,y);}
    corners::counting::sum_equal(64,a,|y: int|b(y)+c(y));corners::counting::sum_add(64,b,c);
    vertical_count::outside_row(labels,verticals,false);assert(sum(65,c)==sum(64,c));
}

pub(crate) proof fn point_coordinate(labels: Seq<u16>,verticals: Set<int>,x: int,y: int,value: u16)
    requires split_geometry::point(verticals,x,y,value,labels),
    ensures vertical_count::coordinate(verticals,x,y,true),
{
    let v=choose|v: int| verticals.contains(v) && #[trigger] good::line(v)==x
        && good::start(v)<=y<=good::end(v) && good::value(labels,v)==value;
    assert(vertical_count::included(v,y,true));
}

/// 每个闭点属于恰好一个同标签 overlap component，角点接触不重复计数。
pub(crate) proof fn point_event(labels: Seq<u16>,verticals: Set<int>,y: int,x: int)
    requires split_geometry::selected(labels,verticals),0<=y<=64,0<=x<=64,
    ensures continuation_count::points(row_count::source(labels,y),continuation_count::selected_points(labels,verticals,y),x)
        ==bit(vertical_count::coordinate(verticals,x,y,true)),
{
    let f=row_count::source(labels,y);let b=continuation_count::selected_points(labels,verticals,y);
    if f(x)!=0 && f(x)!=f(x-1) && b(x,f(x)) && f(x-1)!=0 && b(x,f(x-1)) {
        split_geometry::point_has_color(labels,verticals,x,y,f(x));
        split_geometry::point_has_color(labels,verticals,x,y,f(x-1));
        good::interior_value_unique(labels,x,y,f(x),f(x-1));
    }
    if b(x,f(x)) {point_coordinate(labels,verticals,x,y,f(x));}
    if b(x,f(x-1)) {point_coordinate(labels,verticals,x,y,f(x-1));}
    if vertical_count::coordinate(verticals,x,y,true) {
        let v=choose|v: int| verticals.contains(v) && #[trigger] good::line(v)==x && vertical_count::included(v,y,true);
        assert(good::good(labels,v));let value=good::value(labels,v);
        assert(split_geometry::point(verticals,x,y,value,labels));
        split_geometry::point_has_color(labels,verticals,x,y,value);
        assert(f(x)==value || f(x-1)==value);
        if f(x)==value {assert(b(x,f(x)));}else{assert(b(x,f(x-1)));}
    }
}

pub(crate) proof fn total_point_count(labels: Seq<u16>,verticals: Set<int>)
    requires split_geometry::selected(labels,verticals),
    ensures sum(65,|y: int|sum(65,|x: int|continuation_count::points(row_count::source(labels,y),
        continuation_count::selected_points(labels,verticals,y),x)))==vertical_count::total(verticals,true),
{
    let a=|y: int|sum(65,|x: int|continuation_count::points(row_count::source(labels,y),
        continuation_count::selected_points(labels,verticals,y),x));
    let b=|y: int|vertical_count::row_count(verticals,y,true);
    assert forall|y: int| 0<=y<65 implies #[trigger] a(y)==#[trigger] b(y) by {
        let c=|x: int|continuation_count::points(row_count::source(labels,y),continuation_count::selected_points(labels,verticals,y),x);
        let d=|x: int|bit(vertical_count::coordinate(verticals,x,y,true));
        assert forall|x: int| 0<=x<65 implies #[trigger] c(x)==#[trigger] d(x) by {point_event(labels,verticals,y,x);}
        corners::counting::sum_equal(65,c,d);
    }
    corners::counting::sum_equal(65,a,b);
}

}
