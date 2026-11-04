//! 实际区间/位掩码到有序行 runs；证明前缀只借用本次有效输出。

use super::{Interval, proof as axis};
use crate::optimal::types::{ActiveRect, Run};
use vstd::prelude::*;

verus! {

pub(super) open spec fn bit128(mask: u128,p: u32) -> bool {(mask>>p)&1==1}
pub(super) open spec fn bit64(mask: u64,p: u8) -> bool {(mask>>p)&1==1}

pub(super) open spec fn valid(run: Run) -> bool {run.value!=0 && run.x_start<run.x_end<=64}

pub(super) open spec fn ordered(runs: Seq<Run>) -> bool {
    (forall|i: int|0<=i<runs.len() ==> #[trigger] valid(runs[i]))
        && (forall|i: int,j: int|0<=i<j<runs.len() ==> #[trigger] runs[i].x_end<=#[trigger] runs[j].x_start)
}

pub(super) open spec fn covers(run: Run,p: int) -> bool {run.x_start<=p<run.x_end}

pub(super) open spec fn raw_run(line: Seq<Interval>,mask: u128,run: Run) -> bool {
    valid(run)
        && (axis::line_value(line,run.x_start as int-1)!=run.value || bit128(mask,run.x_start as u32))
        && (axis::line_value(line,run.x_end as int)!=run.value || bit128(mask,run.x_end as u32))
        && (forall|p: int| covers(run,p) ==> #[trigger] axis::line_value(line,p)==run.value)
        && (forall|p: u32|run.x_start<p<run.x_end ==> !#[trigger] bit128(mask,p))
}

pub(super) open spec fn progress(line: Seq<Interval>,mask: u128,runs: Seq<Run>,end: int) -> bool {
    ordered(runs) && 0<=end<=64 && runs.len()<=end
        && (forall|i: int|0<=i<runs.len() ==> #[trigger] raw_run(line,mask,runs[i]) && runs[i].x_end<=end)
        && (forall|p: int|0<=p<end ==> (#[trigger] axis::line_value(line,p)!=0
            <==> exists|i: int|0<=i<runs.len() && #[trigger] covers(runs[i],p)))
}

pub(super) open spec fn remaining(original: u128,mask: u64,start: int,end: int) -> bool {
    forall|p: u64|p<64 ==> (#[trigger] axis::bit(mask,p) <==> bit128(original,p as u32) && start<p<end)
}

pub(super) proof fn key_equal(active: ActiveRect,run: Run)
    ensures ((active.value as u32)|((active.x_start as u32)<<16)|((active.x_end as u32)<<24))
        ==((run.value as u32)|((run.x_start as u32)<<16)|((run.x_end as u32)<<24))
        <==> active.value==run.value && active.x_start==run.x_start && active.x_end==run.x_end,
{
    let av=active.value;let ax=active.x_start;let ae=active.x_end;
    let rv=run.value;let rx=run.x_start;let re=run.x_end;
    assert(((av as u32)|((ax as u32)<<16)|((ae as u32)<<24))
        ==((rv as u32)|((rx as u32)<<16)|((re as u32)<<24))
        <==> av==rv && ax==rx && ae==re) by(bit_vector);
}

pub(super) proof fn interval_boundaries(line: Seq<Interval>,i: int)
    requires axis::line_valid(line),0<=i<line.len(),
    ensures axis::line_value(line,line[i].start as int-1)!=line[i].value,
        axis::line_value(line,line[i].end as int)!=line[i].value,
{
    let interval=line[i];
    assert(axis::interval_valid(interval));
    if exists|j: int|0<=j<line.len() && #[trigger] line[j].start<=interval.start as int-1<line[j].end {
        let j=choose|j: int|0<=j<line.len() && #[trigger] line[j].start<=interval.start as int-1<line[j].end;
        assert(axis::interval_valid(line[j]));
        axis::value_at(line,interval.start as int-1,j);
        assert(j<i);assert(line[j].end==interval.start);
    }
    if exists|j: int|0<=j<line.len() && #[trigger] line[j].start<=(interval.end as int)<line[j].end {
        let j=choose|j: int|0<=j<line.len() && #[trigger] line[j].start<=(interval.end as int)<line[j].end;
        assert(axis::interval_valid(line[j]));
        axis::value_at(line,interval.end as int,j);
        assert(i<j);assert(line[j].start==interval.end);
    }
}

pub(super) proof fn piece(line: Seq<Interval>,mask: u128,i: int,start: u8,end: u8)
    requires axis::line_valid(line),0<=i<line.len(),line[i].start<=start<end<=line[i].end,
        start==line[i].start || bit128(mask,start as u32),end==line[i].end || bit128(mask,end as u32),
        forall|p: u32|start<p<end ==> !#[trigger] bit128(mask,p),
    ensures raw_run(line,mask,Run{value:line[i].value,x_start:start,x_end:end}),
{
    assert(axis::interval_valid(line[i]));
    interval_boundaries(line,i);
    assert forall|p: int|start<=p<end implies #[trigger] axis::line_value(line,p)==line[i].value by {axis::value_at(line,p,i);}
}

pub(super) proof fn started(line: Seq<Interval>,mask: u128)
    ensures progress(line,mask,Seq::empty(),0),
{}

pub(super) proof fn advance_gap(line: Seq<Interval>,mask: u128,runs: Seq<Run>,i: int)
    requires axis::line_valid(line),0<=i<line.len(),
        progress(line,mask,runs,if i==0{0}else{line[i-1].end as int}),
    ensures progress(line,mask,runs,line[i].start as int),
{
    let end=if i==0{0}else{line[i-1].end as int};let start=line[i].start as int;
    assert(axis::interval_valid(line[i]));
    if i>0 {assert(axis::interval_valid(line[i-1]));}
    assert(end<=start);
    assert(progress(line,mask,runs,end));
    assert(0<=start<=64 && runs.len()<=start && ordered(runs));
    assert forall|p: int|end<=p<start implies #[trigger] axis::line_value(line,p)==0 by {
        if exists|j: int|0<=j<line.len() && #[trigger] line[j].start<=p<line[j].end {
            let j=choose|j: int|0<=j<line.len() && #[trigger] line[j].start<=p<line[j].end;
            assert(axis::interval_valid(line[j]));
            if j<i {assert(line[j].end<=end);}else{assert(start<=line[j].start);}
        }
    }
    assert forall|p: int|0<=p<start implies (#[trigger] axis::line_value(line,p)!=0
        <==>exists|j: int|0<=j<runs.len() && #[trigger] covers(runs[j],p)) by {
        if p>=end {
            assert(axis::line_value(line,p)==0);
            if exists|j: int|0<=j<runs.len() && #[trigger] covers(runs[j],p) {
                let j=choose|j: int|0<=j<runs.len() && #[trigger] covers(runs[j],p);
                assert(raw_run(line,mask,runs[j]));assert(runs[j].x_end<=end);
            }
        }
    }
    assert forall|j: int|0<=j<runs.len() implies #[trigger] raw_run(line,mask,runs[j]) && runs[j].x_end<=start by {
        assert(raw_run(line,mask,runs[j]));
    }
}

pub(super) proof fn append_piece(line: Seq<Interval>,mask: u128,runs: Seq<Run>,i: int,start: u8,end: u8)
    requires axis::line_valid(line),0<=i<line.len(),progress(line,mask,runs,start as int),
        line[i].start<=start<end<=line[i].end,
        raw_run(line,mask,Run{value:line[i].value,x_start:start,x_end:end}),
    ensures progress(line,mask,runs.push(Run{value:line[i].value,x_start:start,x_end:end}),end as int),
{
    let item=Run{value:line[i].value,x_start:start,x_end:end};let result=runs.push(item);
    assert(valid(item));
    assert(ordered(result)) by {
        assert forall|j: int|0<=j<result.len() implies #[trigger] valid(result[j]) by {if j<runs.len() {assert(valid(runs[j]));}}
        assert forall|a: int,b: int|0<=a<b<result.len() implies #[trigger] result[a].x_end<=#[trigger] result[b].x_start by {
            if b==runs.len() {assert(raw_run(line,mask,runs[a]));assert(runs[a].x_end<=start);}
        }
    }
    assert forall|j: int|0<=j<result.len() implies #[trigger] raw_run(line,mask,result[j]) && result[j].x_end<=end by {
        if j<runs.len() {assert(raw_run(line,mask,runs[j]));}
    }
    assert forall|p: int|0<=p<end implies (#[trigger] axis::line_value(line,p)!=0
        <==>exists|j: int|0<=j<result.len() && #[trigger] covers(result[j],p)) by {
        if p<start {
            if axis::line_value(line,p)!=0 {
                let j=choose|j: int|0<=j<runs.len() && #[trigger] covers(runs[j],p);
                assert(covers(result[j],p));
            }
        }else {
            axis::value_at(line,p,i);assert(covers(item,p));assert(covers(result[runs.len() as int],p));
        }
    }
}

/// 每个互不重叠的非空区间至少占一格，整行至多 64 段。
pub(super) proof fn line_count_bound(line: Seq<Interval>, count: nat)
    requires axis::line_valid(line), count <= line.len(),
    ensures count <= 64, count > 0 ==> count <= line[count as int - 1].end,
    decreases count,
{
    if count > 0 {
        line_count_bound(line, (count - 1) as nat);
        assert(axis::interval_valid(line[count as int - 1]));
        if count > 1 {
            assert(line[count as int - 2].end <= line[count as int - 1].start);
        }
    }
}

pub(super) proof fn append_unsplit(line: Seq<Interval>, runs: Seq<Run>, i: int)
    requires axis::line_valid(line), 0 <= i < line.len(), progress(line, 0, runs, line[i].start as int),
    ensures progress(line, 0, runs.push(Run { value: line[i].value, x_start: line[i].start, x_end: line[i].end }), line[i].end as int),
{
    let item = line[i];
    assert(axis::interval_valid(item));
    assert forall|p: u32| item.start < p < item.end implies !#[trigger] bit128(0, p) by {
        assert((0u128 >> p) & 1 == 0) by(bit_vector);
    }
    piece(line, 0, i, item.start, item.end);
    append_piece(line, 0, runs, i, item.start, item.end);
}

