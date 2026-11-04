//! 降序事件链、闭端点活跃 owner 与实际边前缀。

use super::proof as graph;
use super::{ConflictFinalizeScratch, SparseEdge, ValuedChord};
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) open spec fn bit(mask: u64,y: int) -> bool { 0<=y<64 && ((mask>>(y as u64))&1)==1 }

pub(crate) proof fn same_line(chords: Seq<ValuedChord>,vertical: bool,a: int,b: int,t: int)
    requires graph::chords_valid(chords,vertical),0<=a<chords.len(),0<=b<chords.len(),
        chords[a].line()==chords[b].line(),chords[a].start()<=t<=chords[a].end(),chords[b].start()<=t<=chords[b].end(),
    ensures a==b,
{
    if a<b {}else if b<a {}else{}
}

pub(crate) proof fn chord_capacity(chords: Seq<ValuedChord>,vertical: bool)
    requires graph::chords_valid(chords,vertical),
    ensures chords.len()<=1953,
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=Set::<int>::range(0,chords.len() as int);
    let f=|i: int|(chords[i].line()-1)*31+(chords[i].start()-1)/2;
    let slots=Set::<int>::range(0,1953);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int|input.contains(a) && input.contains(b) && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            assert(chords[a].well_formed(vertical) && chords[b].well_formed(vertical));
            let sa=chords[a].start();let sb=chords[b].start();
            assert((sa-1)/2<31 && (sb-1)/2<31);
            assert(chords[a].line()==chords[b].line());
            if sa<=sb {same_line(chords,vertical,a,b,sb);}else{same_line(chords,vertical,a,b,sa);}
        }
    }
    let output=input.map(f);
    vstd::set_lib::lemma_int_range(0,chords.len() as int);vstd::set_lib::lemma_int_range(0,1953);
    vstd::set_lib::lemma_map_size(input,output,f);
    assert(output.subset_of(slots)) by {
        assert forall|s: int|#[trigger] output.contains(s) implies slots.contains(s) by {
            let i=choose|i: int|input.contains(i) && #[trigger] f(i)==s;
            assert(chords[i].well_formed(vertical));
        }
    }
    vstd::set_lib::lemma_len_subset(output,slots);
}

pub(crate) open spec fn edges_valid(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>) -> bool {
    edges.no_duplicates() && forall|p: int|0<=p<edges.len() ==> (#[trigger] edges[p]).left<horizontal.len()
        && edges[p].right<vertical.len() && graph::closed_conflict(horizontal[edges[p].left as int],vertical[edges[p].right as int])
}

proof fn point_injective(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,a: SparseEdge,b: SparseEdge)
    requires graph::graph_chords_valid(horizontal,vertical),a.left<horizontal.len(),b.left<horizontal.len(),a.right<vertical.len(),b.right<vertical.len(),
        graph::closed_conflict(horizontal[a.left as int],vertical[a.right as int]),graph::closed_conflict(horizontal[b.left as int],vertical[b.right as int]),
        horizontal[a.left as int].line()==horizontal[b.left as int].line(),vertical[a.right as int].line()==vertical[b.right as int].line(),
    ensures a==b,
{
    same_line(horizontal,false,a.left as int,b.left as int,vertical[a.right as int].line());
    same_line(vertical,true,a.right as int,b.right as int,horizontal[a.left as int].line());
}

pub(crate) proof fn edge_capacity(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>)
    requires graph::graph_chords_valid(horizontal,vertical),edges_valid(horizontal,vertical,edges),
    ensures edges.len()<=3969,
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=Set::<int>::range(0,edges.len() as int);
    let f=|p: int|(vertical[edges[p].right as int].line()-1)*63+horizontal[edges[p].left as int].line()-1;
    let slots=Set::<int>::range(0,3969);let output=input.map(f);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int|input.contains(a) && input.contains(b) && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            let ea=edges[a];let eb=edges[b];
            assert(horizontal[ea.left as int].well_formed(false) && horizontal[eb.left as int].well_formed(false));
            assert(vertical[ea.right as int].well_formed(true) && vertical[eb.right as int].well_formed(true));
            assert(horizontal[ea.left as int].line()==horizontal[eb.left as int].line());
            assert(vertical[ea.right as int].line()==vertical[eb.right as int].line());
            point_injective(horizontal,vertical,ea,eb);
        }
    }
    vstd::set_lib::lemma_int_range(0,edges.len() as int);vstd::set_lib::lemma_int_range(0,3969);
    vstd::set_lib::lemma_map_size(input,output,f);
    assert(output.subset_of(slots)) by {
        assert forall|s: int|#[trigger] output.contains(s) implies slots.contains(s) by {
            let p=choose|p: int|input.contains(p) && #[trigger] f(p)==s;
            assert(horizontal[edges[p].left as int].well_formed(false));assert(vertical[edges[p].right as int].well_formed(true));
        }
    }
    vstd::set_lib::lemma_len_subset(output,slots);
}

/// kind=0/1 为水平 start/end，kind=2 为竖向 query。
pub(crate) open spec fn coordinate(chord: ValuedChord,kind: int) -> int {
    if kind==0{chord.start()}else if kind==1{chord.end()}else{chord.line()}
}

pub(crate) open spec fn last(chords: Seq<ValuedChord>,kind: int,count: int,x: int) -> int
    decreases count,
{
    if count<=0{-1}else if coordinate(chords[count-1],kind)==x{count-1}else{last(chords,kind,count-1,x)}
}

pub(crate) proof fn last_properties(chords: Seq<ValuedChord>,kind: int,count: int,x: int)
    requires 0<=count<=chords.len(),
    ensures -1<=last(chords,kind,count,x)<count,
        last(chords,kind,count,x)>=0 ==> coordinate(chords[last(chords,kind,count,x)],kind)==x,
        forall|i: int|last(chords,kind,count,x)<i<count ==> #[trigger] coordinate(chords[i],kind)!=x,
    decreases count,
{
    if count>0 {last_properties(chords,kind,count-1,x);}
}

pub(crate) open spec fn encoded(index: int) -> u16 { if index<0{65535u16}else{index as u16} }
pub(crate) open spec fn cursor(index: u16) -> int { if index==65535{-1}else{index as int} }

