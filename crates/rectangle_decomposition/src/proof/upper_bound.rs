//! 原始 run、V split 与真实同 key 续接的精确数量上界。

use super::{
    Rectangle, continuation_count, corners, good, neighbor_count, optimal, optimality, partition,
    row_count, split_count, split_geometry, vertical_count,
};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn continuation_line(labels: Seq<u16>,verticals: Set<int>,y: int) -> int {
    sum(65,|end: int|bit(split_geometry::continuation_end(labels,verticals,y,end)))
}

pub(crate) open spec fn continuations(labels: Seq<u16>,verticals: Set<int>) -> int {
    sum(65,|y: int|continuation_line(labels,verticals,y))
}

pub(crate) open spec fn result_count(labels: Seq<u16>,verticals: Set<int>) -> int {
    split_count::total_runs(labels,verticals)-continuations(labels,verticals)
}

pub(crate) open spec fn point_line(labels: Seq<u16>,verticals: Set<int>,y: int) -> int {
    sum(65,|x: int|continuation_count::points(row_count::source(labels,y),continuation_count::selected_points(labels,verticals,y),x))
}

pub(crate) open spec fn blocked_line(labels: Seq<u16>,verticals: Set<int>,y: int) -> int {
    sum(65,|x: int|continuation_count::blocked(row_count::source(labels,y),row_count::marks(labels,y),
        continuation_count::selected_points(labels,verticals,y),x))
}

pub(crate) proof fn line_count(labels: Seq<u16>,verticals: Set<int>,y: int)
    requires split_geometry::selected(labels,verticals),0<=y<=64,
    ensures continuation_line(labels,verticals,y)==row_count::identical_line(labels,y)
        +point_line(labels,verticals,y)-blocked_line(labels,verticals,y),
{
    continuation_count::selected_valid(labels,verticals,y);
    let f=row_count::source(labels,y);let r=row_count::marks(labels,y);let b=continuation_count::selected_points(labels,verticals,y);
    continuation_count::total_count(f,r,b);
    let a=|end: int|continuation_count::continued(f,r,b,end);
    let c=|end: int|bit(split_geometry::continuation_end(labels,verticals,y,end));
    assert forall|end: int| 0<=end<65 implies #[trigger] a(end)==#[trigger] c(end) by {
        continuation_count::actual_continuation(labels,verticals,y,end);
    }
    corners::counting::sum_equal(65,a,c);
}

pub(crate) proof fn continuation_count(labels: Seq<u16>,verticals: Set<int>)
    requires split_geometry::selected(labels,verticals),
    ensures continuations(labels,verticals)==row_count::identical(labels)+vertical_count::total(verticals,true)
        -optimality::neighbors(labels,verticals).len() as int,
{
    let w=|y: int|continuation_line(labels,verticals,y);let a=|y: int|row_count::identical_line(labels,y);
    let b=|y: int|point_line(labels,verticals,y);let c=|y: int|blocked_line(labels,verticals,y);
    assert forall|y: int| 0<=y<65 implies #[trigger] w(y)==#[trigger] a(y)+#[trigger] b(y)-#[trigger] c(y) by {
        line_count(labels,verticals,y);
    }
    corners::counting::sum_add_sub(65,w,a,b,c);
    split_count::total_point_count(labels,verticals);neighbor_count::blocked_neighbors_count(labels,verticals);
    corners::counting::sum_equal(65,b,|y: int|sum(65,|x: int|continuation_count::points(row_count::source(labels,y),
        continuation_count::selected_points(labels,verticals,y),x)));
    corners::counting::sum_equal(65,c,|y: int|sum(65,|x: int|continuation_count::blocked(row_count::source(labels,y),row_count::marks(labels,y),
        continuation_count::selected_points(labels,verticals,y),x)));
}

/// V 的开放行数与闭点数相消，邻域对同一 H 只计一次。
pub(crate) proof fn exact_result_count(labels: Seq<u16>,verticals: Set<int>)
    requires split_geometry::selected(labels,verticals),
    ensures result_count(labels,verticals)==row_count::total_runs(labels)-row_count::identical(labels)
        -verticals.len() as int+optimality::neighbors(labels,verticals).len() as int,
{
    split_count::run_count(labels,verticals);
    continuation_count(labels,verticals);
    vertical_count::closed_minus_open(labels,verticals);
}

/// 实际扫描只需建立精确覆盖及出生−续接计数，即得到全部竞争分区中的最优性。
pub(crate) proof fn optimal_from_scan(labels: Seq<u16>,rectangles: Seq<Rectangle>,selected: Set<int>)
    requires partition(labels,rectangles),good::maximum_independent(labels,selected),
        rectangles.len() as int==result_count(labels,optimality::vertical_part(selected)),
    ensures optimal(labels,rectangles),
{
    broadcast use vstd::set_lib::group_set_properties;
    let vs=optimality::vertical_part(selected);
    assert(split_geometry::selected(labels,vs));
    exact_result_count(labels,vs);
    optimality::from_run_count(labels,rectangles,selected);
}

}
