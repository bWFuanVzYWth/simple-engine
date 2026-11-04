//! 矩形扫描的唯一数学序列：按关闭行及 x 起点排序，不增加执行期存储。

use super::{RangeU8, Rectangle, corners, covers, partition, split_geometry};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn start_at(labels: Seq<u16>,verticals: Set<int>,y: int,x: int) -> int
    decreases if x>0 {x}else{0},
{
    if x<=0 || corners::at(labels,x-1,y)!=corners::at(labels,x,y) || split_geometry::active(verticals,x,y) {x}
    else {start_at(labels,verticals,y,x-1)}
}

pub(crate) open spec fn end_at(labels: Seq<u16>,verticals: Set<int>,y: int,x: int) -> int
    decreases if x<64 {64-x}else{0},
{
    if x>=63 || corners::at(labels,x+1,y)!=corners::at(labels,x,y) || split_geometry::active(verticals,x+1,y) {x+1}
    else {end_at(labels,verticals,y,x+1)}
}

pub(crate) proof fn row_prefix(labels: Seq<u16>,verticals: Set<int>,y: int,x: int)
    requires 0<=y<64,0<=x<64,corners::at(labels,x,y)!=0,
    ensures 0<=start_at(labels,verticals,y,x)<=x,
        corners::at(labels,start_at(labels,verticals,y,x)-1,y)!=corners::at(labels,x,y)
            ||split_geometry::active(verticals,start_at(labels,verticals,y,x),y),
        forall|p: int| start_at(labels,verticals,y,x)<=p<=x ==> #[trigger] corners::at(labels,p,y)==corners::at(labels,x,y),
        forall|p: int| start_at(labels,verticals,y,x)<p<=x ==> !#[trigger] split_geometry::active(verticals,p,y),
    decreases x,
{
    if x>0 && corners::at(labels,x-1,y)==corners::at(labels,x,y) && !split_geometry::active(verticals,x,y) {
        row_prefix(labels,verticals,y,x-1);
    }
}

pub(crate) proof fn row_suffix(labels: Seq<u16>,verticals: Set<int>,y: int,x: int)
    requires 0<=y<64,0<=x<64,corners::at(labels,x,y)!=0,
    ensures x<end_at(labels,verticals,y,x)<=64,
        corners::at(labels,end_at(labels,verticals,y,x),y)!=corners::at(labels,x,y)
            ||split_geometry::active(verticals,end_at(labels,verticals,y,x),y),
        forall|p: int| x<=p<end_at(labels,verticals,y,x) ==> #[trigger] corners::at(labels,p,y)==corners::at(labels,x,y),
        forall|p: int| x<p<end_at(labels,verticals,y,x) ==> !#[trigger] split_geometry::active(verticals,p,y),
    decreases 64-x,
{
    if x<63 && corners::at(labels,x+1,y)==corners::at(labels,x,y) && !split_geometry::active(verticals,x+1,y) {
        row_suffix(labels,verticals,y,x+1);
    }
}

pub(crate) proof fn pixel_run(labels: Seq<u16>,verticals: Set<int>,y: int,x: int)
    requires 0<=y<64,0<=x<64,corners::at(labels,x,y)!=0,
    ensures split_geometry::run(labels,verticals,y,start_at(labels,verticals,y,x),
        end_at(labels,verticals,y,x),corners::at(labels,x,y)),
        start_at(labels,verticals,y,x)<=x<end_at(labels,verticals,y,x),
{
    row_prefix(labels,verticals,y,x);row_suffix(labels,verticals,y,x);
}

pub(crate) proof fn overlap_unique(labels: Seq<u16>,verticals: Set<int>,y: int,
    a: int,b: int,value: u16,c: int,d: int,other: u16,x: int)
    requires split_geometry::run(labels,verticals,y,a,b,value),split_geometry::run(labels,verticals,y,c,d,other),
        a<=x<b,c<=x<d,
    ensures a==c,b==d,value==other,
{
    assert(corners::at(labels,x,y)==value && corners::at(labels,x,y)==other);
    if a<c {assert(corners::at(labels,c-1,y)==value);assert(!split_geometry::active(verticals,c,y));}
    if c<a {assert(corners::at(labels,a-1,y)==other);assert(!split_geometry::active(verticals,a,y));}
    split_geometry::row_start_unique(labels,verticals,y,a,b,d,value,other);
}

