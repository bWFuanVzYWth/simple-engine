//! 原始最大行 runs、相邻行正重叠与局部角点恒等式。

use super::{chain_marks, chains, colored, corners, good};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn run(labels: Seq<u16>,y: int,start: int,end: int,value: u16) -> bool {
    0<=y<64 && 0<=start<end<=64 && value!=0
        && corners::at(labels,start-1,y)!=value && corners::at(labels,end,y)!=value
        && forall|x: int| start<=x<end ==> #[trigger] corners::at(labels,x,y)==value
}

pub(crate) open spec fn overlap(labels: Seq<u16>,y: int,t: int) -> u16 {
    if corners::at(labels,t,y-1)==corners::at(labels,t,y) {corners::at(labels,t,y)} else {0}
}

pub(crate) open spec fn source(labels: Seq<u16>,y: int) -> spec_fn(int)->u16 {
    |t: int|overlap(labels,y,t)
}

pub(crate) open spec fn marks(labels: Seq<u16>,y: int) -> spec_fn(int,u16)->bool {
    |t: int,value: u16|good::reflex(labels,false,y,t,value)
}

pub(crate) open spec fn good_line(labels: Seq<u16>,y: int) -> int {
    sum(65,|t: int|chain_marks::paired(source(labels,y),marks(labels,y),t))
}

pub(crate) open spec fn identical_line(labels: Seq<u16>,y: int) -> int {
    sum(65,|t: int|chain_marks::both_unmarked(source(labels,y),marks(labels,y),t))
}

pub(crate) open spec fn signed_line(labels: Seq<u16>,y: int) -> int {
    sum(65,|t: int|chain_marks::signed(source(labels,y),marks(labels,y),t))
}

pub(crate) open spec fn color_weight(la: u16,lb: u16,lc: u16,ld: u16,q: int) -> int {
    let value=colored::pick(la,lb,lc,ld,q);
    if colored::canonical(la,lb,lc,ld,q) {corners::weight(la==value,lb==value,lc==value,ld==value)} else {0}
}

pub(crate) open spec fn row_ends(la: u16,lb: u16,lc: u16,ld: u16) -> int {
    chains::begin(la,lb)+chains::end(la,lb)+chains::begin(ld,lc)+chains::end(ld,lc)
}

pub(crate) open spec fn diagram_overlap(a: u16,b: u16) -> u16 {if a==b {a} else {0}}

pub(crate) open spec fn diagram_reflex(la: u16,lb: u16,lc: u16,ld: u16,value: u16) -> int {
    bit(value!=0 && corners::count(la==value,lb==value,lc==value,ld==value)==3)
}

pub(crate) open spec fn diagram_signed(la: u16,lb: u16,lc: u16,ld: u16) -> int {
    let a=diagram_overlap(la,ld);let b=diagram_overlap(lb,lc);
    (if b!=0 && a!=b {2*diagram_reflex(la,lb,lc,ld,b)-1}else{0})
        +(if a!=0 && a!=b {2*diagram_reflex(la,lb,lc,ld,a)-1}else{0})
}

/// 多标签、对角点接触与边界外 0 共用同一个局部恒等式。
pub(crate) proof fn local_run_identity(la: u16,lb: u16,lc: u16,ld: u16)
    ensures color_weight(la,lb,lc,ld,0)+color_weight(la,lb,lc,ld,1)
        +color_weight(la,lb,lc,ld,2)+color_weight(la,lb,lc,ld,3)
        ==row_ends(la,lb,lc,ld)+2*diagram_signed(la,lb,lc,ld),
{
    if la==0 {} if lb==0 {} if lc==0 {} if ld==0 {}
    if la==lb {} if la==lc {} if la==ld {} if lb==lc {} if lb==ld {} if lc==ld {}
}

pub(crate) open spec fn vertex_row_ends(labels: Seq<u16>,v: int) -> int {
    row_ends(colored::label(labels,v,0),colored::label(labels,v,1),colored::label(labels,v,2),colored::label(labels,v,3))
}

