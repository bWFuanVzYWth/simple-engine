//! 像素域上的 good chord；有限编号只用于擦除式集合与单射。

use super::{colored, corners};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn quadrants(labels: Seq<u16>,x: int,y: int,value: u16) -> int {
    corners::count(corners::at(labels,x-1,y-1)==value,corners::at(labels,x,y-1)==value,
        corners::at(labels,x,y)==value,corners::at(labels,x-1,y)==value)
}

pub(crate) open spec fn unit_inside(labels: Seq<u16>,vertical: bool,line: int,t: int,value: u16) -> bool {
    value!=0 && (if vertical {
        corners::at(labels,line-1,t)==value && corners::at(labels,line,t)==value
    } else {
        corners::at(labels,t,line-1)==value && corners::at(labels,t,line)==value
    })
}

pub(crate) open spec fn reflex(labels: Seq<u16>,vertical: bool,line: int,t: int,value: u16) -> bool {
    value!=0 && quadrants(labels,if vertical {line}else{t},if vertical{t}else{line},value)==3
}

pub(crate) open spec fn chord(labels: Seq<u16>,vertical: bool,line: int,start: int,end: int,value: u16) -> bool {
    1<=line<=63 && 1<=start<end<=63 && value!=0
        && reflex(labels,vertical,line,start,value) && reflex(labels,vertical,line,end,value)
        && forall|t: int| start<=t<end ==> #[trigger] unit_inside(labels,vertical,line,t,value)
}

pub(crate) proof fn interior_full(labels: Seq<u16>,vertical: bool,line: int,start: int,end: int,value: u16,t: int)
    requires chord(labels,vertical,line,start,end,value), start<t<end,
    ensures quadrants(labels,if vertical{line}else{t},if vertical{t}else{line},value)==4,
{
    assert(unit_inside(labels,vertical,line,t-1,value));
    assert(unit_inside(labels,vertical,line,t,value));
}

pub(crate) proof fn interior_value_unique(labels: Seq<u16>,x: int,y: int,a: u16,b: u16)
    requires a!=0,b!=0,quadrants(labels,x,y,a)>=3,quadrants(labels,x,y,b)>=3,
    ensures a==b,
{
    if corners::at(labels,x-1,y-1)==a {} if corners::at(labels,x,y-1)==a {}
    if corners::at(labels,x,y)==a {} if corners::at(labels,x-1,y)==a {}
}

pub(crate) proof fn closed_point(labels: Seq<u16>,vertical: bool,line: int,start: int,end: int,value: u16,t: int)
    requires chord(labels,vertical,line,start,end,value),start<=t<=end,
    ensures quadrants(labels,if vertical{line}else{t},if vertical{t}else{line},value)>=3,
{
    if start<t<end {interior_full(labels,vertical,line,start,end,value,t);}
}

/// 不同标签的真实 good chord 连闭端点也不相交。
pub(crate) proof fn crossing_value(labels: Seq<u16>,hy: int,hx0: int,hx1: int,hvalue: u16,
    vx: int,vy0: int,vy1: int,vvalue: u16)
    requires chord(labels,false,hy,hx0,hx1,hvalue),chord(labels,true,vx,vy0,vy1,vvalue),
        hx0<=vx<=hx1,vy0<=hy<=vy1,
    ensures hvalue==vvalue,
{
    closed_point(labels,false,hy,hx0,hx1,hvalue,vx);
    closed_point(labels,true,vx,vy0,vy1,vvalue,hy);
    interior_value_unique(labels,vx,hy,hvalue,vvalue);
}

/// 同标签每条 chord 已是内部直段的最大范围。
pub(crate) proof fn endpoint_blocks(labels: Seq<u16>,vertical: bool,line: int,start: int,end: int,value: u16)
    requires chord(labels,vertical,line,start,end,value),
    ensures !unit_inside(labels,vertical,line,start-1,value),!unit_inside(labels,vertical,line,end,value),
{
    assert(unit_inside(labels,vertical,line,start,value));
    assert(unit_inside(labels,vertical,line,end-1,value));
}

