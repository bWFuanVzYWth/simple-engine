//! 两种真实输入策略的首个失败位置与确定错误；不规定两种策略之间的优先级。

use super::{QuadLeaf64, SparseQuadError, input_proof as input, proof as axis};
use vstd::prelude::*;

verus! {

pub(super) open spec fn raster_at(leaves: Seq<QuadLeaf64>, i: int, error: SparseQuadError) -> bool {
    0 <= i < leaves.len() && input::valid(leaves.take(i))
        && input::checked(leaves.take(i + 1), Err(error))
}

pub(super) open spec fn raster_error(leaves: Seq<QuadLeaf64>, error: SparseQuadError) -> bool {
    exists|i: int| #[trigger] raster_at(leaves, i, error)
}

pub(super) open spec fn seed_leaf(prefix: Seq<QuadLeaf64>, leaf: QuadLeaf64, error: SparseQuadError) -> bool {
    match error {
        SparseQuadError::LodOutOfRange | SparseQuadError::Misaligned | SparseQuadError::OutOfBounds =>
            axis::leaf_checked(leaf, Err(error)),
        SparseQuadError::Overlap => axis::leaf_valid(leaf)
            && exists|i: int| 0 <= i < prefix.len() && prefix[i].u == leaf.u && prefix[i].v == leaf.v,
        _ => false,
    }
}

pub(super) open spec fn seed_at(leaves: Seq<QuadLeaf64>, i: int, error: SparseQuadError) -> bool {
    0 <= i < leaves.len() && input::distinct_origins(leaves.take(i))
        && seed_leaf(leaves.take(i), leaves[i], error)
}

pub(super) open spec fn seed_error(leaves: Seq<QuadLeaf64>, error: SparseQuadError) -> bool {
    exists|i: int| #[trigger] seed_at(leaves, i, error)
}

pub(super) open spec fn failure_for(leaves: Seq<QuadLeaf64>) -> Option<SparseQuadError> {
    if leaves.len() >= super::dense::DENSE_LEAF_THRESHOLD {
        if exists|error: SparseQuadError| #[trigger] raster_error(leaves, error) {
            Some(choose|error: SparseQuadError| #[trigger] raster_error(leaves, error))
        } else { None }
    } else if exists|error: SparseQuadError| #[trigger] seed_error(leaves, error) {
        Some(choose|error: SparseQuadError| #[trigger] seed_error(leaves, error))
    } else if !input::valid(leaves) {
        Some(SparseQuadError::Overlap)
    } else { None }
}

pub(super) proof fn raster_failed(leaves: Seq<QuadLeaf64>, i: int, error: SparseQuadError)
    requires 0 <= i < leaves.len(), input::valid(leaves.take(i)),
        input::checked(leaves.take(i + 1), Err(error)),
    ensures raster_error(leaves, error),
{
    assert(raster_at(leaves, i, error));
}

pub(super) proof fn seed_failed(leaves: Seq<QuadLeaf64>, i: int, error: SparseQuadError)
    requires 0 <= i < leaves.len(), input::distinct_origins(leaves.take(i)),
        seed_leaf(leaves.take(i), leaves[i], error),
    ensures seed_error(leaves, error),
{
    assert(seed_at(leaves, i, error));
}

pub(super) proof fn origin_prefix(leaves: Seq<QuadLeaf64>, count: int)
    requires input::distinct_origins(leaves), 0 <= count <= leaves.len(),
    ensures input::distinct_origins(leaves.take(count)),
{
    assert forall|i: int| 0 <= i < count implies #[trigger] axis::leaf_valid(leaves.take(count)[i]) by {
        assert(leaves.take(count)[i] == leaves[i]);
    }
    assert forall|i: int, j: int| 0 <= i < j < count
        implies leaves.take(count)[i].u != leaves.take(count)[j].u
            || leaves.take(count)[i].v != leaves.take(count)[j].v by {
        assert(leaves.take(count)[i] == leaves[i] && leaves.take(count)[j] == leaves[j]);
    }
}

pub(super) proof fn valid_origins(leaves: Seq<QuadLeaf64>)
    requires input::valid(leaves),
    ensures input::distinct_origins(leaves),
{
    assert forall|i: int, j: int| 0 <= i < j < leaves.len()
        implies leaves[i].u != leaves[j].u || leaves[i].v != leaves[j].v by {
        if leaves[i].u == leaves[j].u && leaves[i].v == leaves[j].v {
            assert(axis::leaf_valid(leaves[i]) && axis::leaf_valid(leaves[j]));
            let p = input::origin(leaves[i]);
            assert(input::covers(leaves[i], p) && input::covers(leaves[j], p));
        }
    }
}

