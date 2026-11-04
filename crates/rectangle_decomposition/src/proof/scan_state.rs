//! 拉取扫描的不可变输入视图、原缓冲前沿及 canonical 前缀连接。

use super::super::{AxisIntervals, proof as axis, scan_proof as raw};
use super::RectScan;
use crate::optimal::types::{ActiveRect, Run};
use crate::partition::{corners, good, scan_model, split_geometry};
use vstd::prelude::*;

verus! {

pub(in crate::optimal::sparse) open spec fn selection(labels: Seq<u16>,vertical: Seq<u128>) -> Set<int> {
    good::chords(labels,true).filter(|id: int|good::start(id)<vertical.len()
        && raw::bit128(vertical[good::start(id)],good::line(id) as u32))
}

pub(in crate::optimal::sparse) open spec fn masks_match(labels: Seq<u16>,vertical: Seq<u128>) -> bool {
    vertical.len()==64 && forall|y: int,x: u32|0<=y<64 && x<=64
        ==> (#[trigger] raw::bit128(vertical[y],x) <==> split_geometry::active(selection(labels,vertical),x as int,y))
}

pub(in crate::optimal::sparse) open spec fn blocked(horizontal: Seq<u64>,y: int,a: int,b: int) -> bool {
    0<=y<64 && 0<=a<b<=64 && raw::overlaps(horizontal[y],a as u8,b as u8)
}

pub(in crate::optimal::sparse) open spec fn block_fn(horizontal: Seq<u64>) -> spec_fn(int,int,int)->bool {
    |y: int,a: int,b: int|blocked(horizontal,y,a,b)
}

pub(in crate::optimal::sparse) open spec fn smooth(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,y: int) -> bool {
    rows.line_spec(y)==rows.line_spec(y-1) && horizontal[y]==0 && vertical[y]==vertical[y-1]
}

pub(in crate::optimal::sparse) open spec fn ready(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,cut_boundaries: u64) -> bool {
    rows.valid() && rows.lines@.len()==64 && rows.boundaries_complete() && horizontal.len()==64
        && masks_match(rows.labels(),vertical)
        && forall|y: u64|0<y<64 && !#[trigger] axis::bit(rows.boundaries|cut_boundaries,y)
            ==> smooth(rows,horizontal,vertical,y as int)
}

pub(in crate::optimal::sparse) open spec fn row_runs(labels: Seq<u16>,selected: Set<int>,y: int,runs: Seq<Run>) -> bool {
    raw::ordered(runs) && runs.len()<=64
        && (forall|i: int|0<=i<runs.len() ==> #[trigger] split_geometry::run(labels,selected,y,
            runs[i].x_start as int,runs[i].x_end as int,runs[i].value))
        && (forall|a: int,b: int,value: u16| #[trigger] split_geometry::run(labels,selected,y,a,b,value)
            ==> exists|i: int|0<=i<runs.len() && #[trigger] runs[i]==Run{value,x_start:a as u8,x_end:b as u8})
}

proof fn line_background(line: Seq<super::super::Interval>,p: int)
    requires axis::line_valid(line),p<0 || p>=64,
    ensures axis::line_value(line,p)==0,
{
    if exists|i: int|0<=i<line.len() && #[trigger] line[i].start<=p<line[i].end {
        let i=choose|i: int|0<=i<line.len() && #[trigger] line[i].start<=p<line[i].end;
        assert(axis::interval_valid(line[i]));
    }
}

pub(in crate::optimal::sparse) proof fn row_label(rows: &AxisIntervals,y: int,p: int)
    requires rows.valid(),rows.lines@.len()==64,0<=y<64,
    ensures axis::line_value(rows.line_spec(y),p)==corners::at(rows.labels(),p,y),
{
    if 0<=p<64 {
        let pixel=y*64+p;
        assert(pixel%64==p && pixel/64==y) by(nonlinear_arith) requires pixel==y*64+p,0<=p<64;
    }else{line_background(rows.line_spec(y),p);}
}

pub(in crate::optimal::sparse) proof fn raw_run_math(rows: &AxisIntervals,vertical: Seq<u128>,y: int,run: Run)
    requires rows.valid(),rows.lines@.len()==64,masks_match(rows.labels(),vertical),0<=y<64,
    ensures raw::raw_run(rows.line_spec(y),vertical[y],run) <==> split_geometry::run(rows.labels(),selection(rows.labels(),vertical),y,
        run.x_start as int,run.x_end as int,run.value),
{
    let line=rows.line_spec(y);let mask=vertical[y];let selected=selection(rows.labels(),vertical);
    if raw::raw_run(line,mask,run) || split_geometry::run(rows.labels(),selected,y,run.x_start as int,run.x_end as int,run.value) {
        row_label(rows,y,run.x_start as int-1);row_label(rows,y,run.x_end as int);
        assert forall|p: int|run.x_start<=p<run.x_end implies
            #[trigger] axis::line_value(line,p)==corners::at(rows.labels(),p,y) by {row_label(rows,y,p);}
        assert forall|p: int|run.x_start<p<run.x_end implies
            #[trigger] split_geometry::active(selected,p,y) <==> raw::bit128(mask,p as u32) by {}
        assert forall|p: u32|run.x_start<p<run.x_end implies
            #[trigger] raw::bit128(mask,p) <==> split_geometry::active(selected,p as int,y) by {}
    }
}

pub(in crate::optimal::sparse) proof fn raw_row_math(rows: &AxisIntervals,vertical: Seq<u128>,y: int,runs: Seq<Run>)
    requires rows.valid(),rows.lines@.len()==64,masks_match(rows.labels(),vertical),0<=y<64,
        raw::progress(rows.line_spec(y),vertical[y],runs,64),
    ensures row_runs(rows.labels(),selection(rows.labels(),vertical),y,runs),
{
    let selected=selection(rows.labels(),vertical);let line=rows.line_spec(y);
    assert forall|i: int|0<=i<runs.len() implies #[trigger] split_geometry::run(rows.labels(),selected,y,
        runs[i].x_start as int,runs[i].x_end as int,runs[i].value) by {
        assert(raw::raw_run(line,vertical[y],runs[i]));raw_run_math(rows,vertical,y,runs[i]);
    }
    assert forall|a: int,b: int,value: u16| #[trigger] split_geometry::run(rows.labels(),selected,y,a,b,value)
        implies exists|i: int|0<=i<runs.len() && #[trigger] runs[i]==Run{value,x_start:a as u8,x_end:b as u8} by {
        row_label(rows,y,a);assert(axis::line_value(line,a)==value);
        let i=choose|i: int|0<=i<runs.len() && #[trigger] raw::covers(runs[i],a);
        assert(raw::raw_run(line,vertical[y],runs[i]));raw_run_math(rows,vertical,y,runs[i]);
        scan_model::overlap_unique(rows.labels(),selected,y,a,b,value,runs[i].x_start as int,runs[i].x_end as int,runs[i].value,a);
        assert(runs[i]==Run{value,x_start:a as u8,x_end:b as u8});
    }
}

