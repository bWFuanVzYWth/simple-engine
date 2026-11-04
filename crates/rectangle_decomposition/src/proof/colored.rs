//! 有限标签站点：每个格点只取四邻元中每个非零标签的首次出现。

use super::{Rectangle, corners, partition};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn label(labels: Seq<u16>, v: int, q: int) -> u16 {
    corners::at(labels,corners::qx(v%65,q),corners::qy(v/65,q))
}

pub(crate) open spec fn canonical(a: u16, b: u16, c: u16, d: u16, q: int) -> bool {
    if q==0 { a!=0 }
    else if q==1 { b!=0 && b!=a }
    else if q==2 { c!=0 && c!=a && c!=b }
    else { d!=0 && d!=a && d!=b && d!=c }
}

pub(crate) open spec fn pick(a: u16, b: u16, c: u16, d: u16, q: int) -> u16 {
    if q==0 { a } else if q==1 { b } else if q==2 { c } else { d }
}

pub(crate) open spec fn mask(owner: int, value: u16, target: u16) -> int {
    if value==target && target!=0 { owner } else { -1 }
}

pub(crate) open spec fn consistent(a: int, b: int, c: int, d: int,
    la: u16, lb: u16, lc: u16, ld: u16) -> bool {
    a>=-1 && b>=-1 && c>=-1 && d>=-1
        && (a>=0 <==> la!=0) && (b>=0 <==> lb!=0) && (c>=0 <==> lc!=0) && (d>=0 <==> ld!=0)
        && (a>=0 && a==b ==> la==lb) && (a>=0 && a==c ==> la==lc)
        && (a>=0 && a==d ==> la==ld) && (b>=0 && b==c ==> lb==lc)
        && (b>=0 && b==d ==> lb==ld) && (c>=0 && c==d ==> lc==ld)
}

pub(crate) proof fn masked_legal(a: int, b: int, c: int, d: int,
    la: u16, lb: u16, lc: u16, ld: u16, target: u16)
    requires corners::legal(a,b,c,d), consistent(a,b,c,d,la,lb,lc,ld),
    ensures corners::legal(mask(a,la,target),mask(b,lb,target),mask(c,lc,target),mask(d,ld,target)),
{}

pub(crate) open spec fn colored_corners(a: int, b: int, c: int, d: int,
    la: u16, lb: u16, lc: u16, ld: u16, q: int) -> int {
    let target=pick(la,lb,lc,ld,q);
    if canonical(la,lb,lc,ld,q) {
        corners::local_corners(mask(a,la,target),mask(b,lb,target),mask(c,lc,target),mask(d,ld,target))
    } else { 0 }
}

/// 按标签分开点接触不会丢失或重复任何真实矩形角。
pub(crate) proof fn color_corner_split(a: int, b: int, c: int, d: int,
    la: u16, lb: u16, lc: u16, ld: u16)
    requires consistent(a,b,c,d,la,lb,lc,ld),
    ensures corners::local_corners(a,b,c,d) == colored_corners(a,b,c,d,la,lb,lc,ld,0)
        +colored_corners(a,b,c,d,la,lb,lc,ld,1)+colored_corners(a,b,c,d,la,lb,lc,ld,2)
        +colored_corners(a,b,c,d,la,lb,lc,ld,3),
{
    if la==0 {} if lb==0 {} if lc==0 {} if ld==0 {}
    if la==lb {} if la==lc {} if la==ld {} if lb==lc {} if lb==ld {} if lc==ld {}
}

pub(crate) proof fn actual_consistent(labels: Seq<u16>, rectangles: Seq<Rectangle>, v: int)
    requires partition(labels,rectangles), 0<=v<4225,
    ensures consistent(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,1),
        corners::oq(rectangles,v%65,v/65,2),corners::oq(rectangles,v%65,v/65,3),
        label(labels,v,0),label(labels,v,1),label(labels,v,2),label(labels,v,3)),
{
    corners::owner_exact(labels,rectangles,v%65-1,v/65-1);
    corners::owner_exact(labels,rectangles,v%65,v/65-1);
    corners::owner_exact(labels,rectangles,v%65,v/65);
    corners::owner_exact(labels,rectangles,v%65-1,v/65);
}

