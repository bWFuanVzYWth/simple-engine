//! 双凹角 cut 链的实际有限 good chord 集；集合对应同一竞争分区。

use super::{Rectangle, chain_marks, chains, corners, good};
use corners::counting::{bit, sum};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn direction(key: int) -> bool {key>=4225}
pub(crate) open spec fn line(key: int) -> int {key%4225/65}
pub(crate) open spec fn end(key: int) -> int {key%65}

pub(crate) open spec fn paired(labels: Seq<u16>,rectangles: Seq<Rectangle>,key: int) -> bool {
    0<=key<8450 && chain_marks::paired(chain_marks::source(labels,rectangles,direction(key),line(key)),
        chain_marks::mark_source(labels,direction(key),line(key)),end(key))==1
}

pub(crate) open spec fn event_id(labels: Seq<u16>,rectangles: Seq<Rectangle>,key: int) -> int {
    good::encode(direction(key),line(key),chain_marks::start_at(
        chain_marks::source(labels,rectangles,direction(key),line(key)),end(key)-1),end(key))
}

pub(crate) open spec fn events(labels: Seq<u16>,rectangles: Seq<Rectangle>) -> Set<int> {
    Set::<int>::range(0,8450).filter(|key: int|paired(labels,rectangles,key))
}

pub(crate) open spec fn chords(labels: Seq<u16>,rectangles: Seq<Rectangle>) -> Set<int> {
    events(labels,rectangles).map(|key: int|event_id(labels,rectangles,key))
}

pub(crate) proof fn actual_good(labels: Seq<u16>,rectangles: Seq<Rectangle>,key: int)
    requires paired(labels,rectangles,key),
    ensures good::good(labels,event_id(labels,rectangles,key)),
        good::vertical(event_id(labels,rectangles,key))==direction(key),
        good::line(event_id(labels,rectangles,key))==line(key),
        good::end(event_id(labels,rectangles,key))==end(key),
        good::start(event_id(labels,rectangles,key))==chain_marks::start_at(
            chain_marks::source(labels,rectangles,direction(key),line(key)),end(key)-1),
        good::value(labels,event_id(labels,rectangles,key))==
            chains::line_value(labels,rectangles,direction(key),line(key),end(key)-1),
{
    let v=direction(key);let l=line(key);let t=end(key);
    let f=chain_marks::source(labels,rectangles,v,l);
    chain_marks::paired_good(labels,rectangles,v,l,t);
    good::encoded_chord(labels,v,l,chain_marks::start_at(f,t-1),t,f(t-1));
}

pub(crate) proof fn chord_cut(labels: Seq<u16>,rectangles: Seq<Rectangle>,id: int,t: int)
    requires chords(labels,rectangles).contains(id),good::start(id)<=t<good::end(id),
    ensures chains::line_value(labels,rectangles,good::vertical(id),good::line(id),t)==good::value(labels,id),
{
    broadcast use Set::lemma_map_contains;
    let key=choose|key: int|events(labels,rectangles).contains(key) && #[trigger] event_id(labels,rectangles,key)==id;
    actual_good(labels,rectangles,key);
    let f=chain_marks::source(labels,rectangles,direction(key),line(key));
    let end=end(key);
    chain_marks::start_prefix(f,end-1);
    assert(f(t)==f(end-1));
}

pub(crate) proof fn event_injective(labels: Seq<u16>,rectangles: Seq<Rectangle>,a: int,b: int)
    requires paired(labels,rectangles,a),paired(labels,rectangles,b),
        event_id(labels,rectangles,a)==event_id(labels,rectangles,b),
    ensures a==b,
{
    actual_good(labels,rectangles,a);actual_good(labels,rectangles,b);
    assert(a==(if direction(a){4225int}else{0})+line(a)*65+end(a)) by(nonlinear_arith)
        requires 0<=a<8450;
    assert(b==(if direction(b){4225int}else{0})+line(b)*65+end(b)) by(nonlinear_arith)
        requires 0<=b<8450;
}