pub(crate) proof fn same_line_disjoint(labels: Seq<u16>,vertical: bool,line: int,a0: int,a1: int,av: u16,
    b0: int,b1: int,bv: u16)
    requires chord(labels,vertical,line,a0,a1,av),chord(labels,vertical,line,b0,b1,bv),
        a0!=b0 || a1!=b1 || av!=bv,
    ensures a1<b0 || b1<a0,
{
    if !(a1<b0 || b1<a0) {
        let t=if a0>b0 {a0}else{b0};
        closed_point(labels,vertical,line,a0,a1,av,t);
        closed_point(labels,vertical,line,b0,b1,bv,t);
        interior_value_unique(labels,if vertical{line}else{t},if vertical{t}else{line},av,bv);
        endpoint_blocks(labels,vertical,line,a0,a1,av);
        endpoint_blocks(labels,vertical,line,b0,b1,bv);
        if a0<b0 {
            assert(unit_inside(labels,vertical,line,b0-1,av));
        } else if b0<a0 {
            assert(unit_inside(labels,vertical,line,a0-1,bv));
        } else if a1<b1 {
            assert(unit_inside(labels,vertical,line,a1,bv));
        } else if b1<a1 {
            assert(unit_inside(labels,vertical,line,b1,av));
        }
    }
}

pub(crate) open spec fn vertical(id: int) -> bool { id>=262144 }
pub(crate) open spec fn line(id: int) -> int { id%262144/4096 }
pub(crate) open spec fn start(id: int) -> int { id%4096/64 }
pub(crate) open spec fn end(id: int) -> int { id%64 }
pub(crate) open spec fn value(labels: Seq<u16>,id: int) -> u16 {
    if vertical(id) {corners::at(labels,line(id),start(id))}
    else {corners::at(labels,start(id),line(id))}
}

pub(crate) open spec fn good(labels: Seq<u16>,id: int) -> bool {
    0<=id<524288 && chord(labels,vertical(id),line(id),start(id),end(id),value(labels,id))
}

pub(crate) open spec fn encode(vertical: bool,line: int,start: int,end: int) -> int {
    (if vertical {262144int}else{0})+4096*line+64*start+end
}

pub(crate) proof fn encoded_chord(labels: Seq<u16>,v: bool,l: int,s: int,e: int,value: u16)
    requires chord(labels,v,l,s,e,value),
    ensures good(labels,encode(v,l,s,e)),vertical(encode(v,l,s,e))==v,
        line(encode(v,l,s,e))==l,start(encode(v,l,s,e))==s,end(encode(v,l,s,e))==e,
        self::value(labels,encode(v,l,s,e))==value,
{
    let id=encode(v,l,s,e);
    assert(unit_inside(labels,v,l,s,value));
    if v {
        assert(id==262144+4096*l+64*s+e);
    } else {
        assert(id==4096*l+64*s+e);
    }
    assert(id%262144==4096*l+64*s+e) by(nonlinear_arith)
        requires 1<=l<=63,1<=s<e<=63,id==(if v {262144int}else{0})+4096*l+64*s+e;
    assert((4096*l+64*s+e)/4096==l) by(nonlinear_arith) requires 1<=s<e<=63;
    assert(id%4096==64*s+e) by(nonlinear_arith)
        requires 1<=l<=63,1<=s<e<=63,id==(if v {262144int}else{0})+4096*l+64*s+e;
    assert((64*s+e)/64==s) by(nonlinear_arith) requires 1<=e<=63;
    assert(id%64==e) by(nonlinear_arith)
        requires 1<=l<=63,1<=s<e<=63,id==(if v {262144int}else{0})+4096*l+64*s+e;
}

