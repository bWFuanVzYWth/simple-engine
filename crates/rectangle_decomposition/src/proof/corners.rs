//! 数字区域四象限及真实矩形 owner；点接触按标签分别计数。

use super::{covers, partition, valid};
use crate::optimal::Rectangle;
use vstd::prelude::*;

#[path = "counting.rs"]
pub(crate) mod counting;
use counting::{bit, sum};

verus! {

pub(crate) open spec fn at(labels: Seq<u16>, x: int, y: int) -> u16 {
    if 0 <= x < 64 && 0 <= y < 64 { labels[y * 64 + x] } else { 0 }
}

pub(crate) open spec fn cell(x: int, y: int) -> int { y * 64 + x }

pub(crate) open spec fn inside(rectangle: Rectangle, x: int, y: int) -> bool {
    rectangle.x.start <= x < rectangle.x.end && rectangle.y.start <= y < rectangle.y.end
}

pub(crate) proof fn cell_coordinates(x: int, y: int)
    requires 0 <= x < 64, 0 <= y < 64,
    ensures 0 <= cell(x,y) < 4096, cell(x,y) % 64 == x, cell(x,y) / 64 == y,
{
    assert((y * 64 + x) % 64 == x) by(nonlinear_arith) requires 0 <= x < 64;
    assert((y * 64 + x) / 64 == y) by(nonlinear_arith) requires 0 <= x < 64;
}

pub(crate) proof fn covers_coordinates(rectangle: Rectangle, x: int, y: int)
    requires valid(rectangle),
    ensures inside(rectangle,x,y) <==> (0 <= x < 64 && 0 <= y < 64 && covers(rectangle,cell(x,y))),
{
    if 0 <= x < 64 && 0 <= y < 64 { cell_coordinates(x,y); }
}

/// -1 表示背景或边界外；非负值始终是原分区的唯一矩形编号。
pub(crate) open spec fn owner(rectangles: Seq<Rectangle>, x: int, y: int) -> int {
    if exists|i: int| 0 <= i < rectangles.len() && #[trigger] inside(rectangles[i],x,y) {
        choose|i: int| 0 <= i < rectangles.len() && #[trigger] inside(rectangles[i],x,y)
    } else { -1 }
}

pub(crate) proof fn owner_exact(labels: Seq<u16>, rectangles: Seq<Rectangle>, x: int, y: int)
    requires partition(labels,rectangles),
    ensures owner(rectangles,x,y) == -1 <==> at(labels,x,y) == 0,
        owner(rectangles,x,y) >= -1,
        owner(rectangles,x,y) >= 0 ==> owner(rectangles,x,y) < rectangles.len()
            && inside(rectangles[owner(rectangles,x,y)],x,y)
            && rectangles[owner(rectangles,x,y)].value == at(labels,x,y),
        forall|i: int| 0 <= i < rectangles.len() ==> (#[trigger] inside(rectangles[i],x,y)
            <==> owner(rectangles,x,y) == i),
{
    assert forall|i: int| 0 <= i < rectangles.len() implies
        (#[trigger] inside(rectangles[i],x,y) <==> owner(rectangles,x,y) == i) by {
        covers_coordinates(rectangles[i],x,y);
        if inside(rectangles[i],x,y) {
            let k = owner(rectangles,x,y);
            assert(0 <= k < rectangles.len() && inside(rectangles[k],x,y));
            covers_coordinates(rectangles[k],x,y);
        }
    }
    if 0 <= x < 64 && 0 <= y < 64 {
        cell_coordinates(x,y);
        if at(labels,x,y) != 0 {
            let i = choose|i: int| 0 <= i < rectangles.len() && #[trigger] covers(rectangles[i],cell(x,y));
            covers_coordinates(rectangles[i],x,y);
        }
    }
    if owner(rectangles,x,y) >= 0 {
        let i = owner(rectangles,x,y);
        covers_coordinates(rectangles[i],x,y);
        assert(covers(rectangles[i],cell(x,y)));
    }
}

