//! canonical 扫描序列的出生、续接及关闭计数。

use super::{corners, scan_model, split_count, split_geometry};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn continuation_end(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,
    y: int,end: int) -> bool {
    exists|a: int,value: u16| #[trigger] scan_model::continues(labels,verticals,blocked,y,a,end,value)
}

pub(crate) open spec fn continuation_start(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,
    y: int,a: int) -> bool {
    0<y<=64 && 0<=a<64 && corners::at(labels,a,y-1)!=0 && scan_model::start_at(labels,verticals,y-1,a)==a
        && scan_model::continues(labels,verticals,blocked,y,a,scan_model::end_at(labels,verticals,y-1,a),corners::at(labels,a,y-1))
}

pub(crate) open spec fn continuation_line(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,y: int) -> int {
    sum(65,|e: int|bit(continuation_end(labels,verticals,blocked,y,e)))
}

pub(crate) open spec fn continuations(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool) -> int {
    sum(65,|y: int|continuation_line(labels,verticals,blocked,y))
}

proof fn start_properties(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,y: int,a: int)
    requires continuation_start(labels,verticals,blocked,y,a),
    ensures scan_model::continues(labels,verticals,blocked,y,a,scan_model::end_at(labels,verticals,y-1,a),corners::at(labels,a,y-1)),
        0<=a<scan_model::end_at(labels,verticals,y-1,a)<=64,
{
    scan_model::pixel_run(labels,verticals,y-1,a);
}

proof fn from_end(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,y: int,a: int,e: int,value: u16)
    requires scan_model::continues(labels,verticals,blocked,y,a,e,value),
    ensures continuation_start(labels,verticals,blocked,y,a),scan_model::end_at(labels,verticals,y-1,a)==e,
{
    scan_model::run_at(labels,verticals,y-1,a,e,value,a);
}

pub(crate) proof fn continuation_bijection(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,y: int)
    ensures sum(64,|a: int|bit(continuation_start(labels,verticals,blocked,y,a)))
        ==continuation_line(labels,verticals,blocked,y),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let p=|a: int|continuation_start(labels,verticals,blocked,y,a);
    let q=|e: int|continuation_end(labels,verticals,blocked,y,e);
    let input=Set::<int>::range(0,64).filter(p);let target=Set::<int>::range(0,65).filter(q);
    let f=|a: int|scan_model::end_at(labels,verticals,y-1,a);let image=input.map(f);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int| input.contains(a) && input.contains(b)
            && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            start_properties(labels,verticals,blocked,y,a);start_properties(labels,verticals,blocked,y,b);
            split_geometry::row_end_unique(labels,verticals,y-1,a,b,f(a),corners::at(labels,a,y-1),corners::at(labels,b,y-1));
        }
    }
    vstd::set_lib::lemma_map_size(input,image,f);
    assert(image =~=target) by {
        assert forall|e: int| #[trigger] image.contains(e) implies target.contains(e) by {
            let a=choose|a: int|input.contains(a) && f(a)==e;start_properties(labels,verticals,blocked,y,a);
            assert(exists|a: int,value: u16|#[trigger] scan_model::continues(labels,verticals,blocked,y,a,e,value));
        }
        assert forall|e: int| #[trigger] target.contains(e) implies image.contains(e) by {
            let (a,value)=choose|a: int,value: u16|#[trigger] scan_model::continues(labels,verticals,blocked,y,a,e,value);
            from_end(labels,verticals,blocked,y,a,e,value);assert(input.contains(a));assert(image.contains(f(a)));
        }
    }
    corners::counting::range_count(64,p);corners::counting::range_count(65,q);
    corners::counting::sum_equal(64,|a: int|bit(p(a)),|a: int|bit(continuation_start(labels,verticals,blocked,y,a)));
    corners::counting::sum_equal(65,|e: int|bit(q(e)),|e: int|bit(continuation_end(labels,verticals,blocked,y,e)));
}

pub(crate) proof fn start_equivalent(labels: Seq<u16>,verticals: Set<int>,y: int,a: int)
    requires 0<=y<64,0<=a<64,
    ensures split_count::start(labels,verticals,y,a) <==> corners::at(labels,a,y)!=0 && scan_model::start_at(labels,verticals,y,a)==a,
{
    if corners::at(labels,a,y)!=0 {scan_model::row_prefix(labels,verticals,y,a);}
}

