//! 局部节点逐层翻译到目标池；映射只借用目标构造持有的节点。
use super::built::{ImportScratch, LocalTree};
use super::commit::commit_node;
use crate::hashdag::{DAG_DEPTH, Error};
use crate::node::{Child, PoolEntry};
use crate::pool::Pool;
#[cfg(not(verus_only))]
use crate::{simd as simd_ops, vec_ops};
#[cfg(verus_only)]
use hash_dag_std::{simd_ops, vec_ops};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "../proof/write_import.rs"]
pub(crate) mod proof;

#[cfg_attr(verus_only, verus_verify)]
impl ImportScratch {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires super::commit::proof::building(old(pools)), proof::source_valid(tree),
        ensures super::commit::proof::building(final(pools)),
            crate::state::build_step(crate::state::pools_view(old(pools)), crate::state::pools_view(final(pools))),
            result is Ok ==> proof::imported(tree, crate::state::pools_view(final(pools)), final(self)),
    ))]
    pub(super) fn import(
        &mut self,
        pools: &mut [Pool; DAG_DEPTH as usize],
        tree: &LocalTree,
    ) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost initial = crate::state::pools_view(pools); }
        self.clear();
        let mut level = 0usize;
        #[cfg_attr(verus_only, verus_spec(
            invariant level <= 10, self.empty(), proof::source_valid(tree),
                super::commit::proof::building(pools), crate::state::pools_view(pools) == initial,
                forall|l:int| 0 <= l < level ==> hash_dag_std::vec_model::capacity(self.maps@[l]) >= crate::state::pools_view(&tree.pools)[l].nodes.len(),
            decreases 10-level,
        ))]
        while level < DAG_DEPTH as usize {
            #[cfg(verus_only)]
            proof_decl! { let ghost old_maps = proof::maps(self); }
            vec_ops::reserve_exact(&mut self.maps[level], tree.pools[level].arrays().0.len())?;
            #[cfg(verus_only)]
            proof! { assert(proof::maps(self) =~= old_maps); assert(self.empty()); }
            level += 1;
        }
        level = 0;
        #[cfg_attr(verus_only, verus_spec(
            invariant level <= 10, proof::source_valid(tree), super::commit::proof::building(pools),
                initial == crate::state::pools_view(old(pools)),
                crate::state::build_step(initial, crate::state::pools_view(pools)),
                proof::prefix(tree, crate::state::pools_view(pools), proof::maps(self), level as nat, 0),
                forall|l:int| 0 <= l < 10 ==> hash_dag_std::vec_model::capacity(self.maps@[l]) >= crate::state::pools_view(&tree.pools)[l].nodes.len(),
            decreases 10-level,
        ))]
        while level < DAG_DEPTH as usize {
            let (values, masks) = tree.pools[level].arrays();
            let row_count = values.len().min(masks.len());
            let mut position = 0usize;
            #[cfg_attr(verus_only, verus_spec(
                invariant level < 10, proof::source_valid(tree), super::commit::proof::building(pools),
                    values@.len() == masks@.len() == crate::state::pools_view(&tree.pools)[level as int].nodes.len(),
                    row_count == values@.len(),
                    position <= values@.len(), initial == crate::state::pools_view(old(pools)),
                    crate::state::build_step(initial, crate::state::pools_view(pools)),
                    proof::prefix(tree, crate::state::pools_view(pools), proof::maps(self), level as nat, position as nat),
                    forall|l:int| 0 <= l < 10 ==> hash_dag_std::vec_model::capacity(self.maps@[l]) >= crate::state::pools_view(&tree.pools)[l].nodes.len(),
                    forall|i:int| 0 <= i < values@.len() ==> crate::state::pools_view(&tree.pools)[level as int].nodes[i]
                        == Some(crate::node::proof::children(PoolEntry { values: values@[i], mask: masks@[i] })),
                decreases values@.len()-position,
            ))]
            while position < row_count {
                let mut entry = PoolEntry {
                    values: values[position],
                    mask: masks[position],
                };
                #[cfg(verus_only)]
                proof_decl! { let ghost source_entry = entry; let ghost step = crate::state::pools_view(pools); let ghost old_maps = proof::maps(self); }
                #[cfg(verus_only)]
                proof! { proof::row_domain(tree, step, old_maps, level as nat, position as nat); }
                self.translate_entry(&mut entry, level)?;
                #[cfg(verus_only)]
                proof! { proof::entry_ready(tree, step, old_maps, level as nat, source_entry, entry); }
                let result = commit_node(pools, level as u8, &entry);
                #[cfg(verus_only)]
                proof! { crate::balance::build_transitive(initial, step, crate::state::pools_view(pools)); }
                let child = result?;
                #[cfg(verus_only)]
                proof! { proof::map_position(self, old_maps, level as nat, position as nat); }
                vec_ops::push(&mut self.maps[level], child);
                #[cfg(verus_only)]
                proof! { proof::map_pushed(old_maps, self, level as nat, child); proof::row_added(tree, step, crate::state::pools_view(pools), old_maps, proof::maps(self), level as nat, position as nat, source_entry, entry, child); }
                position += 1;
            }
            level += 1;
        }
        let lod = tree.lod.ok_or(Error::SubtreeNotBuilt)?;
        let count = if lod.is_multiple_of(2) { 1 } else { 8 };
        let mut part = 0usize;
        #[cfg_attr(verus_only, verus_spec(
            invariant part <= count <= 8, lod == tree.lod->Some_0, count == proof::parts(lod),
                proof::source_valid(tree), super::commit::proof::building(pools),
                crate::state::build_step(initial, crate::state::pools_view(pools)),
                proof::prefix(tree, crate::state::pools_view(pools), proof::maps(self), 10, 0),
                proof::roots_prefix(tree, crate::state::pools_view(pools), self.roots@, part as nat),
            decreases count-part,
        ))]
        while part < count {
            #[cfg(verus_only)]
            proof! { proof::child_domain(tree, crate::state::pools_view(pools), proof::maps(self), tree.roots@[part as int], (lod/2) as nat); }
            self.roots[part] = self.translate(usize::from(lod / 2), tree.roots[part])?;
            part += 1;
        }
        Ok(())
    }

    #[inline(always)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires height <= 10, proof::lookup_domain(proof::maps(self), child, height as nat),
        ensures result == Ok(proof::mapped(proof::maps(self), child, height as nat)),
    ))]
    fn translate(&self, height: usize, child: Child) -> Result<Child, Error> {
        match child {
            Child::Leaf(value) => Ok(Child::Leaf(value)),
            Child::Ptr(index) => {
                let layer = height.checked_sub(1).ok_or(Error::NodeMissing(index))?;
                let map = self.maps.get(layer).ok_or(Error::NodeMissing(index))?;
                let position = index.as_usize()?;
                map.get(position).copied().ok_or(Error::NodeMissing(index))
            }
        }
    }

    #[inline(always)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires level <= 10, proof::row_lookup(proof::maps(self), crate::node::proof::children(*old(entry)), level as nat),
        ensures result is Ok, crate::node::proof::children(*final(entry)) == proof::row_mapped(proof::maps(self), crate::node::proof::children(*old(entry)), level as nat),
    ))]
    fn translate_entry(&self, entry: &mut PoolEntry, level: usize) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost row = crate::node::proof::children(*entry); }
        let mut bits = entry.mask.bits();
        #[cfg(verus_only)]
        proof! { proof::pending_initial(*entry, proof::maps(self), level as nat); }
        #[cfg_attr(verus_only, verus_spec(
            invariant level <= 10, proof::row_lookup(proof::maps(self), row, level as nat),
                proof::pending(row, crate::node::proof::children(*entry), proof::maps(self), level as nat, bits),
            decreases bits,
        ))]
        while bits != 0 {
            #[cfg(verus_only)]
            proof_decl! { let ghost before = *entry; let ghost old_bits = bits; }
            let slot = simd_ops::trailing_zeros(bits) as u8;
            bits &= bits - 1;
            #[cfg(verus_only)]
            proof! { proof::lowest_cleared(old_bits, bits, slot); }
            if let Child::Ptr(index) = entry.child(slot) {
                #[cfg(verus_only)]
                proof! { assert(crate::node::proof::children(before)[slot as int]==row[slot as int]); }
                let mapped = self.translate(level, Child::Ptr(index))?;
                entry.set_child(slot, mapped);
            }
            #[cfg(verus_only)]
            proof! { proof::pending_step(row, crate::node::proof::children(before), crate::node::proof::children(*entry), proof::maps(self), level as nat, old_bits, bits, slot); }
        }
        #[cfg(verus_only)]
        proof! { proof::pending_finished(row, crate::node::proof::children(*entry), proof::maps(self), level as nat); }
        Ok(())
    }
}
