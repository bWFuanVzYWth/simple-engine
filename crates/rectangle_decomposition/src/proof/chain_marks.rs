//! 内部直链的标记端点预算；一条链至多消耗一个未配成 good chord 的凹角。

use super::corners::counting::{bit, sum};
use super::{Rectangle, chains, colored, corners, good, partition};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn start_at(f: spec_fn(int)->u16,t: int) -> int
    decreases if t>0 {t}else{0},
{
    if t<=0 || f(t)!=f(t-1) {t} else {start_at(f,t-1)}
}

pub(crate) proof fn start_prefix(f: spec_fn(int)->u16,t: int)
    requires 0<=t<64,f(t)!=0,
    ensures 0<=start_at(f,t)<=t,
        forall|u: int| start_at(f,t)<=u<=t ==> #[trigger] f(u)==f(t),
        start_at(f,t)==0 || f(start_at(f,t)-1)!=f(t),
    decreases t,
{
    if t>0 && f(t)==f(t-1) {start_prefix(f,t-1);}
}

pub(crate) open spec fn credit(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,t: int) -> int {
    if 0<=t<64 && f(t)!=0 {bit(!r(start_at(f,t),f(t)))} else {0}
}

pub(crate) open spec fn marked(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,t: int) -> int {
    chains::begin(f(t-1),f(t))*bit(r(t,f(t)))+chains::end(f(t-1),f(t))*bit(r(t,f(t-1)))
}

pub(crate) open spec fn paired(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,t: int) -> int {
    chains::end(f(t-1),f(t))*bit(r(t,f(t-1)) && r(start_at(f,t-1),f(t-1)))
}

pub(crate) proof fn mark_step(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,t: int)
    requires f(-1)==0,f(64)==0,0<=t<=64,
    ensures marked(f,r,t)<=chains::begin(f(t-1),f(t))+paired(f,r,t)+credit(f,r,t-1)-credit(f,r,t),
{
    if t==0 {} else if t==64 {} else {}
    if f(t)==0 {} else if f(t-1)==0 {} else if f(t)==f(t-1) {} else {}
}

/// 累计每条直链的两端：若两端均标记，额外计一条 paired 链。
pub(crate) proof fn marked_chain_bound(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool)
    requires f(-1)==0,f(64)==0,
    ensures sum(65,|t: int|marked(f,r,t))<=sum(65,|t: int|chains::begin(f(t-1),f(t)))
        +sum(65,|t: int|paired(f,r,t)),
{
    let w=|t: int|marked(f,r,t);
    let b=|t: int|chains::begin(f(t-1),f(t));
    let g=|t: int|paired(f,r,t);
    let p=|t: int|credit(f,r,t);
    assert forall|t: int| 0<=t<65 implies #[trigger] w(t)<=#[trigger] b(t)+#[trigger] g(t)
        +#[trigger] p(t-1)-#[trigger] p(t) by {mark_step(f,r,t);}
    super::corners::counting::sum_credit_bound(65,w,b,g,p);
}

pub(crate) open spec fn source(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool,line: int)
    -> spec_fn(int)->u16 { |t: int|chains::line_value(labels,rectangles,vertical,line,t) }

pub(crate) open spec fn mark_source(labels: Seq<u16>,vertical: bool,line: int)
    -> spec_fn(int,u16)->bool { |t: int,value: u16|good::reflex(labels,vertical,line,t,value) }

pub(crate) open spec fn paired_line(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool,line: int) -> int {
    sum(65,|t: int|paired(source(labels,rectangles,vertical,line),mark_source(labels,vertical,line),t))
}

pub(crate) proof fn line_unit(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool,line: int,t: int,value: u16)
    requires 0<=line<=64,0<=t<64,value!=0,chains::line_value(labels,rectangles,vertical,line,t)==value,
    ensures good::unit_inside(labels,vertical,line,t,value),
{}