pub(super) proof fn mask_started(original: u128,coordinates: u128,start: u8,end: u8,mask: u64)
    requires start<end<=64,crate::geometry::coord_bits((start+1) as u8,(end-1) as u8,coordinates),
        mask==(original & coordinates) as u64,
    ensures remaining(original,mask,start as int,end as int),
{
    assert forall|p: u64|p<64 implies (#[trigger] axis::bit(mask,p)
        <==> bit128(original,p as u32) && start<p<end) by {
        let q=p as u8;
        assert(((coordinates>>q)&1)==if start<p<end{1u128}else{0u128});
        assert(((mask>>p)&1)==if ((original>>(p as u32))&1)==1 && start<p<end{1u64}else{0u64}) by(bit_vector)
            requires p<64,mask==(original&coordinates) as u64,
                ((coordinates>>(p as u8))&1)==if start<p<end{1u128}else{0u128};
    }
}

proof fn prefix_span(mask: u64,len: nat,start: int,end: int)
    requires len<=64,0<=start<end<=64,
        forall|p: u64|p<64 && #[trigger] axis::bit(mask,p) ==> start<p<end,
    ensures crate::std_adapter::bit_count::prefix(mask as u128,len)
        <=if len<=start+1{0}else if len>=end{end-start-1}else{len-start-1},
    decreases len,
{
    if len>0 {
        prefix_span(mask,(len-1) as nat,start,end);let p=(len-1) as u32;
        if ((mask as u128)>>p)&1==1 {
            assert(axis::bit(mask,p as u64)) by(bit_vector) requires p<64,((mask as u128)>>p)&1==1;
            assert(start<p<end);
        }
    }
}