pub(crate) open spec fn heads(chords: Seq<ValuedChord>,kind: int,count: int,heads: Seq<u16>) -> bool {
    heads.len()==65 && forall|x: int|0<=x<65 ==> #[trigger] heads[x]==encoded(last(chords,kind,count,x))
}

pub(crate) open spec fn links(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,layout: Seq<u32>,h: int,v: int) -> bool {
    layout.len()==horizontal.len()+vertical.len() && 0<=h<=horizontal.len() && 0<=v<=vertical.len()
        && (forall|i: int|0<=i<h ==> #[trigger] layout[i]==((encoded(last(horizontal,0,i,horizontal[i].start())) as u32)
            | ((encoded(last(horizontal,1,i,horizontal[i].end())) as u32)<<16)))
        && (forall|i: int|0<=i<v ==> #[trigger] layout[horizontal.len() as int+i]==encoded(last(vertical,2,i,vertical[i].line())) as u32)
}

pub(crate) open(in crate::optimal) spec fn events(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,scratch: &ConflictFinalizeScratch) -> bool {
    graph::graph_chords_valid(horizontal,vertical) && horizontal.len()<=1953 && vertical.len()<=1953
        && scratch.right_layout.valid() && links(horizontal,vertical,scratch.right_layout@,horizontal.len() as int,vertical.len() as int)
        && heads(horizontal,0,horizontal.len() as int,scratch.horizontal_start_heads@)
        && heads(horizontal,1,horizontal.len() as int,scratch.horizontal_end_heads@)
        && heads(vertical,2,vertical.len() as int,scratch.vertical_query_heads@)
}

pub(crate) proof fn head_cursor(chords: Seq<ValuedChord>,kind: int,count: int,x: int,head: u16)
    requires 0<=count<=chords.len(),count<=1953,head==encoded(last(chords,kind,count,x)),
    ensures -1<=cursor(head)<count,head!=65535 ==> coordinate(chords[cursor(head)],kind)==x,
        forall|i: int|cursor(head)<i<count ==> #[trigger] coordinate(chords[i],kind)!=x,
{
    last_properties(chords,kind,count,x);
}

pub(crate) proof fn next_cursor(chords: Seq<ValuedChord>,kind: int,current: int,x: int)
    requires 0<=current<chords.len(),current<=1953,coordinate(chords[current],kind)==x,
    ensures -1<=cursor(encoded(last(chords,kind,current,x)))<current,
        cursor(encoded(last(chords,kind,current,x)))>=0 ==> coordinate(chords[cursor(encoded(last(chords,kind,current,x)))],kind)==x,
        forall|i: int|cursor(encoded(last(chords,kind,current,x)))<i<current ==> #[trigger] coordinate(chords[i],kind)!=x,
{
    head_cursor(chords,kind,current,x,encoded(last(chords,kind,current,x)));
}

pub(crate) proof fn heads_step(chords: Seq<ValuedChord>,kind: int,count: int,before: Seq<u16>,after: Seq<u16>)
    requires heads(chords,kind,count,before),0<=count<chords.len(),count<1953,
        0<=coordinate(chords[count],kind)<65,after==before.update(coordinate(chords[count],kind),count as u16),
    ensures heads(chords,kind,count+1,after),
{
    assert forall|x: int|0<=x<65 implies #[trigger] after[x]==encoded(last(chords,kind,count+1,x)) by {
        reveal_with_fuel(last,1);
    }
}

pub(crate) proof fn horizontal_link(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,before: Seq<u32>,after: Seq<u32>,count: int,value: u32)
    requires links(horizontal,vertical,before,count,0),0<=count<horizontal.len(),after==before.update(count,value),
        value==((encoded(last(horizontal,0,count,horizontal[count].start())) as u32)
            | ((encoded(last(horizontal,1,count,horizontal[count].end())) as u32)<<16)),
    ensures links(horizontal,vertical,after,count+1,0),
{}

pub(crate) proof fn vertical_link(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,before: Seq<u32>,after: Seq<u32>,count: int,value: u32)
    requires links(horizontal,vertical,before,horizontal.len() as int,count),0<=count<vertical.len(),after==before.update(horizontal.len() as int+count,value),
        value==encoded(last(vertical,2,count,vertical[count].line())) as u32,
    ensures links(horizontal,vertical,after,horizontal.len() as int,count+1),
{}

pub(crate) open spec fn live(chord: ValuedChord,index: int,x: int,phase: int,limit: int) -> bool {
    chord.start()<=x<=chord.end() && if phase==0 {chord.start()<x ||limit<index}
        else if phase==1 {true} else {x<chord.end() ||index<=limit}
}

pub(crate) open spec fn active(horizontal: Seq<ValuedChord>,owners: Seq<u16>,mask: u64,x: int,phase: int,limit: int) -> bool {
    owners.len()==65 && 0<=phase<=2
        && (forall|i: int|0<=i<horizontal.len() && #[trigger] live(horizontal[i],i,x,phase,limit)
            ==> bit(mask,horizontal[i].line()) && owners[horizontal[i].line()]==i)
        && forall|y: int|0<=y<64 && #[trigger] bit(mask,y)
            ==> exists|i: int|0<=i<horizontal.len() && horizontal[i].line()==y && live(horizontal[i],i,x,phase,limit)
}

pub(crate) proof fn live_step(horizontal: Seq<ValuedChord>,x: int,current: int,phase: int)
    requires 0<=current<horizontal.len(),current<=1953,phase==0 ||phase==2,
        horizontal[current].well_formed(false),
        coordinate(horizontal[current],if phase==0{0}else{1})==x,
    ensures forall|i: int|#![trigger horizontal[i]] 0<=i<horizontal.len() ==> live(horizontal[i],i,x,phase,last(horizontal,if phase==0{0}else{1},current,x))
        == if phase==0{live(horizontal[i],i,x,phase,current)||i==current}else{live(horizontal[i],i,x,phase,current)&&i!=current},
{
    let kind=if phase==0{0}else{1};last_properties(horizontal,kind,current,x);
    assert forall|i: int|0<=i<horizontal.len() implies #[trigger] live(horizontal[i],i,x,phase,last(horizontal,kind,current,x))
        == if phase==0{live(horizontal[i],i,x,phase,current)||i==current}else{live(horizontal[i],i,x,phase,current)&&i!=current} by {
        if last(horizontal,kind,current,x)<i<current {assert(coordinate(horizontal[i],kind)!=x);}
    }
}

