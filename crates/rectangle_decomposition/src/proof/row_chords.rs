//! 相邻最大行 runs 的交集事件与全部水平 good chord 的双向对应。

use super::{chain_marks, colored, corners, good, row_count};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn event(labels: Seq<u16>,key: int) -> bool {
    0<=key<4225 && chain_marks::paired(row_count::source(labels,key/65),row_count::marks(labels,key/65),key%65)==1
}

pub(crate) open spec fn event_id(labels: Seq<u16>,key: int) -> int {
    good::encode(false,key/65,chain_marks::start_at(row_count::source(labels,key/65),key%65-1),key%65)
}

pub(crate) open spec fn events(labels: Seq<u16>) -> Set<int> {
    Set::<int>::range(0,4225).filter(|key: int|event(labels,key))
}

pub(crate) proof fn overlap_unit(labels: Seq<u16>,y: int,t: int,value: u16)
    requires value!=0,
    ensures row_count::overlap(labels,y,t)==value <==> good::unit_inside(labels,false,y,t,value),
{}

pub(crate) proof fn actual_good(labels: Seq<u16>,key: int)
    requires event(labels,key),
    ensures good::good(labels,event_id(labels,key)),!good::vertical(event_id(labels,key)),
        good::line(event_id(labels,key))==key/65,good::end(event_id(labels,key))==key%65,
        good::start(event_id(labels,key))==chain_marks::start_at(row_count::source(labels,key/65),key%65-1),
        good::value(labels,event_id(labels,key))==row_count::overlap(labels,key/65,key%65-1),
{
    let y=key/65;let t=key%65;let f=row_count::source(labels,y);let value=f(t-1);
    let s=chain_marks::start_at(f,t-1);
    assert(f(-1)==0 && f(64)==0);
    assert(t>0 && value!=0);
    chain_marks::start_prefix(f,t-1);
    chain_marks::reflex_interior(labels,false,y,t,value);
    chain_marks::reflex_interior(labels,false,y,s,value);
    assert forall|u: int| s<=u<t implies #[trigger] good::unit_inside(labels,false,y,u,value) by {
        assert(f(u)==value);overlap_unit(labels,y,u,value);
    }
    good::encoded_chord(labels,false,y,s,t,value);
}

pub(crate) proof fn maximal_overlap_start(labels: Seq<u16>,y: int,s: int,e: int,value: u16)
    requires 0<=s<e<=64,value!=0,row_count::overlap(labels,y,s-1)!=value,
        forall|u: int| s<=u<e ==> #[trigger] row_count::overlap(labels,y,u)==value,
    ensures chain_marks::start_at(row_count::source(labels,y),e-1)==s,
{
    let f=row_count::source(labels,y);
    assert(f(e-1)==value);
    chain_marks::start_prefix(f,e-1);
    let a=chain_marks::start_at(f,e-1);
    if a<s {assert(f(s-1)==value);}
    if s<a {assert(row_count::overlap(labels,y,a-1)==value);}
}

pub(crate) proof fn complete(labels: Seq<u16>,id: int)
    requires good::good(labels,id),!good::vertical(id),
    ensures event(labels,good::line(id)*65+good::end(id)),
        event_id(labels,good::line(id)*65+good::end(id))==id,
{
    let y=good::line(id);let s=good::start(id);let e=good::end(id);let value=good::value(labels,id);
    good::endpoint_blocks(labels,false,y,s,e,value);
    overlap_unit(labels,y,s-1,value);overlap_unit(labels,y,e,value);
    assert forall|u: int| s<=u<e implies #[trigger] row_count::overlap(labels,y,u)==value by {
        assert(good::unit_inside(labels,false,y,u,value));overlap_unit(labels,y,u,value);
    }
    maximal_overlap_start(labels,y,s,e,value);
    overlap_unit(labels,y,e-1,value);
    let key=y*65+e;
    assert(key/65==y && key%65==e) by(nonlinear_arith) requires key==y*65+e,1<=e<=63;
    good::decoded_id(id);
}

proof fn injection(labels: Seq<u16>,a: int,b: int)
    requires event(labels,a),event(labels,b),event_id(labels,a)==event_id(labels,b),
    ensures a==b,
{
    actual_good(labels,a);actual_good(labels,b);
    assert(a==a/65*65+a%65);assert(b==b/65*65+b%65);
}

