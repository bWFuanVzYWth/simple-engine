//! 物理子边认领与失败回滚；重复目标逐边计数。
use super::{Error, Pool, PoolEntry};
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
#[path = "proof/pool_counts.rs"]
mod proof;

#[cfg_attr(verus_only, verus_verify)]
impl Pool {
    /// 按物理边认领；失败仅撤销本次已经认领的边，不触发节点回收。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(), old(self).edge_domain(super::proof::entry_edges(*entry)),
        ensures final(self).valid(), final(self).same_storage(old(self)),
            result is Ok ==> final(self).credits(old(self), super::proof::entry_edges(*entry)),
            result is Err ==> final(self).counts() == old(self).counts(),
    ))]
    pub(crate) fn retain_edges(&mut self, entry: &PoolEntry) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost base = *self; let ghost edges = super::proof::entry_edges(*entry); }
        let mut pointers = entry.pointers();
        let mut retained = 0usize;
        #[cfg(verus_only)]
        proof! { proof::empty(&base); assert(edges.subrange(0, 0) == Seq::empty()); }
        #[cfg_attr(verus_only, verus_spec(
            invariant self.valid(), base.valid(), base.edge_domain(edges), edges.len() <= 64,
                base.same_storage(old(self)), base.counts() == old(self).counts(),
                edges == super::proof::entry_edges(*entry),
                retained <= edges.len(), self.credits(&base, edges.subrange(0, retained as int)),
                proof::bounded(edges, 0, retained as int, self.counts().len()),
                crate::node::proof::remaining(pointers) == edges.subrange(retained as int, edges.len() as int),
            decreases crate::node::proof::remaining(pointers).len(),
        ))]
        loop {
            let Some(idx) = pointers.next() else {
                #[cfg(verus_only)]
                proof! { assert(retained == edges.len()); assert(edges.subrange(0, retained as int) == edges); }
                return Ok(());
            };
            #[cfg(verus_only)]
            proof! { proof::slice_step(edges, 0, retained as int); proof::domain(&base, self, edges, retained as int); }
            #[cfg(verus_only)]
            proof_decl! { let ghost before_increment = *self; }
            if let Err(error) = self.rc_inc(idx) {
                #[cfg(verus_only)]
                proof_decl! { let ghost acquired = retained as int; }
                let mut rollback = entry.pointers();
                #[cfg_attr(verus_only, verus_spec(
                    invariant self.valid(), base.valid(), base.edge_domain(edges), edges.len() <= 64,
                        base.same_storage(old(self)), base.counts() == old(self).counts(),
                        edges == super::proof::entry_edges(*entry),
                        retained <= acquired <= edges.len(),
                        proof::bounded(edges, 0, acquired, self.counts().len()),
                        proof::outstanding(&base, self, edges, acquired - retained, acquired),
                        crate::node::proof::remaining(rollback) == edges.subrange(acquired - retained, edges.len() as int),
                    decreases retained,
                ))]
                while retained != 0 {
                    #[cfg(verus_only)]
                    proof! { proof::rollback_target(&base, self, edges, acquired - retained, acquired);
                    proof::slice_step(edges, 0, acquired - retained); }
                    let previous = rollback.next().ok_or(Error::NodeMissing(idx))?;
                    #[cfg(verus_only)]
                    proof_decl! { let ghost before_rollback = *self; }
                    let count = self
                        .rcs
                        .get_mut(previous.as_usize()?)
                        .ok_or(Error::NodeMissing(previous))?;
                    *count -= 1;
                    #[cfg(verus_only)]
                    proof! { proof::rollback(&base, &before_rollback, self, edges, acquired - retained, acquired); }
                    retained -= 1;
                }
                #[cfg(verus_only)]
                proof! { assert(edges.subrange(acquired, acquired) == Seq::empty()); reveal(Seq::filter);
                assert(self.same_storage(&base)); assert(self.counts() =~= base.counts()); }
                return Err(error);
            }
            #[cfg(verus_only)]
            proof! { proof::increment(&base, &before_increment, self, edges, retained as int); }
            retained += 1;
        }
    }
}
