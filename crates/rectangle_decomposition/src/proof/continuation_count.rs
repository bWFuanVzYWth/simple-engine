//! 同色 overlap 内共同边界的有限扫描计数，不保存执行期边界表。

use super::{chain_marks, chains, corners, good};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn hit(f: spec_fn(int)->u16,b: spec_fn(int,u16)->bool,t: int) -> bool
    decreases if t>0 {t}else{0},
{
    if t<=0 || f(t)!=f(t-1) {b(t,f(t))} else {hit(f,b,t-1) || b(t,f(t))}
}

pub(crate) open spec fn potential(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,
    b: spec_fn(int,u16)->bool,t: int) -> int {
    if 0<=t<64 && f(t)!=0 {bit(r(chain_marks::start_at(f,t),f(t)) && hit(f,b,t))} else {0}
}

pub(crate) open spec fn points(f: spec_fn(int)->u16,b: spec_fn(int,u16)->bool,t: int) -> int {
    bit(f(t)!=0 && f(t)!=f(t-1) && b(t,f(t)))+bit(f(t-1)!=0 && b(t,f(t-1)))
}

pub(crate) open spec fn blocked(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,
    b: spec_fn(int,u16)->bool,t: int) -> int {
    bit(f(t-1)!=0 && f(t)!=f(t-1) && r(t,f(t-1))
        && r(chain_marks::start_at(f,t-1),f(t-1)) && (hit(f,b,t-1) || b(t,f(t-1))))
}

pub(crate) open spec fn continued(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,
    b: spec_fn(int,u16)->bool,t: int) -> int {
    bit(f(t-1)!=0 && (b(t,f(t-1)) || (f(t)!=f(t-1) && !r(t,f(t-1))))
        && (!r(chain_marks::start_at(f,t-1),f(t-1)) || hit(f,b,t-1)))
}