pub(crate) proof fn flat_axis(labels: Seq<u16>,rectangles: Seq<Rectangle>,dir: bool)
    ensures sum(4225,|p: int|chain_marks::paired(chain_marks::source(labels,rectangles,dir,p/65),
        chain_marks::mark_source(labels,dir,p/65),p%65))
        ==sum(65,|line: int|chain_marks::paired_line(labels,rectangles,dir,line)),
{
    let f=|t: int,line: int|chain_marks::paired(chain_marks::source(labels,rectangles,dir,line),
        chain_marks::mark_source(labels,dir,line),t);
    corners::counting::grid_sum(65,65,f);
    corners::counting::sum_equal(4225,|p: int|f(p%65,p/65),
        |p: int|chain_marks::paired(chain_marks::source(labels,rectangles,dir,p/65),
            chain_marks::mark_source(labels,dir,p/65),p%65));
    let g=|line: int|sum(65,|t: int|f(t,line));
    let h=|line: int|chain_marks::paired_line(labels,rectangles,dir,line);
    assert forall|line: int| 0<=line<65 implies #[trigger] g(line)==#[trigger] h(line) by {
        corners::counting::sum_equal(65,|t: int|f(t,line),
            |t: int|chain_marks::paired(chain_marks::source(labels,rectangles,dir,line),
                chain_marks::mark_source(labels,dir,line),t));
    }
    corners::counting::sum_equal(65,g,h);
}

pub(crate) proof fn count_events(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    ensures events(labels,rectangles).len() as int==chain_marks::total_paired(labels,rectangles),
{
    let p=|key: int|paired(labels,rectangles,key);
    let count=|key: int|bit(p(key));
    corners::counting::range_count(8450,p);
    corners::counting::sum_concat(4225,4225,count);
    let h=|p: int|chain_marks::paired(chain_marks::source(labels,rectangles,false,p/65),
        chain_marks::mark_source(labels,false,p/65),p%65);
    let v=|p: int|chain_marks::paired(chain_marks::source(labels,rectangles,true,p/65),
        chain_marks::mark_source(labels,true,p/65),p%65);
    assert forall|key: int| 0<=key<4225 implies #[trigger] count(key)==#[trigger] h(key) by {
        let a=chain_marks::source(labels,rectangles,false,key/65);
        let r=chain_marks::mark_source(labels,false,key/65);
        assert(0<=chain_marks::paired(a,r,key%65)<=1);
    }
    assert forall|key: int| 0<=key<4225 implies #[trigger] count(4225+key)==#[trigger] v(key) by {
        assert((4225+key)%4225==key) by(nonlinear_arith) requires 0<=key<4225;
        assert((4225+key)%65==key%65) by(nonlinear_arith);
        let a=chain_marks::source(labels,rectangles,true,key/65);
        let r=chain_marks::mark_source(labels,true,key/65);
        assert(0<=chain_marks::paired(a,r,key%65)<=1);
    }
    corners::counting::sum_equal(4225,count,h);
    corners::counting::sum_equal(4225,|key: int|count(4225+key),v);
    flat_axis(labels,rectangles,false);flat_axis(labels,rectangles,true);
}

/// 集合没有重复 good chord，其基数就是两端原凹角的 cut 链数。
pub(crate) proof fn actual_chord_set(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    ensures chords(labels,rectangles).len() as int==chain_marks::total_paired(labels,rectangles),
        forall|id: int| #[trigger] chords(labels,rectangles).contains(id) ==> good::good(labels,id),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let input=events(labels,rectangles);
    let f=|key: int|event_id(labels,rectangles,key);
    let output=chords(labels,rectangles);
    assert(input.injective_on(f)) by {
        assert forall|a: int,b: int| input.contains(a) && input.contains(b)
            && #[trigger] f(a)==#[trigger] f(b) implies a==b by {event_injective(labels,rectangles,a,b);}
    }
    vstd::set_lib::lemma_map_size(input,output,f);
    count_events(labels,rectangles);
    assert forall|id: int| #[trigger] output.contains(id) implies good::good(labels,id) by {
        let key=choose|key: int|input.contains(key) && f(key)==id;
        actual_good(labels,rectangles,key);
    }
}

}