pub(crate) proof fn set_bit(mask: u64,y: u32)
    requires y<64,
    ensures forall|p: int|0<=p<64 ==> #[trigger] bit(mask|(1u64<<y),p)==(bit(mask,p)||p==y),
        forall|p: int|0<=p<64 ==> #[trigger] bit(mask&!(1u64<<y),p)==(bit(mask,p)&&p!=y),
{
    assert forall|p: int|0<=p<64 implies #[trigger] bit(mask|(1u64<<y),p)==(bit(mask,p)||p==y) by {
        let q=p as u64;
        assert(((mask|(1u64<<y))>>q)&1==if q==y as u64{1u64}else{(mask>>q)&1}) by(bit_vector) requires y<64,q<64;
    }
    assert forall|p: int|0<=p<64 implies #[trigger] bit(mask&!(1u64<<y),p)==(bit(mask,p)&&p!=y) by {
        let q=p as u64;
        assert(((mask&!(1u64<<y))>>q)&1==if q==y as u64{0u64}else{(mask>>q)&1}) by(bit_vector) requires y<64,q<64;
    }
}

pub(crate) proof fn bit_and(mask: u64,y: u32)
    requires y<64,
    ensures (mask&(1u64<<y)!=0)<==>bit(mask,y as int),
{
    assert((mask&(1u64<<y)!=0)<==>((mask>>(y as u64))&1)==1) by(bit_vector) requires y<64;
}

pub(crate) proof fn activation(horizontal: Seq<ValuedChord>,owners: Seq<u16>,mask: u64,x: int,current: int)
    requires graph::chords_valid(horizontal,false),horizontal.len()<=1953,0<=current<horizontal.len(),
        horizontal[current].start()==x,active(horizontal,owners,mask,x,0,current),
    ensures !bit(mask,horizontal[current].line()),
        active(horizontal,owners.update(horizontal[current].line(),current as u16),mask|(1u64<<(horizontal[current].line() as u32)),
            x,0,last(horizontal,0,current,x)),
{
    let y=horizontal[current].line();assert(horizontal[current].well_formed(false));
    live_step(horizontal,x,current,0);set_bit(mask,y as u32);
    if bit(mask,y) {
        let i=choose|i: int|0<=i<horizontal.len() && horizontal[i].line()==y && live(horizontal[i],i,x,0,current);
        same_line(horizontal,false,current,i,x);assert(!live(horizontal[current],current,x,0,current));
    }
    let after=owners.update(y,current as u16);let result=mask|(1u64<<(y as u32));
    assert forall|i: int|0<=i<horizontal.len() && #[trigger] live(horizontal[i],i,x,0,last(horizontal,0,current,x))
        implies bit(result,horizontal[i].line()) && after[horizontal[i].line()]==i by {
        if i!=current && horizontal[i].line()==y {same_line(horizontal,false,current,i,x);}
    }
    assert forall|p: int|0<=p<64 && #[trigger] bit(result,p) implies
        exists|i: int|0<=i<horizontal.len() && horizontal[i].line()==p && live(horizontal[i],i,x,0,last(horizontal,0,current,x)) by {
        if p==y {assert(live(horizontal[current],current,x,0,last(horizontal,0,current,x)));}
        else {let i=choose|i: int|0<=i<horizontal.len() && horizontal[i].line()==p && live(horizontal[i],i,x,0,current);}
    }
}

pub(crate) proof fn deactivation(horizontal: Seq<ValuedChord>,owners: Seq<u16>,mask: u64,x: int,current: int)
    requires graph::chords_valid(horizontal,false),horizontal.len()<=1953,0<=current<horizontal.len(),
        horizontal[current].end()==x,active(horizontal,owners,mask,x,2,current),
    ensures bit(mask,horizontal[current].line()),owners[horizontal[current].line()]==current,
        active(horizontal,owners,mask&!(1u64<<(horizontal[current].line() as u32)),x,2,last(horizontal,1,current,x)),
{
    let y=horizontal[current].line();assert(horizontal[current].well_formed(false));
    assert(live(horizontal[current],current,x,2,current));live_step(horizontal,x,current,2);set_bit(mask,y as u32);
    let result=mask&!(1u64<<(y as u32));
    assert forall|i: int|0<=i<horizontal.len() && #[trigger] live(horizontal[i],i,x,2,last(horizontal,1,current,x))
        implies bit(result,horizontal[i].line()) && owners[horizontal[i].line()]==i by {
        if horizontal[i].line()==y {same_line(horizontal,false,current,i,x);}
    }
    assert forall|p: int|0<=p<64 && #[trigger] bit(result,p) implies
        exists|i: int|0<=i<horizontal.len() && horizontal[i].line()==p && live(horizontal[i],i,x,2,last(horizontal,1,current,x)) by {
        let i=choose|i: int|0<=i<horizontal.len() && horizontal[i].line()==p && live(horizontal[i],i,x,2,current);
        assert(i!=current);
    }
}

pub(crate) proof fn phase_begin(horizontal: Seq<ValuedChord>,owners: Seq<u16>,mask: u64,x: int,head: u16,starting: bool)
    requires graph::chords_valid(horizontal,false),horizontal.len()<=1953,
        head==encoded(last(horizontal,if starting{0}else{1},horizontal.len() as int,x)),
        if starting{active(horizontal,owners,mask,x,0,1953)}else{active(horizontal,owners,mask,x,1,0)},
    ensures active(horizontal,owners,mask,x,if starting{0}else{2},cursor(head)),
{
    let kind=if starting{0}else{1};head_cursor(horizontal,kind,horizontal.len() as int,x,head);
    assert forall|i: int|#![trigger horizontal[i]] 0<=i<horizontal.len() implies
        live(horizontal[i],i,x,if starting{0}else{2},cursor(head))==live(horizontal[i],i,x,if starting{0}else{1},if starting{1953}else{0}) by {
        if cursor(head)<i {assert(coordinate(horizontal[i],kind)!=x);}
    }
}

