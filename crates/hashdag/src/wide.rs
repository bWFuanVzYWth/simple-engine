#[cfg(not(verus_only))]
use crate::simd as simd_ops;
#[cfg(verus_only)]
use hash_dag_std::simd_ops;
use std::simd::Simd;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
#[path = "proof/wide.rs"]
pub(crate) mod proof;

use crate::hashdag::NODE_FANOUT;

#[cfg_attr(verus_only, verus_verify)]
const LANES_8: usize = 8;
type U32x8 = Simd<u32, LANES_8>;

#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires offset <= 56,
    ensures hash_dag_std::view32(result) == values@.subrange(offset as int, offset as int + 8),
    no_unwind
))]
const fn load8(values: &[u32; 64], offset: usize) -> U32x8 {
    let lanes = U32x8::from_array([
        values[offset],
        values[offset + 1],
        values[offset + 2],
        values[offset + 3],
        values[offset + 4],
        values[offset + 5],
        values[offset + 6],
        values[offset + 7],
    ]);
    #[cfg(verus_only)]
    proof! { assert(hash_dag_std::view32(lanes) =~= values@.subrange(offset as int, offset as int + 8)); }
    lanes
}

/// 完整比较原生 payload；四轮各处理两个 8-lane 向量，不提前结束。
#[must_use]
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => ensures result <==> a@ == b@, no_unwind))]
pub(crate) fn equal_groups64(a: &[u32; 64], b: &[u32; 64]) -> bool {
    let mut same = true;
    let mut part = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant part <= 4, same == proof::equal_range(a@, b@, 0, (part * 16) as int),
        decreases 4 - part,
    ))]
    while part < 4 {
        let offset = part * 16;
        let a0 = load8(a, offset);
        let b0 = load8(b, offset);
        let a1 = load8(a, offset + 8);
        let b1 = load8(b, offset + 8);
        let lower = simd_ops::equal8(a0, b0);
        let upper = simd_ops::equal8(a1, b1);
        #[cfg(verus_only)]
        proof! {
            proof::lane_equality(a@, b@, offset as int, hash_dag_std::view32(a0), hash_dag_std::view32(b0));
            proof::lane_equality(a@, b@, offset as int + 8, hash_dag_std::view32(a1), hash_dag_std::view32(b1));
            proof::equal_step(a@, b@, part as nat);
        }
        same &= lower;
        same &= upper;
        part += 1;
    }
    #[cfg(verus_only)]
    proof! { if same { assert(a@ =~= b@); } }
    same
}

#[must_use]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == if values@.len() == 64 { proof::uniform(values@) } else { None }, no_unwind
))]
pub fn fold_leaf_slice(values: &[u32]) -> Option<u32> {
    if values.len() != NODE_FANOUT {
        return None;
    }
    let first = values[0];
    if values[NODE_FANOUT - 1] != first {
        return None;
    }
    uniform_values(values)
}

#[must_use]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => ensures result == proof::uniform(values@), no_unwind))]
pub fn uniform_values(values: &[u32]) -> Option<u32> {
    if values.is_empty() {
        return None;
    }
    let first = values[0];
    let first_lanes = U32x8::splat(first);
    let (chunks, _) = values.as_chunks::<LANES_8>();
    let mut part = 0usize;
    #[cfg_attr(verus_only, verus_spec(
        invariant values@.len() > 0, first == values@[0], part <= chunks@.len(),
            chunks@.len() == values@.len() / 8,
            forall|i: int, j: int| 0 <= i < chunks@.len() && 0 <= j < 8 ==>
                #[trigger] chunks@[i]@[j] == values@[i * 8 + j],
            forall|i: int| 0 <= i < part * 8 ==> #[trigger] values@[i] == first,
            hash_dag_std::view32(first_lanes) == Seq::new(8, |i: int| first),
        decreases chunks@.len() - part,
    ))]
    while part < chunks.len() {
        let lanes = U32x8::from_array(chunks[part]);
        if !simd_ops::equal8(lanes, first_lanes) {
            #[cfg(verus_only)]
            proof! { proof::unequal_block(values@, chunks@, part as int); }
            return None;
        }
        #[cfg(verus_only)]
        proof! {
            assert(hash_dag_std::view32(lanes) == chunks@[part as int]@);
            assert forall|j: int| 0 <= j < 8 implies #[trigger] chunks@[part as int]@[j] == first by {
                assert(hash_dag_std::view32(lanes)[j] == hash_dag_std::view32(first_lanes)[j]);
            }
            proof::block_step(values@, chunks@, part as int);
        }
        part += 1;
    }
    let mut offset = part * LANES_8;
    #[cfg_attr(verus_only, verus_spec(
        invariant values@.len() > 0, first == values@[0], offset <= values@.len(),
            forall|i: int| 0 <= i < offset ==> #[trigger] values@[i] == first,
        decreases values@.len() - offset,
    ))]
    while offset < values.len() {
        if values[offset] != first {
            return None;
        }
        offset += 1;
    }
    Some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_values_accepts_full_node() {
        assert_eq!(uniform_values(&[5u32; NODE_FANOUT]), Some(5));
    }

    #[test]
    fn fold_leaf_slice_rejects_mixed_values() {
        let mut values = [1; NODE_FANOUT];
        values[3] = 2;

        assert_eq!(fold_leaf_slice(&values), None);
    }

    #[test]
    fn uniform_values_rejects_non_uniform_tail() {
        let mut values = [7u32; 17];
        values[16] = 8;

        assert_eq!(uniform_values(&values), None);
    }
}