pub(in crate::optimal::sparse) open spec fn run_of(active: ActiveRect) -> Run {
    Run{value:active.value,x_start:active.x_start,x_end:active.x_end}
}

pub(in crate::optimal::sparse) open spec fn lift(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,runs: Seq<Run>) -> Seq<ActiveRect> {
    runs.map_values(|run: Run|ActiveRect{value:run.value,x_start:run.x_start,x_end:run.x_end,
        y_start:scan_model::first_y(labels,selected,block_fn(horizontal),
            y,run.x_start as int,run.x_end as int,run.value) as u8})
}

pub(in crate::optimal::sparse) open spec fn active_rows(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,active: Seq<ActiveRect>) -> bool {
    row_runs(labels,selected,y,active.map_values(|item: ActiveRect|run_of(item)))
        && (forall|i: int|0<=i<active.len() ==> #[trigger] active[i].y_start as int
            ==scan_model::first_y(labels,selected,block_fn(horizontal),
                y,active[i].x_start as int,active[i].x_end as int,active[i].value))
}

pub(in crate::optimal::sparse) open spec fn pending(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,y: int,mask: u64) -> bool {
    (forall|p: u64|p<64 && #[trigger] axis::bit(mask,p) ==> y<p)
        && (forall|p: u64|y<p<64 && !#[trigger] axis::bit(mask,p) ==> smooth(rows,horizontal,vertical,p as int))
}

pub(in crate::optimal::sparse) open spec fn order(run: Run) -> int {run.x_start as int*65536+run.value as int}

pub(in crate::optimal::sparse) open spec fn continuable(active: ActiveRect,run: Run,mask: u64) -> bool {
    run_of(active)==run && !raw::overlaps(mask,active.x_start,active.x_end)
}

pub(in crate::optimal::sparse) open spec fn anchor_at(y: int,active: Seq<ActiveRect>,index: int) -> nat {
    ((y-1)*64+if index<active.len(){active[index].x_start as int}else{64}) as nat
}

pub(in crate::optimal::sparse) open spec fn merge(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,
    active: Seq<ActiveRect>,runs: Seq<Run>,next: Seq<ActiveRect>,ai: int,ri: int) -> bool {
    0<y<64 && horizontal.len()==64 && active_rows(labels,selected,horizontal,y-1,active)
        && row_runs(labels,selected,y,runs) && 0<=ai<=active.len() && 0<=ri<=runs.len()
        && next==lift(labels,selected,horizontal,y,runs.take(ri))
        && (forall|i: int,j: int|0<=i<ai && ri<=j<runs.len() ==> !#[trigger] continuable(active[i],runs[j],horizontal[y]))
        && (forall|i: int,j: int|ai<=i<active.len() && 0<=j<ri ==> #[trigger] order(runs[j])<#[trigger] order(run_of(active[i])))
}

impl RectScan<'_> {
    pub(in crate::optimal::sparse) open spec fn inputs(&self) -> bool {
        self.rows.valid() && self.rows.lines@.len()==64 && self.horizontal@.len()==64
            && masks_match(self.rows.labels(),self.vertical@)
    }

    pub(in crate::optimal::sparse) open spec fn valid(&self) -> bool {
        let labels=self.rows.labels();let selected=selection(labels,self.vertical@);
        self.inputs() && self.runs.valid() && self.active.valid() && self.next.valid() && self.y<=64
            && pending(self.rows,self.horizontal@,self.vertical@,self.y as int,self.boundaries)
            && self.active_index<=self.active@.len()
            && if self.y==0 {
                self.active_index==0 && self.run_index==0 && self.next@.len()==0
                    && active_rows(labels,selected,self.horizontal@,0,self.active@)
                    && row_runs(labels,selected,0,self.runs@)
            }else if self.y==64 {
                self.run_index==0 && self.next@.len()==0 && self.boundaries==0
                    && active_rows(labels,selected,self.horizontal@,63,self.active@)
            }else {
                merge(labels,selected,self.horizontal@,self.y as int,self.active@,self.runs@,self.next@,
                    self.active_index as int,self.run_index as int)
            }
    }

    pub(in crate::optimal::sparse) open spec fn frontier(&self) -> nat {
        if self.y==0 {0} else {anchor_at(self.y as int,self.active@,self.active_index as int)}
    }

    pub(in crate::optimal::sparse) open spec fn emitted(&self) -> Seq<crate::optimal::Rectangle> {
        scan_model::prefix(self.rows.labels(),selection(self.rows.labels(),self.vertical@),
            block_fn(self.horizontal@),self.frontier())
    }

    pub(in crate::optimal::sparse) open spec fn expected(&self) -> Seq<crate::optimal::Rectangle> {
        scan_model::canonical(self.rows.labels(),selection(self.rows.labels(),self.vertical@),
            block_fn(self.horizontal@))
    }
}

pub(in crate::optimal::sparse) open spec fn initial(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,runs: Seq<Run>,active: Seq<ActiveRect>,index: int) -> bool {
    row_runs(rows.labels(),selection(rows.labels(),vertical),0,runs) && 0<=index<=runs.len()
        && active.len()==index
        && forall|i: int|0<=i<index ==> #[trigger] active[i]==ActiveRect{value:runs[i].value,
            x_start:runs[i].x_start,x_end:runs[i].x_end,y_start:0}
}

pub(in crate::optimal::sparse) proof fn initial_next(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,runs: Seq<Run>,active: Seq<ActiveRect>,index: int)
    requires initial(rows,horizontal,vertical,runs,active,index),index<runs.len(),
    ensures initial(rows,horizontal,vertical,runs,active.push(ActiveRect{value:runs[index].value,
        x_start:runs[index].x_start,x_end:runs[index].x_end,y_start:0}),index+1),
{}

pub(in crate::optimal::sparse) proof fn initial_complete(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,runs: Seq<Run>,active: Seq<ActiveRect>)
    requires initial(rows,horizontal,vertical,runs,active,runs.len() as int),
    ensures active_rows(rows.labels(),selection(rows.labels(),vertical),horizontal,0,active),
{
    let keys=active.map_values(|item: ActiveRect|run_of(item));
    assert(keys =~=runs);
    assert forall|i: int|0<=i<active.len() implies #[trigger] active[i].y_start as int
        ==scan_model::first_y(rows.labels(),selection(rows.labels(),vertical),block_fn(horizontal),
            0,active[i].x_start as int,active[i].x_end as int,active[i].value) by {}
}

pub(in crate::optimal::sparse) proof fn new_scan(scan: &RectScan<'_>,cut_boundaries: u64)
    requires ready(scan.rows,scan.horizontal@,scan.vertical@,cut_boundaries),
        scan.y==0,scan.active_index==0,scan.run_index==0,scan.boundaries==(scan.rows.boundaries|cut_boundaries)&!1u64,
        scan.runs.valid(),scan.active.valid(),scan.next.valid(),scan.next@.len()==0,
        initial(scan.rows,scan.horizontal@,scan.vertical@,scan.runs@,scan.active@,scan.runs@.len() as int),
    ensures scan.valid(),scan.emitted()==Seq::empty(),
{
    initial_complete(scan.rows,scan.horizontal@,scan.vertical@,scan.runs@,scan.active@);
    let all=scan.rows.boundaries|cut_boundaries;let remaining=scan.boundaries;
    assert forall|p: u64|p<64 && #[trigger] axis::bit(remaining,p) implies 0<p by {
        assert(p!=0) by(bit_vector) requires remaining==all&!1u64,(remaining>>p)&1==1,p<64;
    }
    assert forall|p: u64|0<p<64 && !#[trigger] axis::bit(remaining,p)
        implies smooth(scan.rows,scan.horizontal@,scan.vertical@,p as int) by {
        assert(!axis::bit(all,p)) by(bit_vector) requires remaining==all&!1u64,(remaining>>p)&1!=1,0<p<64;
    }
}

pub(in crate::optimal::sparse) proof fn smooth_run(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,y: int,a: int,b: int,value: u16)
    requires rows.valid(),rows.lines@.len()==64,horizontal.len()==64,masks_match(rows.labels(),vertical),
        0<y<64,smooth(rows,horizontal,vertical,y),0<=a<b<=64,value!=0,
    ensures split_geometry::run(rows.labels(),selection(rows.labels(),vertical),y-1,a,b,value)
        <==>split_geometry::run(rows.labels(),selection(rows.labels(),vertical),y,a,b,value),
        !blocked(horizontal,y,a,b),
        split_geometry::run(rows.labels(),selection(rows.labels(),vertical),y,a,b,value) ==>
            scan_model::first_y(rows.labels(),selection(rows.labels(),vertical),block_fn(horizontal),y,a,b,value)
            ==scan_model::first_y(rows.labels(),selection(rows.labels(),vertical),block_fn(horizontal),y-1,a,b,value),
{
    let run=Run{value,x_start:a as u8,x_end:b as u8};
    raw_run_math(rows,vertical,y-1,run);raw_run_math(rows,vertical,y,run);
    raw::empty_overlap(a as u8,b as u8);
}

pub(in crate::optimal::sparse) proof fn gap_run(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,low: int,high: int,a: int,b: int,value: u16)
    requires rows.valid(),rows.lines@.len()==64,horizontal.len()==64,masks_match(rows.labels(),vertical),
        0<=low<=high<64,0<=a<b<=64,value!=0,
        forall|y: int|low<y<=high ==> #[trigger] smooth(rows,horizontal,vertical,y),
    ensures split_geometry::run(rows.labels(),selection(rows.labels(),vertical),low,a,b,value)
        <==>split_geometry::run(rows.labels(),selection(rows.labels(),vertical),high,a,b,value),
        split_geometry::run(rows.labels(),selection(rows.labels(),vertical),high,a,b,value) ==>
            scan_model::first_y(rows.labels(),selection(rows.labels(),vertical),block_fn(horizontal),high,a,b,value)
            ==scan_model::first_y(rows.labels(),selection(rows.labels(),vertical),block_fn(horizontal),low,a,b,value),
    decreases high-low,
{
    if low<high {
        gap_run(rows,horizontal,vertical,low,high-1,a,b,value);
        smooth_run(rows,horizontal,vertical,high,a,b,value);
    }
}

pub(in crate::optimal::sparse) proof fn gap_active(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,low: int,high: int,active: Seq<ActiveRect>)
    requires rows.valid(),rows.lines@.len()==64,horizontal.len()==64,masks_match(rows.labels(),vertical),0<=low<=high<64,
        active_rows(rows.labels(),selection(rows.labels(),vertical),horizontal,low,active),
        forall|y: int|low<y<=high ==> #[trigger] smooth(rows,horizontal,vertical,y),
    ensures active_rows(rows.labels(),selection(rows.labels(),vertical),horizontal,high,active),
{
    let labels=rows.labels();let selected=selection(labels,vertical);let keys=active.map_values(|item: ActiveRect|run_of(item));
    assert forall|i: int|0<=i<keys.len() implies #[trigger] split_geometry::run(labels,selected,high,
        keys[i].x_start as int,keys[i].x_end as int,keys[i].value) by {
        assert(split_geometry::run(labels,selected,low,keys[i].x_start as int,keys[i].x_end as int,keys[i].value));
        gap_run(rows,horizontal,vertical,low,high,keys[i].x_start as int,keys[i].x_end as int,keys[i].value);
    }
    assert forall|a: int,b: int,value: u16| #[trigger] split_geometry::run(labels,selected,high,a,b,value)
        implies exists|i: int|0<=i<keys.len() && #[trigger] keys[i]==Run{value,x_start:a as u8,x_end:b as u8} by {
        gap_run(rows,horizontal,vertical,low,high,a,b,value);
    }
    assert forall|i: int|0<=i<active.len() implies #[trigger] active[i].y_start as int
        ==scan_model::first_y(labels,selected,block_fn(horizontal),high,
            active[i].x_start as int,active[i].x_end as int,active[i].value) by {
        assert(keys[i]==run_of(active[i]));
        assert(split_geometry::run(labels,selected,low,keys[i].x_start as int,keys[i].x_end as int,keys[i].value));
        gap_run(rows,horizontal,vertical,low,high,active[i].x_start as int,active[i].x_end as int,active[i].value);
    }
}

