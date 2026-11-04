//! 选中竖线的开放单位行与闭端点数；有限求和不产生运行期遍历。

use super::{corners, good, split_geometry};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn included(id: int,y: int,closed: bool) -> bool {
    good::start(id)<=y<good::end(id)+(if closed{1int}else{0})
}

pub(crate) open spec fn active_set(verticals: Set<int>,y: int,closed: bool) -> Set<int> {
    verticals.filter(|id: int|included(id,y,closed))
}

pub(crate) open spec fn coordinate(verticals: Set<int>,x: int,y: int,closed: bool) -> bool {
    exists|id: int| #[trigger] verticals.contains(id) && good::line(id)==x && included(id,y,closed)
}

pub(crate) open spec fn row_count(verticals: Set<int>,y: int,closed: bool) -> int {
    sum(65,|x: int|bit(coordinate(verticals,x,y,closed)))
}

pub(crate) open spec fn total(verticals: Set<int>,closed: bool) -> int {
    sum(65,|y: int|row_count(verticals,y,closed))
}

pub(crate) open spec fn load(verticals: Set<int>,closed: bool) -> int {
    sum(524288,|id: int|if verticals.contains(id) {good::end(id)-good::start(id)+(if closed{1int}else{0})}else{0})
}

proof fn coordinate_unique(labels: Seq<u16>,verticals: Set<int>,a: int,b: int,y: int,closed: bool)
    requires split_geometry::selected(labels,verticals),active_set(verticals,y,closed).contains(a),
        active_set(verticals,y,closed).contains(b),good::line(a)==good::line(b),
    ensures a==b,
{
    assert(good::good(labels,a) && good::good(labels,b));
    good::decoded_id(a);good::decoded_id(b);
    if a!=b {
        good::same_line_disjoint(labels,true,good::line(a),good::start(a),good::end(a),good::value(labels,a),
            good::start(b),good::end(b),good::value(labels,b));
    }
}

proof fn position_bound(labels: Seq<u16>,verticals: Set<int>,id: int)
    requires split_geometry::selected(labels,verticals),verticals.contains(id),
    ensures 1<=good::line(id)<=63,
{}

pub(crate) proof fn row_cardinality(labels: Seq<u16>,verticals: Set<int>,y: int,closed: bool)
    requires split_geometry::selected(labels,verticals),
    ensures row_count(verticals,y,closed)==active_set(verticals,y,closed).len() as int,
{
    hide(good::chord);
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=active_set(verticals,y,closed);let f=|id: int|good::line(id);let image=input.map(f);
    let slots=Set::<int>::range(0,65).filter(|x: int|coordinate(verticals,x,y,closed));
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int| input.contains(a) && input.contains(b)
            && #[trigger] f(a)==#[trigger] f(b) implies a==b by {coordinate_unique(labels,verticals,a,b,y,closed);}
    }
    vstd::set_lib::lemma_map_size(input,image,f);
    assert(image =~=slots) by {
        assert forall|x: int| #[trigger] image.contains(x) implies slots.contains(x) by {
            let id=choose|id: int|input.contains(id) && f(id)==x;
            position_bound(labels,verticals,id);
        }
        assert forall|x: int| #[trigger] slots.contains(x) implies image.contains(x) by {
            let id=choose|id: int|verticals.contains(id) && #[trigger] good::line(id)==x && included(id,y,closed);
            assert(input.contains(id));assert(image.contains(f(id)));
        }
    }
    let p=|x: int|coordinate(verticals,x,y,closed);
    corners::counting::range_count(65,p);
    corners::counting::sum_equal(65,|x: int|bit(p(x)),|x: int|bit(coordinate(verticals,x,y,closed)));
}

