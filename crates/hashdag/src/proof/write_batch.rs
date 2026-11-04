//! 批次的输入边界和逐体素覆盖语义；只存在于擦除的规范。
use crate::{Child, Error, HashDag, RootId, state};
use vstd::prelude::*;

verus! {
pub(crate) trait BatchModel {
    spec fn accepted(&self) -> bool;
    spec fn effect(&self, initial: spec_fn(int) -> u32, path: int) -> u32;
}

pub(crate) open spec fn root_value(dag: &HashDag, root: RootId, path: int) -> u32 {
    match dag.roots.lookup(root) {
        Some(child) => state::eval(state::pools_view(&dag.pools), child, 10, path),
        None => 0,
    }
}

pub open(crate) spec fn input_written(before: &HashDag, after: &HashDag, root: RootId, accepted: bool,
    effect: spec_fn(spec_fn(int) -> u32, int) -> u32, result: Result<(), Error>) -> bool {
    &&& after.valid()
    &&& (before.identity_owned() ==> after.identity_owned())
    &&& crate::balance::no_orphans(state::pools_view(&after.pools), after.root_children())
    &&& after.roots.owner == before.roots.owner
    &&& (result is Err ==> after.roots.same_entries(&before.roots))
    &&& (result is Ok ==> before.roots.lookup(root) is Some && accepted
        && after.roots.lookup(root) is Some
        && forall|path: int| 0 <= path < state::span(10) ==>
            #[trigger] root_value(after, root, path) == effect(|p: int| root_value(before, root, p), path))
    &&& (forall|key: RootId| key != root || result is Err ==>
        after.roots.lookup(key) == before.roots.lookup(key))
    &&& (forall|key: RootId, path: int| before.roots.lookup(key) is Some && (key != root || result is Err)
        && 0 <= path < state::span(10) ==>
            #[trigger] root_value(after, key, path) == root_value(before, key, path))
}

pub(crate) open spec fn written<B: BatchModel>(before: &HashDag, after: &HashDag, root: RootId, batch: B, result: Result<(), Error>) -> bool {
    input_written(before, after, root, batch.accepted(), |initial: spec_fn(int) -> u32, path: int| batch.effect(initial, path), result)
}

proof fn phase_value(before: state::Pools, after: state::Pools, child: Child, height: nat, path: int)
    requires state::valid_pools(before), state::child_valid(before, child, height),
        crate::layers::proof::phase(before, after, true),
    ensures state::eval(before, child, height, path) == state::eval(after, child, height, path),
    decreases height,
{
    if let Child::Ptr(index) = child {
        let i = crate::node::proof::idx(index);
        let row = before[height - 1].nodes[i]->Some_0;
        let slot = (path / state::span((height - 1) as nat) as int) % 64;
        if 0 <= slot < row.len() {
            phase_value(before, after, row[slot], (height - 1) as nat, path % state::span((height - 1) as nat) as int);
        }
    }
}

pub(crate) proof fn finish<B: BatchModel>(before: &HashDag, phase: state::Pools, built: &HashDag, published: &HashDag,
    after: &HashDag, root: RootId, old_root: Child, batch: B, candidate: Result<Child, Error>, result: Result<(), Error>)
    requires before.valid(), before.roots.lookup(root) == Some(old_root), batch.accepted(),
        crate::layers::proof::phase(state::pools_view(&before.pools), phase, true),
        state::build_step(phase, state::pools_view(&built.pools)), built.roots.same_entries(&before.roots),
        candidate is Ok ==> state::child_valid(state::pools_view(&built.pools), candidate->Ok_0, 10)
            && forall|path: int| 0 <= path < state::span(10) ==>
                #[trigger] state::eval(state::pools_view(&built.pools), candidate->Ok_0, 10, path)
                    == batch.effect(|p: int| state::eval(phase, old_root, 10, p), path),
        candidate is Ok ==> published.publish_result(built, root, old_root, candidate->Ok_0, result),
        candidate is Err ==> result is Err && published.roots.same_entries(&built.roots)
            && state::pools_view(&published.pools) == state::pools_view(&built.pools),
        after.valid(), after.roots.same_entries(&published.roots),
        before.identity_owned() ==> after.identity_owned(),
        crate::balance::no_orphans(state::pools_view(&after.pools), after.root_children()),
        state::frozen(state::pools_view(&after.pools), state::pools_view(&published.pools)),
    ensures written(before, after, root, batch, result),
{
    let b = state::pools_view(&before.pools);
    let a = state::pools_view(&after.pools);
    let built_pools = state::pools_view(&built.pools);
    crate::hashdag::view_proof::root_valid(before, root, old_root);
    crate::layers::proof::valid_phase(b, phase, true);
    crate::balance::frozen_transitive(b, phase, built_pools);
    crate::balance::frozen_transitive(a, state::pools_view(&published.pools), built_pools);
    assert forall|path: int| state::eval(phase, old_root, 10, path) == root_value(before, root, path) by {
        phase_value(b, phase, old_root, 10, path);
    }
    assert((|p: int| state::eval(phase, old_root, 10, p)) =~= (|p: int| root_value(before, root, p)));
    assert forall|key: RootId, path: int| before.roots.lookup(key) is Some && (key != root || result is Err)
        && 0 <= path < state::span(10) implies root_value(after, key, path) == root_value(before, key, path) by {
        let child = before.roots.lookup(key)->Some_0;
        crate::hashdag::view_proof::root_valid(before, key, child);
        crate::hashdag::view_proof::root_valid(after, key, child);
        crate::canonical::frozen_value(b, built_pools, child, 10, path);
        crate::canonical::frozen_value(a, built_pools, child, 10, path);
    }
    if result is Ok {
        let child = candidate->Ok_0;
        assert(after.roots.lookup(root) == Some(child));
        crate::hashdag::view_proof::root_valid(after, root, child);
        assert forall|path: int| 0 <= path < state::span(10) implies
            root_value(after, root, path) == batch.effect(|p: int| root_value(before, root, p), path) by {
            crate::canonical::frozen_value(a, built_pools, child, 10, path);
        }
    }
}
}