/// 矩形包含相对的两个象限时必包含四象限，不允许 L 形 owner 或对角 owner。
pub(crate) proof fn diagonal_closure(rectangle: Rectangle, x: int, y: int)
    ensures (inside(rectangle,x-1,y-1) && inside(rectangle,x,y))
            ==> inside(rectangle,x,y-1) && inside(rectangle,x-1,y),
        (inside(rectangle,x,y-1) && inside(rectangle,x-1,y))
            ==> inside(rectangle,x-1,y-1) && inside(rectangle,x,y),
{}

pub(crate) open spec fn legal(a: int, b: int, c: int, d: int) -> bool {
    a >= -1 && b >= -1 && c >= -1 && d >= -1
        && (a >= 0 && a == c ==> a == b && a == d)
        && (b >= 0 && b == d ==> b == a && b == c)
}

pub(crate) proof fn owners_legal(labels: Seq<u16>, rectangles: Seq<Rectangle>, x: int, y: int)
    requires partition(labels,rectangles),
    ensures legal(owner(rectangles,x-1,y-1), owner(rectangles,x,y-1),
        owner(rectangles,x,y),owner(rectangles,x-1,y)),
{
    owner_exact(labels,rectangles,x-1,y-1);
    owner_exact(labels,rectangles,x,y-1);
    owner_exact(labels,rectangles,x,y);
    owner_exact(labels,rectangles,x-1,y);
    let a = owner(rectangles,x-1,y-1);
    let b = owner(rectangles,x,y-1);
    let c = owner(rectangles,x,y);
    let d = owner(rectangles,x-1,y);
    if a >= 0 && a == c { diagonal_closure(rectangles[a],x,y); }
    if b >= 0 && b == d { diagonal_closure(rectangles[b],x,y); }
}

pub(crate) open spec fn count(a: bool, b: bool, c: bool, d: bool) -> int {
    bit(a)+bit(b)+bit(c)+bit(d)
}

pub(crate) open spec fn weight(a: bool, b: bool, c: bool, d: bool) -> int {
    let n = count(a,b,c,d);
    if n == 1 { 1 } else if n == 3 { 3 }
    else if n == 2 && (a && c || b && d) { 2 } else { 0 }
}

pub(crate) open spec fn local_corners(a: int, b: int, c: int, d: int) -> int {
    bit(a>=0 && a!=b && a!=d) + bit(b>=0 && b!=a && b!=c)
        + bit(c>=0 && c!=b && c!=d) + bit(d>=0 && d!=a && d!=c)
}

pub(crate) open spec fn cut(a: int, b: int) -> bool { a>=0 && b>=0 && a!=b }

pub(crate) open spec fn endpoints(a: int, b: int, c: int, d: int) -> int {
    bit(cut(a,b) != cut(d,c)) + bit(cut(a,d) != cut(b,c))
}

pub(crate) open spec fn cross(a: int, b: int, c: int, d: int) -> bool {
    cut(a,b) && cut(b,c) && cut(c,d) && cut(d,a)
}

/// LB1/LB2 的局部合并式，允许原凹角双射线、T、X 与对角点接触。
pub(crate) proof fn local_identity(a: int, b: int, c: int, d: int)
    requires legal(a,b,c,d),
    ensures weight(a>=0,b>=0,c>=0,d>=0)
        == local_corners(a,b,c,d) + 4*bit(count(a>=0,b>=0,c>=0,d>=0)==3)
            - 2*endpoints(a,b,c,d) - 4*bit(cross(a,b,c,d)),
{
    if a>=0 { } if b>=0 { } if c>=0 { } if d>=0 { }
    if a==b { } if a==c { } if a==d { } if b==c { } if b==d { } if c==d { }
}

