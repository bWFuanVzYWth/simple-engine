//! 同标签内部 cut 的最大直链及端点计数；不建立执行期 owner 图。

use super::{Rectangle, colored, corners, partition};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn cut_value(a: int,b: int,la: u16,lb: u16) -> u16 {
    if la!=0 && la==lb && a!=b { la } else { 0 }
}

/// vertical 时固定 x，其余时固定 y；位置 t 表示一条单位边。
pub(crate) open spec fn line_value(labels: Seq<u16>, rectangles: Seq<Rectangle>,
    vertical: bool, line: int, t: int) -> u16 {
    if 0<=line<=64 && 0<=t<64 {
        let x=if vertical {line} else {t};
        let y=if vertical {t} else {line};
        let x0=if vertical {x-1} else {x};
        let y0=if vertical {y} else {y-1};
        cut_value(corners::owner(rectangles,x0,y0),corners::owner(rectangles,x,y),
            corners::at(labels,x0,y0),corners::at(labels,x,y))
    } else {0}
}

pub(crate) open spec fn begin(a: u16,b: u16) -> int { bit(b!=0 && a!=b) }
pub(crate) open spec fn end(a: u16,b: u16) -> int { bit(a!=0 && a!=b) }

pub(crate) proof fn transition(a: u16,b: u16)
    ensures begin(a,b)-end(a,b)==bit(b!=0)-bit(a!=0),
{}

pub(crate) open spec fn line_begin(labels: Seq<u16>, rectangles: Seq<Rectangle>,
    vertical: bool,line: int,t: int) -> int {
    begin(line_value(labels,rectangles,vertical,line,t-1),line_value(labels,rectangles,vertical,line,t))
}

pub(crate) open spec fn line_end(labels: Seq<u16>, rectangles: Seq<Rectangle>,
    vertical: bool,line: int,t: int) -> int {
    end(line_value(labels,rectangles,vertical,line,t-1),line_value(labels,rectangles,vertical,line,t))
}

pub(crate) open spec fn line_chains(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool,line: int) -> int {
    sum(65,|t: int|line_begin(labels,rectangles,vertical,line,t))
}

pub(crate) open spec fn line_ends(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool,line: int) -> int {
    sum(65,|t: int|line_begin(labels,rectangles,vertical,line,t)+line_end(labels,rectangles,vertical,line,t))
}

pub(crate) proof fn line_chain_ends(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool,line: int)
    ensures line_ends(labels,rectangles,vertical,line)==2*line_chains(labels,rectangles,vertical,line),
{
    let b=|t: int|line_begin(labels,rectangles,vertical,line,t);
    let e=|t: int|line_end(labels,rectangles,vertical,line,t);
    let occ=|t: int|bit(line_value(labels,rectangles,vertical,line,t)!=0);
    let diff=|t: int|occ(t)-occ(t-1);
    assert forall|t: int| 0<=t<65 implies #[trigger] b(t)-#[trigger] e(t)==#[trigger] diff(t) by {
        transition(line_value(labels,rectangles,vertical,line,t-1),line_value(labels,rectangles,vertical,line,t));
    }
    let d=|t: int|b(t)-e(t);
    corners::counting::sum_equal(65,d,diff);
    corners::counting::sum_add(65,d,e);
    corners::counting::telescope(65,occ);
    corners::counting::sum_equal(65,|t: int|d(t)+e(t),b);
    corners::counting::sum_add(65,b,e);
    corners::counting::sum_equal(65,|t: int|b(t)+e(t),
        |t: int|line_begin(labels,rectangles,vertical,line,t)+line_end(labels,rectangles,vertical,line,t));
}

pub(crate) open spec fn color_ends(a: int,b: int,c: int,d: int,
    la: u16,lb: u16,lc: u16,ld: u16,q: int) -> int {
    let target=colored::pick(la,lb,lc,ld,q);
    if colored::canonical(la,lb,lc,ld,q) {
        corners::endpoints(colored::mask(a,la,target),colored::mask(b,lb,target),
            colored::mask(c,lc,target),colored::mask(d,ld,target))
    } else {0}
}

