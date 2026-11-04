//! 局部结果所有者的健康域；正常返回由实际构造与重建接口归纳维持。
use super::*;
use crate::state;
use vstd::prelude::*;

verus! {
impl LocalTree {
    pub open(in crate::write) spec fn valid_local(&self) -> bool {
        state::well_formed(&self.pools) && state::all_occupied_live(state::pools_view(&self.pools))
            && state::balanced(state::pools_view(&self.pools), Seq::empty())
            && (forall|l: int| 0 <= l < 10 ==> #[trigger] state::pools_view(&self.pools)[l].free.len() == 0)
            && match self.lod {
                None => self.empty_local(),
                Some(_) => super::super::import::proof::source_valid(self),
            }
    }
}

pub(super) proof fn empty_valid(tree: &LocalTree)
    requires tree.empty_local(),
    ensures tree.valid_local(),
{
    proof::empty_graph(&tree.pools);
}

pub(super) proof fn built_valid(tree: &LocalTree, lod: u8, values: Seq<u32>)
    requires tree.built_from(lod, values),
    ensures tree.valid_local(),
{
    assert forall|p: int| 0 <= p < super::super::import::proof::parts(lod) implies
        #[trigger] state::child_valid(state::pools_view(&tree.pools), tree.roots@[p], (lod / 2) as nat) by {
        assert(proof::built_parts(state::pools_view(&tree.pools), tree.roots@, values, (lod / 2) as nat,
            proof::root_count(lod), state::span((lod / 2) as nat)));
    }
}
}