/// 四象限按 NW、NE、SE、SW 排列。
pub(crate) open spec fn qx(x: int, q: int) -> int {
    if q == 0 || q == 3 { x-1 } else { x }
}

pub(crate) open spec fn qy(y: int, q: int) -> int {
    if q == 0 || q == 1 { y-1 } else { y }
}

pub(crate) open spec fn oq(rectangles: Seq<Rectangle>, x: int, y: int, q: int) -> int {
    owner(rectangles,qx(x,q),qy(y,q))
}

pub(crate) open spec fn corner_owner(rectangles: Seq<Rectangle>, x: int, y: int, q: int) -> bool {
    let a = oq(rectangles,x,y,q);
    a>=0 && a!=oq(rectangles,x,y,(q+1)%4) && a!=oq(rectangles,x,y,(q+3)%4)
}

pub(crate) open spec fn corner_q(rectangle: Rectangle, x: int, y: int, q: int) -> bool {
    if q == 0 { x == rectangle.x.end && y == rectangle.y.end }
    else if q == 1 { x == rectangle.x.start && y == rectangle.y.end }
    else if q == 2 { x == rectangle.x.start && y == rectangle.y.start }
    else { x == rectangle.x.end && y == rectangle.y.start }
}

pub(crate) proof fn corner_q_cells(rectangle: Rectangle, x: int, y: int, q: int)
    requires valid(rectangle), 0 <= q < 4,
    ensures corner_q(rectangle,x,y,q) <==> (inside(rectangle,qx(x,q),qy(y,q))
        && !inside(rectangle,qx(x,(q+1)%4),qy(y,(q+1)%4))
        && !inside(rectangle,qx(x,(q+3)%4),qy(y,(q+3)%4))),
{
    if q==0 {} else if q==1 {} else if q==2 {} else {}
}

pub(crate) proof fn actual_corner(labels: Seq<u16>, rectangles: Seq<Rectangle>,
    x: int, y: int, q: int, i: int)
    requires partition(labels,rectangles), 0<=q<4, 0<=i<rectangles.len(),
    ensures corner_q(rectangles[i],x,y,q)
        <==> corner_owner(rectangles,x,y,q) && oq(rectangles,x,y,q)==i,
{
    owner_exact(labels,rectangles,qx(x,q),qy(y,q));
    owner_exact(labels,rectangles,qx(x,(q+1)%4),qy(y,(q+1)%4));
    owner_exact(labels,rectangles,qx(x,(q+3)%4),qy(y,(q+3)%4));
    corner_q_cells(rectangles[i],x,y,q);
}

pub(crate) open spec fn vertex_corner(rectangles: Seq<Rectangle>, v: int) -> int {
    sum(4, |q: int| bit(corner_owner(rectangles,v%65,v/65,q)))
}

pub(crate) open spec fn rectangle_corners(rectangles: Seq<Rectangle>) -> int {
    sum(4225, |v: int| vertex_corner(rectangles,v))
}

pub(crate) open spec fn vertex_q_count(rectangles: Seq<Rectangle>, v: int, q: int) -> int {
    sum(rectangles.len(), |i: int| bit(corner_q(rectangles[i],v%65,v/65,q)))
}

pub(crate) open spec fn rectangle_q_count(rectangle: Rectangle, q: int) -> int {
    sum(4225, |v: int| bit(corner_q(rectangle,v%65,v/65,q)))
}

pub(crate) open spec fn all_corner_count(rectangle: Rectangle) -> int {
    sum(4225, |v: int| sum(4, |q: int| bit(corner_q(rectangle,v%65,v/65,q))))
}