pub(in crate::optimal::sparse) proof fn pending_next(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,y: int,mask: u64)
    requires pending(rows,horizontal,vertical,y,mask),mask!=0,0<=y<64,
    ensures y<vstd::std_specs::bits::u64_trailing_zeros(mask)<64,
        pending(rows,horizontal,vertical,vstd::std_specs::bits::u64_trailing_zeros(mask) as int,mask&((mask-1) as u64)),
        forall|q: int|y<q<vstd::std_specs::bits::u64_trailing_zeros(mask) ==> #[trigger] smooth(rows,horizontal,vertical,q),
{
    let next=vstd::std_specs::bits::u64_trailing_zeros(mask) as u64;let after=mask&((mask-1) as u64);
    axis::cleared_lowest(mask,next,after);
    assert forall|q: u64|q<64 && #[trigger] axis::bit(after,q) implies next<q by {}
    assert forall|q: u64|next<q<64 && !#[trigger] axis::bit(after,q)
        implies smooth(rows,horizontal,vertical,q as int) by {assert(!axis::bit(mask,q));}
    assert forall|q: int|y<q<next implies #[trigger] smooth(rows,horizontal,vertical,q) by {
        let p=q as u64;assert(!axis::bit(mask,p));
    }
}

pub(in crate::optimal::sparse) proof fn pending_empty(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,y: int)
    requires pending(rows,horizontal,vertical,y,0),0<=y<64,
    ensures pending(rows,horizontal,vertical,64,0),
        forall|q: int|y<q<64 ==> #[trigger] smooth(rows,horizontal,vertical,q),
{
    assert forall|q: int|y<q<64 implies #[trigger] smooth(rows,horizontal,vertical,q) by {
        let p=q as u64;assert(!axis::bit(0,p)) by(bit_vector);
    }
    assert forall|p: u64|p<64 implies !#[trigger] axis::bit(0,p) by {assert((0u64>>p)&1==0) by(bit_vector);}
}

