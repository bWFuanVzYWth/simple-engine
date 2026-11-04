//! 从生产存储导出的共享状态；高度只在擦除的规范和证明中递归。

use crate::node::{Child, NodeIdx};
use vstd::prelude::*;

verus! {

pub(crate) type NodeModel = Seq<Child>;
pub(crate) type Pools = Seq<PoolState>;

pub(crate) struct PoolState {
    pub(crate) nodes: Seq<Option<NodeModel>>,
    pub(crate) rcs: Seq<usize>,
    pub(crate) generations: Seq<u64>,
    pub(crate) next_generation: u64,
    pub(crate) constructing: bool,
    pub(crate) retained: Seq<NodeIdx>,
    pub(crate) free: Seq<NodeIdx>,
}

pub(crate) struct Address {
    pub(crate) level: nat,
    pub(crate) index: int,
}

pub(crate) open spec fn pools_view(pools: &[crate::pool::Pool; 10]) -> Pools {
    Seq::new(10, |level: int| pools@[level].state())
}

pub(crate) open spec fn well_formed(pools: &[crate::pool::Pool; 10]) -> bool {
    valid_pools(pools_view(pools))
        && forall|level: int| #![trigger pools@[level].valid()] 0 <= level < 10 ==> pools@[level].valid()
}

pub(crate) open spec fn occupied(pool: PoolState, index: int) -> bool {
    0 <= index < pool.nodes.len() && pool.nodes[index] is Some
}

pub(crate) open spec fn live(pool: PoolState, index: int) -> bool {
    occupied(pool, index) && index < pool.rcs.len() && pool.rcs[index] > 0
}

pub(crate) open spec fn valid_pool(pool: PoolState) -> bool {
    &&& pool.nodes.len() == pool.rcs.len() == pool.generations.len()
    &&& pool.nodes.len() <= u32::MAX as int + 1
    &&& pool.free.no_duplicates()
    &&& pool.retained.no_duplicates()
    &&& forall|i: int| #![trigger pool.free[i]] 0 <= i < pool.free.len() ==>
        0 <= crate::node::proof::idx(pool.free[i]) < pool.nodes.len()
    &&& forall|i: int| #![trigger pool.nodes[i]] 0 <= i < pool.nodes.len() ==>
        (pool.nodes[i] is None <==> pool.free.contains(crate::node::proof::at_index(i as u32)))
        && (pool.nodes[i] is None ==> pool.rcs[i] == 0)
        && pool.generations[i] <= pool.next_generation
        && (pool.nodes[i] is Some ==> pool.nodes[i]->Some_0.len() == 64 && 0 < pool.generations[i])
    &&& forall|i: int| #![auto] 0 <= i < pool.retained.len() ==>
        live(pool, crate::node::proof::idx(pool.retained[i]))
    &&& forall|i: int, j: int| #![auto]
        occupied(pool, i) && occupied(pool, j) && pool.nodes[i] == pool.nodes[j] ==> i == j
    &&& forall|i: int, j: int| #![auto]
        occupied(pool, i) && occupied(pool, j) && pool.generations[i] == pool.generations[j] ==> i == j
}

pub(crate) open spec fn child_valid(pools: Pools, child: Child, height: nat) -> bool {
    height <= pools.len() && match child {
        Child::Leaf(_) => true,
        Child::Ptr(index) => height > 0 && live(pools[height - 1], crate::node::proof::idx(index)),
    }
}

pub(crate) open spec fn uniform(node: NodeModel, value: u32) -> bool {
    node.len() == 64 && forall|slot: int| 0 <= slot < 64 ==> #[trigger] node[slot] == Child::Leaf(value)
}

pub(crate) open spec fn valid_pools(pools: Pools) -> bool {
    &&& pools.len() == 10
    &&& forall|level: int| #![auto] 0 <= level < pools.len() ==> valid_pool(pools[level])
    &&& forall|level: int, index: int| #![auto]
        0 <= level < pools.len() && occupied(pools[level], index) ==>
            (!(exists|value: u32| uniform(pools[level].nodes[index]->Some_0, value))
            && forall|slot: int| 0 <= slot < 64 ==>
                child_valid(pools, #[trigger] pools[level].nodes[index]->Some_0[slot], level as nat))
}

pub(crate) open spec fn span(height: nat) -> nat
    decreases height,
{
    if height == 0 { 1 } else { 64 * span((height - 1) as nat) }
}

pub(crate) open spec fn eval(pools: Pools, child: Child, height: nat, path: int) -> u32
    decreases height,
{
    match child {
        Child::Leaf(value) => value,
        Child::Ptr(index) => {
            let i = crate::node::proof::idx(index);
            if height > 0 && height <= pools.len() && occupied(pools[height - 1], i) {
                let row = pools[height - 1].nodes[i]->Some_0;
                let slot = path / (span((height - 1) as nat) as int) % 64;
                if 0 <= slot < row.len() {
                    eval(pools, row[slot], (height - 1) as nat, path % (span((height - 1) as nat) as int))
                } else { 0 }
            } else { 0 }
        },
    }
}

pub(crate) open spec fn same_value(pools: Pools, a: Child, b: Child, height: nat) -> bool {
    forall|path: int| 0 <= path < span(height) ==>
        #[trigger] eval(pools, a, height, path) == eval(pools, b, height, path)
}

pub(crate) open spec fn reachable(pools: Pools, child: Child, height: nat, address: Address) -> bool
    decreases height,
{
    match child {
        Child::Leaf(_) => false,
        Child::Ptr(index) => height > 0 && height <= pools.len()
            && occupied(pools[height - 1], crate::node::proof::idx(index))
            && (address.level == height - 1 && address.index == crate::node::proof::idx(index)
                || exists|slot: int| 0 <= slot < pools[height - 1].nodes[crate::node::proof::idx(index)]->Some_0.len()
                    && reachable(pools,
                        #[trigger] pools[height - 1].nodes[crate::node::proof::idx(index)]->Some_0[slot],
                        (height - 1) as nat, address)),
    }
}

pub(crate) open spec fn frozen(before: Pools, after: Pools) -> bool {
    before.len() == after.len()
        && forall|level: int, index: int| #![auto]
            0 <= level < before.len() && live(before[level], index) ==>
                live(after[level], index)
                && after[level].nodes[index] == before[level].nodes[index]
                && after[level].generations[index] == before[level].generations[index]
}

pub(crate) open spec fn occurrences<A>(items: Seq<A>, value: A) -> nat {
    items.filter(|item: A| item == value).len()
}

pub(crate) open spec fn parent_edges(pool: PoolState, child: Child, count: nat) -> nat
    decreases count,
{
    if count == 0 { 0 } else {
        let i = (count - 1) as int;
        parent_edges(pool, child, (count - 1) as nat)
            + if i < pool.nodes.len() && pool.nodes[i] is Some {
                occurrences(pool.nodes[i]->Some_0, child)
            } else { 0 }
    }
}

pub(crate) open spec fn balanced(pools: Pools, roots: Seq<Child>) -> bool {
    valid_pools(pools)
        && (forall|r: int| #![auto] 0 <= r < roots.len() ==> child_valid(pools, roots[r], 10))
        && forall|level: int, index: int| #![auto] 0 <= level < 10 && occupied(pools[level], index) ==>
            pools[level].rcs[index] ==
                (if level == 9 { occurrences(roots, Child::Ptr(crate::node::proof::at_index(index as u32))) } else { 0 })
                + (if level < 9 { parent_edges(pools[level + 1], Child::Ptr(crate::node::proof::at_index(index as u32)), pools[level + 1].nodes.len()) } else { 0 })
                + occurrences(pools[level].retained, crate::node::proof::at_index(index as u32))
}

pub(crate) open spec fn stable(pools: Pools, roots: Seq<Child>) -> bool {
    balanced(pools, roots)
        && forall|level: int| #![auto] 0 <= level < 10 ==>
            pools[level].retained.len() == 0 && !pools[level].constructing
            && forall|index: int| occupied(pools[level], index) ==> live(pools[level], index)
}

pub(crate) open spec fn all_occupied_live(pools: Pools) -> bool {
    forall|level: int, index: int| #![trigger occupied(pools[level], index)]
        0 <= level < pools.len() && occupied(pools[level], index) ==> live(pools[level], index)
}

pub(crate) open spec fn net_credit(pools: Pools, level: int, index: int) -> int {
    let child = Child::Ptr(crate::node::proof::at_index(index as u32));
    (if 0 <= level < pools.len() && occupied(pools[level], index) { pools[level].rcs[index] as int } else { 0 })
        - (if 0 <= level && level + 1 < pools.len() {
            parent_edges(pools[level + 1], child, pools[level + 1].nodes.len()) as int
        } else { 0 })
        - (if 0 <= level < pools.len() { occurrences(pools[level].retained, crate::node::proof::at_index(index as u32)) as int } else { 0 })
}

/// 构建只增加受保护节点；外部根持有由 RC 的整数余量隐式参数化。
pub(crate) open spec fn build_step(before: Pools, after: Pools) -> bool {
    &&& valid_pools(before) && valid_pools(after) && frozen(before, after) && all_occupied_live(after)
    &&& forall|level: int| #![auto] 0 <= level < 10 ==>
        before[level].nodes.len() <= after[level].nodes.len()
        && before[level].next_generation <= after[level].next_generation
        && before[level].constructing == after[level].constructing
        && before[level].retained.is_prefix_of(after[level].retained)
        && after[level].free.is_prefix_of(before[level].free)
    &&& forall|level: int, index: int| #![auto]
        0 <= level < 10 && 0 <= index < after[level].nodes.len() ==>
            net_credit(before, level, index) == net_credit(after, level, index)
}

}