pub(crate) proof fn vertex_double_count(labels: Seq<u16>, rectangles: Seq<Rectangle>, v: int)
    requires partition(labels,rectangles), 0<=v<4225,
    ensures vertex_corner(rectangles,v) == sum(rectangles.len(), |i: int|
        sum(4, |q: int| bit(corner_q(rectangles[i],v%65,v/65,q)))),
{
    let f = |i: int, q: int| bit(corner_q(rectangles[i],v%65,v/65,q));
    counting::sum_swap(rectangles.len(),4,f);
    assert forall|q: int| 0<=q<4 implies
        #[trigger] vertex_q_count(rectangles,v,q) == bit(corner_owner(rectangles,v%65,v/65,q))
        && sum(rectangles.len(),|i: int| f(i,q)) == vertex_q_count(rectangles,v,q) by {
        let a = oq(rectangles,v%65,v/65,q);
        counting::sum_equal(rectangles.len(), |i: int| f(i,q),
            |i: int| bit(corner_q(rectangles[i],v%65,v/65,q)));
        assert forall|i: int| 0<=i<rectangles.len() implies #[trigger] f(i,q)
            == bit(corner_owner(rectangles,v%65,v/65,q) && a==i) by {
            actual_corner(labels,rectangles,v%65,v/65,q,i);
        }
        counting::sum_equal(rectangles.len(),|i: int| f(i,q),
            |i: int| bit(corner_owner(rectangles,v%65,v/65,q) && a==i));
        if corner_owner(rectangles,v%65,v/65,q) {
            owner_exact(labels,rectangles,qx(v%65,q),qy(v/65,q));
            counting::sum_single(rectangles.len(),a);
            counting::sum_equal(rectangles.len(),
                |i: int| bit(corner_owner(rectangles,v%65,v/65,q) && a==i), |i: int| bit(i==a));
        } else {
            counting::sum_constant(rectangles.len(),0);
            counting::sum_equal(rectangles.len(),
                |i: int| bit(corner_owner(rectangles,v%65,v/65,q) && a==i), |_: int| 0);
        }
    }
    counting::sum_equal(4, |q: int| sum(rectangles.len(),|i: int|f(i,q)),
        |q: int| vertex_q_count(rectangles,v,q));
    counting::sum_equal(4, |q: int| vertex_q_count(rectangles,v,q),
        |q: int| bit(corner_owner(rectangles,v%65,v/65,q)));
    assert forall|i: int| 0<=i<rectangles.len() implies
        #[trigger] rectangles[i] == rectangles[i]
        && sum(4,|q: int|f(i,q)) == sum(4,|q: int|bit(corner_q(rectangles[i],v%65,v/65,q))) by {
        counting::sum_equal(4, |q: int|f(i,q), |q: int|bit(corner_q(rectangles[i],v%65,v/65,q)));
    }
    counting::sum_equal(rectangles.len(), |i: int|sum(4,|q: int|f(i,q)),
        |i: int|sum(4,|q: int|bit(corner_q(rectangles[i],v%65,v/65,q))));
}