pub(in crate::optimal::sparse) proof fn active_find(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,active: Seq<ActiveRect>,a: int,b: int,value: u16)
    requires active_rows(labels,selected,horizontal,y,active),split_geometry::run(labels,selected,y,a,b,value),
    ensures exists|i: int|0<=i<active.len() && #[trigger] run_of(active[i])==(Run{value,x_start:a as u8,x_end:b as u8}),
{
    let keys=active.map_values(|item: ActiveRect|run_of(item));
    let i=choose|i: int|0<=i<keys.len() && #[trigger] keys[i]==Run{value,x_start:a as u8,x_end:b as u8};
    assert(run_of(active[i])==keys[i]);
}

pub(in crate::optimal::sparse) proof fn active_min(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,active: Seq<ActiveRect>,a: int,b: int,value: u16)
    requires active_rows(labels,selected,horizontal,y,active),split_geometry::run(labels,selected,y,a,b,value),
    ensures active.len()>0,active[0].x_start<=a,
{
    active_find(labels,selected,horizontal,y,active,a,b,value);
    let i=choose|i: int|0<=i<active.len() && #[trigger] run_of(active[i])==(Run{value,x_start:a as u8,x_end:b as u8});
    let keys=active.map_values(|item: ActiveRect|run_of(item));
    assert(keys[i]==run_of(active[i]));assert(raw::valid(keys[0]));
    if i>0 {assert(keys[0].x_end<=keys[i].x_start);}
}