pub(crate) open spec fn vertex_signed(labels: Seq<u16>,v: int) -> int {
    chain_marks::signed(source(labels,v/65),marks(labels,v/65),v%65)
}

pub(crate) proof fn vertex_run_identity(labels: Seq<u16>,v: int)
    requires 0<=v<4225,
    ensures sum(4,|q: int|colored::site_weight(labels,4*v+q))
        ==vertex_row_ends(labels,v)+2*vertex_signed(labels,v),
{
    local_run_identity(colored::label(labels,v,0),colored::label(labels,v,1),
        colored::label(labels,v,2),colored::label(labels,v,3));
    assert forall|q: int| 0<=q<4 implies #[trigger] colored::site_weight(labels,4*v+q)
        ==color_weight(colored::label(labels,v,0),colored::label(labels,v,1),
            colored::label(labels,v,2),colored::label(labels,v,3),q) by {
        assert((4*v+q)/4==v && (4*v+q)%4==q) by(nonlinear_arith) requires 0<=q<4;
        if q==0 {} else if q==1 {} else if q==2 {} else {}
    }
    reveal_with_fuel(sum,5);
}

pub(crate) open spec fn row_event(labels: Seq<u16>,y: int,t: int) -> int {
    chains::begin(corners::at(labels,t-1,y),corners::at(labels,t,y))
        +chains::end(corners::at(labels,t-1,y),corners::at(labels,t,y))
}

pub(crate) open spec fn rows_begin(labels: Seq<u16>,y: int) -> int {
    sum(65,|t: int|chains::begin(corners::at(labels,t-1,y),corners::at(labels,t,y)))
}

pub(crate) open spec fn rows_end(labels: Seq<u16>,y: int) -> int {
    sum(65,|t: int|row_event(labels,y,t))
}

pub(crate) open spec fn total_runs(labels: Seq<u16>) -> int {sum(64,|y: int|rows_begin(labels,y))}
pub(crate) open spec fn identical(labels: Seq<u16>) -> int {sum(65,|y: int|identical_line(labels,y))}
pub(crate) open spec fn good_events(labels: Seq<u16>) -> int {sum(65,|y: int|good_line(labels,y))}

pub(crate) proof fn row_ends_twice(labels: Seq<u16>,y: int)
    ensures rows_end(labels,y)==2*rows_begin(labels,y),
{
    let f=|t: int|corners::at(labels,t,y);
    let b=|t: int|chains::begin(f(t-1),f(t));
    let e=|t: int|chains::end(f(t-1),f(t));
    let occ=|t: int|bit(f(t)!=0);
    let diff=|t: int|b(t)-e(t);
    assert forall|t: int| 0<=t<65 implies #[trigger] diff(t)==occ(t)-occ(t-1) by {
        chains::transition(f(t-1),f(t));
    }
    corners::counting::sum_equal(65,diff,|t: int|occ(t)-occ(t-1));
    corners::counting::telescope(65,occ);
    corners::counting::sum_add(65,diff,e);
    corners::counting::sum_equal(65,|t: int|diff(t)+e(t),b);
    corners::counting::sum_add(65,b,e);
    corners::counting::sum_equal(65,|t: int|b(t)+e(t),|t: int|row_event(labels,y,t));
    corners::counting::sum_equal(65,b,
        |t: int|chains::begin(corners::at(labels,t-1,y),corners::at(labels,t,y)));
}

proof fn empty_row(labels: Seq<u16>,y: int)
    requires y<0 || y>=64,
    ensures rows_end(labels,y)==0,
{
    corners::counting::sum_equal(65,|t: int|row_event(labels,y,t),|_: int|0);
    corners::counting::sum_constant(65,0);
}

