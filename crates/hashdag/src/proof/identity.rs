//! 公开内容身份读取与真实池代次的连接。
use super::*;
use vstd::prelude::*;
verus! {
impl HashDag {
    pub open(crate) spec fn identity_result(&self, level: u8, index: NodeIdx, result: Result<NodeIdentity, Error>) -> bool {
        let pools = crate::state::pools_view(&self.pools);
        let i = crate::node::proof::idx(index);
        &&& (result is Ok <==> level < 10 && crate::state::live(pools[level as int], i))
        &&& (result is Ok ==> result->Ok_0.owner == self.roots.owner as u64
            && result->Ok_0.generation == pools[level as int].generations[i])
        &&& (level >= 10 ==> result is Err && result->Err_0 is InvalidLod)
        &&& (level < 10 && !crate::state::live(pools[level as int], i) ==> result is Err && result->Err_0 is NodeMissing)
    }
}
}