pub(super) proof fn split_capacity(original: u128,mask: u64,start: int,end: int)
    requires remaining(original,mask,start,end),0<=start<end<=64,
    ensures crate::std_adapter::u64_ones(mask)<=end-start-1,
{
    prefix_span(mask,64,start,end);
}

pub(super) proof fn next_split(original: u128,mask: u64,start: int,end: int)
    requires remaining(original,mask,start,end),mask!=0,0<=start<end<=64,
    ensures start<vstd::std_specs::bits::u64_trailing_zeros(mask)<end,
        bit128(original,vstd::std_specs::bits::u64_trailing_zeros(mask)),
        remaining(original,mask & ((mask-1) as u64),vstd::std_specs::bits::u64_trailing_zeros(mask) as int,end),
        mask & ((mask-1) as u64)<mask,
        forall|p: u32|start<p<vstd::std_specs::bits::u64_trailing_zeros(mask) ==> !#[trigger] bit128(original,p),
{
    let split=vstd::std_specs::bits::u64_trailing_zeros(mask) as u64;
    let next=mask & ((mask-1) as u64);
    axis::cleared_lowest(mask,split,next);
    assert(start<split<end);
    assert forall|p: u64|p<64 implies (#[trigger] axis::bit(next,p)
        <==>bit128(original,p as u32) && split<p<end) by {
        if p<split {assert(!axis::bit(mask,p));}
    }
    assert forall|p: u32|start<p<split implies !#[trigger] bit128(original,p) by {
        assert(!axis::bit(mask,p as u64));
    }
}

