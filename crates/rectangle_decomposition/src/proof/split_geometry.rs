//! 选中 good 竖线对行 runs 的细分；续接用共同右端点计数。

use super::{corners, good};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn selected(labels: Seq<u16>,verticals: Set<int>) -> bool {
    verticals.subset_of(good::chords(labels,true))
}

pub(crate) open spec fn active(verticals: Set<int>,x: int,y: int) -> bool {
    exists|v: int| #[trigger] verticals.contains(v) && good::line(v)==x
        && good::start(v)<=y<good::end(v)
}

pub(crate) open spec fn point(verticals: Set<int>,x: int,y: int,value: u16,labels: Seq<u16>) -> bool {
    exists|v: int| #[trigger] verticals.contains(v) && good::line(v)==x
        && good::start(v)<=y<=good::end(v) && good::value(labels,v)==value
}

pub(crate) open spec fn run(labels: Seq<u16>,verticals: Set<int>,y: int,start: int,end: int,value: u16) -> bool {
    0<=y<64 && 0<=start<end<=64 && value!=0
        && (corners::at(labels,start-1,y)!=value || active(verticals,start,y))
        && (corners::at(labels,end,y)!=value || active(verticals,end,y))
        && (forall|x: int| start<=x<end ==> #[trigger] corners::at(labels,x,y)==value)
        && (forall|x: int| start<x<end ==> !#[trigger] active(verticals,x,y))
}

pub(crate) open spec fn continuation_end(labels: Seq<u16>,verticals: Set<int>,y: int,end: int) -> bool {
    exists|start: int,value: u16| #[trigger] run(labels,verticals,y-1,start,end,value)
        && #[trigger] run(labels,verticals,y,start,end,value)
}

pub(crate) proof fn active_interior(labels: Seq<u16>,verticals: Set<int>,x: int,y: int)
    requires selected(labels,verticals),active(verticals,x,y),
    ensures 1<=x<=63,0<=y<64,corners::at(labels,x-1,y)==corners::at(labels,x,y),
        corners::at(labels,x,y)!=0,
{
    let v=choose|v: int| verticals.contains(v) && #[trigger] good::line(v)==x && good::start(v)<=y<good::end(v);
    assert(good::good(labels,v));
    assert(good::unit_inside(labels,true,x,y,good::value(labels,v)));
}

pub(crate) proof fn point_has_color(labels: Seq<u16>,verticals: Set<int>,x: int,y: int,value: u16)
    requires selected(labels,verticals),point(verticals,x,y,value,labels),
    ensures value!=0,good::quadrants(labels,x,y,value)>=3,1<=x<=63,1<=y<=63,
{
    let v=choose|v: int| verticals.contains(v) && #[trigger] good::line(v)==x
        && good::start(v)<=y<=good::end(v) && good::value(labels,v)==value;
    assert(good::good(labels,v));
    good::closed_point(labels,true,x,good::start(v),good::end(v),value,y);
}

pub(crate) proof fn active_for_value(labels: Seq<u16>,verticals: Set<int>,x: int,y: int,value: u16)
    requires selected(labels,verticals),active(verticals,x,y),
        corners::at(labels,x-1,y)==value || corners::at(labels,x,y)==value,
    ensures point(verticals,x,y,value,labels),point(verticals,x,y+1,value,labels),
{
    let v=choose|v: int| verticals.contains(v) && #[trigger] good::line(v)==x
        && good::start(v)<=y<good::end(v);
    assert(good::good(labels,v));
    assert(good::unit_inside(labels,true,x,y,good::value(labels,v)));
    assert(good::value(labels,v)==value);
    assert(good::start(v)<=y+1<=good::end(v));
}

pub(crate) proof fn point_full(labels: Seq<u16>,verticals: Set<int>,x: int,y: int,value: u16)
    requires selected(labels,verticals),value!=0,good::quadrants(labels,x,y,value)==4,
    ensures point(verticals,x,y,value,labels) <==> active(verticals,x,y),
        point(verticals,x,y,value,labels) <==> active(verticals,x,y-1),
{
    if active(verticals,x,y) {active_for_value(labels,verticals,x,y,value);}
    if active(verticals,x,y-1) {active_for_value(labels,verticals,x,y-1,value);}
    if point(verticals,x,y,value,labels) {
        let v=choose|v: int| verticals.contains(v) && #[trigger] good::line(v)==x
            && good::start(v)<=y<=good::end(v) && good::value(labels,v)==value;
        assert(good::good(labels,v));
        assert(good::start(v)<y<good::end(v));
    }
}

pub(crate) open spec fn boundary(labels: Seq<u16>,verticals: Set<int>,y: int,x: int,value: u16) -> bool {
    corners::at(labels,x-1,y)!=value || corners::at(labels,x,y)!=value || active(verticals,x,y)
}

pub(crate) open spec fn common(labels: Seq<u16>,verticals: Set<int>,y: int,x: int,value: u16) -> bool {
    boundary(labels,verticals,y-1,x,value) && boundary(labels,verticals,y,x,value)
}

/// 在同色 overlap 内，任何内部 V 边界同时出现在上下两行。
pub(crate) proof fn interior_boundary(labels: Seq<u16>,verticals: Set<int>,y: int,x: int,value: u16)
    requires selected(labels,verticals),value!=0,good::quadrants(labels,x,y,value)==4,
    ensures common(labels,verticals,y,x,value) <==> point(verticals,x,y,value,labels),
        boundary(labels,verticals,y-1,x,value) <==> boundary(labels,verticals,y,x,value),
{
    point_full(labels,verticals,x,y,value);
}

/// overlap 端点若是原直边，两行本来共有边界；若是凹角，V 必须补齐缺少的一边。
pub(crate) proof fn overlap_boundary(labels: Seq<u16>,verticals: Set<int>,y: int,x: int,value: u16)
    requires selected(labels,verticals),value!=0,
        super::row_count::overlap(labels,y,x-1)==value || super::row_count::overlap(labels,y,x)==value,
    ensures common(labels,verticals,y,x,value)
        <==> point(verticals,x,y,value,labels) ||
            (super::row_count::overlap(labels,y,x-1)!=super::row_count::overlap(labels,y,x)
                && !good::reflex(labels,false,y,x,value)),
{
    let n=good::quadrants(labels,x,y,value);
    assert(2<=n<=4);
    if n==4 {interior_boundary(labels,verticals,y,x,value);}
    else {
        if active(verticals,x,y) && (corners::at(labels,x-1,y)==value || corners::at(labels,x,y)==value) {
            active_for_value(labels,verticals,x,y,value);
        }
        if active(verticals,x,y-1) && (corners::at(labels,x-1,y-1)==value || corners::at(labels,x,y-1)==value) {
            active_for_value(labels,verticals,x,y-1,value);
        }
        if point(verticals,x,y,value,labels) {
            point_has_color(labels,verticals,x,y,value);
            let v=choose|v: int| verticals.contains(v) && #[trigger] good::line(v)==x
                && good::start(v)<=y<=good::end(v) && good::value(labels,v)==value;
            assert(good::good(labels,v));
            if good::start(v)<y {
                assert(good::unit_inside(labels,true,x,y-1,value));
                assert(active(verticals,x,y-1));
            }
            if y<good::end(v) {
                assert(good::unit_inside(labels,true,x,y,value));
                assert(active(verticals,x,y));
            }
        }
    }
}

pub(crate) proof fn row_start_unique(labels: Seq<u16>,verticals: Set<int>,y: int,a: int,b: int,c: int,value: u16,other: u16)
    requires run(labels,verticals,y,a,b,value),run(labels,verticals,y,a,c,other),
    ensures b==c,value==other,
{
    assert(corners::at(labels,a,y)==value && corners::at(labels,a,y)==other);
    if b<c {assert(corners::at(labels,b,y)==other);assert(!active(verticals,b,y));}
    if c<b {assert(corners::at(labels,c,y)==value);assert(!active(verticals,c,y));}
}

pub(crate) proof fn row_end_unique(labels: Seq<u16>,verticals: Set<int>,y: int,a: int,b: int,end: int,value: u16,other: u16)
    requires run(labels,verticals,y,a,end,value),run(labels,verticals,y,b,end,other),
    ensures a==b,value==other,
{
    assert(corners::at(labels,end-1,y)==value && corners::at(labels,end-1,y)==other);
    if a<b {assert(corners::at(labels,b-1,y)==value);assert(!active(verticals,b,y));}
    if b<a {assert(corners::at(labels,a-1,y)==other);assert(!active(verticals,a,y));}
}

/// 相同 split run 若跨过 good H，必有 V 在该 H 上形成共同起点。
pub(crate) proof fn continued_hits_horizontal(labels: Seq<u16>,verticals: Set<int>,h: int,
    start: int,end: int,value: u16)
    requires selected(labels,verticals),good::good(labels,h),!good::vertical(h),
        run(labels,verticals,good::line(h)-1,start,end,value),
        run(labels,verticals,good::line(h),start,end,value),
        start<good::end(h),good::start(h)<end,
    ensures exists|v: int| #[trigger] verticals.contains(v) && good::intersects(h,v),
{
    let y=good::line(h);let a=good::start(h);let b=good::end(h);
    let p=if start>a {start}else{a};
    assert(good::unit_inside(labels,false,y,p,good::value(labels,h)));
    assert(corners::at(labels,p,y)==value);
    assert(value==good::value(labels,h));
    good::endpoint_blocks(labels,false,y,a,b,value);
    if start<a {
        assert(corners::at(labels,a-1,y-1)==value && corners::at(labels,a-1,y)==value);
        assert(good::unit_inside(labels,false,y,a-1,value));
    }
    assert(a<=start<b);
    assert(common(labels,verticals,y,start,value));
    if start==a {overlap_boundary(labels,verticals,y,start,value);}
    else {
        good::interior_full(labels,false,y,a,b,value,start);
        interior_boundary(labels,verticals,y,start,value);
    }
    assert(point(verticals,start,y,value,labels));
    let v=choose|v: int| verticals.contains(v) && #[trigger] good::line(v)==start
        && good::start(v)<=y<=good::end(v) && good::value(labels,v)==value;
    assert(good::good(labels,v));
    assert(good::intersects(h,v));
}

/// 独立选线中的 H 不会额外阻断本可延长的同 key run。
pub(crate) proof fn horizontal_redundant(labels: Seq<u16>,verticals: Set<int>,h: int,
    start: int,end: int,value: u16)
    requires selected(labels,verticals),good::good(labels,h),!good::vertical(h),
        forall|v: int| #[trigger] verticals.contains(v) ==> !good::intersects(h,v),
        run(labels,verticals,good::line(h)-1,start,end,value),
        run(labels,verticals,good::line(h),start,end,value),
    ensures end<=good::start(h) || good::end(h)<=start,
{
    if !(end<=good::start(h) || good::end(h)<=start) {
        continued_hits_horizontal(labels,verticals,h,start,end,value);
    }
}

}