pub(crate) proof fn ending_balance(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,y: int,a: int)
    requires 0<=y<64,0<=a<64,
    ensures bit(scan_model::ending(labels,verticals,blocked,y*64+a))==bit(split_count::start(labels,verticals,y,a))
        -bit(continuation_start(labels,verticals,blocked,y+1,a)),
{
    start_equivalent(labels,verticals,y,a);
    assert((y*64+a)/64==y && (y*64+a)%64==a) by(nonlinear_arith) requires 0<=a<64;
}

pub(crate) proof fn ending_row_count(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,y: int)
    requires 0<=y<64,
    ensures sum(64,|a: int|bit(scan_model::ending(labels,verticals,blocked,y*64+a)))
        ==split_count::row_runs(labels,verticals,y)-continuation_line(labels,verticals,blocked,y+1),
{
    let w=|a: int|bit(scan_model::ending(labels,verticals,blocked,y*64+a));
    let a=|a: int|bit(split_count::start(labels,verticals,y,a));let b=|a: int|bit(continuation_start(labels,verticals,blocked,y+1,a));
    assert forall|x: int| 0<=x<64 implies #[trigger] w(x)==#[trigger] a(x)-#[trigger] b(x) by {ending_balance(labels,verticals,blocked,y,x);}
    corners::counting::sum_difference(64,w,a,b);continuation_bijection(labels,verticals,blocked,y+1);
}

/// 每个开始的 run 恰关闭一次；同 key 续接只减少一次出生，不按高度重复发出。
pub(crate) proof fn canonical_count(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool)
    ensures scan_model::canonical(labels,verticals,blocked).len() as int
        ==split_count::total_runs(labels,verticals)-continuations(labels,verticals,blocked),
{
    scan_model::anchor_prefix(labels,verticals,blocked,4096);
    let f=|a: int,y: int|bit(scan_model::ending(labels,verticals,blocked,y*64+a));
    corners::counting::grid_sum(64,64,f);
    let source=|p: int|bit(scan_model::ending(labels,verticals,blocked,p));
    corners::counting::sum_equal(4096,source,|p: int|f(p%64,p/64));
    let w=|y: int|sum(64,|a: int|f(a,y));let a=|y: int|split_count::row_runs(labels,verticals,y);
    let b=|y: int|continuation_line(labels,verticals,blocked,y+1);
    assert forall|y: int| 0<=y<64 implies #[trigger] w(y)==#[trigger] a(y)-#[trigger] b(y) by {
        ending_row_count(labels,verticals,blocked,y);
        corners::counting::sum_equal(64,|a: int|f(a,y),|a: int|bit(scan_model::ending(labels,verticals,blocked,y*64+a)));
    }
    corners::counting::sum_difference(64,w,a,b);
    let c=|y: int|continuation_line(labels,verticals,blocked,y);
    corners::counting::sum_concat(1,64,c);
    assert(!continuation_end(labels,verticals,blocked,0,0));
    corners::counting::sum_equal(65,|e: int|bit(continuation_end(labels,verticals,blocked,0,e)),|_: int|0);
    corners::counting::sum_constant(65,0);assert(c(0)==0);
    assert(sum(0,c)==0);assert(sum(1,c)==0);
    corners::counting::sum_equal(64,b,|y: int|c(1+y));
}

pub(crate) open spec fn unblocked(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool) -> bool {
    forall|y: int,a: int,b: int,value: u16| #[trigger] split_geometry::run(labels,verticals,y-1,a,b,value)
        && #[trigger] split_geometry::run(labels,verticals,y,a,b,value) ==> !blocked(y,a,b)
}