pub(crate) proof fn row_endpoint_count(labels: Seq<u16>)
    ensures sum(4225,|v: int|vertex_row_ends(labels,v))==4*total_runs(labels),
{
    let f=|x: int,y: int|row_event(labels,y-1,x)+row_event(labels,y,x);
    corners::counting::grid_sum(65,65,f);
    corners::counting::sum_equal(4225,|v: int|f(v%65,v/65),|v: int|vertex_row_ends(labels,v));
    let g=|y: int|sum(65,|x: int|f(x,y));
    let h=|y: int|rows_end(labels,y-1)+rows_end(labels,y);
    assert forall|y: int| 0<=y<65 implies #[trigger] g(y)==#[trigger] h(y) by {
        let a=|x: int|row_event(labels,y-1,x);let b=|x: int|row_event(labels,y,x);
        corners::counting::sum_add(65,a,b);
        corners::counting::sum_equal(65,|x: int|a(x)+b(x),|x: int|f(x,y));
    }
    corners::counting::sum_equal(65,g,h);
    let r=|y: int|rows_end(labels,y);
    let prev=|y: int|r(y-1);
    corners::counting::sum_add(65,prev,r);
    corners::counting::sum_equal(65,|y: int|prev(y)+r(y),h);
    corners::counting::sum_shift(65,r);
    empty_row(labels,-1);empty_row(labels,64);
    assert(sum(65,r)==sum(64,r));
    let starts=|y: int|rows_begin(labels,y);
    assert forall|y: int| 0<=y<64 implies #[trigger] r(y)==2*#[trigger] starts(y) by {row_ends_twice(labels,y);}
    corners::counting::sum_equal(64,r,|y: int|2*starts(y));
    corners::counting::sum_scale(64,2,starts);
}

pub(crate) proof fn signed_count(labels: Seq<u16>)
    ensures sum(4225,|v: int|vertex_signed(labels,v))==2*good_events(labels)-2*identical(labels),
{
    let f=|t: int,y: int|chain_marks::signed(source(labels,y),marks(labels,y),t);
    corners::counting::grid_sum(65,65,f);
    corners::counting::sum_equal(4225,|v: int|f(v%65,v/65),|v: int|vertex_signed(labels,v));
    let g=|y: int|sum(65,|t: int|f(t,y));
    let h=|y: int|signed_line(labels,y);
    assert forall|y: int| 0<=y<65 implies #[trigger] g(y)==#[trigger] h(y) by {
        corners::counting::sum_equal(65,|t: int|f(t,y),|t: int|chain_marks::signed(source(labels,y),marks(labels,y),t));
    }
    corners::counting::sum_equal(65,g,h);
    assert forall|y: int| 0<=y<65 implies #[trigger] signed_line(labels,y)
        ==2*good_line(labels,y)-2*identical_line(labels,y) by {
        chain_marks::signed_chain_identity(source(labels,y),marks(labels,y));
    }
    corners::counting::sum_double_difference(65,h,|y: int|good_line(labels,y),|y: int|identical_line(labels,y));
}

/// 原图角点预算：与孔数及连通分量无关，good_events 后续连接完整 H 集合。
pub(crate) proof fn base_run_count(labels: Seq<u16>)
    ensures colored::b4(labels)==4*(total_runs(labels)-identical(labels)+good_events(labels)),
{
    let src=|s: int|colored::site_weight(labels,s);
    corners::counting::sum_four_groups(4225,src);
    let w=|v: int|sum(4,|q: int|src(4*v+q));
    let c=|v: int|vertex_row_ends(labels,v);let s=|v: int|vertex_signed(labels,v);
    assert forall|v: int| 0<=v<4225 implies #[trigger] w(v)==#[trigger] c(v)+2*#[trigger] s(v) by {
        vertex_run_identity(labels,v);
        corners::counting::sum_equal(4,|q: int|src(4*v+q),|q: int|colored::site_weight(labels,4*v+q));
    }
    corners::counting::sum_two_identity(4225,w,c,s);
    row_endpoint_count(labels);signed_count(labels);
}

}
