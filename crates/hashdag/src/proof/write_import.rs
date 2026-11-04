//! 局部物理编号到目标 Child 的逐层语义对应。
use super::*;
use crate::write::{commit::proof as commit, dense_chunks::proof as dense};
use crate::{
    node::proof as node,
    state::{self, Pools},
};
use vstd::prelude::*;
verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(in crate::write) open(in crate::write) spec fn parts(lod:u8)->nat { if lod % 2 == 0 { 1 } else { 8 } }
pub(in crate::write) open(in crate::write) spec fn source_valid(tree:&LocalTree)->bool {
    &&& tree.lod is Some && tree.lod->Some_0 <= 20
    &&& state::well_formed(&tree.pools)
    &&& state::all_occupied_live(state::pools_view(&tree.pools))
    &&& forall|l:int| 0 <= l < 10 ==> #[trigger] state::pools_view(&tree.pools)[l].free.len() == 0
    &&& forall|p:int| 0 <= p < parts(tree.lod->Some_0) ==> #[trigger]
        state::child_valid(state::pools_view(&tree.pools), tree.roots@[p], (tree.lod->Some_0/2) as nat)
}
pub(in crate::write) open(in crate::write) spec fn maps(scratch:&ImportScratch)->Seq<Seq<Child>> {
    Seq::new(10, |l:int| scratch.maps@[l]@)
}
pub(in crate::write) proof fn map_position(scratch:&ImportScratch,before:Seq<Seq<Child>>,level:nat,position:nat)
    requires level<10,maps(scratch)==before,before[level as int].len()==position,
    ensures scratch.maps@[level as int]@.len()==position,
{}
pub(in crate::write) proof fn map_pushed(before:Seq<Seq<Child>>,scratch:&ImportScratch,level:nat,child:Child)
    requires level<10,before.len()==10,scratch.maps@[level as int]@==before[level as int].push(child),
        forall|l:int|0<=l<10 && l!=level ==> #[trigger] scratch.maps@[l]@==before[l],
    ensures maps(scratch)==before.update(level as int,before[level as int].push(child)),
{
    assert(maps(scratch) =~= before.update(level as int,before[level as int].push(child)));
}
pub(in crate::write) open(in crate::write) spec fn lookup_domain(maps:Seq<Seq<Child>>,child:Child,height:nat)->bool {
    match child { Child::Leaf(_)=>true,Child::Ptr(index)=>0 < height <= maps.len() && node::idx(index) < maps[height-1].len() }
}
pub(in crate::write) open(in crate::write) spec fn mapped(maps:Seq<Seq<Child>>,child:Child,height:nat)->Child {
    match child { Child::Leaf(value)=>Child::Leaf(value),Child::Ptr(index)=>maps[height-1][node::idx(index)] }
}
pub(in crate::write) open(in crate::write) spec fn corresponds(source:Pools,target:Pools,original:Child,translated:Child,height:nat)->bool {
    state::child_valid(source,original,height) && state::child_valid(target,translated,height)
        && dense::content(source,original,height)==dense::content(target,translated,height)
}
pub(in crate::write) proof fn leaf_corresponds(source:Pools,target:Pools,value:u32,height:nat)
    requires height<=source.len(),height<=target.len(),
    ensures corresponds(source,target,Child::Leaf(value),Child::Leaf(value),height),
{
    assert(dense::content(source,Child::Leaf(value),height) =~= dense::content(target,Child::Leaf(value),height));
}
pub(in crate::write) open(in crate::write) spec fn prefix(tree:&LocalTree,target:Pools,maps:Seq<Seq<Child>>,level:nat,done:nat)->bool {
    let source=state::pools_view(&tree.pools);
    &&& level <= 10 && maps.len()==10
    &&& (level < 10 ==> done <= source[level as int].nodes.len())
    &&& forall|l:int| 0<=l<10 ==> #[trigger] maps[l].len()
        == if l<level {source[l].nodes.len()} else if l==level {done} else {0nat}
    &&& forall|l:int,i:int| #![trigger maps[l][i]] 0<=l<10 && 0<=i<maps[l].len() ==>
        corresponds(source,target,Child::Ptr(node::at_index(i as u32)),maps[l][i],(l+1) as nat)
}
pub(in crate::write) open(in crate::write) spec fn row_lookup(maps:Seq<Seq<Child>>,row:Seq<Child>,height:nat)->bool {
    row.len()==64 && forall|i:int|0<=i<64 ==> #[trigger] lookup_domain(maps,row[i],height)
}
pub(in crate::write) open(in crate::write) spec fn row_mapped(maps:Seq<Seq<Child>>,row:Seq<Child>,height:nat)->Seq<Child> {
    row.map_values(|child:Child| mapped(maps,child,height))
}
pub(in crate::write) open(in crate::write) spec fn pending(row:Seq<Child>,entry:Seq<Child>,maps:Seq<Seq<Child>>,height:nat,bits:u64)->bool {
    row.len()==entry.len()==64
        && forall|i:u8| i<64 ==> #[trigger] entry[i as int] == if bits & (1u64 << i)!=0 {row[i as int]}else{mapped(maps,row[i as int],height)}
            && (bits & (1u64 << i)!=0 ==> row[i as int] is Ptr)
}
pub(in crate::write) proof fn pending_initial(entry:PoolEntry,maps:Seq<Seq<Child>>,height:nat)
    requires row_lookup(maps,node::children(entry),height),
    ensures pending(node::children(entry),node::children(entry),maps,height,node::mask(entry.mask)),
{}
pub(in crate::write) proof fn pending_finished(row:Seq<Child>,entry:Seq<Child>,maps:Seq<Seq<Child>>,height:nat)
    requires pending(row,entry,maps,height,0),
    ensures entry==row_mapped(maps,row,height),
{
    assert forall|i:int|0<=i<64 implies entry[i]==mapped(maps,row[i],height) by {
        assert(0u64 & (1u64 << (i as u8))==0) by(bit_vector);
        assert(entry[i as u8 as int]==mapped(maps,row[i as u8 as int],height));
    }
    assert(entry =~= row_mapped(maps,row,height));
}
pub(in crate::write) proof fn lowest_cleared(before:u64,after:u64,slot:u8)
    requires before!=0,after==before & ((before-1) as u64),slot==before.trailing_zeros() as u8,
    ensures slot<64,before & (1u64 << slot)!=0,after<before,
        forall|i:u8|i<64 ==> (after & (1u64 << i)!=0 <==> before & (1u64 << i)!=0 && i!=slot),
{
    node::nonzero_bit(before); node::cleared_least(before);
    assert(before!=0 && after==before & ((before-1) as u64) ==> after<before) by(bit_vector);
    assert forall|i:u8|i<64 implies (after & (1u64 << i)!=0 <==> before & (1u64 << i)!=0 && i!=slot) by {
        assert(slot<64 && i<64 ==> ((before & !(1u64 << slot)) & (1u64 << i)!=0 <==> before & (1u64 << i)!=0 && i!=slot)) by(bit_vector);
    }
}
pub(in crate::write) proof fn pending_step(row:Seq<Child>,before:Seq<Child>,after:Seq<Child>,maps:Seq<Seq<Child>>,height:nat,old_bits:u64,bits:u64,slot:u8)
    requires pending(row,before,maps,height,old_bits),row_lookup(maps,row,height),
        old_bits!=0,bits==old_bits & ((old_bits-1) as u64),slot==old_bits.trailing_zeros() as u8,
        after==before.update(slot as int,mapped(maps,before[slot as int],height)),
    ensures pending(row,after,maps,height,bits),
{
    lowest_cleared(old_bits,bits,slot);
    assert forall|i:u8|i<64 implies #[trigger] after[i as int] == if bits & (1u64 << i)!=0 {row[i as int]}else{mapped(maps,row[i as int],height)}
        && (bits & (1u64 << i)!=0 ==> row[i as int] is Ptr) by {
        if i==slot {assert(before[i as int]==row[i as int]);}
    }
}