pub(crate) open spec fn intersects(a: int,b: int) -> bool {
    if vertical(a)==vertical(b) {
        line(a)==line(b) && start(a)<=end(b) && start(b)<=end(a)
    } else if vertical(a) {
        start(a)<=line(b)<=end(a) && start(b)<=line(a)<=end(b)
    } else {
        start(b)<=line(a)<=end(b) && start(a)<=line(b)<=end(a)
    }
}

pub(crate) proof fn decoded_id(id: int)
    requires 0<=id<524288,
    ensures id==encode(vertical(id),line(id),start(id),end(id)),
{
    if vertical(id) {
        assert(id==encode(vertical(id),line(id),start(id),end(id))) by(nonlinear_arith)
            requires 262144<=id<524288;
    } else {
        assert(id==encode(vertical(id),line(id),start(id),end(id))) by(nonlinear_arith)
            requires 0<=id<262144;
    }
}

pub(crate) open spec fn capacity_slot(id: int) -> int { (line(id)-1)*31+(start(id)-1)/2 }

/// 同线闭段不交，起点至少相隔 2；每线 63 点只能容纳 31 条 chord。
pub(crate) proof fn capacity_injection(labels: Seq<u16>,a: int,b: int)
    requires good(labels,a),good(labels,b),vertical(a)==vertical(b),capacity_slot(a)==capacity_slot(b),
    ensures a==b,
{
    decoded_id(a);
    decoded_id(b);
    assert(line(a)==line(b)) by(nonlinear_arith)
        requires 1<=line(a)<=63,1<=line(b)<=63,1<=start(a)<end(a)<=63,1<=start(b)<end(b)<=63,
            capacity_slot(a)==capacity_slot(b);
    if a!=b {
        same_line_disjoint(labels,vertical(a),line(a),start(a),end(a),value(labels,a),
            start(b),end(b),value(labels,b));
        assert(false) by(nonlinear_arith)
            requires end(a)<start(b) || end(b)<start(a),1<=start(a)<end(a)<=63,
                1<=start(b)<end(b)<=63,(start(a)-1)/2==(start(b)-1)/2;
    }
}

pub(crate) open spec fn chords(labels: Seq<u16>,direction: bool) -> Set<int> {
    Set::<int>::range(0,524288).filter(|id: int|good(labels,id) && vertical(id)==direction)
}

pub(crate) proof fn chord_capacity(labels: Seq<u16>,direction: bool)
    ensures chords(labels,direction).len()<=1953,
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=chords(labels,direction);
    let slots=Set::<int>::range(0,1953);
    let f=|id: int|capacity_slot(id);
    let image=input.map(f);
    vstd::set_lib::lemma_int_range(0,524288);
    vstd::set_lib::lemma_int_range(0,1953);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int| input.contains(a) && input.contains(b)
            && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            capacity_injection(labels,a,b);
        }
    }
    vstd::set_lib::lemma_map_size(input,image,f);
    assert(image.subset_of(slots)) by {
        assert forall|s: int| #[trigger] image.contains(s) implies slots.contains(s) by {
            let id=choose|id: int| input.contains(id) && f(id)==s;
            assert(0<=capacity_slot(id)<1953);
        }
    }
    vstd::set_lib::lemma_len_subset(image,slots);
}

pub(crate) proof fn conflict_point_injection(labels: Seq<u16>,h: int,v: int,other_h: int,other_v: int)
    requires good(labels,h),good(labels,v),good(labels,other_h),good(labels,other_v),
        !vertical(h),vertical(v),!vertical(other_h),vertical(other_v),
        intersects(h,v),intersects(other_h,other_v),line(h)==line(other_h),line(v)==line(other_v),
    ensures h==other_h,v==other_v,
{
    decoded_id(h); decoded_id(v); decoded_id(other_h); decoded_id(other_v);
    if h!=other_h {
        same_line_disjoint(labels,false,line(h),start(h),end(h),value(labels,h),
            start(other_h),end(other_h),value(labels,other_h));
    }
    if v!=other_v {
        same_line_disjoint(labels,true,line(v),start(v),end(v),value(labels,v),
            start(other_v),end(other_v),value(labels,other_v));
    }
}