pub(in crate::optimal::sparse) proof fn boundary_prefix(scan: &RectScan<'_>,old_row: int)
    requires scan.valid(),0<=old_row<scan.y,scan.active_index==0,
        forall|y: int|old_row<y<scan.y ==> #[trigger] smooth(scan.rows,scan.horizontal@,scan.vertical@,y),
    ensures scan.emitted()==scan_model::prefix(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),
        block_fn(scan.horizontal@),(old_row*64) as nat),
{
    let labels=scan.rows.labels();let selected=selection(labels,scan.vertical@);
    let block=block_fn(scan.horizontal@);
    let row=scan.y as int-1;let high=scan.frontier();let low=(old_row*64) as nat;
    assert(active_rows(labels,selected,scan.horizontal@,row,scan.active@));
    if scan.active@.len()>0 {
        let keys=scan.active@.map_values(|item: ActiveRect|run_of(item));assert(raw::valid(keys[0]));
    }
    assert(low<=high<=4096);
    assert forall|p: int|low<=p<high implies !#[trigger] scan_model::ending(labels,selected,block,p) by {
        if scan_model::ending(labels,selected,block,p) {
            scan_model::ending_run(labels,selected,block,p);
            let y=p/64;let a=p%64;let b=scan_model::end_at(labels,selected,y,a);let value=corners::at(labels,a,y);
            assert(p==y*64+a);assert(old_row<=y<=row);
            if y<row {
                smooth_run(scan.rows,scan.horizontal@,scan.vertical@,y+1,a,b,value);
                assert(scan_model::continues(labels,selected,block,y+1,a,b,value));
            }else{active_min(labels,selected,scan.horizontal@,row,scan.active@,a,b,value);}
        }
    }
    scan_model::prefix_gap(labels,selected,block,low,high);
}

pub(in crate::optimal::sparse) open spec fn boundary_entry(scan: &RectScan<'_>) -> bool {
    scan.inputs() && scan.runs.valid() && scan.active.valid() && scan.next.valid() && scan.y<64
        && pending(scan.rows,scan.horizontal@,scan.vertical@,scan.y as int,scan.boundaries)
        && active_rows(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),scan.horizontal@,scan.y as int,scan.active@)
}

pub(in crate::optimal::sparse) proof fn boundary_done(scan: &RectScan<'_>,old_row: int)
    requires scan.inputs(),scan.runs.valid(),scan.active.valid(),scan.next.valid(),0<=old_row<scan.y<=64,
        pending(scan.rows,scan.horizontal@,scan.vertical@,scan.y as int,scan.boundaries),
        scan.active_index==0,scan.run_index==0,scan.next@.len()==0,
        active_rows(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),scan.horizontal@,scan.y as int-1,scan.active@),
        scan.y<64 ==>row_runs(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),scan.y as int,scan.runs@),
        scan.y==64 ==>scan.boundaries==0,
        forall|y: int|old_row<y<scan.y ==> #[trigger] smooth(scan.rows,scan.horizontal@,scan.vertical@,y),
    ensures scan.valid(),scan.emitted()==scan_model::prefix(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),
        block_fn(scan.horizontal@),(old_row*64) as nat),
{
    assert(scan.next@ =~=lift(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),scan.horizontal@,
        scan.y as int,scan.runs@.take(0)));
    assert(scan.valid());boundary_prefix(scan,old_row);
}

pub(in crate::optimal::sparse) proof fn run_order(runs: Seq<Run>,i: int,j: int)
    requires raw::ordered(runs),0<=i<j<runs.len(),
    ensures runs[i].x_start<runs[j].x_start,order(runs[i])<order(runs[j]),
{
    assert(raw::valid(runs[i]));assert(raw::valid(runs[j]));
}

pub(in crate::optimal::sparse) proof fn active_order(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,active: Seq<ActiveRect>,i: int,j: int)
    requires active_rows(labels,selected,horizontal,y,active),0<=i<j<active.len(),
    ensures active[i].x_start<active[j].x_start,order(run_of(active[i]))<order(run_of(active[j])),
{
    let keys=active.map_values(|item: ActiveRect|run_of(item));run_order(keys,i,j);
}

pub(in crate::optimal::sparse) proof fn active_item(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,active: Seq<ActiveRect>,i: int)
    requires active_rows(labels,selected,horizontal,y,active),0<=i<active.len(),
    ensures split_geometry::run(labels,selected,y,active[i].x_start as int,active[i].x_end as int,active[i].value),
        raw::valid(run_of(active[i])),active[i].y_start<=y,
{
    let keys=active.map_values(|item: ActiveRect|run_of(item));
    assert(raw::valid(keys[i]));assert(split_geometry::run(labels,selected,y,keys[i].x_start as int,keys[i].x_end as int,keys[i].value));
    scan_model::first_rows(labels,selected,block_fn(horizontal),y,keys[i].x_start as int,keys[i].x_end as int,keys[i].value);
}

