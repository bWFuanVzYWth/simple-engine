//! 稳定所有者到实际差分读取视图的连接，不增加执行期快照。
use super::*;
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

proof fn layer(dag: &HashDag, level: int)
    requires dag.valid(), 0 <= level < 10,
    ensures dag@.nodes[level] == crate::state::pools_view(&dag.pools)[level].nodes,
        dag@.nodes[level].len() <= usize::MAX,
{
    let pool = &dag.pools@[level];
    let state = crate::state::pools_view(&dag.pools)[level];
    assert(pool.valid());
    pool.read_projection();
    assert forall|i: int| #![trigger pool.read_nodes()[i]] 0 <= i < pool.read_nodes().len()
        implies pool.read_nodes()[i] == state.nodes[i] by {
        pool.read_node_at(i);
        pool.read_model(i);
        if crate::state::occupied(state, i) {
            assert(crate::state::live(state, i));
        }
    }
    assert(pool.read_nodes() =~= state.nodes);
}

pub(crate) proof fn valid(dag: &HashDag)
    requires dag.valid(),
    ensures crate::diff::proof::model::valid(dag@),
{
    let pools = crate::state::pools_view(&dag.pools);
    assert forall|l: int| 0 <= l < 10 implies
        dag@.nodes[l] == pools[l].nodes && dag@.nodes[l].len() <= u32::MAX as int + 1
            && dag@.nodes[l].len() <= usize::MAX by { layer(dag, l); }
    assert forall|a: crate::diff::NodeAddress| crate::diff::proof::model::has_node(dag@, a) implies {
        let row = dag@.nodes[a.level as int][crate::node::proof::idx(a.index)]->Some_0;
        row.len() == 64 && forall|slot: int| 0 <= slot < 64 && row[slot] is Ptr ==>
            a.level > 0 && crate::diff::proof::model::has_node(dag@, crate::diff::NodeAddress {
                level: (a.level - 1) as u8, index: crate::diff::proof::model::pointer(row[slot]) })
    } by {
        let l = a.level as int;
        let i = crate::node::proof::idx(a.index);
        assert(crate::state::occupied(pools[l], i));
        assert(pools[l].nodes[i]->Some_0.len() == 64);
        assert forall|slot: int| 0 <= slot < 64 && pools[l].nodes[i]->Some_0[slot] is Ptr implies
            a.level > 0 && crate::diff::proof::model::has_node(dag@, crate::diff::NodeAddress {
                level: (a.level - 1) as u8,
                index: crate::diff::proof::model::pointer(pools[l].nodes[i]->Some_0[slot]) }) by {
            let child = pools[l].nodes[i]->Some_0[slot];
            assert(crate::state::child_valid(pools, child, l as nat));
        }
    }
    assert forall|root: RootId| dag.roots.lookup(root) is Some implies
        crate::diff::proof::model::root_valid(dag@, dag.roots.lookup(root)->Some_0) by {
        let child = dag.roots.lookup(root)->Some_0;
        proof::active_contains(&dag.roots, root, child);
        let i = choose|i: int| 0 <= i < dag.root_children().len() && dag.root_children()[i] == child;
        assert(crate::state::child_valid(pools, dag.root_children()[i], 10));
    }
}

pub(crate) proof fn root_valid(dag: &HashDag, root: RootId, child: Child)
    requires dag.roots.valid(), crate::state::balanced(crate::state::pools_view(&dag.pools), dag.root_children()),
        dag.roots.lookup(root) == Some(child),
    ensures crate::state::child_valid(crate::state::pools_view(&dag.pools), child, 10),
{
    proof::active_contains(&dag.roots, root, child);
    let i = choose|i: int| 0 <= i < dag.root_children().len() && dag.root_children()[i] == child;
    assert(crate::state::child_valid(crate::state::pools_view(&dag.pools), dag.root_children()[i], 10));
}
}