pub(crate) open spec fn conflicts(labels: Seq<u16>) -> Set<int> {
    Set::<int>::range(0,274877906944).filter(|edge: int| {
        let h=edge/524288;
        let v=edge%524288;
        good(labels,h) && good(labels,v) && !vertical(h) && vertical(v) && intersects(h,v)
    })
}

pub(crate) open spec fn conflict_slot(edge: int) -> int {
    (line(edge%524288)-1)*63+line(edge/524288)-1
}

/// 闭冲突边单射到 63×63 内部格点，提供实际写边之前的全域容量界。
pub(crate) proof fn conflict_capacity(labels: Seq<u16>)
    ensures conflicts(labels).len()<=3969,
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=conflicts(labels);
    let slots=Set::<int>::range(0,3969);
    let f=|edge: int|conflict_slot(edge);
    let image=input.map(f);
    vstd::set_lib::lemma_int_range(0,274877906944);
    vstd::set_lib::lemma_int_range(0,3969);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int| input.contains(a) && input.contains(b)
            && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            let ah=a/524288;let av=a%524288;
            let bh=b/524288;let bv=b%524288;
            assert(line(ah)==line(bh) && line(av)==line(bv)) by(nonlinear_arith)
                requires 1<=line(ah)<=63,1<=line(av)<=63,1<=line(bh)<=63,1<=line(bv)<=63,
                    (line(av)-1)*63+line(ah)-1==(line(bv)-1)*63+line(bh)-1;
            conflict_point_injection(labels,ah,av,bh,bv);
            assert(a==a/524288*524288+a%524288);
            assert(b==b/524288*524288+b%524288);
        }
    }
    vstd::set_lib::lemma_map_size(input,image,f);
    assert(image.subset_of(slots)) by {
        assert forall|s: int| #[trigger] image.contains(s) implies slots.contains(s) by {
            let edge=choose|edge: int| input.contains(edge) && f(edge)==s;
        }
    }
    vstd::set_lib::lemma_len_subset(image,slots);
}

pub(crate) open spec fn neighbors(labels: Seq<u16>,v: int) -> Set<int> {
    chords(labels,false).filter(|h: int|intersects(h,v))
}

pub(crate) open spec fn independent(labels: Seq<u16>,selected: Set<int>) -> bool {
    (forall|id: int| #[trigger] selected.contains(id) ==> good(labels,id))
        && (forall|h: int,v: int| #[trigger] selected.contains(h) && #[trigger] selected.contains(v)
            && !vertical(h) && vertical(v) ==> !intersects(h,v))
}

pub(crate) open spec fn maximum_independent(labels: Seq<u16>,selected: Set<int>) -> bool {
    independent(labels,selected) && forall|other: Set<int>| #[trigger] independent(labels,other)
        ==> other.len()<=selected.len()
}

pub(crate) proof fn degree_capacity(labels: Seq<u16>,v: int)
    requires good(labels,v),vertical(v),
    ensures neighbors(labels,v).len()<=63,
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=neighbors(labels,v);
    let slots=Set::<int>::range(1,64);
    let f=|h: int|line(h);
    let image=input.map(f);
    vstd::set_lib::lemma_int_range(0,524288);
    vstd::set_lib::lemma_int_range(1,64);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int| input.contains(a) && input.contains(b)
            && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            conflict_point_injection(labels,a,v,b,v);
        }
    }
    vstd::set_lib::lemma_map_size(input,image,f);
    assert(image.subset_of(slots)) by {
        assert forall|s: int| #[trigger] image.contains(s) implies slots.contains(s) by {
            let h=choose|h: int|input.contains(h) && f(h)==s;
        }
    }
    vstd::set_lib::lemma_len_subset(image,slots);
}

}