pub(crate) proof fn run_at(labels: Seq<u16>,verticals: Set<int>,y: int,a: int,b: int,value: u16,x: int)
    requires split_geometry::run(labels,verticals,y,a,b,value),a<=x<b,
    ensures start_at(labels,verticals,y,x)==a,end_at(labels,verticals,y,x)==b,corners::at(labels,x,y)==value,
{
    assert(corners::at(labels,x,y)==value);
    pixel_run(labels,verticals,y,x);
    overlap_unique(labels,verticals,y,a,b,value,start_at(labels,verticals,y,x),end_at(labels,verticals,y,x),value,x);
}

pub(crate) open spec fn continues(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,
    y: int,a: int,b: int,value: u16) -> bool {
    split_geometry::run(labels,verticals,y-1,a,b,value) && split_geometry::run(labels,verticals,y,a,b,value)
        && !blocked(y,a,b)
}

pub(crate) open spec fn first_y(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,
    y: int,a: int,b: int,value: u16) -> int
    decreases if y>0 {y}else{0},
{
    if y<=0 || !continues(labels,verticals,blocked,y,a,b,value) {y}
    else {first_y(labels,verticals,blocked,y-1,a,b,value)}
}

pub(crate) open spec fn last_y(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,
    y: int,a: int,b: int,value: u16) -> int
    decreases if y<64 {64-y}else{0},
{
    if y>=63 || !continues(labels,verticals,blocked,y+1,a,b,value) {y}
    else {last_y(labels,verticals,blocked,y+1,a,b,value)}
}

pub(crate) proof fn first_rows(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,
    y: int,a: int,b: int,value: u16)
    requires split_geometry::run(labels,verticals,y,a,b,value),
    ensures 0<=first_y(labels,verticals,blocked,y,a,b,value)<=y,
        first_y(labels,verticals,blocked,y,a,b,value)==0
            || !continues(labels,verticals,blocked,first_y(labels,verticals,blocked,y,a,b,value),a,b,value),
        forall|row: int| first_y(labels,verticals,blocked,y,a,b,value)<=row<=y
            ==> #[trigger] split_geometry::run(labels,verticals,row,a,b,value),
        forall|row: int| first_y(labels,verticals,blocked,y,a,b,value)<row<=y ==> !#[trigger] blocked(row,a,b),
    decreases y,
{
    if y>0 && continues(labels,verticals,blocked,y,a,b,value) {first_rows(labels,verticals,blocked,y-1,a,b,value);}
}

pub(crate) proof fn last_rows(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,
    y: int,a: int,b: int,value: u16)
    requires split_geometry::run(labels,verticals,y,a,b,value),
    ensures y<=last_y(labels,verticals,blocked,y,a,b,value)<64,
        last_y(labels,verticals,blocked,y,a,b,value)==63
            || !continues(labels,verticals,blocked,last_y(labels,verticals,blocked,y,a,b,value)+1,a,b,value),
        forall|row: int| y<=row<=last_y(labels,verticals,blocked,y,a,b,value)
            ==> #[trigger] split_geometry::run(labels,verticals,row,a,b,value),
        forall|row: int| y<row<=last_y(labels,verticals,blocked,y,a,b,value) ==> !#[trigger] blocked(row,a,b),
    decreases 64-y,
{
    if y<63 && continues(labels,verticals,blocked,y+1,a,b,value) {last_rows(labels,verticals,blocked,y+1,a,b,value);}
}

pub(crate) open spec fn ending(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,p: int) -> bool {
    let y=p/64;let a=p%64;let b=end_at(labels,verticals,y,a);let value=corners::at(labels,a,y);
    0<=p<4096 && value!=0 && start_at(labels,verticals,y,a)==a
        && !continues(labels,verticals,blocked,y+1,a,b,value)
}

pub(crate) open spec fn rectangle(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,p: int) -> Rectangle {
    let y=p/64;let a=p%64;let b=end_at(labels,verticals,y,a);let value=corners::at(labels,a,y);
    Rectangle {value,x: RangeU8{start:a as u8,end:b as u8},
        y:RangeU8{start:first_y(labels,verticals,blocked,y,a,b,value) as u8,end:(y+1) as u8}}
}

