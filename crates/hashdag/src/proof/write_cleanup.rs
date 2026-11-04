//! 构造持有的弹出与级联释放；保持实际根和其余存活图。
use crate::{
    HashDag,
    node::{Child, NodeIdx, proof as node},
    state::*,
};
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;
pub(super) open spec fn representation(before:Pools,after:Pools)->bool {
    before.len()==after.len()==10 && forall|l:int|0<=l<10 ==> #[trigger] after[l].nodes==before[l].nodes
        && after[l].rcs==before[l].rcs && after[l].generations==before[l].generations
        && after[l].next_generation==before[l].next_generation && after[l].free==before[l].free
        && after[l].retained.is_prefix_of(before[l].retained)
}
proof fn same_graph(before:Pools,after:Pools)
    requires valid_pools(before),all_occupied_live(before),representation(before,after),
    ensures valid_pools(after),all_occupied_live(after),frozen(before,after),frozen(after,before),
{
    assert forall|l:int|0<=l<10 implies #[trigger] valid_pool(after[l]) by {
        assert(valid_pool(before[l]));
        assert(after[l].nodes==before[l].nodes);
        assert forall|i:int|0<=i<after[l].retained.len() implies #[trigger] after[l].retained[i]==before[l].retained[i]
            && live(after[l],node::idx(after[l].retained[i])) by {}
        assert(after[l].retained.no_duplicates()) by {
            assert forall|i:int,j:int|0<=i<j<after[l].retained.len() implies after[l].retained[i]!=after[l].retained[j] by {}
        }
        assert forall|i:int|0<=i<after[l].nodes.len() implies #[trigger] after[l].nodes[i]==before[l].nodes[i] by {}
        assert forall|i:int,j:int|occupied(after[l],i) && occupied(after[l],j) && after[l].nodes[i]==after[l].nodes[j] implies i==j by {
            assert(occupied(before[l],i));assert(occupied(before[l],j));
        }
        assert forall|i:int,j:int|occupied(after[l],i) && occupied(after[l],j) && after[l].generations[i]==after[l].generations[j] implies i==j by {
            assert(occupied(before[l],i));assert(occupied(before[l],j));
        }
    }
    assert forall|l:int,i:int|0<=l<10 && occupied(after[l],i) implies #[trigger] live(after[l],i) by {
        assert(occupied(before[l],i));assert(live(before[l],i));
    }
    assert forall|l:int,i:int|0<=l<10 && occupied(after[l],i) implies
        !(exists|v:u32|uniform(after[l].nodes[i]->Some_0,v))
            && forall|slot:int|0<=slot<64 ==> #[trigger] child_valid(after,after[l].nodes[i]->Some_0[slot],l as nat) by {
        assert(occupied(before[l],i));assert(after[l].nodes[i]==before[l].nodes[i]);
        assert forall|slot:int|0<=slot<64 implies #[trigger] child_valid(after,after[l].nodes[i]->Some_0[slot],l as nat) by {
            assert(child_valid(before,before[l].nodes[i]->Some_0[slot],l as nat));
            if let Child::Ptr(index)=after[l].nodes[i]->Some_0[slot] {
                assert(after[l-1].nodes==before[l-1].nodes);assert(after[l-1].rcs==before[l-1].rcs);
            }
        }
    }
}
pub(crate) open spec fn eligible(dag:&HashDag)->bool {
    dag.roots.valid() && well_formed(&dag.pools)
        && balanced(pools_view(&dag.pools),dag.root_children())
        && all_occupied_live(pools_view(&dag.pools))
        && hash_dag_std::vec_model::capacity(dag.scratch_stack)>=crate::rc::proof::capacity()
}
pub(crate) open spec fn pop_frame(before:Pools,after:Pools,level:u8,index:NodeIdx)->bool {
    before.len()==after.len()==10 && level<10 && before[level as int].retained.len()>0
        && before[level as int].retained.last()==index
        && after[level as int]==(PoolState{retained:before[level as int].retained.drop_last(),..before[level as int]})
        && forall|l:int|0<=l<10 && l!=level ==> #[trigger] after[l]==before[l]
}
pub(crate) proof fn popped(before:Pools,after:Pools,roots:Seq<Child>,level:u8,index:NodeIdx)
    requires balanced(before,roots),all_occupied_live(before),pop_frame(before,after,level,index),
    ensures crate::rc::proof::eligible_model(after,index,level),
        valid_pools(after),all_occupied_live(after),frozen(before,after),frozen(after,before),
        forall|l:int,i:int|0<=l<10 && 0<=i<before[l].nodes.len() ==>
            #[trigger] net_credit(after,l,i)==net_credit(before,l,i)+if l==level && i==node::idx(index){1int}else{0int},
{
    assert(representation(before,after));same_graph(before,after);
    assert forall|l:int|0<=l<10 implies #[trigger] after[l].nodes==before[l].nodes by {}
    assert forall|l:int,i:int|0<=l<10 && 0<=i<before[l].nodes.len() implies
        #[trigger] net_credit(after,l,i)==net_credit(before,l,i)+if l==level && i==node::idx(index){1int}else{0int} by {
        if l<9 {crate::balance::same_parent_edges(before[l+1],after[l+1],Child::Ptr(node::at_index(i as u32)),before[l+1].nodes.len());}
        if l==level {
            assert(before[l].retained==after[l].retained.push(index));
            crate::rc::proof::push_count(after[l].retained,index,node::at_index(i as u32));
        }
        crate::balance::net_matches_roots(before,roots,l,i);
    }
    assert forall|l:int,i:int|0<=l<10 && 0<=i<after[l].nodes.len() implies #[trigger] net_credit(after,l,i)>=0 by {
        assert(valid_pool(before[l]));crate::balance::net_matches_roots(before,roots,l,i);
    }
    let target=node::idx(index);
    assert(before[level as int].retained[(before[level as int].retained.len()-1) as int]==index);
    assert(live(before[level as int],target));
    crate::balance::net_matches_roots(before,roots,level as int,target);
    assert(net_credit(after,level as int,target)>0);
}
pub(crate) proof fn released(before:Pools,middle:Pools,after:Pools,roots:Seq<Child>,level:u8,index:NodeIdx)
    requires balanced(before,roots),all_occupied_live(before),pop_frame(before,middle,level,index),
        crate::rc::proof::release_step(middle,after,index,level),
    ensures balanced(after,roots),all_occupied_live(after),frozen(after,before),
        forall|l:int|0<=l<10 ==> #[trigger] after[l].nodes.len()==before[l].nodes.len()
            && after[l].generations==before[l].generations && after[l].next_generation==before[l].next_generation
            && after[l].constructing==before[l].constructing
            && after[l].retained==if l==level {before[l].retained.drop_last()}else{before[l].retained},
{
    popped(before,middle,roots,level,index);
    crate::balance::frozen_transitive(after,middle,before);
    assert forall|l:int,i:int|0<=l<10 && 0<=i<after[l].nodes.len() implies
        #[trigger] net_credit(after,l,i)==if l==9 {occurrences(roots,Child::Ptr(node::at_index(i as u32))) as int}else{0int} by {
        crate::balance::net_matches_roots(before,roots,l,i);
    }
    crate::balance::from_net(after,roots);
}
pub(crate) proof fn ended(before:Pools,after:Pools,roots:Seq<Child>,level:u8)
    requires balanced(before,roots),all_occupied_live(before),level<10,before.len()==after.len(),
        after[level as int]==(PoolState{constructing:false,..before[level as int]}),
        forall|l:int|0<=l<10 && l!=level ==> #[trigger] after[l]==before[l],
    ensures balanced(after,roots),all_occupied_live(after),frozen(before,after),frozen(after,before),
{
    assert(representation(before,after));same_graph(before,after);
    assert forall|l:int|0<=l<10 implies #[trigger] after[l].nodes==before[l].nodes by {}
    assert forall|l:int,i:int|0<=l<10 && 0<=i<after[l].nodes.len() implies
        #[trigger] net_credit(after,l,i)==net_credit(before,l,i) by {
        if l<9 {crate::balance::same_parent_edges(before[l+1],after[l+1],Child::Ptr(node::at_index(i as u32)),before[l+1].nodes.len());}
    }
    assert forall|l:int,i:int|0<=l<10 && 0<=i<after[l].nodes.len() implies
        #[trigger] net_credit(after,l,i)==if l==9{occurrences(roots,Child::Ptr(node::at_index(i as u32))) as int}else{0int} by {
        crate::balance::net_matches_roots(before,roots,l,i);
    }
    crate::balance::from_net(after,roots);
}
pub(crate) proof fn preserved_roots(before:Pools,after:Pools,roots:Seq<Child>)
    requires balanced(before,roots),balanced(after,roots),frozen(after,before),
    ensures forall|r:int|0<=r<roots.len() ==> #[trigger] child_valid(after,roots[r],10),
        forall|r:int,path:int|0<=r<roots.len() && 0<=path<span(10) ==>
            #[trigger] eval(after,roots[r],10,path)==eval(before,roots[r],10,path),
{
    assert forall|r:int,path:int|0<=r<roots.len() && 0<=path<span(10) implies
        #[trigger] eval(after,roots[r],10,path)==eval(before,roots[r],10,path) by {
        assert(child_valid(after,roots[r],10));
        crate::canonical::frozen_value(after,before,roots[r],10,path);
    }
}
}