pub(in crate::write) proof fn row_domain(tree:&LocalTree,target:Pools,maps:Seq<Seq<Child>>,level:nat,position:nat)
    requires source_valid(tree),prefix(tree,target,maps,level,position),level<10,
        position < state::pools_view(&tree.pools)[level as int].nodes.len(),
    ensures state::pools_view(&tree.pools)[level as int].nodes[position as int] is Some,
        row_lookup(maps,state::pools_view(&tree.pools)[level as int].nodes[position as int]->Some_0,level),
        forall|i:int|0<=i<64 ==> #[trigger] state::child_valid(state::pools_view(&tree.pools),state::pools_view(&tree.pools)[level as int].nodes[position as int]->Some_0[i],level),
{
    let source=state::pools_view(&tree.pools);
    let row=source[level as int].nodes[position as int]->Some_0;
    assert(source[level as int].free.len()==0);
    assert(state::occupied(source[level as int],position as int));
    assert forall|i:int|0<=i<64 implies #[trigger] lookup_domain(maps,row[i],level) by {
        assert(state::child_valid(source,row[i],level));
    }
}

pub(in crate::write) proof fn child_domain(tree:&LocalTree,target:Pools,maps:Seq<Seq<Child>>,child:Child,height:nat)
    requires source_valid(tree),prefix(tree,target,maps,10,0),height<=10,state::valid_pools(target),
        state::child_valid(state::pools_view(&tree.pools),child,height),
    ensures lookup_domain(maps,child,height),corresponds(state::pools_view(&tree.pools),target,child,mapped(maps,child,height),height),
{
    if let Child::Ptr(index)=child {
        let i=node::idx(index);let source=state::pools_view(&tree.pools);
        assert(i<=u32::MAX);
        assert(corresponds(source,target,Child::Ptr(node::at_index(i as u32)),maps[height-1][i],height));
    } else if let Child::Leaf(value)=child {leaf_corresponds(state::pools_view(&tree.pools),target,value,height);}
}