pub(crate) open spec fn site(labels: Seq<u16>, s: int) -> bool {
    let v=s/4;
    canonical(label(labels,v,0),label(labels,v,1),label(labels,v,2),label(labels,v,3),s%4)
}

pub(crate) open spec fn first(labels: Seq<u16>,v: int,value: u16) -> int {
    if label(labels,v,0)==value {0} else if label(labels,v,1)==value {1}
    else if label(labels,v,2)==value {2} else {3}
}

pub(crate) open spec fn site_for(labels: Seq<u16>,x: int,y: int,value: u16) -> int {
    4*(y*65+x)+first(labels,y*65+x,value)
}

pub(crate) proof fn canonical_site(labels: Seq<u16>,x: int,y: int,value: u16)
    requires 0<=x<=64,0<=y<=64,value!=0,
        corners::count(corners::at(labels,x-1,y-1)==value,corners::at(labels,x,y-1)==value,
            corners::at(labels,x,y)==value,corners::at(labels,x-1,y)==value)>=1,
    ensures 0<=site_for(labels,x,y,value)<16900,site(labels,site_for(labels,x,y,value)),
        site_for(labels,x,y,value)/4==y*65+x,
        label(labels,y*65+x,site_for(labels,x,y,value)%4)==value,
{
    let v=y*65+x;
    let q=first(labels,v,value);
    assert(v%65==x && v/65==y) by(nonlinear_arith) requires v==y*65+x,0<=x<65;
    assert((4*v+q)/4==v && (4*v+q)%4==q) by(nonlinear_arith) requires 0<=q<4;
    if label(labels,v,0)==value {} else if label(labels,v,1)==value {}
    else if label(labels,v,2)==value {} else {}
}

pub(crate) open spec fn site_owner(labels: Seq<u16>, rectangles: Seq<Rectangle>, s: int, q: int) -> int {
    mask(corners::oq(rectangles,s/4%65,s/4/65,q), label(labels,s/4,q),label(labels,s/4,s%4))
}

pub(crate) open spec fn site_count(labels: Seq<u16>, s: int) -> int {
    let v=s/4;
    let target=label(labels,v,s%4);
    corners::count(label(labels,v,0)==target, label(labels,v,1)==target,
        label(labels,v,2)==target,label(labels,v,3)==target)
}

pub(crate) open spec fn site_weight(labels: Seq<u16>, s: int) -> int {
    if site(labels,s) {
        let v=s/4;
        let target=label(labels,v,s%4);
        corners::weight(label(labels,v,0)==target,label(labels,v,1)==target,
            label(labels,v,2)==target,label(labels,v,3)==target)
    } else { 0 }
}

pub(crate) open spec fn site_corner(labels: Seq<u16>, rectangles: Seq<Rectangle>, s: int) -> int {
    if site(labels,s) { corners::local_corners(site_owner(labels,rectangles,s,0),
        site_owner(labels,rectangles,s,1),site_owner(labels,rectangles,s,2),site_owner(labels,rectangles,s,3)) }
    else { 0 }
}

pub(crate) open spec fn site_reflex(labels: Seq<u16>, s: int) -> int {
    bit(site(labels,s) && site_count(labels,s)==3)
}

pub(crate) open spec fn site_endpoints(labels: Seq<u16>, rectangles: Seq<Rectangle>, s: int) -> int {
    if site(labels,s) { corners::endpoints(site_owner(labels,rectangles,s,0),
        site_owner(labels,rectangles,s,1),site_owner(labels,rectangles,s,2),site_owner(labels,rectangles,s,3)) }
    else { 0 }
}

pub(crate) open spec fn site_cross(labels: Seq<u16>, rectangles: Seq<Rectangle>, s: int) -> int {
    bit(site(labels,s) && corners::cross(site_owner(labels,rectangles,s,0),
        site_owner(labels,rectangles,s,1),site_owner(labels,rectangles,s,2),site_owner(labels,rectangles,s,3)))
}