pub(crate) proof fn rectangle_corner_position(rectangle: Rectangle, q: int)
    requires valid(rectangle), 0<=q<4,
    ensures exists|v: int| 0<=v<4225 && #[trigger] corner_q(rectangle,v%65,v/65,q)
        && (forall|w: int| 0<=w<4225 && #[trigger] corner_q(rectangle,w%65,w/65,q) ==> w==v),
{
    let x = if q==0 || q==3 { rectangle.x.end as int } else { rectangle.x.start as int };
    let y = if q==0 || q==1 { rectangle.y.end as int } else { rectangle.y.start as int };
    let v = y*65+x;
    assert(v%65==x && v/65==y) by(nonlinear_arith) requires 0<=x<65, v==y*65+x;
    assert forall|w: int| 0<=w<4225 && #[trigger] corner_q(rectangle,w%65,w/65,q)
        implies w==v by {
        if q==0 {} else if q==1 {} else if q==2 {} else {}
        assert(w==w/65*65+w%65);
    }
    assert(corner_q(rectangle,v%65,v/65,q));
}

pub(crate) proof fn rectangle_four_corners(rectangle: Rectangle)
    requires valid(rectangle),
    ensures sum(4225, |v: int| sum(4, |q: int| bit(corner_q(rectangle,v%65,v/65,q)))) == 4,
{
    let f = |v: int, q: int| bit(corner_q(rectangle,v%65,v/65,q));
    counting::sum_swap(4225,4,f);
    assert forall|q: int| 0<=q<4 implies #[trigger] rectangle_q_count(rectangle,q)==1
        && sum(4225, |v: int|f(v,q))==rectangle_q_count(rectangle,q) by {
        rectangle_corner_position(rectangle,q);
        let k = choose|v: int| 0<=v<4225 && #[trigger] corner_q(rectangle,v%65,v/65,q)
            && (forall|w: int| 0<=w<4225 && #[trigger] corner_q(rectangle,w%65,w/65,q) ==> w==v);
        counting::sum_single(4225,k);
        counting::sum_equal(4225,|v: int|f(v,q),|v: int|bit(v==k));
        counting::sum_equal(4225,|v: int|f(v,q),
            |v: int|bit(corner_q(rectangle,v%65,v/65,q)));
    }
    counting::sum_equal(4, |q: int| sum(4225,|v: int|f(v,q)),
        |q: int| rectangle_q_count(rectangle,q));
    counting::sum_equal(4, |q: int| rectangle_q_count(rectangle,q), |_: int|1);
    counting::sum_constant(4,1);
    let g = |v: int|sum(4,|q: int|f(v,q));
    let h = |v: int|sum(4,|q: int|bit(corner_q(rectangle,v%65,v/65,q)));
    assert forall|v: int| 0<=v<4225 implies #[trigger] g(v)==#[trigger] h(v) by {
        counting::sum_equal(4,|q: int|f(v,q),|q: int|bit(corner_q(rectangle,v%65,v/65,q)));
    }
    counting::sum_equal(4225,g,h);
}

/// 每个真实竞争矩形恰贡献四个角，未对竞争分区限制 T/X 或仅角点接触。
pub(crate) proof fn corners_double_count(labels: Seq<u16>, rectangles: Seq<Rectangle>)
    requires partition(labels,rectangles),
    ensures rectangle_corners(rectangles) == 4 * rectangles.len(),
{
    let f = |v: int, i: int| sum(4,|q: int|bit(corner_q(rectangles[i],v%65,v/65,q)));
    assert forall|v: int| 0<=v<4225 implies #[trigger] vertex_corner(rectangles,v)
        == sum(rectangles.len(),|i: int|f(v,i)) by {
        vertex_double_count(labels,rectangles,v);
        counting::sum_equal(rectangles.len(), |i: int| f(v,i),
            |i: int| sum(4, |q: int| bit(corner_q(rectangles[i],v%65,v/65,q))));
    }
    counting::sum_equal(4225, |v: int| vertex_corner(rectangles,v),
        |v: int| sum(rectangles.len(), |i: int| f(v,i)));
    counting::sum_swap(4225,rectangles.len(),f);
    assert forall|i: int| 0<=i<rectangles.len() implies #[trigger] all_corner_count(rectangles[i])==4
        && sum(4225, |v: int|f(v,i))==all_corner_count(rectangles[i]) by {
        rectangle_four_corners(rectangles[i]);
        counting::sum_equal(4225, |v: int| f(v,i),
            |v: int|sum(4,|q: int|bit(corner_q(rectangles[i],v%65,v/65,q))));
    }
    counting::sum_equal(rectangles.len(), |i: int|sum(4225,|v: int|f(v,i)),
        |i: int| all_corner_count(rectangles[i]));
    counting::sum_equal(rectangles.len(), |i: int|all_corner_count(rectangles[i]), |_: int|4);
    counting::sum_constant(rectangles.len(),4);
    assert(4*rectangles.len()==rectangles.len()*4) by(nonlinear_arith);
}

}