pub(crate) proof fn ending_run(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,p: int)
    requires ending(labels,verticals,blocked,p),
    ensures super::valid(rectangle(labels,verticals,blocked,p)),
        split_geometry::run(labels,verticals,p/64,p%64,end_at(labels,verticals,p/64,p%64),corners::at(labels,p%64,p/64)),
        rectangle(labels,verticals,blocked,p).x.start as int==p%64,
        rectangle(labels,verticals,blocked,p).x.end as int==end_at(labels,verticals,p/64,p%64),
        rectangle(labels,verticals,blocked,p).y.end as int==p/64+1,
        rectangle(labels,verticals,blocked,p).y.start as int==first_y(labels,verticals,blocked,p/64,p%64,
            end_at(labels,verticals,p/64,p%64),corners::at(labels,p%64,p/64)),
{
    let y=p/64;let a=p%64;let b=end_at(labels,verticals,y,a);let value=corners::at(labels,a,y);
    pixel_run(labels,verticals,y,a);first_rows(labels,verticals,blocked,y,a,b,value);
}

pub(crate) proof fn ending_covers(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,p: int,q: int)
    requires ending(labels,verticals,blocked,p),covers(rectangle(labels,verticals,blocked,p),q),
    ensures labels[q]==rectangle(labels,verticals,blocked,p).value,
{
    ending_run(labels,verticals,blocked,p);
    let y=p/64;let a=p%64;let b=end_at(labels,verticals,y,a);let value=corners::at(labels,a,y);
    first_rows(labels,verticals,blocked,y,a,b,value);
    assert(split_geometry::run(labels,verticals,q/64,a,b,value));
    assert(corners::at(labels,q%64,q/64)==value);
    assert(q==q/64*64+q%64);
}

pub(crate) proof fn covering_ending(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,pixel: int)
    requires 0<=pixel<4096,labels[pixel]!=0,
    ensures exists|p: int| #[trigger] ending(labels,verticals,blocked,p)
        && covers(rectangle(labels,verticals,blocked,p),pixel),
{
    let x=pixel%64;let y=pixel/64;
    assert(pixel==y*64+x);let value=labels[pixel];
    pixel_run(labels,verticals,y,x);
    let a=start_at(labels,verticals,y,x);let b=end_at(labels,verticals,y,x);
    last_rows(labels,verticals,blocked,y,a,b,value);let z=last_y(labels,verticals,blocked,y,a,b,value);
    let p=z*64+a;
    assert(p%64==a && p/64==z) by(nonlinear_arith) requires p==z*64+a,0<=a<64;
    assert(split_geometry::run(labels,verticals,z,a,b,value));run_at(labels,verticals,z,a,b,value,a);
    assert(ending(labels,verticals,blocked,p));
    first_rows(labels,verticals,blocked,z,a,b,value);let first=first_y(labels,verticals,blocked,z,a,b,value);
    if first>y {
        assert(split_geometry::run(labels,verticals,first-1,a,b,value));
        assert(split_geometry::run(labels,verticals,first,a,b,value));
        assert(!blocked(first,a,b));assert(continues(labels,verticals,blocked,first,a,b,value));
    }
    ending_run(labels,verticals,blocked,p);
    assert(covers(rectangle(labels,verticals,blocked,p),pixel));
}

pub(crate) proof fn disjoint_endings(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,p: int,q: int,pixel: int)
    requires ending(labels,verticals,blocked,p),ending(labels,verticals,blocked,q),
        covers(rectangle(labels,verticals,blocked,p),pixel),covers(rectangle(labels,verticals,blocked,q),pixel),
    ensures p==q,
{
    ending_run(labels,verticals,blocked,p);ending_run(labels,verticals,blocked,q);
    let py=p/64;let px=p%64;let pb=end_at(labels,verticals,py,px);let pv=corners::at(labels,px,py);
    let qy=q/64;let qx=q%64;let qb=end_at(labels,verticals,qy,qx);let qv=corners::at(labels,qx,qy);
    first_rows(labels,verticals,blocked,py,px,pb,pv);first_rows(labels,verticals,blocked,qy,qx,qb,qv);
    assert(split_geometry::run(labels,verticals,pixel/64,px,pb,pv));
    assert(split_geometry::run(labels,verticals,pixel/64,qx,qb,qv));
    overlap_unique(labels,verticals,pixel/64,px,pb,pv,qx,qb,qv,pixel%64);
    if py<qy {
        assert(split_geometry::run(labels,verticals,py+1,px,pb,pv));assert(!blocked(py+1,px,pb));
        assert(continues(labels,verticals,blocked,py+1,px,pb,pv));
    }
    if qy<py {
        assert(split_geometry::run(labels,verticals,qy+1,qx,qb,qv));assert(!blocked(qy+1,qx,qb));
        assert(continues(labels,verticals,blocked,qy+1,qx,qb,qv));
    }
    assert(p==py*64+px);assert(q==qy*64+qx);
}