pub(crate) proof fn phase_end(horizontal: Seq<ValuedChord>,owners: Seq<u16>,mask: u64,x: int,starting: bool)
    requires graph::chords_valid(horizontal,false),horizontal.len()<=1953,active(horizontal,owners,mask,x,if starting{0}else{2},-1),
    ensures if starting{active(horizontal,owners,mask,x,1,0)}else{active(horizontal,owners,mask,x+1,0,1953)},
{
    assert forall|i: int|#![trigger horizontal[i]] 0<=i<horizontal.len() implies
        live(horizontal[i],i,x,if starting{0}else{2},-1)==
        if starting{live(horizontal[i],i,x,1,0)}else{live(horizontal[i],i,x+1,0,1953)} by {}
}

pub(in crate::optimal) proof fn packed_next(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,scratch: &ConflictFinalizeScratch,current: int,starting: bool)
    requires events(horizontal,vertical,scratch),0<=current<horizontal.len(),
    ensures cursor(if starting{(scratch.right_layout@[current]&0xffff) as u16}else{(scratch.right_layout@[current]>>16) as u16})
        ==last(horizontal,if starting{0}else{1},current,coordinate(horizontal[current],if starting{0}else{1})),
{
    let first=encoded(last(horizontal,0,current,horizontal[current].start()));let end=encoded(last(horizontal,1,current,horizontal[current].end()));
    last_properties(horizontal,0,current,horizontal[current].start());last_properties(horizontal,1,current,horizontal[current].end());
    let value=scratch.right_layout@[current];
    assert((value&0xffff) as u16==first && (value>>16) as u16==end) by(bit_vector)
        requires value==(first as u32)|((end as u32)<<16);
}

pub(crate) open(in crate::optimal) spec fn active_frame(before: &ConflictFinalizeScratch,after: &ConflictFinalizeScratch) -> bool {
    before.right_layout==after.right_layout && before.right_degrees==after.right_degrees && before.adjacency_offsets==after.adjacency_offsets
        && before.adjacency_edges==after.adjacency_edges && before.next_offsets==after.next_offsets && before.edge_buffer==after.edge_buffer
        && before.edge_count==after.edge_count && before.horizontal_start_heads==after.horizontal_start_heads
        && before.horizontal_end_heads==after.horizontal_end_heads && before.vertical_query_heads==after.vertical_query_heads
}

pub(crate) open spec fn between(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>,x: int,limit: int) -> bool {
    edges_valid(horizontal,vertical,edges)
        && forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len() && graph::closed_conflict(horizontal[h],vertical[v])
            ==> (#[trigger] edges.contains(SparseEdge{left:h as u16,right:v as u16}) <==>vertical[v].line()<x ||vertical[v].line()==x && limit<v)
}

pub(crate) open spec fn partial(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>,x: int,current: int,remaining: u64) -> bool {
    edges_valid(horizontal,vertical,edges)
        && forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len() && graph::closed_conflict(horizontal[h],vertical[v])
            ==> (#[trigger] edges.contains(SparseEdge{left:h as u16,right:v as u16}) <==>vertical[v].line()<x ||vertical[v].line()==x &&
                (current<v ||v==current && !bit(remaining,horizontal[h].line())))
}

pub(crate) open spec fn query_mask(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,owners: Seq<u16>,mask: u64,x: int,current: int) -> bool {
    owners.len()==65 && (forall|y: int|0<=y<64 && #[trigger] bit(mask,y) ==> 1<=y<=63 && 0<=owners[y]<horizontal.len()
        && horizontal[owners[y] as int].line()==y && graph::closed_conflict(horizontal[owners[y] as int],vertical[current]))
        && forall|h: int|0<=h<horizontal.len() && #[trigger] graph::closed_conflict(horizontal[h],vertical[current])
            ==> bit(mask,horizontal[h].line()) && owners[horizontal[h].line()]==h
}

pub(crate) proof fn query_begin(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,owners: Seq<u16>,active_y: u64,range: u64,
    edges: Seq<SparseEdge>,x: int,current: int)
    requires graph::graph_chords_valid(horizontal,vertical),horizontal.len()<=1953,vertical.len()<=1953,
        active(horizontal,owners,active_y,x,1,0),0<=current<vertical.len(),vertical[current].line()==x,
        crate::chord_graph::proof::mask_matches(range,vertical[current].chord.y1,vertical[current].chord.y2),between(horizontal,vertical,edges,x,current),
    ensures partial(horizontal,vertical,edges,x,current,active_y&range),query_mask(horizontal,vertical,owners,active_y&range,x,current),
{
    let mask=active_y&range;assert(vertical[current].well_formed(true));
    assert forall|y: int|0<=y<64 implies #[trigger] bit(mask,y)==(bit(active_y,y)&&bit(range,y)) by {
        let p=y as u64;
        assert((((active_y&range)>>p)&1==1)<==>(((active_y>>p)&1==1)&&((range>>p)&1==1))) by(bit_vector) requires p<64;
    }
    assert forall|y: int|0<=y<64 && #[trigger] bit(mask,y) implies 1<=y<=63 && 0<=owners[y]<horizontal.len()
        && horizontal[owners[y] as int].line()==y && graph::closed_conflict(horizontal[owners[y] as int],vertical[current]) by {
        let i=choose|i: int|0<=i<horizontal.len() && horizontal[i].line()==y && live(horizontal[i],i,x,1,0);
        assert(owners[y]==i);assert(horizontal[i].well_formed(false));
    }
    assert forall|h: int|0<=h<horizontal.len() && #[trigger] graph::closed_conflict(horizontal[h],vertical[current])
        implies bit(mask,horizontal[h].line()) && owners[horizontal[h].line()]==h by {
        assert(horizontal[h].well_formed(false));assert(live(horizontal[h],h,x,1,0));
        assert(bit(range,horizontal[h].line()));
    }
}