pub(crate) proof fn color_end_split(a: int,b: int,c: int,d: int,
    la: u16,lb: u16,lc: u16,ld: u16)
    requires colored::consistent(a,b,c,d,la,lb,lc,ld),
    ensures color_ends(a,b,c,d,la,lb,lc,ld,0)+color_ends(a,b,c,d,la,lb,lc,ld,1)
        +color_ends(a,b,c,d,la,lb,lc,ld,2)+color_ends(a,b,c,d,la,lb,lc,ld,3)
        == begin(cut_value(a,b,la,lb),cut_value(d,c,ld,lc))
            +end(cut_value(a,b,la,lb),cut_value(d,c,ld,lc))
            +begin(cut_value(a,d,la,ld),cut_value(b,c,lb,lc))
            +end(cut_value(a,d,la,ld),cut_value(b,c,lb,lc)),
{
    if la==0 {} if lb==0 {} if lc==0 {} if ld==0 {}
    if la==lb {} if la==lc {} if la==ld {} if lb==lc {} if lb==ld {} if lc==ld {}
}

pub(crate) open spec fn vertex_ends(labels: Seq<u16>,rectangles: Seq<Rectangle>,v: int) -> int {
    line_begin(labels,rectangles,true,v%65,v/65)+line_end(labels,rectangles,true,v%65,v/65)
        +line_begin(labels,rectangles,false,v/65,v%65)+line_end(labels,rectangles,false,v/65,v%65)
}

pub(crate) proof fn vertex_end_values(labels: Seq<u16>,rectangles: Seq<Rectangle>,v: int)
    requires 0<=v<4225,
    ensures line_value(labels,rectangles,true,v%65,v/65-1)
        ==cut_value(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,1),
            colored::label(labels,v,0),colored::label(labels,v,1)),
        line_value(labels,rectangles,true,v%65,v/65)
        ==cut_value(corners::oq(rectangles,v%65,v/65,3),corners::oq(rectangles,v%65,v/65,2),
            colored::label(labels,v,3),colored::label(labels,v,2)),
        line_value(labels,rectangles,false,v/65,v%65-1)
        ==cut_value(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,3),
            colored::label(labels,v,0),colored::label(labels,v,3)),
        line_value(labels,rectangles,false,v/65,v%65)
        ==cut_value(corners::oq(rectangles,v%65,v/65,1),corners::oq(rectangles,v%65,v/65,2),
            colored::label(labels,v,1),colored::label(labels,v,2)),
{}

pub(crate) proof fn vertex_colored_ends(labels: Seq<u16>,rectangles: Seq<Rectangle>,v: int)
    requires partition(labels,rectangles),0<=v<4225,
    ensures sum(4,|q: int|colored::site_endpoints(labels,rectangles,4*v+q))==vertex_ends(labels,rectangles,v),
{
    colored::actual_consistent(labels,rectangles,v);
    vertex_end_values(labels,rectangles,v);
    color_end_split(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,1),
        corners::oq(rectangles,v%65,v/65,2),corners::oq(rectangles,v%65,v/65,3),
        colored::label(labels,v,0),colored::label(labels,v,1),colored::label(labels,v,2),colored::label(labels,v,3));
    assert forall|q: int| 0<=q<4 implies #[trigger] colored::site_endpoints(labels,rectangles,4*v+q)
        ==color_ends(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,1),
        corners::oq(rectangles,v%65,v/65,2),corners::oq(rectangles,v%65,v/65,3),
        colored::label(labels,v,0),colored::label(labels,v,1),colored::label(labels,v,2),colored::label(labels,v,3),q) by {
        assert((4*v+q)/4==v && (4*v+q)%4==q) by(nonlinear_arith) requires 0<=q<4;
        if q==0 {} else if q==1 {} else if q==2 {} else {}
    }
    reveal_with_fuel(sum,5);
}

pub(crate) open spec fn axis_ends(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool) -> int {
    sum(65,|line: int|line_ends(labels,rectangles,vertical,line))
}

pub(crate) open spec fn axis_chains(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool) -> int {
    sum(65,|line: int|line_chains(labels,rectangles,vertical,line))
}

pub(crate) open spec fn chain_count(labels: Seq<u16>,rectangles: Seq<Rectangle>) -> int {
    axis_chains(labels,rectangles,true)+axis_chains(labels,rectangles,false)
}