pub(crate) open spec fn valid(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,
    b: spec_fn(int,u16)->bool) -> bool {
    f(-1)==0 && f(64)==0
        && (forall|t: int| 0<=t<=64 ==> (
            f(t)!=0 && f(t)!=f(t-1) && !#[trigger] r(t,f(t)) ==> !#[trigger] b(t,f(t))))
        && (forall|t: int| 0<=t<=64 ==> (
            f(t-1)!=0 && f(t)!=f(t-1) && !#[trigger] r(t,f(t-1)) ==> !#[trigger] b(t,f(t-1))))
}

pub(crate) proof fn local_count(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,
    b: spec_fn(int,u16)->bool,t: int)
    requires valid(f,r,b),0<=t<=64,
    ensures continued(f,r,b,t)==points(f,b,t)+chain_marks::both_unmarked(f,r,t)
        -blocked(f,r,b,t)+potential(f,r,b,t-1)-potential(f,r,b,t),
{
    reveal_with_fuel(hit,2);
    reveal_with_fuel(chain_marks::start_at,2);
    assert(f(t)!=0 && f(t)!=f(t-1) && !r(t,f(t)) ==> !b(t,f(t)));
    assert(f(t-1)!=0 && f(t)!=f(t-1) && !r(t,f(t-1)) ==> !b(t,f(t-1)));
    if t==0 {} else if t==64 {} else {}
    if f(t)==0 {} else if f(t-1)==0 {} else if f(t)==f(t-1) {} else {}
}

pub(crate) proof fn prefix_count(n: nat,f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,
    b: spec_fn(int,u16)->bool)
    requires valid(f,r,b),n<=65,
    ensures sum(n,|t: int|continued(f,r,b,t))==sum(n,|t: int|points(f,b,t))
        +sum(n,|t: int|chain_marks::both_unmarked(f,r,t))-sum(n,|t: int|blocked(f,r,b,t))
        +potential(f,r,b,-1)-potential(f,r,b,n-1),
    decreases n,
{
    if n>0 {prefix_count((n-1) as nat,f,r,b);local_count(f,r,b,n-1);}
}

/// 新续接数等于原相同 run 对数加交点数，再按被命中的 good H 去重扣减。
pub(crate) proof fn total_count(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,
    b: spec_fn(int,u16)->bool)
    requires valid(f,r,b),
    ensures sum(65,|t: int|continued(f,r,b,t))==sum(65,|t: int|points(f,b,t))
        +sum(65,|t: int|chain_marks::both_unmarked(f,r,t))-sum(65,|t: int|blocked(f,r,b,t)),
{
    prefix_count(65,f,r,b);
}

pub(crate) open spec fn selected_points(labels: Seq<u16>,verticals: Set<int>,y: int)
    -> spec_fn(int,u16)->bool {
    |x: int,value: u16|super::split_geometry::point(verticals,x,y,value,labels)
}

/// good V 不能落在原上下直边的两格点，因此有限扫描的端点前提由真实几何建立。
pub(crate) proof fn selected_valid(labels: Seq<u16>,verticals: Set<int>,y: int)
    requires super::split_geometry::selected(labels,verticals),0<=y<=64,
    ensures valid(super::row_count::source(labels,y),super::row_count::marks(labels,y),selected_points(labels,verticals,y)),
{
    let f=super::row_count::source(labels,y);let r=super::row_count::marks(labels,y);
    let b=selected_points(labels,verticals,y);
    assert forall|t: int| 0<=t<=64 && f(t)!=0 && f(t)!=f(t-1) && !#[trigger] r(t,f(t))
        implies !#[trigger] b(t,f(t)) by {
        if f(t)!=0 && f(t)!=f(t-1) && !r(t,f(t)) && b(t,f(t)) {
            super::split_geometry::point_has_color(labels,verticals,t,y,f(t));
        }
    }
    assert forall|t: int| 0<=t<=64 && f(t-1)!=0 && f(t)!=f(t-1) && !#[trigger] r(t,f(t-1))
        implies !#[trigger] b(t,f(t-1)) by {
        if f(t-1)!=0 && f(t)!=f(t-1) && !r(t,f(t-1)) && b(t,f(t-1)) {
            super::split_geometry::point_has_color(labels,verticals,t,y,f(t-1));
        }
    }
}

pub(crate) proof fn hit_exists(f: spec_fn(int)->u16,b: spec_fn(int,u16)->bool,t: int)
    requires 0<=t<64,f(t)!=0,
    ensures hit(f,b,t) <==> exists|u: int| chain_marks::start_at(f,t)<=u<=t && #[trigger] b(u,f(t)),
    decreases t,
{
    chain_marks::start_prefix(f,t);
    if t>0 && f(t)==f(t-1) {
        hit_exists(f,b,t-1);
        if exists|u: int| chain_marks::start_at(f,t)<=u<=t && #[trigger] b(u,f(t)) {
            let u=choose|u: int| chain_marks::start_at(f,t)<=u<=t && #[trigger] b(u,f(t));
            if u<t {
                assert(exists|u: int| chain_marks::start_at(f,t-1)<=u<=t-1 && #[trigger] b(u,f(t-1)));
            }
        }
        if hit(f,b,t-1) {
            let u=choose|u: int| chain_marks::start_at(f,t-1)<=u<=t-1 && #[trigger] b(u,f(t-1));
            assert(exists|u: int|chain_marks::start_at(f,t)<=u<=t && #[trigger] b(u,f(t)));
        }
    }
    if b(t,f(t)) {assert(exists|u: int|chain_marks::start_at(f,t)<=u<=t && #[trigger] b(u,f(t)));}
}

pub(crate) open spec fn previous(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,
    b: spec_fn(int,u16)->bool,t: int) -> int
    decreases if t>0 {t}else{0},
{
    if b(t,f(t)) {t}
    else if t<=0 || f(t)!=f(t-1) {if !r(t,f(t)) {t}else{-1}}
    else {previous(f,r,b,t-1)}
}

pub(crate) proof fn previous_spec(f: spec_fn(int)->u16,r: spec_fn(int,u16)->bool,
    b: spec_fn(int,u16)->bool,t: int)
    requires 0<=t<64,f(t)!=0,
    ensures -1<=previous(f,r,b,t)<=t,
        previous(f,r,b,t)>=0 <==> !r(chain_marks::start_at(f,t),f(t)) || hit(f,b,t),
        previous(f,r,b,t)>=0 ==> chain_marks::start_at(f,t)<=previous(f,r,b,t)
            && (previous(f,r,b,t)==chain_marks::start_at(f,t) && !r(chain_marks::start_at(f,t),f(t))
                || b(previous(f,r,b,t),f(t))),
        forall|u: int| previous(f,r,b,t)<u<=t && chain_marks::start_at(f,t)<=u ==> !#[trigger] b(u,f(t)),
    decreases t,
{
    chain_marks::start_prefix(f,t);
    if t>0 && f(t)==f(t-1) {previous_spec(f,r,b,t-1);}
}

pub(crate) proof fn continued_to_runs(labels: Seq<u16>,verticals: Set<int>,y: int,end: int)
    requires super::split_geometry::selected(labels,verticals),0<=y<=64,0<=end<=64,
        continued(super::row_count::source(labels,y),super::row_count::marks(labels,y),
            selected_points(labels,verticals,y),end)==1,
    ensures super::split_geometry::continuation_end(labels,verticals,y,end),
{
    let f=super::row_count::source(labels,y);let r=super::row_count::marks(labels,y);
    let b=selected_points(labels,verticals,y);let value=f(end-1);
    assert(f(-1)==0 && f(64)==0);assert(end>0 && value!=0);
    chain_marks::start_prefix(f,end-1);
    previous_spec(f,r,b,end-1);
    let a=previous(f,r,b,end-1);let s=chain_marks::start_at(f,end-1);
    assert(0<=s<=a<end<=64);
    assert(f(a)==value);
    assert(corners::at(labels,a,y-1)==value && corners::at(labels,a,y)==value);
    assert(1<=y<=63);
    super::split_geometry::overlap_boundary(labels,verticals,y,a,value);
    super::split_geometry::overlap_boundary(labels,verticals,y,end,value);
    assert(super::split_geometry::common(labels,verticals,y,a,value));
    assert(super::split_geometry::common(labels,verticals,y,end,value));
    assert forall|u: int| a<u<end implies !#[trigger] super::split_geometry::active(verticals,u,y) by {
        assert(f(u)==value && f(u-1)==value);
        assert(good::quadrants(labels,u,y,value)==4);
        super::split_geometry::point_full(labels,verticals,u,y,value);
        assert(!b(u,value));
    }
    assert forall|u: int| a<u<end implies !#[trigger] super::split_geometry::active(verticals,u,y-1) by {
        assert(f(u)==value && f(u-1)==value);
        assert(good::quadrants(labels,u,y,value)==4);
        super::split_geometry::point_full(labels,verticals,u,y,value);
        assert(!b(u,value));
    }
    assert forall|u: int| a<=u<end implies #[trigger] corners::at(labels,u,y)==value by {assert(f(u)==value);}
    assert forall|u: int| a<=u<end implies #[trigger] corners::at(labels,u,y-1)==value by {assert(f(u)==value);}
    assert(super::split_geometry::run(labels,verticals,y-1,a,end,value));
    assert(super::split_geometry::run(labels,verticals,y,a,end,value));
    assert(exists|start: int,value: u16| #[trigger] super::split_geometry::run(labels,verticals,y-1,start,end,value)
        && #[trigger] super::split_geometry::run(labels,verticals,y,start,end,value));
}

pub(crate) proof fn runs_to_continued(labels: Seq<u16>,verticals: Set<int>,y: int,start: int,end: int,value: u16)
    requires super::split_geometry::selected(labels,verticals),
        super::split_geometry::run(labels,verticals,y-1,start,end,value),
        super::split_geometry::run(labels,verticals,y,start,end,value),
    ensures continued(super::row_count::source(labels,y),super::row_count::marks(labels,y),
        selected_points(labels,verticals,y),end)==1,
{
    let f=super::row_count::source(labels,y);let r=super::row_count::marks(labels,y);
    let b=selected_points(labels,verticals,y);
    assert(f(end-1)==value);
    chain_marks::start_prefix(f,end-1);
    let s=chain_marks::start_at(f,end-1);
    if start<s {assert(f(s-1)==value);}
    assert(s<=start);
    assert(f(start)==value);
    super::split_geometry::overlap_boundary(labels,verticals,y,start,value);
    super::split_geometry::overlap_boundary(labels,verticals,y,end,value);
    assert(super::split_geometry::common(labels,verticals,y,start,value));
    assert(super::split_geometry::common(labels,verticals,y,end,value));
    if start>s {
        assert(f(start-1)==value);
        assert(good::quadrants(labels,start,y,value)==4);
        assert(b(start,value));
    }
    if b(start,value) {
        hit_exists(f,b,end-1);
        assert(exists|u: int| s<=u<=end-1 && #[trigger] b(u,f(end-1)));
    }
}

/// 抽象共同边界扫描与真实上下 split run 的同 key 续接完全对应。
pub(crate) proof fn actual_continuation(labels: Seq<u16>,verticals: Set<int>,y: int,end: int)
    requires super::split_geometry::selected(labels,verticals),0<=y<=64,0<=end<=64,
    ensures continued(super::row_count::source(labels,y),super::row_count::marks(labels,y),
        selected_points(labels,verticals,y),end)
        ==bit(super::split_geometry::continuation_end(labels,verticals,y,end)),
{
    if super::split_geometry::continuation_end(labels,verticals,y,end) {
        let (start,value)=choose|start: int,value: u16|
            #[trigger] super::split_geometry::run(labels,verticals,y-1,start,end,value)
                && #[trigger] super::split_geometry::run(labels,verticals,y,start,end,value);
        runs_to_continued(labels,verticals,y,start,end,value);
    } else {
        if continued(super::row_count::source(labels,y),super::row_count::marks(labels,y),
            selected_points(labels,verticals,y),end)==1 {continued_to_runs(labels,verticals,y,end);}
    }
}

}