pub(crate) proof fn clear_lowest(mask: u64)
    requires mask!=0,
    ensures 0<=vstd::std_specs::bits::u64_trailing_zeros(mask)<64,bit(mask,vstd::std_specs::bits::u64_trailing_zeros(mask) as int),
        mask&((mask-1) as u64)<mask,
        forall|y: int|0<=y<64 ==> #[trigger] bit(mask&((mask-1) as u64),y)==(bit(mask,y)&&y!=vstd::std_specs::bits::u64_trailing_zeros(mask)),
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(mask);
    let low=vstd::std_specs::bits::u64_trailing_zeros(mask);
    assert(mask&((mask-1) as u64)<mask) by(bit_vector) requires mask!=0;
    assert forall|y: int|0<=y<64 implies #[trigger] bit(mask&((mask-1) as u64),y)==(bit(mask,y)&&y!=low) by {
        let p=y as u64;
        if p<low {
            assert((mask>>p)&1==0);
            assert(((mask&((mask-1) as u64))>>p)&1==0) by(bit_vector) requires (mask>>p)&1==0;
        }else{
            assert(((((mask&((mask-1) as u64))>>p)&1)==1)<==>(((mask>>p)&1==1)&&p!=low as u64)) by(bit_vector)
                requires low as u64<=p<64,low<64,(mask>>low)&1==1,mask<<((64-low) as u64)==0;
        }
    }
}