pub(crate) open spec fn anchors(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,n: nat) -> Seq<int>
    decreases n,
{
    if n==0 {Seq::empty()} else {
        let prefix=anchors(labels,verticals,blocked,(n-1) as nat);
        if ending(labels,verticals,blocked,n-1) {prefix.push(n-1)} else {prefix}
    }
}

pub(crate) proof fn anchor_prefix(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,n: nat)
    ensures anchors(labels,verticals,blocked,n).len() as int==sum(n,|p: int|bit(ending(labels,verticals,blocked,p))),
        forall|i: int| 0<=i<anchors(labels,verticals,blocked,n).len()
            ==> 0<=#[trigger] anchors(labels,verticals,blocked,n)[i]<n
                && ending(labels,verticals,blocked,anchors(labels,verticals,blocked,n)[i]),
        forall|p: int| 0<=p<n ==> (#[trigger] ending(labels,verticals,blocked,p)
            <==> exists|i: int| 0<=i<anchors(labels,verticals,blocked,n).len()
                && #[trigger] anchors(labels,verticals,blocked,n)[i]==p),
        forall|i: int,j: int| 0<=i<j<anchors(labels,verticals,blocked,n).len()
            ==> #[trigger] anchors(labels,verticals,blocked,n)[i]<#[trigger] anchors(labels,verticals,blocked,n)[j],
    decreases n,
{
    if n>0 {
        anchor_prefix(labels,verticals,blocked,(n-1) as nat);
        let previous=anchors(labels,verticals,blocked,(n-1) as nat);
        let current=anchors(labels,verticals,blocked,n);
        assert forall|p: int| 0<=p<n implies (#[trigger] ending(labels,verticals,blocked,p)
            <==> exists|i: int| 0<=i<current.len() && #[trigger] current[i]==p) by {
            if ending(labels,verticals,blocked,p) {
                if p<n-1 {
                    let i=choose|i: int|0<=i<previous.len() && #[trigger] previous[i]==p;
                    assert(current[i]==p);
                    assert(exists|i: int|0<=i<current.len() && #[trigger] current[i]==p);
                } else {
                    assert(current[previous.len() as int]==p);
                    assert(exists|i: int|0<=i<current.len() && #[trigger] current[i]==p);
                }
            } else if exists|i: int|0<=i<current.len() && #[trigger] current[i]==p {
                let i=choose|i: int|0<=i<current.len() && #[trigger] current[i]==p;
                if i<previous.len() {assert(previous[i]==p);}
            }
        }
    }
}

pub(crate) open spec fn canonical(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool) -> Seq<Rectangle> {
    anchors(labels,verticals,blocked,4096).map_values(|p: int|rectangle(labels,verticals,blocked,p))
}

/// 任意 H 阻断谓词与任意 V 细分均得到精确同标签分区，不依赖最优选线。
pub(crate) proof fn canonical_partition(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool)
    requires labels.len()==4096,
    ensures partition(labels,canonical(labels,verticals,blocked)),
{
    let ids=anchors(labels,verticals,blocked,4096);let output=canonical(labels,verticals,blocked);
    anchor_prefix(labels,verticals,blocked,4096);
    assert forall|i: int| 0<=i<output.len() implies #[trigger] super::valid(output[i]) by {ending_run(labels,verticals,blocked,ids[i]);}
    assert forall|i: int,p: int| 0<=i<output.len() && #[trigger] covers(output[i],p)
        implies labels[p]==output[i].value by {ending_covers(labels,verticals,blocked,ids[i],p);}
    assert forall|p: int| 0<=p<4096 implies (labels[p]!=0 <==>
        exists|i: int| 0<=i<output.len() && #[trigger] covers(output[i],p)) by {
        if labels[p]!=0 {
            covering_ending(labels,verticals,blocked,p);
            let anchor=choose|anchor: int| #[trigger] ending(labels,verticals,blocked,anchor)
                && covers(rectangle(labels,verticals,blocked,anchor),p);
            let i=choose|i: int|0<=i<ids.len() && #[trigger] ids[i]==anchor;
            assert(covers(output[i],p));
        } else if exists|i: int| 0<=i<output.len() && #[trigger] covers(output[i],p) {
            let i=choose|i: int|0<=i<output.len() && #[trigger] covers(output[i],p);
            ending_covers(labels,verticals,blocked,ids[i],p);
        }
    }
    assert forall|i: int,j: int,p: int| 0<=i<output.len() && 0<=j<output.len()
        && #[trigger] covers(output[i],p) && #[trigger] covers(output[j],p) implies i==j by {
        disjoint_endings(labels,verticals,blocked,ids[i],ids[j],p);
        if i<j {assert(ids[i]<ids[j]);}if j<i {assert(ids[j]<ids[i]);}
    }
}