pub(crate) proof fn reflex_interior(labels: Seq<u16>,vertical: bool,line: int,t: int,value: u16)
    requires good::reflex(labels,vertical,line,t,value),
    ensures 1<=line<=63,1<=t<=63,
{
    if vertical {
        if line<=0 {} else if line>=64 {} if t<=0 {} else if t>=64 {}
    } else {
        if line<=0 {} else if line>=64 {} if t<=0 {} else if t>=64 {}
    }
}

/// paired 事件对应真实像素 good chord，而非任意两端点之间的子段。
pub(crate) proof fn paired_good(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool,line: int,t: int)
    requires 0<=line<=64,0<=t<=64,
        paired(source(labels,rectangles,vertical,line),mark_source(labels,vertical,line),t)==1,
    ensures good::chord(labels,vertical,line,start_at(source(labels,rectangles,vertical,line),t-1),t,
        chains::line_value(labels,rectangles,vertical,line,t-1)),
{
    let f=source(labels,rectangles,vertical,line);
    let r=mark_source(labels,vertical,line);
    let value=f(t-1);
    let s=start_at(f,t-1);
    assert(f(-1)==0 && f(64)==0);
    assert(t>0 && value!=0);
    start_prefix(f,t-1);
    reflex_interior(labels,vertical,line,t,value);
    reflex_interior(labels,vertical,line,s,value);
    assert forall|u: int| s<=u<t implies #[trigger] good::unit_inside(labels,vertical,line,u,value) by {
        assert(f(u)==value);
        line_unit(labels,rectangles,vertical,line,u,value);
    }
}

pub(crate) open spec fn diagram_reflex(la: u16,lb: u16,lc: u16,ld: u16,value: u16) -> int {
    bit(value!=0 && corners::count(la==value,lb==value,lc==value,ld==value)==3)
}

pub(crate) open spec fn color_marked(a: int,b: int,c: int,d: int,
    la: u16,lb: u16,lc: u16,ld: u16,q: int) -> int {
    let value=colored::pick(la,lb,lc,ld,q);
    if diagram_reflex(la,lb,lc,ld,value)==1 {chains::color_ends(a,b,c,d,la,lb,lc,ld,q)} else {0}
}

pub(crate) proof fn diagram_marks(a: int,b: int,c: int,d: int,la: u16,lb: u16,lc: u16,ld: u16)
    requires colored::consistent(a,b,c,d,la,lb,lc,ld),
    ensures color_marked(a,b,c,d,la,lb,lc,ld,0)+color_marked(a,b,c,d,la,lb,lc,ld,1)
        +color_marked(a,b,c,d,la,lb,lc,ld,2)+color_marked(a,b,c,d,la,lb,lc,ld,3)
        ==chains::begin(chains::cut_value(a,b,la,lb),chains::cut_value(d,c,ld,lc))
            *diagram_reflex(la,lb,lc,ld,chains::cut_value(d,c,ld,lc))
        +chains::end(chains::cut_value(a,b,la,lb),chains::cut_value(d,c,ld,lc))
            *diagram_reflex(la,lb,lc,ld,chains::cut_value(a,b,la,lb))
        +chains::begin(chains::cut_value(a,d,la,ld),chains::cut_value(b,c,lb,lc))
            *diagram_reflex(la,lb,lc,ld,chains::cut_value(b,c,lb,lc))
        +chains::end(chains::cut_value(a,d,la,ld),chains::cut_value(b,c,lb,lc))
            *diagram_reflex(la,lb,lc,ld,chains::cut_value(a,d,la,ld)),
{
    if la==0 {} if lb==0 {} if lc==0 {} if ld==0 {}
    if la==lb {} if la==lc {} if la==ld {} if lb==lc {} if lb==ld {} if lc==ld {}
}

pub(crate) open spec fn marked_line(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool,line: int) -> int {
    sum(65,|t: int|marked(source(labels,rectangles,vertical,line),mark_source(labels,vertical,line),t))
}

pub(crate) open spec fn vertex_marked(labels: Seq<u16>,rectangles: Seq<Rectangle>,v: int) -> int {
    marked(source(labels,rectangles,true,v%65),mark_source(labels,true,v%65),v/65)
        +marked(source(labels,rectangles,false,v/65),mark_source(labels,false,v/65),v%65)
}