pub(in crate::write) proof fn entry_ready(tree:&LocalTree,target:Pools,maps:Seq<Seq<Child>>,level:nat,source_entry:PoolEntry,entry:PoolEntry)
    requires source_valid(tree),level<10,state::valid_pools(target),
        forall|i:int|0<=i<64 ==> #[trigger] state::child_valid(state::pools_view(&tree.pools),node::children(source_entry)[i],level),
        forall|l:int,i:int| #![trigger maps[l][i]] 0<=l<level && 0<=i<maps[l].len() ==>
            corresponds(state::pools_view(&tree.pools),target,Child::Ptr(node::at_index(i as u32)),maps[l][i],(l+1) as nat),
        row_lookup(maps,node::children(source_entry),level),
        node::children(entry)==row_mapped(maps,node::children(source_entry),level),
    ensures commit::entry_ready(target,entry,level as u8),
{
    let source=state::pools_view(&tree.pools);let row=node::children(source_entry);
    assert forall|i:int|0<=i<64 implies #[trigger] state::child_valid(target,node::children(entry)[i],level) by {
        if let Child::Ptr(index)=row[i] {
            let pos=node::idx(index);
            assert(lookup_domain(maps,row[i],level));
            assert(0<=pos<maps[level-1].len());
            assert(corresponds(source,target,Child::Ptr(node::at_index(pos as u32)),maps[level-1][pos],level));
        }
    }
}

pub(in crate::write) proof fn prefix_frozen(tree:&LocalTree,before:Pools,after:Pools,maps:Seq<Seq<Child>>,level:nat,done:nat)
    requires prefix(tree,before,maps,level,done),state::build_step(before,after),
    ensures prefix(tree,after,maps,level,done),
{
    assert forall|l:int,i:int|0<=l<10 && 0<=i<maps[l].len() implies #[trigger]
        corresponds(state::pools_view(&tree.pools),after,Child::Ptr(node::at_index(i as u32)),maps[l][i],(l+1) as nat) by {
        assert(corresponds(state::pools_view(&tree.pools),before,Child::Ptr(node::at_index(i as u32)),maps[l][i],(l+1) as nat));
        dense::content_frozen(before,after,maps[l][i],(l+1) as nat);
    }
}