pub(crate) proof fn canonical_capacity(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool)
    requires labels.len()==4096,
    ensures canonical(labels,verticals,blocked).len()<=4096,
{
    canonical_partition(labels,verticals,blocked);
    super::rectangle_count_bound(labels,canonical(labels,verticals,blocked));
}

pub(crate) proof fn canonical_order(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,i: int,j: int)
    requires 0<=i<j<canonical(labels,verticals,blocked).len(),
    ensures canonical(labels,verticals,blocked)[i].y.end<canonical(labels,verticals,blocked)[j].y.end
        ||canonical(labels,verticals,blocked)[i].y.end==canonical(labels,verticals,blocked)[j].y.end
            &&canonical(labels,verticals,blocked)[i].x.start<canonical(labels,verticals,blocked)[j].x.start,
{
    anchor_prefix(labels,verticals,blocked,4096);let ids=anchors(labels,verticals,blocked,4096);
    ending_run(labels,verticals,blocked,ids[i]);ending_run(labels,verticals,blocked,ids[j]);
    assert(ids[i]==ids[i]/64*64+ids[i]%64);assert(ids[j]==ids[j]/64*64+ids[j]%64);
}

pub(crate) proof fn anchor_prefix_of(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,n: nat,m: nat)
    requires n<=m,
    ensures anchors(labels,verticals,blocked,n).len()<=anchors(labels,verticals,blocked,m).len(),
        anchors(labels,verticals,blocked,m).take(anchors(labels,verticals,blocked,n).len() as int)
            ==anchors(labels,verticals,blocked,n),
    decreases m,
{
    if n<m {
        anchor_prefix_of(labels,verticals,blocked,n,(m-1) as nat);
        let wanted=anchors(labels,verticals,blocked,n);let previous=anchors(labels,verticals,blocked,(m-1) as nat);
        let current=anchors(labels,verticals,blocked,m);
        assert forall|i: int| 0<=i<wanted.len() implies #[trigger] current[i]==wanted[i] by {
            assert(previous[i]==wanted[i]);
        }
        assert(current.take(wanted.len() as int) =~=wanted);
    }
}

pub(crate) open spec fn prefix(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,n: nat) -> Seq<Rectangle> {
    anchors(labels,verticals,blocked,n).map_values(|p: int|rectangle(labels,verticals,blocked,p))
}

/// 扫描游标可用关闭锚点的范围确定已交付前缀，不保存历史矩形数组。
pub(crate) proof fn canonical_prefix(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,n: nat)
    requires n<=4096,
    ensures prefix(labels,verticals,blocked,n).len()<=canonical(labels,verticals,blocked).len(),
        canonical(labels,verticals,blocked).take(prefix(labels,verticals,blocked,n).len() as int)
            ==prefix(labels,verticals,blocked,n),
{
    anchor_prefix_of(labels,verticals,blocked,n,4096);
    let a=anchors(labels,verticals,blocked,n);let b=anchors(labels,verticals,blocked,4096);
    let part=prefix(labels,verticals,blocked,n);let all=canonical(labels,verticals,blocked);
    assert forall|i: int|0<=i<part.len() implies #[trigger] part[i]==all[i] by {assert(a[i]==b[i]);}
    assert(all.take(part.len() as int) =~=part);
}

pub(crate) proof fn prefix_gap(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,low: nat,high: nat)
    requires low<=high,forall|p: int|low<=p<high ==> !#[trigger] ending(labels,verticals,blocked,p),
    ensures prefix(labels,verticals,blocked,low)==prefix(labels,verticals,blocked,high),
    decreases high-low,
{
    if low<high {prefix_gap(labels,verticals,blocked,low,(high-1) as nat);}
}

pub(crate) proof fn prefix_next(labels: Seq<u16>,verticals: Set<int>,blocked: spec_fn(int,int,int)->bool,n: nat)
    ensures prefix(labels,verticals,blocked,n+1)==if ending(labels,verticals,blocked,n as int) {
        prefix(labels,verticals,blocked,n).push(rectangle(labels,verticals,blocked,n as int))
    }else{prefix(labels,verticals,blocked,n)},
{}

}