pub(crate) proof fn vertex_cardinality(labels: Seq<u16>,verticals: Set<int>,y: int,closed: bool)
    requires split_geometry::selected(labels,verticals),
    ensures row_count(verticals,y,closed)==sum(524288,|id: int|bit(verticals.contains(id) && included(id,y,closed))),
{
    broadcast use vstd::set_lib::group_set_properties;
    row_cardinality(labels,verticals,y,closed);
    let p=|id: int|verticals.contains(id) && included(id,y,closed);
    corners::counting::range_count(524288,p);
    assert(Set::<int>::range(0,524288).filter(p) =~=active_set(verticals,y,closed));
    corners::counting::sum_equal(524288,|id: int|bit(p(id)),
        |id: int|bit(verticals.contains(id) && included(id,y,closed)));
}

pub(crate) proof fn total_load(labels: Seq<u16>,verticals: Set<int>,closed: bool)
    requires split_geometry::selected(labels,verticals),
    ensures total(verticals,closed)==load(verticals,closed),
{
    let f=|y: int,id: int|bit(verticals.contains(id) && included(id,y,closed));
    corners::counting::sum_swap(65,524288,f);
    let a=|y: int|row_count(verticals,y,closed);let b=|y: int|sum(524288,|id: int|f(y,id));
    assert forall|y: int| 0<=y<65 implies #[trigger] a(y)==#[trigger] b(y) by {
        vertex_cardinality(labels,verticals,y,closed);
        corners::counting::sum_equal(524288,|id: int|f(y,id),
            |id: int|bit(verticals.contains(id) && included(id,y,closed)));
    }
    corners::counting::sum_equal(65,a,b);
    let g=|id: int|sum(65,|y: int|f(y,id));
    let h=|id: int|if verticals.contains(id) {good::end(id)-good::start(id)+(if closed{1int}else{0})}else{0};
    assert forall|id: int| 0<=id<524288 implies #[trigger] g(id)==#[trigger] h(id) by {
        if verticals.contains(id) {
            assert(good::good(labels,id));
            corners::counting::sum_interval(65,good::start(id),good::end(id)+(if closed{1int}else{0}));
            corners::counting::sum_equal(65,|y: int|f(y,id),
                |y: int|bit(good::start(id)<=y<good::end(id)+(if closed{1int}else{0})));
        } else {
            corners::counting::sum_equal(65,|y: int|f(y,id),|_: int|0);
            corners::counting::sum_constant(65,0);
        }
    }
    corners::counting::sum_equal(524288,g,h);
}

/// 闭点总数比开放单位行数恰好多一份选中线数量，端点不能漏计。
pub(crate) proof fn closed_minus_open(labels: Seq<u16>,verticals: Set<int>)
    requires split_geometry::selected(labels,verticals),
    ensures total(verticals,true)==total(verticals,false)+verticals.len(),
{
    broadcast use vstd::set_lib::group_set_properties;
    total_load(labels,verticals,true);total_load(labels,verticals,false);
    let p=|id: int|verticals.contains(id);
    corners::counting::range_count(524288,p);
    assert(Set::<int>::range(0,524288).filter(p) =~=verticals);
    let a=|id: int|if verticals.contains(id) {good::end(id)-good::start(id)}else{0};
    let b=|id: int|bit(verticals.contains(id));
    corners::counting::sum_equal(524288,|id: int|bit(p(id)),b);
    corners::counting::sum_add(524288,a,b);
    corners::counting::sum_equal(524288,|id: int|a(id)+b(id),
        |id: int|if verticals.contains(id) {good::end(id)-good::start(id)+1}else{0});
    corners::counting::sum_equal(524288,a,
        |id: int|if verticals.contains(id) {good::end(id)-good::start(id)+0}else{0});
}

pub(crate) proof fn outside_row(labels: Seq<u16>,verticals: Set<int>,closed: bool)
    requires split_geometry::selected(labels,verticals),
    ensures row_count(verticals,64,closed)==0,
{
    broadcast use vstd::set_lib::group_set_properties;
    row_cardinality(labels,verticals,64,closed);
    assert(active_set(verticals,64,closed) =~= Set::<int>::empty());
}

}