proof fn unblocked_event(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,y: int,e: int)
    requires unblocked(labels,verticals,blocked),
    ensures continuation_end(labels,verticals,blocked,y,e) <==> split_geometry::continuation_end(labels,verticals,y,e),
{
    if split_geometry::continuation_end(labels,verticals,y,e) {
        let (a,value)=choose|a: int,value: u16|#[trigger] split_geometry::run(labels,verticals,y-1,a,e,value)
            && #[trigger] split_geometry::run(labels,verticals,y,a,e,value);
        assert(!blocked(y,a,e));assert(scan_model::continues(labels,verticals,blocked,y,a,e,value));
        assert(exists|a: int,value: u16|#[trigger] scan_model::continues(labels,verticals,blocked,y,a,e,value));
    }
    if continuation_end(labels,verticals,blocked,y,e) {
        let (a,value)=choose|a: int,value: u16|#[trigger] scan_model::continues(labels,verticals,blocked,y,a,e,value);
        assert(exists|a: int,value: u16|#[trigger] split_geometry::run(labels,verticals,y-1,a,e,value)
            && #[trigger] split_geometry::run(labels,verticals,y,a,e,value));
    }
}

pub(crate) proof fn unblocked_count(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool)
    requires unblocked(labels,verticals,blocked),
    ensures continuations(labels,verticals,blocked)==super::upper_bound::continuations(labels,verticals),
{
    let a=|y: int|continuation_line(labels,verticals,blocked,y);let b=|y: int|super::upper_bound::continuation_line(labels,verticals,y);
    assert forall|y: int| 0<=y<65 implies #[trigger] a(y)==#[trigger] b(y) by {
        let c=|e: int|bit(continuation_end(labels,verticals,blocked,y,e));let d=|e: int|bit(split_geometry::continuation_end(labels,verticals,y,e));
        assert forall|e: int| 0<=e<65 implies #[trigger] c(e)==#[trigger] d(e) by {unblocked_event(labels,verticals,blocked,y,e);}
        corners::counting::sum_equal(65,c,d);
    }
    corners::counting::sum_equal(65,a,b);
}

/// 闭合几何主定理；actual scanner 只需证明输出恰为这个 canonical 序列。
pub(crate) proof fn canonical_optimal(labels: Seq<u16>,selected: Set<int>,blocked: spec_fn(int,int,int)->bool)
    requires labels.len()==4096,super::good::maximum_independent(labels,selected),
        unblocked(labels,super::optimality::vertical_part(selected),blocked),
    ensures super::optimal(labels,scan_model::canonical(labels,super::optimality::vertical_part(selected),blocked)),
{
    let vs=super::optimality::vertical_part(selected);
    scan_model::canonical_partition(labels,vs,blocked);canonical_count(labels,vs,blocked);unblocked_count(labels,vs,blocked);
    super::upper_bound::optimal_from_scan(labels,scan_model::canonical(labels,vs,blocked),selected);
}

pub(crate) open spec fn horizontal_block(selected: Set<int>,y: int,a: int,b: int) -> bool {
    exists|h: int| #[trigger] selected.contains(h) && !super::good::vertical(h) && super::good::line(h)==y
        && super::good::start(h)<b && a<super::good::end(h)
}

/// 独立 good 选线建立无额外 H 阻断，不将这一性质作为几何假设。
pub(crate) proof fn independent_unblocked(labels: Seq<u16>,selected: Set<int>)
    requires super::good::independent(labels,selected),
    ensures unblocked(labels,super::optimality::vertical_part(selected),|y: int,a: int,b: int|horizontal_block(selected,y,a,b)),
{
    broadcast use vstd::set_lib::group_set_properties;
    let vs=super::optimality::vertical_part(selected);
    assert(split_geometry::selected(labels,vs));
    assert forall|y: int,a: int,b: int,value: u16| #[trigger] split_geometry::run(labels,vs,y-1,a,b,value)
        && #[trigger] split_geometry::run(labels,vs,y,a,b,value) implies !horizontal_block(selected,y,a,b) by {
        if horizontal_block(selected,y,a,b) {
            let h=choose|h: int| selected.contains(h) && !super::good::vertical(h) && #[trigger] super::good::line(h)==y
                && super::good::start(h)<b && a<super::good::end(h);
            assert(super::good::good(labels,h));
            assert forall|v: int| #[trigger] vs.contains(v) implies !super::good::intersects(h,v) by {}
            split_geometry::horizontal_redundant(labels,vs,h,a,b,value);
        }
    }
}

/// 几何算法的纯数学终点；生产接线仍须证明实际扫描序列等于 canonical。
pub(crate) proof fn minimum_partition(labels: Seq<u16>,selected: Set<int>)
    requires labels.len()==4096,super::good::maximum_independent(labels,selected),
    ensures super::optimal(labels,scan_model::canonical(labels,super::optimality::vertical_part(selected),
        |y: int,a: int,b: int|horizontal_block(selected,y,a,b))),
{
    independent_unblocked(labels,selected);
    canonical_optimal(labels,selected,|y: int,a: int,b: int|horizontal_block(selected,y,a,b));
}

}