pub(super) proof fn no_split(original: u128,start: int,end: int)
    requires remaining(original,0,start,end),0<=start<end<=64,
    ensures forall|p: u32|start<p<end ==> !#[trigger] bit128(original,p),
{
    assert forall|p: u32|start<p<end implies !#[trigger] bit128(original,p) by {
        assert(!axis::bit(0,p as u64)) by(bit_vector);
    }
}

pub(super) proof fn finished(line: Seq<Interval>,mask: u128,runs: Seq<Run>)
    requires axis::line_valid(line),progress(line,mask,runs,if line.len()==0{0}else{line.last().end as int}),
    ensures progress(line,mask,runs,64),
{
    let end=if line.len()==0{0}else{line.last().end as int};
    if line.len()>0 {assert(axis::interval_valid(line.last()));}
    assert(ordered(runs) && runs.len()<=64);
    assert forall|p: int|end<=p<64 implies #[trigger] axis::line_value(line,p)==0 by {
        if exists|j: int|0<=j<line.len() && #[trigger] line[j].start<=p<line[j].end {
            let j=choose|j: int|0<=j<line.len() && #[trigger] line[j].start<=p<line[j].end;
            assert(axis::interval_valid(line[j]));
            if j<line.len()-1 {assert(line[j].end<=line.last().start);}
        }
    }
    assert forall|p: int|0<=p<64 implies (#[trigger] axis::line_value(line,p)!=0
        <==>exists|j: int|0<=j<runs.len() && #[trigger] covers(runs[j],p)) by {
        if p>=end {
            assert(axis::line_value(line,p)==0);
            if exists|j: int|0<=j<runs.len() && #[trigger] covers(runs[j],p) {
                let j=choose|j: int|0<=j<runs.len() && #[trigger] covers(runs[j],p);
                assert(raw_run(line,mask,runs[j]));assert(runs[j].x_end<=end);
            }
        }
    }
    assert forall|j: int|0<=j<runs.len() implies #[trigger] raw_run(line,mask,runs[j]) && runs[j].x_end<=64 by {
        assert(raw_run(line,mask,runs[j]));
    }
}

pub(super) open spec fn overlaps(mask: u64,start: u8,end: u8) -> bool {
    exists|p: u8|p<64 && start<=p<end && #[trigger] bit64(mask,p)
}

pub(super) proof fn overlap_mask(mask: u64,start: u8,end: u8,range: u64)
    requires start<=64,end<=64,crate::geometry::cell_bits(start,end,range),
    ensures mask&range!=0 <==> overlaps(mask,start,end),
{
    let both=mask&range;
    if both!=0 {
        vstd::std_specs::bits::axiom_u64_trailing_zeros(both);
        let p=vstd::std_specs::bits::u64_trailing_zeros(both) as u8;
        assert(p<64 && bit64(both,p));
        assert(bit64(mask,p) && bit64(range,p)) by(bit_vector)
            requires both==mask&range,(both>>p)&1==1;
        assert(start<=p<end);
        assert(exists|p: u8|p<64 && start<=p<end && #[trigger] bit64(mask,p));
    }
    if overlaps(mask,start,end) {
        let p=choose|p: u8|p<64 && start<=p<end && #[trigger] bit64(mask,p);
        assert(bit64(range,p));
        assert(both!=0) by(bit_vector) requires both==mask&range,(mask>>p)&1==1,(range>>p)&1==1;
    }
}

pub(super) proof fn empty_overlap(start: u8,end: u8)
    ensures !overlaps(0,start,end),
{
    if overlaps(0,start,end) {
        let p=choose|p: u8|p<64 && start<=p<end && #[trigger] bit64(0,p);
        assert((0u64>>p)&1==0) by(bit_vector);
    }
}

}
