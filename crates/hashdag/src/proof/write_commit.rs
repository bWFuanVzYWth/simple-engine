//! 候选节点内容与物理池的连接；不假设整个写入或唯一化正确。
use crate::{
    Child,
    node::{PoolEntry, proof as node},
    pool::{InsertResult, Pool},
    state,
};
use vstd::prelude::*;

verus! {
pub(crate) open spec fn storage(pools:Seq<Pool>)->state::Pools {
    Seq::new(pools.len(),|level:int|pools[level].state())
}
pub(crate) open spec fn valid_storage(pools:Seq<Pool>)->bool {
    forall|level:int| 0<=level<pools.len() ==> #[trigger] pools[level].valid()
}
pub(crate) proof fn storage_view(pools:&[Pool;10])
    ensures storage(pools@)==state::pools_view(pools),
{
    assert(storage(pools@) =~= state::pools_view(pools));
}
pub(crate) open spec fn building(pools:&[Pool;10])->bool {
    state::well_formed(pools) && state::all_occupied_live(state::pools_view(pools))
        && forall|level:int|0<=level<10 ==> #[trigger] state::pools_view(pools)[level].constructing
}
pub(crate) open spec fn entry_ready(pools:state::Pools,entry:PoolEntry,level:u8)->bool {
    level<10 && forall|slot:int|0<=slot<64 ==> #[trigger] state::child_valid(pools,node::children(entry)[slot],level as nat)
}
pub(crate) proof fn entry_domain(pools:&[Pool;10],entry:PoolEntry,level:u8)
    requires building(pools),entry_ready(state::pools_view(pools),entry,level),
    ensures valid_storage(pools@),layer_domain(pools@,level,entry),
{
    if level>0 {
        assert forall|slot:int|0<=slot<64 implies match #[trigger] node::children(entry)[slot] {
            Child::Leaf(_)=>true,Child::Ptr(index)=>state::occupied(pools@[(level-1) as int].state(),node::idx(index)),
        } by {assert(state::child_valid(state::pools_view(pools),node::children(entry)[slot],level as nat));}
        pools@[(level-1) as int].edge_domain_model(entry);
    }
}
pub(crate) proof fn valid_models(pools:&[Pool;10])
    requires valid_storage(pools@),
    ensures forall|level:int|0<=level<10 ==> #[trigger] state::valid_pool(state::pools_view(pools)[level]),
{
    assert forall|level:int|0<=level<10 implies #[trigger] state::valid_pool(state::pools_view(pools)[level]) by {
        pools@[level].valid_model();
    }
}
pub(crate) open spec fn layer_domain(pools:Seq<Pool>,level:u8,entry:PoolEntry)->bool {
    0<level<pools.len() ==> pools[(level-1) as int].edge_domain(crate::pool::proof::entry_edges(entry))
}
pub(crate) open spec fn inserted_index(inserted:InsertResult)->crate::NodeIdx {
    match inserted {InsertResult::New(i)|InsertResult::Existing(i)=>i}
}
pub(crate) open spec fn inserted_delta(before:state::Pools,after:state::Pools,level:u8,entry:PoolEntry,rc:usize,inserted:InsertResult)->bool {
    0<=level<before.len() && before.len()==after.len()
        && crate::pool::proof::interned(before[level as int],after[level as int],node::children(entry),rc,inserted)
        && (forall|other:int| 0<=other<before.len() && other!=level && other+1!=level ==> #[trigger] after[other]==before[other])
        && if level>0 {
            if inserted is New {crate::pool::proof::credited(before[(level-1) as int],after[(level-1) as int],crate::pool::proof::entry_edges(entry))}
            else {after[(level-1) as int]==before[(level-1) as int]}
        }else{true}
}
pub(crate) open spec fn layer_delta(before:state::Pools,after:state::Pools,level:u8,entry:PoolEntry,rc:usize,index:crate::NodeIdx)->bool {
    exists|inserted:InsertResult| inserted_index(inserted)==index && inserted_delta(before,after,level,entry,rc,inserted)
}
pub(crate) proof fn connected(before:state::Pools,after:state::Pools,level:u8,entry:PoolEntry,rc:usize,inserted:InsertResult)
    requires inserted_delta(before,after,level,entry,rc,inserted),
    ensures layer_delta(before,after,level,entry,rc,inserted_index(inserted)),
{
}
pub(crate) open spec fn commit_result(before:state::Pools,after:state::Pools,level:u8,entry:PoolEntry,rc:usize,result:(crate::NodeIdx,Option<u32>))->bool {
    match result.1 {
        Some(value)=>before==after && forall|i:int|0<=i<64 ==> #[trigger] node::children(entry)[i]==Child::Leaf(value),
        None=>!node::uniform(node::children(entry)) && layer_delta(before,after,level,entry,rc,result.0),
    }
}
pub(crate) open spec fn output(result:(crate::NodeIdx,Option<u32>))->Child {
    match result.1 {Some(value)=>Child::Leaf(value),None=>Child::Ptr(result.0)}
}
pub(crate) proof fn committed(before:state::Pools,after:state::Pools,level:u8,entry:PoolEntry,result:(crate::NodeIdx,Option<u32>))
    requires state::valid_pools(before),state::all_occupied_live(before),
        forall|l:int|0<=l<10 ==> #[trigger] before[l].constructing,
        after.len()==10,forall|l:int|0<=l<10 ==> #[trigger] state::valid_pool(after[l]),
        entry_ready(before,entry,level),commit_result(before,after,level,entry,0,result),
    ensures state::build_step(before,after),state::child_valid(after,output(result),(level+1) as nat),
        image(after[level as int],output(result))==node::children(entry),
{
    match result.1 {
        Some(value)=>{
            assert(state::build_step(before,after));
            assert(image(after[level as int],Child::Leaf(value)) =~= node::children(entry));
        },
        None=>{
            let inserted=choose|inserted:InsertResult|inserted_index(inserted)==result.0 && inserted_delta(before,after,level,entry,0,inserted);
            crate::commit_proof::intern_build(before,after,level,entry,inserted);
        },
    }
}
pub(crate) open spec fn present(pool: state::PoolState, child: Child) -> bool {
    match child { Child::Leaf(_) => true, Child::Ptr(index) => state::live(pool, node::idx(index)) }
}
pub(crate) open spec fn image(pool: state::PoolState, child: Child) -> Seq<Child> {
    match child {
        Child::Leaf(value) => Seq::new(64, |i:int| Child::Leaf(value)),
        Child::Ptr(index) => pool.nodes[node::idx(index)]->Some_0,
    }
}
pub(crate) proof fn uniform_image(entry: PoolEntry, value: u32)
    requires node::values(entry.values) == Seq::new(64, |i:int| value), node::mask(entry.mask) == 0,
    ensures node::children(entry) == Seq::new(64, |i:int| Child::Leaf(value)),
{
    assert forall|i:int| 0 <= i < 64 implies node::children(entry)[i] == Child::Leaf(value) by {
        let shift = i as u64;
        assert(0u64 & (1u64 << shift) == 0) by(bit_vector);
    }
    assert(node::children(entry) =~= Seq::new(64, |i:int| Child::Leaf(value)));
}
pub(crate) proof fn data_image(entry:PoolEntry)
    requires node::mask(entry.mask)==0,
    ensures node::children(entry)==Seq::new(64,|i:int|Child::Leaf(node::values(entry.values)[i])),
{
    assert forall|i:int|0<=i<64 implies node::children(entry)[i]==Child::Leaf(node::values(entry.values)[i]) by {
        let shift=i as u64;
        assert(0u64&(1u64<<shift)==0) by(bit_vector);
    }
    assert(node::children(entry) =~= Seq::new(64,|i:int|Child::Leaf(node::values(entry.values)[i])));
}
}