pub(crate) open spec fn site_reflex_twice(labels: Seq<u16>,rectangles: Seq<Rectangle>,s: int) -> int {
    bit(site(labels,s) && site_count(labels,s)==3 && site_endpoints(labels,rectangles,s)==2)
}

pub(crate) open spec fn reflex_twice(labels: Seq<u16>,rectangles: Seq<Rectangle>) -> int {
    sum(16900,|s: int|site_reflex_twice(labels,rectangles,s))
}

pub(crate) open spec fn site_marked(labels: Seq<u16>,rectangles: Seq<Rectangle>,s: int) -> int {
    if site_count(labels,s)==3 {site_endpoints(labels,rectangles,s)} else {0}
}

pub(crate) proof fn site_mark_count(labels: Seq<u16>,rectangles: Seq<Rectangle>,s: int)
    requires partition(labels,rectangles),0<=s<16900,
    ensures site_marked(labels,rectangles,s)==site_reflex(labels,s)+site_reflex_twice(labels,rectangles,s),
{
    if site(labels,s) && site_count(labels,s)==3 {
        let v=s/4;
        actual_consistent(labels,rectangles,v);
        corners::owners_legal(labels,rectangles,v%65,v/65);
        masked_legal(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,1),
            corners::oq(rectangles,v%65,v/65,2),corners::oq(rectangles,v%65,v/65,3),
            label(labels,v,0),label(labels,v,1),label(labels,v,2),label(labels,v,3),label(labels,v,s%4));
        let a=site_owner(labels,rectangles,s,0);let b=site_owner(labels,rectangles,s,1);
        let c=site_owner(labels,rectangles,s,2);let d=site_owner(labels,rectangles,s,3);
        assert(corners::count(a>=0,b>=0,c>=0,d>=0)==3);
        assert(1<=corners::endpoints(a,b,c,d)<=2) by {
            if a>=0 {} if b>=0 {} if c>=0 {} if d>=0 {}
        }
    }
}

pub(crate) proof fn marked_reflex_count(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    requires partition(labels,rectangles),
    ensures sum(16900,|s: int|site_marked(labels,rectangles,s))==reflexes(labels)+reflex_twice(labels,rectangles),
{
    assert forall|s: int| 0<=s<16900 implies #[trigger] site_marked(labels,rectangles,s)
        ==site_reflex(labels,s)+site_reflex_twice(labels,rectangles,s) by {site_mark_count(labels,rectangles,s);}
    corners::counting::sum_equal(16900,|s: int|site_marked(labels,rectangles,s),
        |s: int|site_reflex(labels,s)+site_reflex_twice(labels,rectangles,s));
    let r=|s: int|site_reflex(labels,s);
    let r2=|s: int|site_reflex_twice(labels,rectangles,s);
    corners::counting::sum_add(16900,r,r2);
    corners::counting::sum_equal(16900,|s: int|r(s)+r2(s),
        |s: int|site_reflex(labels,s)+site_reflex_twice(labels,rectangles,s));
}

pub(crate) proof fn site_identity(labels: Seq<u16>, rectangles: Seq<Rectangle>, s: int)
    requires partition(labels,rectangles), 0<=s<16900,
    ensures site_weight(labels,s)==site_corner(labels,rectangles,s)+4*site_reflex(labels,s)
        -2*site_endpoints(labels,rectangles,s)-4*site_cross(labels,rectangles,s),
{
    if site(labels,s) {
        let v=s/4;
        actual_consistent(labels,rectangles,v);
        corners::owners_legal(labels,rectangles,v%65,v/65);
        masked_legal(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,1),
            corners::oq(rectangles,v%65,v/65,2),corners::oq(rectangles,v%65,v/65,3),
            label(labels,v,0),label(labels,v,1),label(labels,v,2),label(labels,v,3),label(labels,v,s%4));
        corners::local_identity(site_owner(labels,rectangles,s,0),site_owner(labels,rectangles,s,1),
            site_owner(labels,rectangles,s,2),site_owner(labels,rectangles,s,3));
    }
}