pub(super) proof fn leaf_error_unique(leaf: QuadLeaf64, first: SparseQuadError, second: SparseQuadError)
    requires axis::leaf_checked(leaf, Err(first)), axis::leaf_checked(leaf, Err(second)),
    ensures first == second, !axis::leaf_valid(leaf),
{
    match first {
        SparseQuadError::LodOutOfRange => {},
        SparseQuadError::Misaligned => {},
        SparseQuadError::OutOfBounds => {},
        _ => {},
    }
}

/// 首个坏前缀的形状错误只能来自新叶子；覆盖冲突保证新叶子形状合法。
pub(super) proof fn raster_leaf(leaves: Seq<QuadLeaf64>, i: int, error: SparseQuadError)
    requires raster_at(leaves, i, error),
    ensures match error {
        SparseQuadError::LodOutOfRange | SparseQuadError::Misaligned | SparseQuadError::OutOfBounds =>
            axis::leaf_checked(leaves[i], Err(error)),
        SparseQuadError::Overlap => axis::leaf_valid(leaves[i]),
        _ => false,
    },
{
    let before = leaves.take(i);
    let after = leaves.take(i + 1);
    match error {
        SparseQuadError::LodOutOfRange | SparseQuadError::Misaligned | SparseQuadError::OutOfBounds => {
            let j = choose|j: int| 0 <= j < after.len()
                && #[trigger] axis::leaf_checked(after[j], Err(error));
            if j < i {
                assert(after[j] == before[j]);
                assert(axis::leaf_valid(before[j]));
                leaf_error_unique(after[j], error, error);
            }
            assert(j == i && after[j] == leaves[i]);
        },
        SparseQuadError::Overlap => {
            let (a, b, p) = choose|a: int, b: int, p: int| 0 <= a < b < after.len()
                && #[trigger] input::covers(after[a], p) && #[trigger] input::covers(after[b], p);
            if b < i {
                assert(after[a] == before[a] && after[b] == before[b]);
            }
            assert(b == i && after[b] == leaves[i]);
        },
        _ => {},
    }
}

pub(super) proof fn raster_first(leaves: Seq<QuadLeaf64>, i: int, error: SparseQuadError, count: int)
    requires raster_at(leaves, i, error), i < count <= leaves.len(),
    ensures !input::valid(leaves.take(count)),
{
    input::checked_status(leaves.take(i + 1), Err(error));
    if input::valid(leaves.take(count)) {
        input::prefix_valid(leaves.take(count), i + 1);
        assert(leaves.take(count).take(i + 1) == leaves.take(i + 1));
    }
}

pub(super) proof fn raster_at_unique(leaves: Seq<QuadLeaf64>, i: int, first: SparseQuadError, j: int, second: SparseQuadError)
    requires raster_at(leaves, i, first), raster_at(leaves, j, second),
    ensures i == j, first == second,
{
    if i < j { raster_first(leaves, i, first, j); }
    if j < i { raster_first(leaves, j, second, i); }
    raster_leaf(leaves, i, first);
    raster_leaf(leaves, j, second);
    match first {
        SparseQuadError::Overlap => {
            match second {
                SparseQuadError::Overlap => {},
                _ => { leaf_error_unique(leaves[i], second, second); },
            }
        },
        _ => {
            match second {
                SparseQuadError::Overlap => { leaf_error_unique(leaves[i], first, first); },
                _ => { leaf_error_unique(leaves[i], first, second); },
            }
        },
    }
}

pub(super) proof fn raster_unique(leaves: Seq<QuadLeaf64>, first: SparseQuadError, second: SparseQuadError)
    requires raster_error(leaves, first), raster_error(leaves, second),
    ensures first == second,
{
    let i = choose|i: int| #[trigger] raster_at(leaves, i, first);
    let j = choose|j: int| #[trigger] raster_at(leaves, j, second);
    raster_at_unique(leaves, i, first, j, second);
}

pub(super) proof fn invalid_checked(leaves: Seq<QuadLeaf64>)
    requires !input::valid(leaves),
    ensures exists|error: SparseQuadError| #[trigger] input::checked(leaves, Err(error)),
{
    if forall|i: int| 0 <= i < leaves.len() ==> #[trigger] axis::leaf_valid(leaves[i]) {
        assert(input::checked(leaves, Err(SparseQuadError::Overlap)));
    } else {
        let i = choose|i: int| 0 <= i < leaves.len() && !#[trigger] axis::leaf_valid(leaves[i]);
        let leaf = leaves[i];
        let size = crate::geometry::side(leaf.lod);
        let error = if leaf.lod > 6 { SparseQuadError::LodOutOfRange }
            else if leaf.u % size != 0 || leaf.v % size != 0 { SparseQuadError::Misaligned }
            else { SparseQuadError::OutOfBounds };
        assert(axis::leaf_checked(leaves[i], Err(error)));
        assert(input::checked(leaves, Err(error)));
    }
}