pub(crate) proof fn count_events(labels: Seq<u16>)
    ensures events(labels).len() as int==row_count::good_events(labels),
{
    let p=|key: int|event(labels,key);
    corners::counting::range_count(4225,p);
    let f=|t: int,y: int|chain_marks::paired(row_count::source(labels,y),row_count::marks(labels,y),t);
    corners::counting::grid_sum(65,65,f);
    corners::counting::sum_equal(4225,|key: int|bit(p(key)),|key: int|f(key%65,key/65));
    let g=|y: int|sum(65,|t: int|f(t,y));
    let h=|y: int|row_count::good_line(labels,y);
    assert forall|y: int| 0<=y<65 implies #[trigger] g(y)==#[trigger] h(y) by {
        corners::counting::sum_equal(65,|t: int|f(t,y),
            |t: int|chain_marks::paired(row_count::source(labels,y),row_count::marks(labels,y),t));
    }
    corners::counting::sum_equal(65,g,h);
}

/// 所有 H 恰好由相邻行 overlap 的双凹角端点事件枚举，顺序不影响基数。
pub(crate) proof fn complete_horizontal_count(labels: Seq<u16>)
    ensures good::chords(labels,false).len() as int==row_count::good_events(labels),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=events(labels);let f=|key: int|event_id(labels,key);let output=input.map(f);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int| input.contains(a) && input.contains(b)
            && #[trigger] f(a)==#[trigger] f(b) implies a==b by {injection(labels,a,b);}
    }
    vstd::set_lib::lemma_map_size(input,output,f);
    assert(output =~= good::chords(labels,false)) by {
        assert forall|id: int| #[trigger] output.contains(id) implies good::chords(labels,false).contains(id) by {
            let key=choose|key: int|input.contains(key) && f(key)==id;actual_good(labels,key);
        }
        assert forall|id: int| #[trigger] good::chords(labels,false).contains(id) implies output.contains(id) by {
            complete(labels,id);
            let key=good::line(id)*65+good::end(id);
            assert(input.contains(key));assert(output.contains(f(key)));
        }
    }
    count_events(labels);
}

/// 原图行 run 基数、同 key 续接与完整 H 集合的角点恒等式。
pub(crate) proof fn original_run_identity(labels: Seq<u16>)
    ensures colored::b4(labels)==4*(row_count::total_runs(labels)-row_count::identical(labels)
        +good::chords(labels,false).len() as int),
{
    row_count::base_run_count(labels);complete_horizontal_count(labels);
}

pub(crate) proof fn identical_runs(labels: Seq<u16>,y: int,t: int)
    requires 0<=y<=64,0<=t<=64,
        chain_marks::both_unmarked(row_count::source(labels,y),row_count::marks(labels,y),t)==1,
    ensures row_count::run(labels,y-1,chain_marks::start_at(row_count::source(labels,y),t-1),t,
        row_count::overlap(labels,y,t-1)),
        row_count::run(labels,y,chain_marks::start_at(row_count::source(labels,y),t-1),t,
        row_count::overlap(labels,y,t-1)),
{
    let f=row_count::source(labels,y);let s=chain_marks::start_at(f,t-1);let value=f(t-1);
    assert(f(-1)==0 && f(64)==0);assert(0<t && value!=0);
    chain_marks::start_prefix(f,t-1);
    assert forall|u: int| s<=u<t implies #[trigger] corners::at(labels,u,y-1)==value by {assert(f(u)==value);}
    assert forall|u: int| s<=u<t implies #[trigger] corners::at(labels,u,y)==value by {assert(f(u)==value);}
    assert(corners::at(labels,s,y-1)==value && corners::at(labels,s,y)==value);
    assert(1<=y<=63);
    if corners::at(labels,s-1,y-1)==value || corners::at(labels,s-1,y)==value {
        assert(good::reflex(labels,false,y,s,value));
    }
    if corners::at(labels,t,y-1)==value || corners::at(labels,t,y)==value {
        assert(good::reflex(labels,false,y,t,value));
    }
}

pub(crate) proof fn run_identical_event(labels: Seq<u16>,y: int,s: int,e: int,value: u16)
    requires row_count::run(labels,y-1,s,e,value),row_count::run(labels,y,s,e,value),
    ensures chain_marks::both_unmarked(row_count::source(labels,y),row_count::marks(labels,y),e)==1,
        chain_marks::start_at(row_count::source(labels,y),e-1)==s,
{
    assert forall|u: int| s<=u<e implies #[trigger] row_count::overlap(labels,y,u)==value by {}
    maximal_overlap_start(labels,y,s,e,value);
    assert(row_count::overlap(labels,y,e-1)==value);
}

}