pub(crate) open spec fn b4(labels: Seq<u16>) -> int { sum(16900, |s: int|site_weight(labels,s)) }
pub(crate) open spec fn reflexes(labels: Seq<u16>) -> int { sum(16900, |s: int|site_reflex(labels,s)) }
pub(crate) open spec fn ends(labels: Seq<u16>, rectangles: Seq<Rectangle>) -> int {
    sum(16900, |s: int|site_endpoints(labels,rectangles,s))
}
pub(crate) open spec fn crosses(labels: Seq<u16>, rectangles: Seq<Rectangle>) -> int {
    sum(16900, |s: int|site_cross(labels,rectangles,s))
}

pub(crate) proof fn vertex_sites(labels: Seq<u16>, rectangles: Seq<Rectangle>, v: int)
    requires partition(labels,rectangles), 0<=v<4225,
    ensures sum(4, |q: int|site_corner(labels,rectangles,4*v+q))==corners::vertex_corner(rectangles,v),
{
    actual_consistent(labels,rectangles,v);
    color_corner_split(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,1),
        corners::oq(rectangles,v%65,v/65,2),corners::oq(rectangles,v%65,v/65,3),
        label(labels,v,0),label(labels,v,1),label(labels,v,2),label(labels,v,3));
    assert forall|q: int| 0<=q<4 implies #[trigger] site_corner(labels,rectangles,4*v+q)
        ==colored_corners(corners::oq(rectangles,v%65,v/65,0),corners::oq(rectangles,v%65,v/65,1),
        corners::oq(rectangles,v%65,v/65,2),corners::oq(rectangles,v%65,v/65,3),
        label(labels,v,0),label(labels,v,1),label(labels,v,2),label(labels,v,3),q) by {
        assert((4*v+q)/4==v && (4*v+q)%4==q) by(nonlinear_arith) requires 0<=q<4;
        if q==0 {} else if q==1 {} else if q==2 {} else {}
    }
    reveal_with_fuel(sum,5);
}

/// 标签分离的有限角点计数仍等于真实竞争分区的四角总数。
pub(crate) proof fn site_corner_count(labels: Seq<u16>, rectangles: Seq<Rectangle>)
    requires partition(labels,rectangles),
    ensures sum(16900, |s: int|site_corner(labels,rectangles,s))==4*rectangles.len(),
{
    let source = |s: int|site_corner(labels,rectangles,s);
    corners::counting::sum_four_groups(4225,source);
    let f = |v: int|sum(4,|q: int|site_corner(labels,rectangles,4*v+q));
    let g = |v: int|corners::vertex_corner(rectangles,v);
    let grouped = |v: int|sum(4,|q: int|source(4*v+q));
    assert forall|v: int| 0<=v<4225 implies #[trigger] grouped(v)==#[trigger] f(v) by {
        corners::counting::sum_equal(4,|q: int|source(4*v+q),
            |q: int|site_corner(labels,rectangles,4*v+q));
    }
    corners::counting::sum_equal(4225,grouped,f);
    assert forall|v: int| 0<=v<4225 implies #[trigger] f(v)==#[trigger] g(v) by {
        vertex_sites(labels,rectangles,v);
    }
    corners::counting::sum_equal(4225,f,g);
    corners::corners_double_count(labels,rectangles);
}

/// 全域 LB1/LB2 合并的角点恒等式；输入只要求既有 Partition。
pub(crate) proof fn global_corner_identity(labels: Seq<u16>, rectangles: Seq<Rectangle>)
    requires partition(labels,rectangles),
    ensures b4(labels)==4*rectangles.len()+4*reflexes(labels)
        -2*ends(labels,rectangles)-4*crosses(labels,rectangles),
{
    assert forall|s: int| 0<=s<16900 implies #[trigger] site_weight(labels,s)
        ==site_corner(labels,rectangles,s)+4*site_reflex(labels,s)
            -2*site_endpoints(labels,rectangles,s)-4*site_cross(labels,rectangles,s) by {
        site_identity(labels,rectangles,s);
    }
    corners::counting::sum_corner_identity(16900,|s: int|site_weight(labels,s),
        |s: int|site_corner(labels,rectangles,s),|s: int|site_reflex(labels,s),
        |s: int|site_endpoints(labels,rectangles,s),|s: int|site_cross(labels,rectangles,s));
    site_corner_count(labels,rectangles);
}

}