/// 完整输入非法时，沿有限前缀必能到达首个坏点。
pub(super) proof fn raster_complete(leaves: Seq<QuadLeaf64>)
    requires !input::valid(leaves),
    ensures exists|error: SparseQuadError| #[trigger] raster_error(leaves, error),
    decreases leaves.len(),
{
    assert(leaves.len() > 0);
    let last = leaves.len() as int - 1;
    let prefix = leaves.take(last);
    if !input::valid(prefix) {
        raster_complete(prefix);
        let error = choose|error: SparseQuadError| #[trigger] raster_error(prefix, error);
        let i = choose|i: int| #[trigger] raster_at(prefix, i, error);
        assert(prefix.take(i) == leaves.take(i));
        assert(prefix.take(i + 1) == leaves.take(i + 1));
        raster_failed(leaves, i, error);
    } else {
        invalid_checked(leaves);
        let error = choose|error: SparseQuadError| #[trigger] input::checked(leaves, Err(error));
        assert(leaves.take(last + 1) == leaves);
        raster_failed(leaves, last, error);
    }
}

pub(super) proof fn seed_leaf_unique(prefix: Seq<QuadLeaf64>, leaf: QuadLeaf64, first: SparseQuadError, second: SparseQuadError)
    requires seed_leaf(prefix, leaf, first), seed_leaf(prefix, leaf, second),
    ensures first == second,
{
    match first {
        SparseQuadError::Overlap => {
            match second {
                SparseQuadError::Overlap => {},
                _ => { leaf_error_unique(leaf, second, second); },
            }
        },
        _ => {
            match second {
                SparseQuadError::Overlap => { leaf_error_unique(leaf, first, first); },
                _ => { leaf_error_unique(leaf, first, second); },
            }
        },
    }
}

pub(super) proof fn seed_first(leaves: Seq<QuadLeaf64>, i: int, error: SparseQuadError, count: int)
    requires seed_at(leaves, i, error), i < count <= leaves.len(),
    ensures !input::distinct_origins(leaves.take(count)),
{
    let prefix = leaves.take(count);
    if input::distinct_origins(prefix) {
        assert(prefix[i] == leaves[i]);
        assert(axis::leaf_valid(leaves[i]));
        match error {
            SparseQuadError::Overlap => {
                let j = choose|j: int| 0 <= j < leaves.take(i).len()
                    && leaves.take(i)[j].u == leaves[i].u && leaves.take(i)[j].v == leaves[i].v;
                assert(prefix[j] == leaves.take(i)[j]);
            },
            _ => { leaf_error_unique(leaves[i], error, error); },
        }
    }
}

pub(super) proof fn seed_at_unique(leaves: Seq<QuadLeaf64>, i: int, first: SparseQuadError, j: int, second: SparseQuadError)
    requires seed_at(leaves, i, first), seed_at(leaves, j, second),
    ensures i == j, first == second,
{
    if i < j { seed_first(leaves, i, first, j); }
    if j < i { seed_first(leaves, j, second, i); }
    seed_leaf_unique(leaves.take(i), leaves[i], first, second);
}

pub(super) proof fn seed_unique(leaves: Seq<QuadLeaf64>, first: SparseQuadError, second: SparseQuadError)
    requires seed_error(leaves, first), seed_error(leaves, second),
    ensures first == second,
{
    let i = choose|i: int| #[trigger] seed_at(leaves, i, first);
    let j = choose|j: int| #[trigger] seed_at(leaves, j, second);
    seed_at_unique(leaves, i, first, j, second);
}

pub(super) proof fn origins_no_seed(leaves: Seq<QuadLeaf64>)
    requires input::distinct_origins(leaves),
    ensures forall|error: SparseQuadError| !#[trigger] seed_error(leaves, error),
{
    assert forall|error: SparseQuadError| !#[trigger] seed_error(leaves, error) by {
        if seed_error(leaves, error) {
            let i = choose|i: int| #[trigger] seed_at(leaves, i, error);
            seed_first(leaves, i, error, leaves.len() as int);
            assert(leaves.take(leaves.len() as int) == leaves);
        }
    }
}

