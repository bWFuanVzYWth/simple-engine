//! 竞争分区 good 链冲突的凹角/X 预算；闭端点冲突也计入。

use super::{Rectangle, chain_set, chains, colored, corners, good, partition};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn bad(labels: Seq<u16>,rectangles: Seq<Rectangle>,s: int) -> bool {
    colored::site_reflex_twice(labels,rectangles,s)==1 || colored::site_cross(labels,rectangles,s)==1
}

pub(crate) open spec fn bad_sites(labels: Seq<u16>,rectangles: Seq<Rectangle>) -> Set<int> {
    Set::<int>::range(0,16900).filter(|s: int|bad(labels,rectangles,s))
}

pub(crate) proof fn bad_count(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    ensures bad_sites(labels,rectangles).len() as int
        <=colored::reflex_twice(labels,rectangles)+colored::crosses(labels,rectangles),
{
    let p=|s: int|bad(labels,rectangles,s);
    let a=|s: int|colored::site_reflex_twice(labels,rectangles,s);
    let b=|s: int|colored::site_cross(labels,rectangles,s);
    corners::counting::range_count(16900,p);
    corners::counting::sum_order(16900,|s: int|bit(p(s)),|s: int|a(s)+b(s));
    corners::counting::sum_add(16900,a,b);
}

pub(crate) proof fn diagram_budget(a: int,b: int,c: int,d: int,la: u16,lb: u16,lc: u16,ld: u16,value: u16)
    requires value!=0,colored::consistent(a,b,c,d,la,lb,lc,ld),
        corners::count(la==value,lb==value,lc==value,ld==value)>=3,
        chains::cut_value(a,b,la,lb)==value || chains::cut_value(d,c,ld,lc)==value,
        chains::cut_value(a,d,la,ld)==value || chains::cut_value(b,c,lb,lc)==value,
        corners::count(la==value,lb==value,lc==value,ld==value)==4 ==>
            chains::cut_value(a,b,la,lb)==value && chains::cut_value(d,c,ld,lc)==value
            && chains::cut_value(a,d,la,ld)==value && chains::cut_value(b,c,lb,lc)==value,
    ensures (corners::count(la==value,lb==value,lc==value,ld==value)==3
        && corners::endpoints(colored::mask(a,la,value),colored::mask(b,lb,value),
            colored::mask(c,lc,value),colored::mask(d,ld,value))==2)
        ||corners::cross(colored::mask(a,la,value),colored::mask(b,lb,value),
            colored::mask(c,lc,value),colored::mask(d,ld,value)),
{
    if la==value {} if lb==value {} if lc==value {} if ld==value {}
}

pub(crate) open spec fn edge_site(labels: Seq<u16>,edge: int) -> int {
    let h=edge/524288;let v=edge%524288;
    colored::site_for(labels,good::line(v),good::line(h),good::value(labels,h))
}

pub(crate) proof fn intersection_budget(labels: Seq<u16>,rectangles: Seq<Rectangle>,h: int,v: int)
    requires partition(labels,rectangles),chain_set::chords(labels,rectangles).contains(h),
        chain_set::chords(labels,rectangles).contains(v),!good::vertical(h),good::vertical(v),good::intersects(h,v),
    ensures 0<=edge_site(labels,h*524288+v)<16900,bad(labels,rectangles,edge_site(labels,h*524288+v)),
{
    chain_set::actual_chord_set(labels,rectangles);
    let x=good::line(v);let y=good::line(h);let value=good::value(labels,h);
    good::crossing_value(labels,y,good::start(h),good::end(h),value,
        x,good::start(v),good::end(v),good::value(labels,v));
    good::closed_point(labels,false,y,good::start(h),good::end(h),value,x);
    colored::canonical_site(labels,x,y,value);
    let s=colored::site_for(labels,x,y,value);
    let vertex=y*65+x;
    assert(vertex%65==x && vertex/65==y) by(nonlinear_arith)
        requires vertex==y*65+x,1<=x<=63;
    colored::actual_consistent(labels,rectangles,vertex);
    chains::vertex_end_values(labels,rectangles,vertex);
    if x<good::end(h) {chain_set::chord_cut(labels,rectangles,h,x);}
    else {chain_set::chord_cut(labels,rectangles,h,x-1);}
    if y<good::end(v) {chain_set::chord_cut(labels,rectangles,v,y);}
    else {chain_set::chord_cut(labels,rectangles,v,y-1);}
    if good::quadrants(labels,x,y,value)==4 {
        assert(good::start(h)<x<good::end(h));
        assert(good::start(v)<y<good::end(v));
        chain_set::chord_cut(labels,rectangles,h,x-1);
        chain_set::chord_cut(labels,rectangles,h,x);
        chain_set::chord_cut(labels,rectangles,v,y-1);
        chain_set::chord_cut(labels,rectangles,v,y);
    }
    diagram_budget(corners::oq(rectangles,x,y,0),corners::oq(rectangles,x,y,1),
        corners::oq(rectangles,x,y,2),corners::oq(rectangles,x,y,3),
        colored::label(labels,vertex,0),colored::label(labels,vertex,1),
        colored::label(labels,vertex,2),colored::label(labels,vertex,3),value);
    assert((h*524288+v)/524288==h && (h*524288+v)%524288==v) by(nonlinear_arith)
        requires 0<=v<524288;
}

pub(crate) open spec fn conflicts(labels: Seq<u16>,rectangles: Seq<Rectangle>) -> Set<int> {
    good::conflicts(labels).filter(|edge: int|
        chain_set::chords(labels,rectangles).contains(edge/524288)
            && chain_set::chords(labels,rectangles).contains(edge%524288))
}

proof fn edge_in_bad(labels: Seq<u16>,rectangles: Seq<Rectangle>,edge: int)
    requires partition(labels,rectangles),conflicts(labels,rectangles).contains(edge),
    ensures 0<=edge_site(labels,edge)<16900,bad(labels,rectangles,edge_site(labels,edge)),
{
    intersection_budget(labels,rectangles,edge/524288,edge%524288);
    assert(edge==edge/524288*524288+edge%524288);
}

proof fn site_injection(labels: Seq<u16>,rectangles: Seq<Rectangle>,a: int,b: int)
    requires partition(labels,rectangles),conflicts(labels,rectangles).contains(a),
        conflicts(labels,rectangles).contains(b),edge_site(labels,a)==edge_site(labels,b),
    ensures a==b,
{
    let ah=a/524288;let av=a%524288;let bh=b/524288;let bv=b%524288;
    intersection_budget(labels,rectangles,ah,av);intersection_budget(labels,rectangles,bh,bv);
    colored::canonical_site(labels,good::line(av),good::line(ah),good::value(labels,ah));
    colored::canonical_site(labels,good::line(bv),good::line(bh),good::value(labels,bh));
    assert(good::line(av)==good::line(bv) && good::line(ah)==good::line(bh)) by(nonlinear_arith)
        requires 1<=good::line(av)<=63,1<=good::line(bv)<=63,1<=good::line(ah)<=63,1<=good::line(bh)<=63,
            good::line(ah)*65+good::line(av)==good::line(bh)*65+good::line(bv);
    good::conflict_point_injection(labels,ah,av,bh,bv);
    assert(a==a/524288*524288+a%524288);assert(b==b/524288*524288+b%524288);
}

/// 每个冲突点至多一条水平/竖直 good 链；r2 与 X 是实际分区中的站点。
pub(crate) proof fn actual_conflict_bound(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    requires partition(labels,rectangles),
    ensures conflicts(labels,rectangles).len() as int
        <=colored::reflex_twice(labels,rectangles)+colored::crosses(labels,rectangles),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=conflicts(labels,rectangles);
    let f=|edge: int|edge_site(labels,edge);
    let image=input.map(f);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int| input.contains(a) && input.contains(b)
            && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            site_injection(labels,rectangles,a,b);
        }
    }
    vstd::set_lib::lemma_map_size(input,image,f);
    assert(image.subset_of(bad_sites(labels,rectangles))) by {
        assert forall|s: int| #[trigger] image.contains(s) implies bad_sites(labels,rectangles).contains(s) by {
            let edge=choose|edge: int|input.contains(edge) && f(edge)==s;
            edge_in_bad(labels,rectangles,edge);
        }
    }
    vstd::set_lib::lemma_len_subset(image,bad_sites(labels,rectangles));
    bad_count(labels,rectangles);
}

}
