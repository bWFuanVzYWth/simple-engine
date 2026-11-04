//! 构造阶段只修改标记，不改变图结构或引用总账。
use crate::state::*;
use vstd::prelude::*;

verus! {
pub(crate) open spec fn prefix(before:Pools,after:Pools,done:nat,active:bool)->bool {
    before.len()==after.len()==10 && done<=10 && forall|l:int|0<=l<10 ==>
        #[trigger] after[l]==(PoolState{constructing:if l<done{active}else{before[l].constructing},..before[l]})
}
pub(crate) open spec fn phase(before:Pools,after:Pools,active:bool)->bool {prefix(before,after,10,active)}
pub(super) proof fn next(before:Pools,middle:Pools,after:Pools,done:nat,active:bool)
    requires prefix(before,middle,done,active),done<10,after.len()==10,
        after[done as int]==(PoolState{constructing:active,..middle[done as int]}),
        forall|l:int|0<=l<10 && l!=done ==> #[trigger] after[l]==middle[l],
    ensures prefix(before,after,done+1,active),
{
    assert forall|l:int|0<=l<10 implies #[trigger] after[l]==(PoolState{constructing:if l<done+1{active}else{before[l].constructing},..before[l]}) by {
        assert(middle[l]==(PoolState{constructing:if l<done{active}else{before[l].constructing},..before[l]}));
    }
}
pub(crate) proof fn valid_phase(before:Pools,after:Pools,active:bool)
    requires valid_pools(before),phase(before,after,active),
    ensures valid_pools(after),all_occupied_live(before)<==>all_occupied_live(after),frozen(before,after),frozen(after,before),
{
    assert forall|l:int|0<=l<10 implies #[trigger] valid_pool(after[l]) by {
        assert(valid_pool(before[l]));
        assert forall|i:int,j:int|occupied(after[l],i) && occupied(after[l],j) && after[l].nodes[i]==after[l].nodes[j] implies i==j by {
            assert(occupied(before[l],i));assert(occupied(before[l],j));
        }
        assert forall|i:int,j:int|occupied(after[l],i) && occupied(after[l],j) && after[l].generations[i]==after[l].generations[j] implies i==j by {
            assert(occupied(before[l],i));assert(occupied(before[l],j));
        }
    }
    assert forall|l:int,i:int|0<=l<10 && occupied(after[l],i) implies
        !(exists|v:u32|uniform(after[l].nodes[i]->Some_0,v))
            && forall|slot:int|0<=slot<64 ==> #[trigger] child_valid(after,after[l].nodes[i]->Some_0[slot],l as nat) by {
        assert(occupied(before[l],i));assert(after[l].nodes[i]==before[l].nodes[i]);
        assert forall|slot:int|0<=slot<64 implies #[trigger] child_valid(after,after[l].nodes[i]->Some_0[slot],l as nat) by {
            assert(child_valid(before,before[l].nodes[i]->Some_0[slot],l as nat));
            if let crate::Child::Ptr(index)=after[l].nodes[i]->Some_0[slot] {
                assert(after[l-1].nodes==before[l-1].nodes);assert(after[l-1].rcs==before[l-1].rcs);
            }
        }
    }
    if all_occupied_live(before) {
        assert forall|l:int,i:int|0<=l<10 && occupied(after[l],i) implies #[trigger] live(after[l],i) by {
            assert(occupied(before[l],i));assert(live(before[l],i));
        }
    }
    if all_occupied_live(after) {
        assert forall|l:int,i:int|0<=l<10 && occupied(before[l],i) implies #[trigger] live(before[l],i) by {
            assert(occupied(after[l],i));assert(live(after[l],i));
        }
    }
}
pub(crate) proof fn phase_balance(before:Pools,after:Pools,roots:Seq<crate::Child>,active:bool)
    requires balanced(before,roots),phase(before,after,active),valid_pools(after),
    ensures balanced(after,roots),
{
    assert forall|r:int|0<=r<roots.len() implies #[trigger] child_valid(after,roots[r],10) by {
        assert(child_valid(before,roots[r],10));
    }
    assert forall|l:int,i:int|0<=l<10 && occupied(after[l],i) implies after[l].rcs[i]==
        (if l==9 {occurrences(roots,crate::Child::Ptr(crate::node::proof::at_index(i as u32)))}else{0})
            +(if l<9{parent_edges(after[l+1],crate::Child::Ptr(crate::node::proof::at_index(i as u32)),after[l+1].nodes.len())}else{0})
            +occurrences(after[l].retained,crate::node::proof::at_index(i as u32)) by {
        assert(occupied(before[l],i));
        if l<9 {crate::balance::same_parent_edges(before[l+1],after[l+1],crate::Child::Ptr(crate::node::proof::at_index(i as u32)),before[l+1].nodes.len());}
    }
}
}
