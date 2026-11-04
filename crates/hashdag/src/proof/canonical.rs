//! 规范化物理表示的语义完备性；由存储不变量推出，不将唯一内容身份设为假设。
use crate::{node::Child, state::*};
use vstd::{arithmetic::div_mod::*, prelude::*};

verus! {

pub(crate) proof fn span_positive(height: nat)
    ensures span(height) > 0,
    decreases height,
{
    if height > 0 { span_positive((height - 1) as nat); }
}

pub(crate) open spec fn child(pools: Pools, value: Child, height: nat, slot: int) -> Child {
    match value {
        Child::Leaf(v) => Child::Leaf(v),
        Child::Ptr(index) => pools[height - 1].nodes[crate::node::proof::idx(index)]->Some_0[slot],
    }
}

proof fn combined_bounds(width: int, slot: int, path: int)
    requires width > 0, 0 <= slot < 64, 0 <= path < width,
    ensures 0 <= slot * width + path < 64 * width,
{
    assert(0 <= slot * width + path < 64 * width) by(nonlinear_arith)
        requires width > 0, 0 <= slot < 64, 0 <= path < width;
}

pub(crate) proof fn descend(pools: Pools, value: Child, height: nat, slot: int, path: int)
    requires valid_pools(pools), child_valid(pools, value, height), 0 < height <= 10,
        0 <= slot < 64, 0 <= path < span((height - 1) as nat),
    ensures child_valid(pools, child(pools, value, height, slot), (height - 1) as nat),
        0 <= slot * (span((height - 1) as nat) as int) + path < span(height),
        eval(pools, value, height, slot * (span((height - 1) as nat) as int) + path)
            == eval(pools, child(pools, value, height, slot), (height - 1) as nat, path),
{
    span_positive((height - 1) as nat);
    let width = span((height - 1) as nat) as int;
    let combined = slot * width + path;
    combined_bounds(width, slot, path);
    lemma_fundamental_div_mod_converse(combined, width, slot, path);
    lemma_small_mod(slot as nat, 64);
}

pub(crate) proof fn descend_equal(pools: Pools, a: Child, b: Child, height: nat, slot: int)
    requires valid_pools(pools), child_valid(pools, a, height), child_valid(pools, b, height),
        0 < height <= 10, 0 <= slot < 64, same_value(pools, a, b, height),
    ensures same_value(pools, child(pools, a, height, slot), child(pools, b, height, slot), (height - 1) as nat),
{
    assert forall|path: int| #![trigger eval(pools, child(pools, a, height, slot), (height - 1) as nat, path)]
        0 <= path < span((height - 1) as nat) implies
        eval(pools, child(pools, a, height, slot), (height - 1) as nat, path)
            == eval(pools, child(pools, b, height, slot), (height - 1) as nat, path) by {
        descend(pools, a, height, slot, path);
        descend(pools, b, height, slot, path);
    }
}

pub(crate) proof fn equivalent_iff_identical(pools: Pools, a: Child, b: Child, height: nat)
    requires valid_pools(pools), child_valid(pools, a, height), child_valid(pools, b, height), height <= 10,
    ensures same_value(pools, a, b, height) <==> a == b,
    decreases height,
{
    span_positive(height);
    if same_value(pools, a, b, height) {
        if a is Leaf && b is Leaf {
            assert(eval(pools, a, height, 0) == eval(pools, b, height, 0));
        } else {
            assert(height > 0);
            assert forall|slot: int| #![trigger child(pools, a, height, slot)] 0 <= slot < 64 implies
                child(pools, a, height, slot) == child(pools, b, height, slot) by {
                span_positive((height - 1) as nat);
                descend(pools, a, height, slot, 0);
                descend(pools, b, height, slot, 0);
                descend_equal(pools, a, b, height, slot);
                equivalent_iff_identical(pools, child(pools, a, height, slot), child(pools, b, height, slot), (height - 1) as nat);
            }
            match (a, b) {
                (Child::Ptr(ai), Child::Ptr(bi)) => {
                    let x = crate::node::proof::idx(ai);
                    let y = crate::node::proof::idx(bi);
                    assert(valid_pool(pools[height - 1]));
                    assert(occupied(pools[height - 1], x) && occupied(pools[height - 1], y));
                    assert forall|slot: int| #![trigger pools[height - 1].nodes[x]->Some_0[slot]] 0 <= slot < 64 implies
                        pools[height - 1].nodes[x]->Some_0[slot] == pools[height - 1].nodes[y]->Some_0[slot] by {
                        assert(child(pools, a, height, slot) == child(pools, b, height, slot));
                    }
                    assert(pools[height - 1].nodes[x]->Some_0 =~= pools[height - 1].nodes[y]->Some_0);
                    assert(x == y);
                },
                (Child::Leaf(value), Child::Ptr(index)) => {
                    let row = pools[height - 1].nodes[crate::node::proof::idx(index)]->Some_0;
                    assert(valid_pool(pools[height - 1]));
                    assert(occupied(pools[height - 1], crate::node::proof::idx(index)));
                    assert forall|slot: int| #![trigger row[slot]] 0 <= slot < 64 implies row[slot] == Child::Leaf(value) by {
                        assert(child(pools, a, height, slot) == child(pools, b, height, slot));
                    }
                    assert(uniform(pools[height - 1].nodes[crate::node::proof::idx(index)]->Some_0, value));
                    assert(false);
                },
                (Child::Ptr(index), Child::Leaf(value)) => {
                    let row = pools[height - 1].nodes[crate::node::proof::idx(index)]->Some_0;
                    assert(valid_pool(pools[height - 1]));
                    assert(occupied(pools[height - 1], crate::node::proof::idx(index)));
                    assert forall|slot: int| #![trigger row[slot]] 0 <= slot < 64 implies row[slot] == Child::Leaf(value) by {
                        assert(child(pools, a, height, slot) == child(pools, b, height, slot));
                    }
                    assert(uniform(pools[height - 1].nodes[crate::node::proof::idx(index)]->Some_0, value));
                    assert(false);
                },
                _ => {},
            }
        }
    }
}

pub(crate) proof fn frozen_value(before: Pools, after: Pools, value: Child, height: nat, path: int)
    requires valid_pools(before), valid_pools(after), frozen(before, after),
        child_valid(before, value, height), height <= 10,
    ensures child_valid(after, value, height), eval(before, value, height, path) == eval(after, value, height, path),
    decreases height,
{
    match value {
        Child::Leaf(_) => {},
        Child::Ptr(index) => {
            assert(height > 0);
            let i = crate::node::proof::idx(index);
            assert(live(before[height - 1], i));
            assert(live(after[height - 1], i));
            assert(before[height - 1].nodes[i] == after[height - 1].nodes[i]);
            let row = before[height - 1].nodes[i]->Some_0;
            assert(valid_pool(before[height - 1]));
            assert(row.len() == 64);
            span_positive((height - 1) as nat);
            let width = span((height - 1) as nat) as int;
            let slot = path / width % 64;
            lemma_mod_bound(path / width, 64);
            assert(child_valid(before, row[slot], (height - 1) as nat));
            frozen_value(before, after, row[slot], (height - 1) as nat, path % width);
        },
    }
}

}