pub(crate) proof fn axes_ends(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool)
    ensures axis_ends(labels,rectangles,vertical)==2*axis_chains(labels,rectangles,vertical),
{
    assert forall|line: int| 0<=line<65 implies #[trigger] line_ends(labels,rectangles,vertical,line)
        ==2*line_chains(labels,rectangles,vertical,line) by {
        line_chain_ends(labels,rectangles,vertical,line);
    }
    corners::counting::sum_equal(65,|line: int|line_ends(labels,rectangles,vertical,line),
        |line: int|2*line_chains(labels,rectangles,vertical,line));
    let f=|line: int|line_chains(labels,rectangles,vertical,line);
    corners::counting::sum_scale(65,2,f);
    corners::counting::sum_equal(65,|line: int|2*f(line),
        |line: int|2*line_chains(labels,rectangles,vertical,line));
}

pub(crate) proof fn grid_ends(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    ensures sum(4225,|v: int|vertex_ends(labels,rectangles,v))
        ==axis_ends(labels,rectangles,true)+axis_ends(labels,rectangles,false),
{
    let vpart=|x: int,y: int|line_begin(labels,rectangles,true,x,y)+line_end(labels,rectangles,true,x,y);
    let hpart=|x: int,y: int|line_begin(labels,rectangles,false,y,x)+line_end(labels,rectangles,false,y,x);
    corners::counting::grid_sum(65,65,vpart);
    corners::counting::grid_sum(65,65,hpart);
    corners::counting::sum_swap(65,65,vpart);
    let vsource=|v: int|vpart(v%65,v/65);
    let hsource=|v: int|hpart(v%65,v/65);
    corners::counting::sum_add(4225,vsource,hsource);
    corners::counting::sum_equal(4225,|v: int|vsource(v)+hsource(v),
        |v: int|vertex_ends(labels,rectangles,v));
    assert forall|line: int| 0<=line<65 implies #[trigger] line_ends(labels,rectangles,true,line)
        ==sum(65,|t: int|vpart(line,t)) by {
        corners::counting::sum_equal(65,|t: int|vpart(line,t),
            |t: int|line_begin(labels,rectangles,true,line,t)+line_end(labels,rectangles,true,line,t));
    }
    assert forall|line: int| 0<=line<65 implies #[trigger] line_ends(labels,rectangles,false,line)
        ==sum(65,|t: int|hpart(t,line)) by {
        corners::counting::sum_equal(65,|t: int|hpart(t,line),
            |t: int|line_begin(labels,rectangles,false,line,t)+line_end(labels,rectangles,false,line,t));
    }
    corners::counting::sum_equal(65,|line: int|line_ends(labels,rectangles,true,line),
        |line: int|sum(65,|t: int|vpart(line,t)));
    corners::counting::sum_equal(65,|line: int|line_ends(labels,rectangles,false,line),
        |line: int|sum(65,|t: int|hpart(t,line)));
}

/// 每条最大同色内部直链恰有两个端点，包括 T 的终止臂及换色位置。
pub(crate) proof fn global_chain_ends(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    requires partition(labels,rectangles),
    ensures colored::ends(labels,rectangles)==2*chain_count(labels,rectangles),
{
    let source=|s: int|colored::site_endpoints(labels,rectangles,s);
    corners::counting::sum_four_groups(4225,source);
    let grouped=|v: int|sum(4,|q: int|source(4*v+q));
    let ends=|v: int|vertex_ends(labels,rectangles,v);
    assert forall|v: int| 0<=v<4225 implies #[trigger] grouped(v)==#[trigger] ends(v) by {
        vertex_colored_ends(labels,rectangles,v);
        corners::counting::sum_equal(4,|q: int|source(4*v+q),
            |q: int|colored::site_endpoints(labels,rectangles,4*v+q));
    }
    corners::counting::sum_equal(4225,grouped,ends);
    grid_ends(labels,rectangles);
    axes_ends(labels,rectangles,true);
    axes_ends(labels,rectangles,false);
}

/// 所有竞争分区的角点预算；最大性下界仍需由 good 链构造独立集。
pub(crate) proof fn partition_budget(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    requires partition(labels,rectangles),
    ensures colored::b4(labels)==4*rectangles.len()+4*colored::reflexes(labels)
        -4*chain_count(labels,rectangles)-4*colored::crosses(labels,rectangles),
{
    colored::global_corner_identity(labels,rectangles);
    global_chain_ends(labels,rectangles);
}

}
