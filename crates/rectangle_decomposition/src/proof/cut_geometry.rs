//! 原始切线、good chord 集合与实际扫描 Ready 的几何连接。

use super::{AxisIntervals, HorizontalCut, VerticalCut, mask_proof as masks, scan};
use super::{proof as axis, scan_proof as bits};
use crate::partition::{good, optimality, scan_count, scan_model, split_geometry};
use vstd::prelude::*;

verus! {

pub(super) open spec fn horizontal_id(cut: HorizontalCut) -> int {
    good::encode(false,cut.y as int,cut.start as int,cut.end as int)
}

pub(super) open spec fn vertical_id(cut: VerticalCut) -> int {
    good::encode(true,cut.x as int,cut.start as int,cut.end as int)
}

pub(super) open spec fn horizontal_ids(cuts: Seq<HorizontalCut>) -> Set<int> {
    Set::<int>::range(0,cuts.len() as int).map(|i: int|horizontal_id(cuts[i]))
}

pub(super) open spec fn vertical_ids(cuts: Seq<VerticalCut>) -> Set<int> {
    Set::<int>::range(0,cuts.len() as int).map(|i: int|vertical_id(cuts[i]))
}

proof fn coordinates(vertical: bool,line: int,start: int,end: int)
    requires 1<=line<=63,1<=start<end<=63,
    ensures 0<=good::encode(vertical,line,start,end)<524288,
        good::vertical(good::encode(vertical,line,start,end))==vertical,
        good::line(good::encode(vertical,line,start,end))==line,
        good::start(good::encode(vertical,line,start,end))==start,
        good::end(good::encode(vertical,line,start,end))==end,
{
    let id=good::encode(vertical,line,start,end);
    assert(id%262144==4096*line+64*start+end) by(nonlinear_arith)
        requires id==(if vertical{262144int}else{0})+4096*line+64*start+end,
            1<=line<=63,1<=start<end<=63;
    assert((4096*line+64*start+end)/4096==line) by(nonlinear_arith) requires 1<=start<end<=63;
    assert(id%4096==64*start+end) by(nonlinear_arith)
        requires id==(if vertical{262144int}else{0})+4096*line+64*start+end,
            1<=line<=63,1<=start<end<=63;
    assert((64*start+end)/64==start) by(nonlinear_arith) requires 1<=end<=63;
    assert(id%64==end) by(nonlinear_arith)
        requires id==(if vertical{262144int}else{0})+4096*line+64*start+end,
            1<=line<=63,1<=start<end<=63;
}

pub(super) proof fn horizontal_coordinates(cut: HorizontalCut)
    requires masks::horizontal_valid(cut),
    ensures !good::vertical(horizontal_id(cut)),good::line(horizontal_id(cut))==cut.y,
        good::start(horizontal_id(cut))==cut.start,good::end(horizontal_id(cut))==cut.end,
{
    coordinates(false,cut.y as int,cut.start as int,cut.end as int);
}

pub(super) proof fn vertical_coordinates(cut: VerticalCut)
    requires masks::vertical_valid(cut),
    ensures good::vertical(vertical_id(cut)),good::line(vertical_id(cut))==cut.x,
        good::start(vertical_id(cut))==cut.start,good::end(vertical_id(cut))==cut.end,
{
    coordinates(true,cut.x as int,cut.start as int,cut.end as int);
}

pub(super) proof fn horizontal_member(cuts: Seq<HorizontalCut>,i: int)
    requires 0<=i<cuts.len(),
    ensures horizontal_ids(cuts).contains(horizontal_id(cuts[i])),
{
    broadcast use Set::lemma_map_contains;
    assert(Set::<int>::range(0,cuts.len() as int).contains(i));
}

pub(super) proof fn vertical_member(cuts: Seq<VerticalCut>,i: int)
    requires 0<=i<cuts.len(),
    ensures vertical_ids(cuts).contains(vertical_id(cuts[i])),
{
    broadcast use Set::lemma_map_contains;
    assert(Set::<int>::range(0,cuts.len() as int).contains(i));
}

pub(super) proof fn vertical_active(cuts: Seq<VerticalCut>,ids: Set<int>,row: int,x: u32)
    requires masks::vertical_input(cuts),ids==vertical_ids(cuts),
    ensures masks::active(cuts,cuts.len() as int,row,x) <==>split_geometry::active(ids,x as int,row),
{
    broadcast use Set::lemma_map_contains;
    if masks::active(cuts,cuts.len() as int,row,x) {
        let i=choose|i: int|0<=i<cuts.len() && #[trigger] cuts[i].x==x && cuts[i].start<=row<cuts[i].end;
        vertical_coordinates(cuts[i]);vertical_member(cuts,i);
        assert(exists|id: int|#[trigger] ids.contains(id) && good::line(id)==x && good::start(id)<=row<good::end(id));
    }
    if split_geometry::active(ids,x as int,row) {
        let id=choose|id: int|ids.contains(id) && #[trigger] good::line(id)==x && good::start(id)<=row<good::end(id);
        let i=choose|i: int|Set::<int>::range(0,cuts.len() as int).contains(i) && #[trigger] vertical_id(cuts[i])==id;
        vertical_coordinates(cuts[i]);assert(masks::active(cuts,cuts.len() as int,row,x));
    }
}

proof fn same_vertical_at(labels: Seq<u16>,a: int,b: int,y: int)
    requires good::good(labels,a),good::good(labels,b),good::vertical(a),good::vertical(b),
        good::line(a)==good::line(b),good::start(a)<=y<=good::end(a),good::start(b)<=y<=good::end(b),
    ensures a==b,
{
    good::decoded_id(a);good::decoded_id(b);
    if a!=b {good::same_line_disjoint(labels,true,good::line(a),good::start(a),good::end(a),good::value(labels,a),
        good::start(b),good::end(b),good::value(labels,b));}
}

pub(super) open spec fn vertical_exact(vertical: Seq<u128>,ids: Set<int>) -> bool {
    vertical.len()==64 && forall|row: int,x: u32|0<=row<64 && x<=64
        ==> (#[trigger] bits::bit128(vertical[row],x) <==>split_geometry::active(ids,x as int,row))
}

pub(super) proof fn built_vertical(horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>,hm: Seq<u64>,vm: Seq<u128>,boundaries: u64)
    requires masks::built(horizontal,vertical,hm,vm,boundaries),
    ensures vertical_exact(vm,vertical_ids(vertical)),
{
    assert forall|row: int,x: u32|0<=row<64 && x<=64 implies
        (#[trigger] bits::bit128(vm[row],x) <==>split_geometry::active(vertical_ids(vertical),x as int,row)) by {
        masks::coverage_bit(vertical,vertical.len() as int,row,x);
        vertical_active(vertical,vertical_ids(vertical),row,x);
    }
}

pub(super) proof fn derived_vertical(labels: Seq<u16>,vertical: Seq<u128>,ids: Set<int>)
    requires ids.subset_of(good::chords(labels,true)),vertical_exact(vertical,ids),
    ensures scan::proof::selection(labels,vertical) =~=ids,scan::proof::masks_match(labels,vertical),
{
    let derived=scan::proof::selection(labels,vertical);
    assert forall|id: int|#[trigger] derived.contains(id) implies ids.contains(id) by {
        let row=good::start(id);let x=good::line(id) as u32;
        assert(bits::bit128(vertical[row],x));assert(split_geometry::active(ids,x as int,row));
        let other=choose|other: int|ids.contains(other) && #[trigger] good::line(other)==x && good::start(other)<=row<good::end(other);
        assert(good::good(labels,other));same_vertical_at(labels,id,other,row);
    }
    assert forall|id: int|#[trigger] ids.contains(id) implies derived.contains(id) by {
        let row=good::start(id);let x=good::line(id) as u32;
        assert(good::good(labels,id));
        assert(exists|other: int|#[trigger] ids.contains(other) && good::line(other)==x && good::start(other)<=row<good::end(other));
        assert(split_geometry::active(ids,x as int,row));assert(bits::bit128(vertical[row],x));
    }
}

pub(super) proof fn horizontal_empty(horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>,hm: Seq<u64>,vm: Seq<u128>,boundaries: u64,row: u8)
    requires masks::built(horizontal,vertical,hm,vm,boundaries),row<64,!bits::bit64(boundaries,row),
    ensures hm[row as int]==0,
{
    let mask=hm[row as int];
    if mask!=0 {
        vstd::std_specs::bits::axiom_u64_trailing_zeros(mask);
        let x=vstd::std_specs::bits::u64_trailing_zeros(mask) as u8;
        assert(x<64 && bits::bit64(mask,x));assert(masks::horizontal_active(horizontal,horizontal.len() as int,row as int,x));
        let i=choose|i: int|0<=i<horizontal.len() && #[trigger] horizontal[i].y==row && horizontal[i].start<=x<horizontal[i].end;
        assert(bits::bit64(boundaries,horizontal[i].y));
    }
}

pub(super) proof fn unmarked_events(vertical: Seq<VerticalCut>,count: int,boundaries: u64,row: u8)
    requires masks::vertical_input(vertical),0<=count<=vertical.len(),row<64,!bits::bit64(boundaries,row),
        masks::vertical_marked(vertical,count,boundaries),
    ensures masks::events(vertical,count,row as int)==0,
    decreases count,
{
    if count>0 {
        unmarked_events(vertical,count-1,boundaries,row);let cut=vertical[count-1];
        assert(bits::bit64(boundaries,cut.start) && bits::bit64(boundaries,cut.end));
        assert(cut.start!=row && cut.end!=row);
        assert((0u128^0u128)==0) by(bit_vector);
    }
}

pub(super) proof fn vertical_unchanged(horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>,hm: Seq<u64>,vm: Seq<u128>,boundaries: u64,row: u8)
    requires masks::built(horizontal,vertical,hm,vm,boundaries),0<row<64,!bits::bit64(boundaries,row),
    ensures vm[row as int]==vm[row as int-1],
{
    unmarked_events(vertical,vertical.len() as int,boundaries,row);
    masks::coverage_transition(vertical,vertical.len() as int,row as int);
    let before=vm[row as int-1];assert(before^0u128==before) by(bit_vector);
}

pub(super) open spec fn selected_cuts(selected: Set<int>,horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>) -> bool {
    horizontal_ids(horizontal)==optimality::horizontal_part(selected) && vertical_ids(vertical)==optimality::vertical_part(selected)
}

pub(super) proof fn ready(rows: &AxisIntervals,selected: Set<int>,horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>,
    hm: Seq<u64>,vm: Seq<u128>,boundaries: u64)
    requires rows.valid(),rows.lines@.len()==64,rows.boundaries_complete(),good::independent(rows.labels(),selected),
        selected_cuts(selected,horizontal,vertical),masks::built(horizontal,vertical,hm,vm,boundaries),
    ensures scan::proof::ready(rows,hm,vm,boundaries),scan::proof::selection(rows.labels(),vm)==optimality::vertical_part(selected),
{
    broadcast use vstd::set_lib::group_set_properties;
    built_vertical(horizontal,vertical,hm,vm,boundaries);let vs=optimality::vertical_part(selected);
    assert(vs.subset_of(good::chords(rows.labels(),true)));derived_vertical(rows.labels(),vm,vs);
    let combined=rows.boundaries|boundaries;
    assert forall|y: u64|0<y<64 && !#[trigger] axis::bit(combined,y) implies scan::proof::smooth(rows,hm,vm,y as int) by {
        let row=y as u8;let own=rows.boundaries;
        assert(!axis::bit(own,y) && !bits::bit64(boundaries,row)) by(bit_vector)
            requires combined==own|boundaries,(combined>>y)&1!=1,0<y<64,row==y as u8;
        horizontal_empty(horizontal,vertical,hm,vm,boundaries,row);vertical_unchanged(horizontal,vertical,hm,vm,boundaries,row);
    }
}

proof fn blocked_to_selected(horizontal: Seq<HorizontalCut>,hm: Seq<u64>,boundaries: u64,
    selected: Set<int>,y: int,a: int,b: int)
    requires masks::horizontal_progress(horizontal,horizontal.len() as int,hm,boundaries),
        horizontal_ids(horizontal)==optimality::horizontal_part(selected),0<=y<64,0<=a<b<=64,
        scan::proof::blocked(hm,y,a,b),
    ensures scan_count::horizontal_block(selected,y,a,b),
{
    broadcast use vstd::set_lib::group_set_properties;
    let x=choose|x: u8|x<64 && (a as u8)<=x<(b as u8) && #[trigger] bits::bit64(hm[y],x);
    assert(masks::horizontal_active(horizontal,horizontal.len() as int,y,x));
    let i=choose|i: int|0<=i<horizontal.len() && #[trigger] horizontal[i].y==y && horizontal[i].start<=x<horizontal[i].end;
    horizontal_coordinates(horizontal[i]);horizontal_member(horizontal,i);let id=horizontal_id(horizontal[i]);
    assert(selected.contains(id));
    assert(exists|h: int|#[trigger] selected.contains(h) && !good::vertical(h) && good::line(h)==y && good::start(h)<b && a<good::end(h));
}

proof fn selected_to_blocked(horizontal: Seq<HorizontalCut>,hm: Seq<u64>,boundaries: u64,
    selected: Set<int>,y: int,a: int,b: int)
    requires masks::horizontal_progress(horizontal,horizontal.len() as int,hm,boundaries),
        horizontal_ids(horizontal)==optimality::horizontal_part(selected),0<=y<64,0<=a<b<=64,
        scan_count::horizontal_block(selected,y,a,b),
    ensures scan::proof::blocked(hm,y,a,b),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let id=choose|id: int|selected.contains(id) && !good::vertical(id) && #[trigger] good::line(id)==y && good::start(id)<b && a<good::end(id);
    assert(horizontal_ids(horizontal).contains(id));
    let i=choose|i: int|Set::<int>::range(0,horizontal.len() as int).contains(i) && #[trigger] horizontal_id(horizontal[i])==id;
    horizontal_coordinates(horizontal[i]);let cut=horizontal[i];let point=if a>cut.start{a}else{cut.start as int};let x=point as u8;
    assert(0<=point<64 && a<=point<b && cut.start<=point<cut.end);
    assert(masks::horizontal_active(horizontal,horizontal.len() as int,y,x));assert(bits::bit64(hm[y],x));
    assert(exists|x: u8|x<64 && (a as u8)<=x<(b as u8) && #[trigger] bits::bit64(hm[y],x));
}

pub(super) proof fn blocked(horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>,hm: Seq<u64>,vm: Seq<u128>,boundaries: u64,
    selected: Set<int>,y: int,a: int,b: int)
    requires masks::built(horizontal,vertical,hm,vm,boundaries),horizontal_ids(horizontal)==optimality::horizontal_part(selected),
        0<=y<64,0<=a<b<=64,
    ensures scan::proof::blocked(hm,y,a,b) <==>scan_count::horizontal_block(selected,y,a,b),
{
    if scan::proof::blocked(hm,y,a,b) {blocked_to_selected(horizontal,hm,boundaries,selected,y,a,b);}
    if scan_count::horizontal_block(selected,y,a,b) {selected_to_blocked(horizontal,hm,boundaries,selected,y,a,b);}
}

pub(super) proof fn unblocked(rows: &AxisIntervals,selected: Set<int>,horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>,
    hm: Seq<u64>,vm: Seq<u128>,boundaries: u64)
    requires rows.valid(),rows.lines@.len()==64,rows.boundaries_complete(),good::independent(rows.labels(),selected),
        selected_cuts(selected,horizontal,vertical),masks::built(horizontal,vertical,hm,vm,boundaries),
    ensures scan_count::unblocked(rows.labels(),scan::proof::selection(rows.labels(),vm),scan::proof::block_fn(hm)),
{
    ready(rows,selected,horizontal,vertical,hm,vm,boundaries);scan_count::independent_unblocked(rows.labels(),selected);
    let vs=optimality::vertical_part(selected);
    assert forall|y: int,a: int,b: int,value: u16| #[trigger] split_geometry::run(rows.labels(),vs,y-1,a,b,value)
        && #[trigger] split_geometry::run(rows.labels(),vs,y,a,b,value) implies !scan::proof::blocked(hm,y,a,b) by {
        blocked(horizontal,vertical,hm,vm,boundaries,selected,y,a,b);
        assert(!scan_count::horizontal_block(selected,y,a,b));
    }
}

/// Raw cut 核与实际 scanner 的 canonical 最优性连接；不信任任何执行算法结论。
pub(super) proof fn prepared_optimal(rows: &AxisIntervals,selected: Set<int>,horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>,
    hm: Seq<u64>,vm: Seq<u128>,boundaries: u64)
    requires rows.valid(),rows.lines@.len()==64,rows.boundaries_complete(),good::maximum_independent(rows.labels(),selected),
        selected_cuts(selected,horizontal,vertical),masks::built(horizontal,vertical,hm,vm,boundaries),
    ensures scan::proof::ready(rows,hm,vm,boundaries),
        scan::proof::selection(rows.labels(),vm)==optimality::vertical_part(selected),
        crate::partition::optimal(rows.labels(),scan_model::canonical(rows.labels(),scan::proof::selection(rows.labels(),vm),scan::proof::block_fn(hm))),
{
    ready(rows,selected,horizontal,vertical,hm,vm,boundaries);unblocked(rows,selected,horizontal,vertical,hm,vm,boundaries);
    scan_count::canonical_optimal(rows.labels(),selected,scan::proof::block_fn(hm));
}

#[cfg(reject_cut_wrong_selection)]
proof fn reject_wrong_selection(rows: &AxisIntervals,selected: Set<int>,horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>,
    hm: Seq<u64>,vm: Seq<u128>,boundaries: u64)
    requires rows.valid(),rows.lines@.len()==64,rows.boundaries_complete(),good::independent(rows.labels(),selected),
        selected_cuts(selected,horizontal,vertical),masks::built(horizontal,vertical,hm,vm,boundaries),
{
    ready(rows,selected,horizontal,vertical,hm,vm,boundaries);
    assert(scan::proof::selection(rows.labels(),vm)!=optimality::vertical_part(selected));
}

#[cfg(reject_cut_wrong_block)]
proof fn reject_wrong_block(horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>,hm: Seq<u64>,vm: Seq<u128>,boundaries: u64,
    selected: Set<int>,y: int,a: int,b: int)
    requires masks::built(horizontal,vertical,hm,vm,boundaries),horizontal_ids(horizontal)==optimality::horizontal_part(selected),
        0<=y<64,0<=a<b<=64,
{
    blocked(horizontal,vertical,hm,vm,boundaries,selected,y,a,b);
    assert(scan::proof::blocked(hm,y,a,b)!=scan_count::horizontal_block(selected,y,a,b));
}

}