pub(in crate::optimal::sparse) proof fn lift_step(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,runs: Seq<Run>,index: int)
    requires 0<=index<runs.len(),
    ensures lift(labels,selected,horizontal,y,runs.take(index+1))==lift(labels,selected,horizontal,y,runs.take(index)).push(
        ActiveRect{value:runs[index].value,x_start:runs[index].x_start,x_end:runs[index].x_end,
            y_start:scan_model::first_y(labels,selected,block_fn(horizontal),
                y,runs[index].x_start as int,runs[index].x_end as int,runs[index].value) as u8}),
{
    assert(runs.take(index+1) =~=runs.take(index).push(runs[index]));
}

pub(in crate::optimal::sparse) proof fn fresh_first(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,
    active: Seq<ActiveRect>,runs: Seq<Run>,next: Seq<ActiveRect>,ai: int,ri: int)
    requires merge(labels,selected,horizontal,y,active,runs,next,ai,ri),ri<runs.len(),
        ai==active.len() || order(runs[ri])<order(run_of(active[ai])),
    ensures scan_model::first_y(labels,selected,block_fn(horizontal),y,
        runs[ri].x_start as int,runs[ri].x_end as int,runs[ri].value)==y,
{
    let run=runs[ri];assert(split_geometry::run(labels,selected,y,run.x_start as int,run.x_end as int,run.value));
    let block=block_fn(horizontal);
    if scan_model::continues(labels,selected,block,y,run.x_start as int,run.x_end as int,run.value) {
        active_find(labels,selected,horizontal,y-1,active,run.x_start as int,run.x_end as int,run.value);
        let i=choose|i: int|0<=i<active.len() && #[trigger] run_of(active[i])==run;
        assert(continuable(active[i],run,horizontal[y]));
        if i<ai {assert(!continuable(active[i],runs[ri],horizontal[y]));}
        else {
            if ai<i {active_order(labels,selected,horizontal,y-1,active,ai,i);}
        }
    }
}

pub(in crate::optimal::sparse) proof fn new_data(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,
    active: Seq<ActiveRect>,runs: Seq<Run>,next: Seq<ActiveRect>,ai: int,ri: int)
    requires merge(labels,selected,horizontal,y,active,runs,next,ai,ri),ri<runs.len(),
        ai==active.len() || order(runs[ri])<order(run_of(active[ai])),
    ensures merge(labels,selected,horizontal,y,active,runs,next.push(ActiveRect{value:runs[ri].value,
        x_start:runs[ri].x_start,x_end:runs[ri].x_end,y_start:y as u8}),ai,ri+1),
{
    fresh_first(labels,selected,horizontal,y,active,runs,next,ai,ri);lift_step(labels,selected,horizontal,y,runs,ri);
    assert forall|i: int,j: int|ai<=i<active.len() && 0<=j<ri+1
        implies #[trigger] order(runs[j])<#[trigger] order(run_of(active[i])) by {
        if j==ri && ai<i {active_order(labels,selected,horizontal,y-1,active,ai,i);}
    }
}

pub(in crate::optimal::sparse) proof fn continued_data(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,
    active: Seq<ActiveRect>,runs: Seq<Run>,next: Seq<ActiveRect>,ai: int,ri: int)
    requires merge(labels,selected,horizontal,y,active,runs,next,ai,ri),ai<active.len(),ri<runs.len(),continuable(active[ai],runs[ri],horizontal[y]),
    ensures merge(labels,selected,horizontal,y,active,runs,next.push(active[ai]),ai+1,ri+1),
        !scan_model::ending(labels,selected,block_fn(horizontal),anchor_at(y,active,ai) as int),
{
    let item=active[ai];let run=runs[ri];let block=block_fn(horizontal);
    active_item(labels,selected,horizontal,y-1,active,ai);
    assert(split_geometry::run(labels,selected,y,run.x_start as int,run.x_end as int,run.value));
    assert(scan_model::continues(labels,selected,block,y,run.x_start as int,run.x_end as int,run.value));
    scan_model::run_at(labels,selected,y-1,item.x_start as int,item.x_end as int,item.value,item.x_start as int);
    let p=anchor_at(y,active,ai) as int;
    assert(p/64==y-1 && p%64==item.x_start as int) by(nonlinear_arith)
        requires p==(y-1)*64+item.x_start as int,item.x_start<64;
    lift_step(labels,selected,horizontal,y,runs,ri);
    assert forall|i: int,j: int|0<=i<ai+1 && ri+1<=j<runs.len()
        implies !#[trigger] continuable(active[i],runs[j],horizontal[y]) by {
        if i==ai {run_order(runs,ri,j);}
    }
    assert forall|i: int,j: int|ai+1<=i<active.len() && 0<=j<ri+1
        implies #[trigger] order(runs[j])<#[trigger] order(run_of(active[i])) by {
        if j==ri {active_order(labels,selected,horizontal,y-1,active,ai,i);}
    }
}