pub(in crate::write) proof fn row_added(tree:&LocalTree,before:Pools,after:Pools,old_maps:Seq<Seq<Child>>,maps:Seq<Seq<Child>>,level:nat,position:nat,source_entry:PoolEntry,entry:PoolEntry,child:Child)
    requires source_valid(tree),level<10,prefix(tree,before,old_maps,level,position),state::build_step(before,after),
        position<state::pools_view(&tree.pools)[level as int].nodes.len(),
        state::pools_view(&tree.pools)[level as int].nodes[position as int]==Some(node::children(source_entry)),
        maps==old_maps.update(level as int,old_maps[level as int].push(child)),
        node::children(entry)==row_mapped(old_maps,node::children(source_entry),level),
        state::child_valid(after,child,level+1),commit::image(after[level as int],child)==node::children(entry),
    ensures prefix(tree,after,maps,level,position+1),
{
    let source=state::pools_view(&tree.pools);let row=node::children(source_entry);
    let original=Child::Ptr(node::at_index(position as u32));
    prefix_frozen(tree,before,after,old_maps,level,position);
    assert(state::occupied(source[level as int],position as int));
    assert(state::child_valid(source,original,level+1));
    assert forall|i:int|0<=i<64 implies #[trigger] corresponds(source,after,row[i],mapped(old_maps,row[i],level),level) by {
        if let Child::Ptr(index)=row[i] {
            let pos=node::idx(index);
            assert(state::child_valid(source,row[i],level));
            assert(pos<old_maps[level-1].len());
            assert(corresponds(source,after,Child::Ptr(node::at_index(pos as u32)),old_maps[level-1][pos],level));
        } else if let Child::Leaf(value)=row[i] {leaf_corresponds(source,after,value,level);}
    }
    assert forall|path:int|0<=path<state::span(level+1) implies
        #[trigger] dense::content(source,original,level+1)[path] == dense::content(after,child,level+1)[path] by {
        crate::canonical::span_positive(level);
        let width=state::span(level) as int;let slot=path/width;let local=path%width;
        vstd::arithmetic::div_mod::lemma_fundamental_div_mod(path,width);
        vstd::arithmetic::div_mod::lemma_mod_bound(path,width);
        assert(state::span(level+1)==64*state::span(level));
        assert(0<=slot<64) by(nonlinear_arith) requires width>0,0<=path<64*width,path==slot*width+local,0<=local<width;
        crate::canonical::descend(source,original,level+1,slot,local);
        crate::canonical::descend(after,child,level+1,slot,local);
        assert(crate::canonical::child(source,original,level+1,slot)==row[slot]);
        assert(node::children(entry)[slot]==mapped(old_maps,row[slot],level));
        assert(commit::image(after[level as int],child)[slot]==node::children(entry)[slot]);
        assert(crate::canonical::child(after,child,level+1,slot)==mapped(old_maps,row[slot],level));
        assert(corresponds(source,after,row[slot],mapped(old_maps,row[slot],level),level));
        assert(dense::content(source,row[slot],level)[local]==dense::content(after,mapped(old_maps,row[slot],level),level)[local]);
    }
    assert(dense::content(source,original,level+1) =~= dense::content(after,child,level+1));
    assert(corresponds(source,after,original,child,level+1));
    assert forall|l:int,i:int| #![trigger maps[l][i]] 0<=l<10 && 0<=i<maps[l].len() implies
        corresponds(source,after,Child::Ptr(node::at_index(i as u32)),maps[l][i],(l+1) as nat) by {
        if l==level && i==position {} else {assert(maps[l][i]==old_maps[l][i]);}
    }
}

pub(in crate::write) open(in crate::write) spec fn roots_prefix(tree:&LocalTree,target:Pools,roots:Seq<Child>,done:nat)->bool {
    roots.len()==8 && done<=parts(tree.lod->Some_0)
        && forall|p:int|0<=p<done ==> #[trigger] corresponds(state::pools_view(&tree.pools),target,tree.roots@[p],roots[p],(tree.lod->Some_0/2) as nat)
}
pub(in crate::write) open(in crate::write) spec fn imported(tree:&LocalTree,target:Pools,scratch:&ImportScratch)->bool {
    prefix(tree,target,maps(scratch),10,0) && roots_prefix(tree,target,scratch.roots@,parts(tree.lod->Some_0))
}
pub(in crate::write) proof fn imported_frozen(tree:&LocalTree,before:Pools,after:Pools,old:&ImportScratch,new:&ImportScratch)
    requires source_valid(tree),imported(tree,before,old),state::build_step(before,after),
        maps(old)==maps(new),old.roots@==new.roots@,
    ensures imported(tree,after,new),
{
    prefix_frozen(tree,before,after,maps(old),10,0);
    assert forall|p:int|0<=p<parts(tree.lod->Some_0) implies #[trigger]
        corresponds(state::pools_view(&tree.pools),after,tree.roots@[p],new.roots@[p],(tree.lod->Some_0/2) as nat) by {
        assert(corresponds(state::pools_view(&tree.pools),before,tree.roots@[p],old.roots@[p],(tree.lod->Some_0/2) as nat));
        dense::content_frozen(before,after,old.roots@[p],(tree.lod->Some_0/2) as nat);
    }
}
}