pub(super) proof fn seed_step(prefix: Seq<QuadLeaf64>, leaf: QuadLeaf64)
    requires input::distinct_origins(prefix), !input::distinct_origins(prefix.push(leaf)),
    ensures exists|error: SparseQuadError| #[trigger] seed_leaf(prefix, leaf, error),
{
    if !axis::leaf_valid(leaf) {
        let size = crate::geometry::side(leaf.lod);
        let error = if leaf.lod > 6 { SparseQuadError::LodOutOfRange }
            else if leaf.u % size != 0 || leaf.v % size != 0 { SparseQuadError::Misaligned }
            else { SparseQuadError::OutOfBounds };
        assert(seed_leaf(prefix, leaf, error));
    } else {
        let after = prefix.push(leaf);
        assert forall|i: int| 0 <= i < after.len() implies #[trigger] axis::leaf_valid(after[i]) by {
            if i < prefix.len() { assert(after[i] == prefix[i]); }
            else { assert(after[i] == leaf); }
        }
        let (i, j) = choose|i: int, j: int| 0 <= i < j < after.len()
            && after[i].u == after[j].u && after[i].v == after[j].v;
        if j < prefix.len() { assert(after[i] == prefix[i] && after[j] == prefix[j]); }
        assert(j == prefix.len() && after[j] == leaf && after[i] == prefix[i]);
        assert(seed_leaf(prefix, leaf, SparseQuadError::Overlap));
    }
}

pub(super) proof fn seed_complete(leaves: Seq<QuadLeaf64>)
    requires !input::distinct_origins(leaves),
    ensures exists|error: SparseQuadError| #[trigger] seed_error(leaves, error),
    decreases leaves.len(),
{
    assert(leaves.len() > 0);
    let last = leaves.len() as int - 1;
    let prefix = leaves.take(last);
    if !input::distinct_origins(prefix) {
        seed_complete(prefix);
        let error = choose|error: SparseQuadError| #[trigger] seed_error(prefix, error);
        let i = choose|i: int| #[trigger] seed_at(prefix, i, error);
        assert(prefix.take(i) == leaves.take(i));
        assert(prefix[i] == leaves[i]);
        seed_failed(leaves, i, error);
    } else {
        input::prefix_step(leaves, last);
        assert(leaves.take(last + 1) == leaves);
        seed_step(prefix, leaves[last]);
        let error = choose|error: SparseQuadError| #[trigger] seed_leaf(prefix, leaves[last], error);
        seed_failed(leaves, last, error);
    }
}

pub(super) proof fn valid_result(leaves: Seq<QuadLeaf64>)
    requires input::valid(leaves),
    ensures failure_for(leaves).is_none(),
{
    valid_origins(leaves);
    origins_no_seed(leaves);
    assert forall|error: SparseQuadError| !#[trigger] raster_error(leaves, error) by {
        if raster_error(leaves, error) {
            let i = choose|i: int| #[trigger] raster_at(leaves, i, error);
            raster_first(leaves, i, error, leaves.len() as int);
            assert(leaves.take(leaves.len() as int) == leaves);
        }
    }
}

pub(super) proof fn raster_result(leaves: Seq<QuadLeaf64>, error: SparseQuadError)
    requires leaves.len() >= super::dense::DENSE_LEAF_THRESHOLD, raster_error(leaves, error),
    ensures failure_for(leaves) == Some(error),
{
    let selected = choose|selected: SparseQuadError| #[trigger] raster_error(leaves, selected);
    raster_unique(leaves, selected, error);
}

pub(super) proof fn seed_result(leaves: Seq<QuadLeaf64>, error: SparseQuadError)
    requires leaves.len() < super::dense::DENSE_LEAF_THRESHOLD, seed_error(leaves, error),
    ensures failure_for(leaves) == Some(error),
{
    let selected = choose|selected: SparseQuadError| #[trigger] seed_error(leaves, selected);
    seed_unique(leaves, selected, error);
}

pub(super) proof fn sweep_result(leaves: Seq<QuadLeaf64>)
    requires leaves.len() < super::dense::DENSE_LEAF_THRESHOLD,
        input::distinct_origins(leaves), !input::valid(leaves),
    ensures failure_for(leaves) == Some(SparseQuadError::Overlap),
{
    origins_no_seed(leaves);
}

pub(super) proof fn failure_status(leaves: Seq<QuadLeaf64>)
    ensures failure_for(leaves).is_none() == input::valid(leaves),
{
    if input::valid(leaves) { valid_result(leaves); }
    else if leaves.len() >= super::dense::DENSE_LEAF_THRESHOLD {
        raster_complete(leaves);
    }
}

}