pub(in crate::optimal::sparse) proof fn closed_data(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,
    active: Seq<ActiveRect>,runs: Seq<Run>,next: Seq<ActiveRect>,ai: int,ri: int)
    requires merge(labels,selected,horizontal,y,active,runs,next,ai,ri),ai<active.len(),
        ri==runs.len() || order(run_of(active[ai]))<=order(runs[ri]) && !continuable(active[ai],runs[ri],horizontal[y]),
    ensures merge(labels,selected,horizontal,y,active,runs,next,ai+1,ri),
        !scan_model::continues(labels,selected,block_fn(horizontal),y,
            active[ai].x_start as int,active[ai].x_end as int,active[ai].value),
{
    assert forall|i: int,j: int|0<=i<ai+1 && ri<=j<runs.len()
        implies !#[trigger] continuable(active[i],runs[j],horizontal[y]) by {
        if i==ai && ri<j {run_order(runs,ri,j);}
    }
    let item=active[ai];let block=block_fn(horizontal);
    if scan_model::continues(labels,selected,block,y,item.x_start as int,item.x_end as int,item.value) {
        let j=choose|j: int|0<=j<runs.len() && #[trigger] runs[j]==run_of(item);
        assert(continuable(item,runs[j],horizontal[y]));
        if j<ri {assert(order(runs[j])<order(run_of(active[ai])));}
        else {assert(!continuable(active[ai],runs[j],horizontal[y]));}
    }
}

pub(in crate::optimal::sparse) proof fn closed_form(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,active: Seq<ActiveRect>,index: int)
    requires 0<y<=64,active_rows(labels,selected,horizontal,y-1,active),0<=index<active.len(),
        !scan_model::continues(labels,selected,block_fn(horizontal),y,
            active[index].x_start as int,active[index].x_end as int,active[index].value),
    ensures scan_model::ending(labels,selected,block_fn(horizontal),anchor_at(y,active,index) as int),
        scan_model::rectangle(labels,selected,block_fn(horizontal),anchor_at(y,active,index) as int)
            ==(crate::optimal::Rectangle{value:active[index].value,
                x:crate::optimal::RangeU8{start:active[index].x_start,end:active[index].x_end},
                y:crate::optimal::RangeU8{start:active[index].y_start,end:y as u8}}),
{
    active_item(labels,selected,horizontal,y-1,active,index);let item=active[index];let p=anchor_at(y,active,index) as int;
    scan_model::run_at(labels,selected,y-1,item.x_start as int,item.x_end as int,item.value,item.x_start as int);
    assert(p/64==y-1 && p%64==item.x_start as int) by(nonlinear_arith)
        requires p==(y-1)*64+item.x_start as int,item.x_start<64;
}

pub(in crate::optimal::sparse) proof fn prefix_step(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,active: Seq<ActiveRect>,index: int)
    requires 0<y<=64,active_rows(labels,selected,horizontal,y-1,active),0<=index<active.len(),
    ensures scan_model::prefix(labels,selected,block_fn(horizontal),anchor_at(y,active,index+1))
        ==if scan_model::ending(labels,selected,block_fn(horizontal),anchor_at(y,active,index) as int) {
            scan_model::prefix(labels,selected,block_fn(horizontal),anchor_at(y,active,index)).push(
                scan_model::rectangle(labels,selected,block_fn(horizontal),anchor_at(y,active,index) as int))
        }else{scan_model::prefix(labels,selected,block_fn(horizontal),anchor_at(y,active,index))},
{
    let low=anchor_at(y,active,index);let high=anchor_at(y,active,index+1);let block=block_fn(horizontal);
    active_item(labels,selected,horizontal,y-1,active,index);
    if index+1<active.len(){active_item(labels,selected,horizontal,y-1,active,index+1);active_order(labels,selected,horizontal,y-1,active,index,index+1);}
    assert(low<high<=4096);
    assert forall|p: int|low<p<high implies !#[trigger] scan_model::ending(labels,selected,block,p) by {
        if scan_model::ending(labels,selected,block,p) {
            scan_model::ending_run(labels,selected,block,p);let a=p%64;let b=scan_model::end_at(labels,selected,y-1,a);let value=corners::at(labels,a,y-1);
            assert(p/64==y-1) by(nonlinear_arith) requires (y-1)*64<=p<y*64;
            active_find(labels,selected,horizontal,y-1,active,a,b,value);
            let i=choose|i: int|0<=i<active.len() && #[trigger] run_of(active[i])==(Run{value,x_start:a as u8,x_end:b as u8});
            if i<index{active_order(labels,selected,horizontal,y-1,active,i,index);}
            if index+1<i{active_order(labels,selected,horizontal,y-1,active,index+1,i);}
        }
    }
    scan_model::prefix_gap(labels,selected,block,low+1,high);scan_model::prefix_next(labels,selected,block,low);
}

pub(in crate::optimal::sparse) open spec fn may_start(scan: &RectScan<'_>,run: Run) -> bool {
    scan.valid() && 0<scan.y<64 && scan.run_index<scan.runs@.len() && run==scan.runs@[scan.run_index as int]
        && (scan.active_index==scan.active@.len() || order(run)<order(run_of(scan.active@[scan.active_index as int])))
}

pub(in crate::optimal::sparse) open spec fn may_close(scan: &RectScan<'_>) -> bool {
    scan.valid() && scan.y>0 && scan.active_index<scan.active@.len()
        && (scan.y==64 || scan.run_index==scan.runs@.len()
            ||order(run_of(scan.active@[scan.active_index as int]))<=order(scan.runs@[scan.run_index as int])
                && !continuable(scan.active@[scan.active_index as int],scan.runs@[scan.run_index as int],scan.horizontal@[scan.y as int]))
}