pub(crate) proof fn vertex_marks(labels: Seq<u16>,rectangles: Seq<Rectangle>,v: int)
    requires partition(labels,rectangles),0<=v<4225,
    ensures sum(4,|q: int|colored::site_marked(labels,rectangles,4*v+q))==vertex_marked(labels,rectangles,v),
{
    colored::actual_consistent(labels,rectangles,v);
    chains::vertex_end_values(labels,rectangles,v);
    diagram_marks(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,1),
        corners::oq(rectangles,v%65,v/65,2),corners::oq(rectangles,v%65,v/65,3),
        colored::label(labels,v,0),colored::label(labels,v,1),colored::label(labels,v,2),colored::label(labels,v,3));
    assert forall|q: int| 0<=q<4 implies #[trigger] colored::site_marked(labels,rectangles,4*v+q)
        ==color_marked(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,1),
        corners::oq(rectangles,v%65,v/65,2),corners::oq(rectangles,v%65,v/65,3),
        colored::label(labels,v,0),colored::label(labels,v,1),colored::label(labels,v,2),colored::label(labels,v,3),q) by {
        assert((4*v+q)/4==v && (4*v+q)%4==q) by(nonlinear_arith) requires 0<=q<4;
        if q==0 {} else if q==1 {} else if q==2 {} else {}
    }
    reveal_with_fuel(sum,5);
}

pub(crate) open spec fn total_paired(labels: Seq<u16>,rectangles: Seq<Rectangle>) -> int {
    sum(65,|line: int|paired_line(labels,rectangles,true,line))
        +sum(65,|line: int|paired_line(labels,rectangles,false,line))
}

pub(crate) proof fn global_mark_count(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    requires partition(labels,rectangles),
    ensures colored::reflexes(labels)+colored::reflex_twice(labels,rectangles)
        ==sum(65,|line: int|marked_line(labels,rectangles,true,line))
            +sum(65,|line: int|marked_line(labels,rectangles,false,line)),
{
    let src=|s: int|colored::site_marked(labels,rectangles,s);
    corners::counting::sum_four_groups(4225,src);
    let g=|v: int|sum(4,|q: int|src(4*v+q));
    let h=|v: int|vertex_marked(labels,rectangles,v);
    assert forall|v: int| 0<=v<4225 implies #[trigger] g(v)==#[trigger] h(v) by {
        vertex_marks(labels,rectangles,v);
        corners::counting::sum_equal(4,|q: int|src(4*v+q),
            |q: int|colored::site_marked(labels,rectangles,4*v+q));
    }
    corners::counting::sum_equal(4225,g,h);
    colored::marked_reflex_count(labels,rectangles);
    let vpart=|line: int,t: int|marked(source(labels,rectangles,true,line),mark_source(labels,true,line),t);
    let hpart=|line: int,t: int|marked(source(labels,rectangles,false,line),mark_source(labels,false,line),t);
    corners::counting::opposite_grid(65,vpart,hpart);
    corners::counting::sum_equal(4225,h,|v: int|vpart(v%65,v/65)+hpart(v/65,v%65));
    let a=|line: int|sum(65,|t: int|vpart(line,t));
    let b=|line: int|marked_line(labels,rectangles,true,line);
    let c=|line: int|sum(65,|t: int|hpart(line,t));
    let d=|line: int|marked_line(labels,rectangles,false,line);
    assert forall|line: int| 0<=line<65 implies #[trigger] a(line)==#[trigger] b(line) by {
        corners::counting::sum_equal(65,|t: int|vpart(line,t),
            |t: int|marked(source(labels,rectangles,true,line),mark_source(labels,true,line),t));
    }
    assert forall|line: int| 0<=line<65 implies #[trigger] c(line)==#[trigger] d(line) by {
        corners::counting::sum_equal(65,|t: int|hpart(line,t),
            |t: int|marked(source(labels,rectangles,false,line),mark_source(labels,false,line),t));
    }
    corners::counting::sum_equal(65,a,b);
    corners::counting::sum_equal(65,c,d);
}