pub(crate) proof fn append_edge(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,owners: Seq<u16>,mask: u64,
    remaining: u64,edges: Seq<SparseEdge>,x: int,current: int,y: int)
    requires graph::graph_chords_valid(horizontal,vertical),horizontal.len()<=1953,vertical.len()<=1953,0<=current<vertical.len(),vertical[current].line()==x,
        query_mask(horizontal,vertical,owners,mask,x,current),0<=y<64,bit(remaining,y),
        forall|p: int|0<=p<64 && #[trigger] bit(remaining,p) ==> bit(mask,p),
        partial(horizontal,vertical,edges,x,current,remaining),remaining!=0,y==vstd::std_specs::bits::u64_trailing_zeros(remaining),
    ensures edges.len()<3969,partial(horizontal,vertical,edges.push(SparseEdge{left:owners[y],right:current as u16}),x,current,remaining&((remaining-1) as u64)),
{
    broadcast use vstd::seq_lib::group_seq_properties;
    clear_lowest(remaining);let added=SparseEdge{left:owners[y],right:current as u16};let result=edges.push(added);
    assert(bit(mask,y));assert(0<=added.left<horizontal.len());
    assert(graph::closed_conflict(horizontal[added.left as int],vertical[current]));
    assert(horizontal[added.left as int].line()==y);
    partial_pair(horizontal,vertical,edges,x,current,remaining,added.left as int,current);
    assert(edges.contains(SparseEdge{left:added.left,right:current as u16}) <==>vertical[current].line()<x ||vertical[current].line()==x &&
        (current<current ||!bit(remaining,horizontal[added.left as int].line())));
    assert(!edges.contains(added));assert(result.no_duplicates());
    assert forall|p: int|0<=p<result.len() implies (#[trigger] result[p]).left<horizontal.len()
        && result[p].right<vertical.len() && graph::closed_conflict(horizontal[result[p].left as int],vertical[result[p].right as int]) by {
        if p<edges.len(){assert(result[p]==edges[p]);}
    }
    assert(edges_valid(horizontal,vertical,result));edge_capacity(horizontal,vertical,result);
    assert forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len() && graph::closed_conflict(horizontal[h],vertical[v])
        implies (#[trigger] result.contains(SparseEdge{left:h as u16,right:v as u16}) <==>vertical[v].line()<x ||vertical[v].line()==x &&
            (current<v ||v==current && !bit(remaining&((remaining-1) as u64),horizontal[h].line()))) by {
        assert(horizontal[h].well_formed(false));
        if v==current && horizontal[h].line()==y {assert(owners[y]==h);}
    }
}

pub(crate) proof fn partial_pair(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>,x: int,current: int,remaining: u64,h: int,v: int)
    requires partial(horizontal,vertical,edges,x,current,remaining),0<=h<horizontal.len(),0<=v<vertical.len(),graph::closed_conflict(horizontal[h],vertical[v]),
    ensures edges.contains(SparseEdge{left:h as u16,right:v as u16}) <==>vertical[v].line()<x ||vertical[v].line()==x &&
        (current<v ||v==current && !bit(remaining,horizontal[h].line())),
{}

pub(crate) proof fn query_end(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>,x: int,current: int)
    requires graph::graph_chords_valid(horizontal,vertical),vertical.len()<=1953,0<=current<vertical.len(),vertical[current].line()==x,
        partial(horizontal,vertical,edges,x,current,0),
    ensures between(horizontal,vertical,edges,x,last(vertical,2,current,x)),
{
    last_properties(vertical,2,current,x);
    assert forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len() && graph::closed_conflict(horizontal[h],vertical[v])
        implies (#[trigger] edges.contains(SparseEdge{left:h as u16,right:v as u16}) <==>vertical[v].line()<x ||vertical[v].line()==x && last(vertical,2,current,x)<v) by {
        if last(vertical,2,current,x)<v<current {assert(coordinate(vertical[v],2)!=x);assert(vertical[v].line()!=x);}
        zero_bit(horizontal[h].line());
    }
}

pub(crate) proof fn zero_bit(y: int)
    ensures !bit(0,y),
{
    let p=y as u64;
    assert((0u64>>p)&1==0) by(bit_vector);
}

pub(crate) open spec fn partial_groups(edges: Seq<SparseEdge>,degrees: Seq<u8>,current: int,begin: int) -> bool {
    graph::degree_domain(degrees) && 0<=current<degrees.len() && 0<=begin<=edges.len()
        && (forall|p: int|0<=p<edges.len() ==> (#[trigger] edges[p]).right<degrees.len())
        && (forall|v: int|0<=v<degrees.len() && v!=current ==> #[trigger] graph::source_run(edges,v,graph::run_start(edges,v,degrees[v] as int),(#[trigger] degrees[v]) as int))
        && forall|p: int|0<=p<edges.len() ==> ((#[trigger] edges[p]).right==current <==>begin<=p)
}

pub(crate) proof fn group_begin(edges: Seq<SparseEdge>,degrees: Seq<u8>,current: int)
    requires graph::source_groups(edges,degrees),0<=current<degrees.len(),
        forall|p: int|0<=p<edges.len() ==> (#[trigger] edges[p]).right!=current,
    ensures partial_groups(edges,degrees,current,edges.len() as int),
{
    hide(graph::source_run);hide(graph::run_start);
    reveal(partial_groups);reveal(graph::source_groups);
    assert(graph::degree_domain(degrees));
}

pub(crate) proof fn group_witness(edges: Seq<SparseEdge>,degrees: Seq<u8>,v: int)
    requires graph::source_groups(edges,degrees),0<=v<degrees.len(),
    ensures exists|start: int|#[trigger] graph::source_run(edges,v,start,degrees[v] as int),
{
    hide(graph::source_run);hide(graph::run_start);reveal(graph::source_groups);
    let count=degrees[v] as int;
    assert(count<64);
    assert(graph::source_run(edges,v,graph::run_start(edges,v,count),count));
    assert(exists|start: int|#[trigger] graph::source_run(edges,v,start,degrees[v] as int));
    let start=choose|start: int|#[trigger] graph::source_run(edges,v,start,degrees[v] as int);
    assert(graph::source_run(edges,v,start,degrees[v] as int));
}

pub(crate) proof fn group_append(edges: Seq<SparseEdge>,degrees: Seq<u8>,current: int,begin: int,added: SparseEdge)
    requires partial_groups(edges,degrees,current,begin),added.right==current,
    ensures partial_groups(edges.push(added),degrees,current,begin),
{
    hide(graph::source_run);hide(graph::run_start);
    reveal(partial_groups);
    let after=edges.push(added);
    assert forall|p: int|0<=p<after.len() implies (#[trigger] after[p]).right<degrees.len() by {if p<edges.len(){assert(after[p]==edges[p]);}}
    assert forall|p: int|0<=p<after.len() implies ((#[trigger] after[p]).right==current <==>begin<=p) by {if p<edges.len(){assert(after[p]==edges[p]);}}
    assert forall|v: int|0<=v<degrees.len() && v!=current implies #[trigger] graph::source_run(after,v,graph::run_start(after,v,degrees[v] as int),(#[trigger] degrees[v]) as int) by {
        let start=graph::run_start(edges,v,degrees[v] as int);
        assert(graph::source_run(edges,v,start,degrees[v] as int));
        assert(graph::source_run(after,v,start,degrees[v] as int)) by {reveal(graph::source_run);}
        graph::run_start_exists(after,v,degrees[v] as int);
    }
}

pub(crate) proof fn group_finish(edges: Seq<SparseEdge>,degrees: Seq<u8>,current: int,begin: int)
    requires partial_groups(edges,degrees,current,begin),edges.len()-begin<=63,
    ensures graph::source_groups(edges,degrees.update(current,(edges.len()-begin) as u8)),
{
    hide(graph::source_run);hide(graph::run_start);
    reveal(partial_groups);reveal(graph::source_groups);
    let count=edges.len()-begin;let after=degrees.update(current,count as u8);
    assert(0<=count<=63);
    assert forall|v: int|0<=v<after.len() implies #[trigger] after[v]<64 by {if v!=current{assert(after[v]==degrees[v]);}}
    assert(graph::degree_domain(after));
    assert forall|v: int|0<=v<after.len() implies #[trigger] graph::source_run(edges,v,graph::run_start(edges,v,after[v] as int),(#[trigger] after[v]) as int) by {
        if v==current {
            let start=if count==0{0}else{begin};assert(graph::source_run(edges,v,start,count)) by {reveal(graph::source_run);}
            assert(graph::source_run(edges,v,start,after[v] as int));
        }else{
            assert(after[v]==degrees[v]);
            let start=graph::run_start(edges,v,degrees[v] as int);
            assert(graph::source_run(edges,v,start,degrees[v] as int));
            assert(graph::source_run(edges,v,start,after[v] as int));
        }
        graph::run_start_exists(edges,v,after[v] as int);
    }
}

pub(crate) proof fn group_capacity(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>,current: int,begin: int)
    requires graph::graph_chords_valid(horizontal,vertical),edges_valid(horizontal,vertical,edges),0<=current<vertical.len(),0<=begin<=edges.len(),
        forall|p: int|begin<=p<edges.len() ==> (#[trigger] edges[p]).right==current,
    ensures edges.len()-begin<=63,
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=Set::<int>::range(begin,edges.len() as int);let f=|p: int|horizontal[edges[p].left as int].line()-1;
    let output=input.map(f);let slots=Set::<int>::range(0,63);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int|input.contains(a) && input.contains(b) && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            point_injective(horizontal,vertical,edges[a],edges[b]);
        }
    }
    vstd::set_lib::lemma_int_range(begin,edges.len() as int);vstd::set_lib::lemma_int_range(0,63);
    vstd::set_lib::lemma_map_size(input,output,f);
    assert(output.subset_of(slots)) by {
        assert forall|s: int|#[trigger] output.contains(s) implies slots.contains(s) by {
            let p=choose|p: int|input.contains(p) && #[trigger] f(p)==s;assert(horizontal[edges[p].left as int].well_formed(false));
        }
    }
    vstd::set_lib::lemma_len_subset(output,slots);
}

pub(crate) proof fn unseen_query(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>,x: int,current: int)
    requires horizontal.len()<=1953,vertical.len()<=1953,0<=current<vertical.len(),vertical[current].line()==x,
        between(horizontal,vertical,edges,x,current),
    ensures forall|p: int|0<=p<edges.len() ==> (#[trigger] edges[p]).right!=current,
{
    broadcast use vstd::seq_lib::group_seq_properties;
    assert forall|p: int|0<=p<edges.len() implies (#[trigger] edges[p]).right!=current by {
        if edges[p].right==current {let h=edges[p].left as int;assert(edges.contains(SparseEdge{left:h as u16,right:current as u16}));}
    }
}

pub(crate) open(in crate::optimal) spec fn source(scratch: &ConflictFinalizeScratch) -> Seq<SparseEdge> {
    scratch.edge_buffer@.take(scratch.edge_count as int)
}

pub(crate) open(in crate::optimal) spec fn query_frame(before: &ConflictFinalizeScratch,after: &ConflictFinalizeScratch) -> bool {
    before.right_layout==after.right_layout && before.adjacency_offsets==after.adjacency_offsets
        && before.adjacency_edges==after.adjacency_edges && before.next_offsets==after.next_offsets
        && before.horizontal_start_heads==after.horizontal_start_heads && before.horizontal_end_heads==after.horizontal_end_heads
        && before.vertical_query_heads==after.vertical_query_heads && before.active_horizontal==after.active_horizontal
}

pub(in crate::optimal) proof fn vertical_next(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,scratch: &ConflictFinalizeScratch,current: int)
    requires events(horizontal,vertical,scratch),0<=current<vertical.len(),
    ensures scratch.right_layout@[horizontal.len() as int+current]<=65535,
        cursor(scratch.right_layout@[horizontal.len() as int+current] as u16)==last(vertical,2,current,vertical[current].line()),
{
    last_properties(vertical,2,current,vertical[current].line());
}

pub(crate) proof fn sweep_started(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,owners: Seq<u16>)
    requires graph::graph_chords_valid(horizontal,vertical),horizontal.len()<=1953,vertical.len()<=1953,owners.len()==65,
    ensures active(horizontal,owners,0,1,0,1953),between(horizontal,vertical,Seq::empty(),1,1953),
        graph::source_groups(Seq::empty(),Seq::new(vertical.len(),|_: int|0u8)),
{
    broadcast use vstd::seq_lib::group_seq_properties;
    assert forall|y: int|0<=y<64 implies !#[trigger] bit(0,y) by {zero_bit(y);}
    assert forall|i: int|0<=i<horizontal.len() implies !#[trigger] live(horizontal[i],i,1,0,1953) by {assert(horizontal[i].well_formed(false));}
    let degrees=Seq::new(vertical.len(),|_: int|0u8);let edges=Seq::<SparseEdge>::empty();
    assert forall|v: int|0<=v<degrees.len() implies #[trigger] graph::source_run(edges,v,graph::run_start(edges,v,degrees[v] as int),(#[trigger] degrees[v]) as int) by {
        assert(graph::source_run(edges,v,0,0));graph::run_start_exists(edges,v,0);
    }
    assert forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len() && graph::closed_conflict(horizontal[h],vertical[v])
        implies (#[trigger] edges.contains(SparseEdge{left:h as u16,right:v as u16}) <==>vertical[v].line()<1 ||vertical[v].line()==1 && 1953<v) by {assert(vertical[v].well_formed(true));}
}

pub(crate) proof fn queries_begin(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>,x: int,head: u16)
    requires vertical.len()<=1953,head==encoded(last(vertical,2,vertical.len() as int,x)),between(horizontal,vertical,edges,x,1953),
    ensures between(horizontal,vertical,edges,x,cursor(head)),
{
    head_cursor(vertical,2,vertical.len() as int,x,head);
    assert forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len() && graph::closed_conflict(horizontal[h],vertical[v])
        implies (#[trigger] edges.contains(SparseEdge{left:h as u16,right:v as u16}) <==>vertical[v].line()<x ||vertical[v].line()==x && cursor(head)<v) by {
        if cursor(head)<v{assert(coordinate(vertical[v],2)!=x);}
    }
}

pub(crate) proof fn queries_end(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>,x: int)
    requires vertical.len()<=1953,between(horizontal,vertical,edges,x,-1),
    ensures between(horizontal,vertical,edges,x+1,1953),
{}

pub(crate) open spec fn complete_edges(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>) -> bool {
    edges_valid(horizontal,vertical,edges) && forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len()
        ==> (#[trigger] edges.contains(SparseEdge{left:h as u16,right:v as u16}) <==>graph::closed_conflict(horizontal[h],vertical[v]))
}

pub(crate) proof fn sweep_finished(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,edges: Seq<SparseEdge>,owners: Seq<u16>,mask: u64)
    requires graph::graph_chords_valid(horizontal,vertical),horizontal.len()<=1953,vertical.len()<=1953,
        between(horizontal,vertical,edges,64,1953),active(horizontal,owners,mask,64,0,1953),
    ensures complete_edges(horizontal,vertical,edges),mask==0,
{
    broadcast use vstd::seq_lib::group_seq_properties;
    assert forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len() implies
        (#[trigger] edges.contains(SparseEdge{left:h as u16,right:v as u16}) <==>graph::closed_conflict(horizontal[h],vertical[v])) by {
        assert(vertical[v].well_formed(true));
        if edges.contains(SparseEdge{left:h as u16,right:v as u16}) {
            let p=choose|p: int|0<=p<edges.len() && #[trigger] edges[p]==SparseEdge{left:h as u16,right:v as u16};
        }
    }
    if mask!=0 {
        vstd::std_specs::bits::axiom_u64_trailing_zeros(mask);let y=vstd::std_specs::bits::u64_trailing_zeros(mask) as int;
        assert(bit(mask,y));let i=choose|i: int|0<=i<horizontal.len() && horizontal[i].line()==y && live(horizontal[i],i,64,0,1953);
        assert(horizontal[i].well_formed(false));
    }
}

pub(crate) open(in crate::optimal) spec fn running(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,scratch: &ConflictFinalizeScratch) -> bool {
    events(horizontal,vertical,scratch) && graph::conflict_buffers_valid(scratch) && scratch.edge_count<=3969
        && scratch.right_degrees@.len()==vertical.len() && graph::source_groups(source(scratch),scratch.right_degrees@)
}

pub(crate) proof fn pair_membership(edges: Seq<SparseEdge>,end: int,left: u16,right: u16)
    requires 0<=end<=edges.len(),
    ensures 0<=graph::pair_count(edges,end,left as int,right as int)<=end,
        (graph::pair_count(edges,end,left as int,right as int)>0 <==>edges.take(end).contains(SparseEdge{left,right})),
        edges.no_duplicates() ==> graph::pair_count(edges,end,left as int,right as int)<=1,
    decreases end,
{
    if end>0 {
        pair_membership(edges,end-1,left,right);
        assert(edges.take(end) =~=edges.take(end-1).push(edges[end-1]));
        if edges.no_duplicates() && edges[end-1].left==left && edges[end-1].right==right {
            assert(!edges.take(end-1).contains(SparseEdge{left,right}));
        }
    }
}

pub(crate) proof fn row_membership(values: Seq<u16>,begin: int,end: int,right: u16)
    requires 0<=begin<=end<=values.len(),
    ensures 0<=graph::row_count(values,begin,end,right as int)<=end-begin,
        (graph::row_count(values,begin,end,right as int)>0 <==>values.subrange(begin,end).contains(right)),
    decreases end-begin,
{
    if end>begin {
        row_membership(values,begin,end-1,right);
        assert(values.subrange(begin,end) =~=values.subrange(begin,end-1).push(values[end-1]));
    }else{assert(values.subrange(begin,end) =~=Seq::<u16>::empty());}
}

pub(crate) proof fn csr_pair(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,source: Seq<SparseEdge>,offsets: Seq<u16>,edges: Seq<u16>,h: int,v: int)
    requires horizontal.len()<=1953,vertical.len()<=1953,complete_edges(horizontal,vertical,source),
        graph::csr_exact(source,offsets,edges,vertical.len()),offsets.len()==horizontal.len()+1,
        crate::optimal::matching::proof::rows_valid(offsets,edges.len()),0<=h<horizontal.len(),0<=v<vertical.len(),
    ensures (crate::optimal::matching::proof::edge_in(offsets,edges,h,v) <==>graph::closed_conflict(horizontal[h],vertical[v])),
        graph::row_count(edges,offsets[h] as int,offsets[h+1] as int,v)==if graph::closed_conflict(horizontal[h],vertical[v]){1int}else{0int},
{
    hide(graph::row_count);hide(graph::pair_count);
    pair_membership(source,source.len() as int,h as u16,v as u16);
    row_membership(edges,offsets[h] as int,offsets[h+1] as int,v as u16);
    assert(source.take(source.len() as int) =~=source);
}

pub(crate) proof fn csr_complete(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,source: Seq<SparseEdge>,offsets: Seq<u16>,edges: Seq<u16>)
    requires horizontal.len()<=1953,vertical.len()<=1953,complete_edges(horizontal,vertical,source),
        graph::csr_exact(source,offsets,edges,vertical.len()),offsets.len()==horizontal.len()+1,
        crate::optimal::matching::proof::rows_valid(offsets,edges.len()),
    ensures graph::graph_exact(horizontal,vertical,offsets,edges),
        forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len() ==>
            #[trigger] graph::row_count(edges,offsets[h] as int,offsets[h+1] as int,v)==if graph::closed_conflict(horizontal[h],vertical[v]){1int}else{0int},
{
    hide(graph::row_count);hide(graph::pair_count);
    hide(complete_edges);hide(edges_valid);hide(graph::csr_exact);hide(crate::optimal::matching::proof::rows_valid);
    hide(crate::optimal::matching::proof::edge_in);hide(graph::closed_conflict);
    assert forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len() implies
        (#[trigger] crate::optimal::matching::proof::edge_in(offsets,edges,h,v) <==>graph::closed_conflict(horizontal[h],vertical[v])) by {
        csr_pair(horizontal,vertical,source,offsets,edges,h,v);
    }
    assert forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len() implies
        #[trigger] graph::row_count(edges,offsets[h] as int,offsets[h+1] as int,v)==if graph::closed_conflict(horizontal[h],vertical[v]){1int}else{0int} by {
        csr_pair(horizontal,vertical,source,offsets,edges,h,v);
    }
}

#[cfg(any(reject_sweep_closed_start,reject_sweep_closed_end))]
fn reject_closed_endpoint() {
    let horizontal=[ValuedChord::on_axis::<true>(2,1,3,7)];
    let x=if cfg!(reject_sweep_closed_start){1u8}else{3u8};
    let vertical=[ValuedChord::on_axis::<false>(x,1,3,7)];
    let mut scratch=super::ConflictScratch{finalize:ConflictFinalizeScratch::new()};
    assert(graph::graph_chords_valid(horizontal@,vertical@));
    let (adjacency,_degrees)=super::build_sparse_conflict_graph_csr(&horizontal,&vertical,&mut scratch);
    assert(!crate::optimal::matching::proof::edge_in(adjacency.offsets@,adjacency.edges@,0,0));
}

#[cfg(reject_sweep_lost_chain)]
fn reject_lost_chain() {
    let horizontal=[ValuedChord::on_axis::<true>(2,1,3,7),ValuedChord::on_axis::<true>(3,1,3,7)];
    let vertical=[ValuedChord::on_axis::<false>(2,1,4,7)];
    let mut scratch=super::ConflictScratch{finalize:ConflictFinalizeScratch::new()};
    assert(graph::graph_chords_valid(horizontal@,vertical@));
    let (adjacency,_degrees)=super::build_sparse_conflict_graph_csr(&horizontal,&vertical,&mut scratch);
    assert(!crate::optimal::matching::proof::edge_in(adjacency.offsets@,adjacency.edges@,0,0));
}

#[cfg(reject_sweep_duplicate_edge)]
fn reject_duplicate_edge() {
    let horizontal=[ValuedChord::on_axis::<true>(2,1,3,7)];
    let vertical=[ValuedChord::on_axis::<false>(2,1,3,7)];
    let mut scratch=super::ConflictScratch{finalize:ConflictFinalizeScratch::new()};
    assert(graph::graph_chords_valid(horizontal@,vertical@));
    let (adjacency,_degrees)=super::build_sparse_conflict_graph_csr(&horizontal,&vertical,&mut scratch);
    assert(graph::row_count(adjacency.edges@,adjacency.offsets@[0] as int,adjacency.offsets@[1] as int,0)==2);
}

}