pub(in crate::optimal::sparse) proof fn lifted_rows(labels: Seq<u16>,selected: Set<int>,horizontal: Seq<u64>,y: int,runs: Seq<Run>)
    requires row_runs(labels,selected,y,runs),
    ensures active_rows(labels,selected,horizontal,y,lift(labels,selected,horizontal,y,runs)),
{
    let active=lift(labels,selected,horizontal,y,runs);let keys=active.map_values(|item: ActiveRect|run_of(item));
    assert(keys =~=runs);
    assert forall|i: int|0<=i<runs.len() implies #[trigger] active[i].y_start as int
        ==scan_model::first_y(labels,selected,block_fn(horizontal),y,
            runs[i].x_start as int,runs[i].x_end as int,runs[i].value) by {
        assert(split_geometry::run(labels,selected,y,runs[i].x_start as int,runs[i].x_end as int,runs[i].value));
        scan_model::first_rows(labels,selected,block_fn(horizontal),y,
            runs[i].x_start as int,runs[i].x_end as int,runs[i].value);
    }
}

pub(in crate::optimal::sparse) proof fn completed_merge(scan: &RectScan<'_>)
    requires scan.valid(),0<scan.y<64,scan.active_index==scan.active@.len(),scan.run_index==scan.runs@.len(),
    ensures active_rows(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),scan.horizontal@,scan.y as int,old(scan.next)@),
        scan.emitted()==scan_model::prefix(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),
            block_fn(scan.horizontal@),scan.y as nat*64),
{
    lifted_rows(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),scan.horizontal@,scan.y as int,scan.runs@);
    assert(scan.runs@.take(scan.runs@.len() as int) =~=scan.runs@);
}

pub(in crate::optimal::sparse) proof fn valid_prefix(scan: &RectScan<'_>)
    requires scan.valid(),
    ensures scan.frontier()<=4096,scan.emitted().len()<=scan.expected().len()<=4096,
        scan.expected().take(scan.emitted().len() as int)==scan.emitted(),
{
    if scan.y>0 && scan.active_index<scan.active@.len() {
        active_item(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),scan.horizontal@,scan.y as int-1,
            scan.active@,scan.active_index as int);
    }
    assert(scan.frontier()<=4096);
    scan_model::canonical_prefix(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),
        block_fn(scan.horizontal@),scan.frontier());
    scan_model::canonical_capacity(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),
        block_fn(scan.horizontal@));
}

pub(in crate::optimal::sparse) proof fn yielded(scan: &RectScan<'_>,original: nat,result: crate::optimal::Rectangle)
    requires scan.valid(),original<=4096,
        scan.emitted()==scan_model::prefix(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),
            block_fn(scan.horizontal@),original).push(result),
    ensures result==scan.expected()[scan_model::prefix(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),
        block_fn(scan.horizontal@),original).len() as int],
{
    valid_prefix(scan);
    let prefix=scan_model::prefix(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),
        block_fn(scan.horizontal@),original);
    assert(scan.emitted()[prefix.len() as int]==result);
}

pub(in crate::optimal::sparse) proof fn finished(scan: &RectScan<'_>)
    requires scan.valid(),scan.y==64,scan.active_index==scan.active@.len(),
    ensures scan.emitted()==scan.expected(),
{}

pub(in crate::optimal::sparse) open spec fn driver(scan: &RectScan<'_>,original: nat) -> bool {
    scan.valid() && original<=4096 && scan.emitted()==scan_model::prefix(scan.rows.labels(),selection(scan.rows.labels(),scan.vertical@),
        block_fn(scan.horizontal@),original)
}

#[cfg(reject_scan_early_none)]
fn reject_early_none(scan: &mut RectScan<'_>)
    requires old(scan).valid(),old(scan).emitted().len()<old(scan).expected().len(),
{
    let result=scan.next_rect();
    assert(result is Ok && result->Ok_0 is None);
}

#[cfg(reject_scan_next_value)]
fn reject_next_value(scan: &mut RectScan<'_>)
    requires old(scan).valid(),old(scan).emitted().len()<old(scan).expected().len(),
{
    let ghost expected=scan.expected()[scan.emitted().len() as int];
    let result=scan.next_rect();
    if let Ok(Some(rectangle))=result {assert(rectangle!=expected);}else{assert(false);}
}

#[cfg(reject_scan_lost_split)]
fn reject_lost_split() {
    let line=[super::super::Interval{start:0,end:3,value:7}];
    let mut runs=crate::optimal::fixed::FixedVec::<Run,64>::new(Run{value:0,x_start:0,x_end:0});
    let result=super::super::build_runs_for_row(&line,1u128<<1,&mut runs);
    assert(result.is_ok());
    assert(runs@.len()==1);
}

#[cfg(reject_scan_endpoint_split)]
fn reject_endpoint_split() {
    let line=[super::super::Interval{start:0,end:64,value:7}];
    let mut runs=crate::optimal::fixed::FixedVec::<Run,64>::new(Run{value:0,x_start:0,x_end:0});
    let result=super::super::build_runs_for_row(&line,1u128<<64,&mut runs);
    assert(result.is_ok());
    assert(runs@.len()==2);
}

#[cfg(reject_scan_missing_boundary)]
proof fn reject_missing_boundary(rows: &AxisIntervals,horizontal: Seq<u64>,vertical: Seq<u128>,boundaries: u64,y: u64)
    requires rows.valid(),rows.lines@.len()==64,rows.boundaries_complete(),horizontal.len()==64,
        masks_match(rows.labels(),vertical),0<y<64,!axis::bit(rows.boundaries|boundaries,y),horizontal[y as int]!=0,
{
    assert(ready(rows,horizontal,vertical,boundaries));
}

}