pub(crate) proof fn axis_mark_bound(labels: Seq<u16>,rectangles: Seq<Rectangle>,vertical: bool)
    ensures sum(65,|line: int|marked_line(labels,rectangles,vertical,line))
        <=chains::axis_chains(labels,rectangles,vertical)+sum(65,|line: int|paired_line(labels,rectangles,vertical,line)),
{
    assert forall|line: int| 0<=line<65 implies #[trigger] marked_line(labels,rectangles,vertical,line)
        <=chains::line_chains(labels,rectangles,vertical,line)+paired_line(labels,rectangles,vertical,line) by {
        let f=source(labels,rectangles,vertical,line);
        let r=mark_source(labels,vertical,line);
        marked_chain_bound(f,r);
        corners::counting::sum_equal(65,|t: int|chains::begin(f(t-1),f(t)),
            |t: int|chains::line_begin(labels,rectangles,vertical,line,t));
    }
    let a=|line: int|marked_line(labels,rectangles,vertical,line);
    let b=|line: int|chains::line_chains(labels,rectangles,vertical,line);
    let c=|line: int|paired_line(labels,rectangles,vertical,line);
    corners::counting::sum_order(65,a,|line: int|b(line)+c(line));
    corners::counting::sum_add(65,b,c);
}

/// 任意真实分区的 good 直链足以吸收凹角预算，尚待冲突删点构造 J。
pub(crate) proof fn global_paired_bound(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    requires partition(labels,rectangles),
    ensures colored::reflexes(labels)+colored::reflex_twice(labels,rectangles)
        <=chains::chain_count(labels,rectangles)+total_paired(labels,rectangles),
{
    global_mark_count(labels,rectangles);
    axis_mark_bound(labels,rectangles,true);
    axis_mark_bound(labels,rectangles,false);
}

pub(crate) open spec fn both_unmarked(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,t: int) -> int {
    chains::end(f(t-1),f(t))*bit(!r(t,f(t-1)) && !r(start_at(f,t-1),f(t-1)))
}

pub(crate) open spec fn signed(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,t: int) -> int {
    chains::begin(f(t-1),f(t))*(2*bit(r(t,f(t)))-1)
        +chains::end(f(t-1),f(t))*(2*bit(r(t,f(t-1)))-1)
}

pub(crate) open spec fn signed_credit(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,t: int) -> int {
    if 0<=t<64 && f(t)!=0 {2*bit(r(start_at(f,t),f(t)))-1} else {0}
}

pub(crate) proof fn signed_step(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,t: int)
    requires f(-1)==0,f(64)==0,0<=t<=64,
    ensures signed(f,r,t)==2*paired(f,r,t)-2*both_unmarked(f,r,t)
        +signed_credit(f,r,t)-signed_credit(f,r,t-1),
{
    if t==0 {} else if t==64 {} else {}
    if f(t)==0 {} else if f(t-1)==0 {} else if f(t)==f(t-1) {} else {}
}

/// 一对端点的符号和仅在两端同为凹角/同为直边时贡献 ±2。
pub(crate) proof fn signed_chain_identity(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool)
    requires f(-1)==0,f(64)==0,
    ensures sum(65,|t: int|signed(f,r,t))==2*sum(65,|t: int|paired(f,r,t))
        -2*sum(65,|t: int|both_unmarked(f,r,t)),
{
    let w=|t: int|signed(f,r,t);
    let g=|t: int|paired(f,r,t);
    let b=|t: int|both_unmarked(f,r,t);
    let p=|t: int|signed_credit(f,r,t);
    assert forall|t: int| 0<=t<65 implies #[trigger] w(t)==2*#[trigger] g(t)-2*#[trigger] b(t)
        +#[trigger] p(t)-#[trigger] p(t-1) by {signed_step(f,r,t);}
    corners::counting::sum_signed_pair(65,w,g,b,p);
}

}
