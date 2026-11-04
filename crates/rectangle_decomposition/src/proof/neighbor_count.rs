//! 被至少一条 V 命中的 H 按集合去重，而非按冲突边重复扣减。

use super::{continuation_count, corners, good, optimality, row_chords, row_count, split_geometry};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

proof fn point_neighbor(labels: Seq<u16>,verticals: Set<int>,h: int,x: int)
    requires split_geometry::selected(labels,verticals),good::good(labels,h),!good::vertical(h),
        good::start(h)<=x<=good::end(h),
        split_geometry::point(verticals,x,good::line(h),good::value(labels,h),labels),
    ensures optimality::neighbors(labels,verticals).contains(h),
{
    let y=good::line(h);let value=good::value(labels,h);
    let v=choose|v: int| verticals.contains(v) && #[trigger] good::line(v)==x
        && good::start(v)<=y<=good::end(v) && good::value(labels,v)==value;
    assert(good::good(labels,v));assert(good::intersects(h,v));
    assert(exists|v: int| #[trigger] verticals.contains(v) && good::intersects(h,v));
}

proof fn neighbor_point(labels: Seq<u16>,verticals: Set<int>,h: int)
    requires split_geometry::selected(labels,verticals),optimality::neighbors(labels,verticals).contains(h),
    ensures exists|x: int| good::start(h)<=x<=good::end(h)
        && #[trigger] split_geometry::point(verticals,x,good::line(h),good::value(labels,h),labels),
{
    let v=choose|v: int| verticals.contains(v) && #[trigger] good::intersects(h,v);
    assert(good::good(labels,v));let x=good::line(v);let y=good::line(h);let value=good::value(labels,h);
    good::crossing_value(labels,y,good::start(h),good::end(h),value,
        x,good::start(v),good::end(v),good::value(labels,v));
    assert(split_geometry::point(verticals,x,y,value,labels));
}

pub(crate) proof fn hit_neighbor(labels: Seq<u16>,verticals: Set<int>,key: int)
    requires split_geometry::selected(labels,verticals),row_chords::event(labels,key),
    ensures (continuation_count::hit(row_count::source(labels,key/65),
        continuation_count::selected_points(labels,verticals,key/65),key%65-1)
            ||split_geometry::point(verticals,key%65,key/65,row_count::overlap(labels,key/65,key%65-1),labels))
        <==>optimality::neighbors(labels,verticals).contains(row_chords::event_id(labels,key)),
{
    hide(good::chord);
    let y=key/65;let t=key%65;let f=row_count::source(labels,y);
    let b=continuation_count::selected_points(labels,verticals,y);let h=row_chords::event_id(labels,key);
    row_chords::actual_good(labels,key);let value=good::value(labels,h);let s=good::start(h);
    assert(f(-1)==0 && f(64)==0);assert(0<t<=64 && f(t-1)!=0);
    super::chain_marks::start_prefix(f,t-1);
    continuation_count::hit_exists(f,b,t-1);
    if continuation_count::hit(f,b,t-1) {
        let x=choose|x: int|s<=x<=t-1 && #[trigger] b(x,f(t-1));
        point_neighbor(labels,verticals,h,x);
    }
    if split_geometry::point(verticals,t,y,value,labels) {
        point_neighbor(labels,verticals,h,t);
    }
    if optimality::neighbors(labels,verticals).contains(h) {
        neighbor_point(labels,verticals,h);
        let x=choose|x: int| s<=x<=t && #[trigger] split_geometry::point(verticals,x,y,value,labels);
        if x<t {
            assert(b(x,f(t-1)));
            assert(exists|x: int|s<=x<=t-1 && #[trigger] b(x,f(t-1)));
        }
    }
}

pub(crate) open spec fn blocked_event(labels: Seq<u16>,verticals: Set<int>,key: int) -> bool {
    row_chords::event(labels,key) && optimality::neighbors(labels,verticals).contains(row_chords::event_id(labels,key))
}

pub(crate) open spec fn blocked_keys(labels: Seq<u16>,verticals: Set<int>) -> Set<int> {
    Set::<int>::range(0,4225).filter(|key: int|blocked_event(labels,verticals,key))
}

pub(crate) proof fn blocked_value(labels: Seq<u16>,verticals: Set<int>,key: int)
    requires split_geometry::selected(labels,verticals),0<=key<4225,
    ensures continuation_count::blocked(row_count::source(labels,key/65),row_count::marks(labels,key/65),
        continuation_count::selected_points(labels,verticals,key/65),key%65)==bit(blocked_event(labels,verticals,key)),
{
    if row_chords::event(labels,key) {hit_neighbor(labels,verticals,key);}
}

pub(crate) proof fn blocked_neighbors_count(labels: Seq<u16>,verticals: Set<int>)
    requires split_geometry::selected(labels,verticals),
    ensures sum(65,|y: int|sum(65,|x: int|continuation_count::blocked(row_count::source(labels,y),row_count::marks(labels,y),
        continuation_count::selected_points(labels,verticals,y),x)))==optimality::neighbors(labels,verticals).len() as int,
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=blocked_keys(labels,verticals);let f=|key: int|row_chords::event_id(labels,key);let image=input.map(f);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int| input.contains(a) && input.contains(b)
            && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            row_chords::actual_good(labels,a);row_chords::actual_good(labels,b);
            assert(a==a/65*65+a%65);assert(b==b/65*65+b%65);
        }
    }
    vstd::set_lib::lemma_map_size(input,image,f);
    assert(image =~=optimality::neighbors(labels,verticals)) by {
        assert forall|id: int| #[trigger] image.contains(id) implies optimality::neighbors(labels,verticals).contains(id) by {
            let key=choose|key: int|input.contains(key) && f(key)==id;
        }
        assert forall|id: int| #[trigger] optimality::neighbors(labels,verticals).contains(id) implies image.contains(id) by {
            row_chords::complete(labels,id);let key=good::line(id)*65+good::end(id);
            assert(input.contains(key));assert(image.contains(f(key)));
        }
    }
    let p=|key: int|blocked_event(labels,verticals,key);
    corners::counting::range_count(4225,p);
    let g=|x: int,y: int|continuation_count::blocked(row_count::source(labels,y),row_count::marks(labels,y),
        continuation_count::selected_points(labels,verticals,y),x);
    corners::counting::grid_sum(65,65,g);
    let a=|key: int|bit(p(key));let b=|key: int|g(key%65,key/65);
    assert forall|key: int| 0<=key<4225 implies #[trigger] a(key)==#[trigger] b(key) by {blocked_value(labels,verticals,key);}
    corners::counting::sum_equal(4225,a,b);
    let c=|y: int|sum(65,|x: int|g(x,y));
    let d=|y: int|sum(65,|x: int|continuation_count::blocked(row_count::source(labels,y),row_count::marks(labels,y),
        continuation_count::selected_points(labels,verticals,y),x));
    assert forall|y: int| 0<=y<65 implies #[trigger] c(y)==#[trigger] d(y) by {
        corners::counting::sum_equal(65,|x: int|g(x,y),
            |x: int|continuation_count::blocked(row_count::source(labels,y),row_count::marks(labels,y),
                continuation_count::selected_points(labels,verticals,y),x));
    }
    corners::counting::sum_equal(65,c,d);
}

}
