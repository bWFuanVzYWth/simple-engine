//! 冲突图的闭段掩码与最终交替可达标记；调用链的几何、CSR 和 HK 不变量另证。

use crate::index::{copy, get_mut};

#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
#[path = "proof/chord_graph.rs"]
pub(crate) mod proof;

/// 扫线产生的左、右顶点编号；按右顶点成组保存。
#[derive(Clone, Copy, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct SparseEdge {
    pub(crate) left: u16,
    pub(crate) right: u16,
}

/// 同一次扫描计算左度数与右组起点，再原位累加为 CSR offsets。
/// offsets 每项是左编号小于该槽号的源边数；right_starts 为对应右组的首条边。
/// 调用方提供已清零的 `左顶点数 + 1`、`右顶点数` 个槽，源边按右顶点成组。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires old(offsets)@.len() > 0, edges@.len() <= 3969,
        old(right_starts)@.len() <= 1953, proof::grouped_right(edges@),
        forall|i: int| 0 <= i < old(offsets)@.len() ==> old(offsets)@[i] == 0,
        forall|i: int| 0 <= i < old(right_starts)@.len() ==> old(right_starts)@[i] == 0,
        forall|i: int| 0 <= i < edges@.len() ==> (#[trigger] edges@[i]).left as int + 1 < old(offsets)@.len(),
        forall|i: int| 0 <= i < edges@.len() ==> (#[trigger] edges@[i]).right < old(right_starts)@.len(),
    ensures final(offsets)@.len() == old(offsets)@.len(),
        proof::offsets_match(edges@, final(offsets)@),
        proof::offsets_shape(edges@, final(offsets)@),
        proof::starts_match(edges@, final(right_starts)@, edges@.len() as int),
        final(right_starts)@.len() == old(right_starts)@.len(),
))]
pub(crate) fn build_csr_layout(
    edges: &[SparseEdge],
    offsets: &mut [u16],
    right_starts: &mut [u32],
) {
    let mut edge = 0;
    let mut previous_right = u16::MAX;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::counted_prefix(edges@, offsets@, edge as int),
            offsets@.len() == old(offsets)@.len(), edges@.len() <= 3969,
            right_starts@.len() == old(right_starts)@.len(), right_starts@.len() <= 1953,
            proof::starts_match(edges@, right_starts@, edge as int), proof::grouped_right(edges@),
            edge == 0 ==> previous_right == u16::MAX,
            edge > 0 ==> previous_right == edges@[edge as int - 1].right,
            forall|i: int| 0 <= i < edges@.len() ==> (#[trigger] edges@[i]).left as int + 1 < offsets@.len(),
            forall|i: int| 0 <= i < edges@.len() ==> (#[trigger] edges@[i]).right < right_starts@.len(),
        decreases edges@.len() - edge,
    ))]
    while edge < edges.len() {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = offsets@; let ghost old_starts = right_starts@; }
        let current = copy(edges, edge);
        let slot = usize::from(current.left) + 1;
        *get_mut(offsets, slot) += 1;
        let right = current.right;
        if right != previous_right {
            *get_mut(right_starts, usize::from(right)) = edge as u32;
            previous_right = right;
        }
        #[cfg(verus_only)]
        proof! {
            proof::count_step(edges@, before, offsets@, edge as int, slot as int);
            proof::starts_step(edges@, old_starts, right_starts@, edge as int);
        }
        edge += 1;
    }
    #[cfg(verus_only)]
    proof! { proof::prefix_start(edges@, offsets@); }
    let mut left = 1;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::prefix_loop(edges@, offsets@, left as int),
            offsets@.len() == old(offsets)@.len(), edges@.len() <= 3969,
        decreases offsets@.len() - left,
    ))]
    while left < offsets.len() {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = offsets@; }
        #[cfg(verus_only)]
        proof! { proof::count_bound(edges@, edges@.len() as int, left as int); }
        let previous = copy(offsets, left - 1);
        *get_mut(offsets, left) += previous;
        #[cfg(verus_only)]
        proof! { proof::prefix_step(edges@, before, offsets@, left as int); }
        left += 1;
    }
    #[cfg(verus_only)]
    proof! { proof::offsets_complete(edges@, offsets@); }
}

/// 将同一右顶点的边散布到各左 CSR 桶；游标只前进，既有前缀和其它桶不被覆盖。
/// 调用方须按各桶剩余空间预留该源组的度数，并建立完整组排列覆盖所有源边一次。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::scatter_inputs(edges@, right, offsets@, old(cursors)@, old(output)@.len()),
    ensures proof::scatter_state(edges@, right, offsets@, old(cursors)@, old(output)@,
        final(cursors)@, final(output)@, edges@.len() as int),
))]
pub(crate) fn scatter_group(
    edges: &[SparseEdge],
    right: u16,
    offsets: &[u16],
    cursors: &mut [u16],
    output: &mut [u16],
) {
    #[cfg(verus_only)]
    proof_decl! { let ghost initial = cursors@; let ghost original = output@; }
    let mut edge = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::scatter_inputs(edges@, right, offsets@, initial, original.len()),
            proof::scatter_state(edges@, right, offsets@, initial, original, cursors@, output@, edge as int),
        decreases edges@.len() - edge,
    ))]
    while edge < edges.len() {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = cursors@; let ghost previous = output@; }
        #[cfg(verus_only)]
        proof! { proof::scatter_available(edges@, right, offsets@, initial, original, cursors@, output@, edge as int); }
        let current = copy(edges, edge);
        let left = usize::from(current.left);
        let slot = get_mut(cursors, left);
        debug_assert_eq!(current.right, right);
        debug_assert!(usize::from(*slot) < usize::from(copy(offsets, left + 1)));
        *get_mut(output, usize::from(*slot)) = current.right;
        *slot += 1;
        #[cfg(verus_only)]
        proof! { proof::scatter_step(edges@, right, offsets@, initial, original, before, previous,
        cursors@, output@, edge as int); }
        edge += 1;
    }
}

/// 仅保留闭段 `[start,end]` 内的内部格点 `1..=63`；反向区间为空。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures proof::mask_matches(result, start, end),
))]
pub(crate) fn internal_mask(start: u8, end: u8) -> u64 {
    let first = if start < 1 { 1 } else { start };
    let last = if end > 63 { 63 } else { end };
    if first > last {
        #[cfg(verus_only)]
        proof! { proof::empty_mask(start, end); }
        return 0;
    }
    let start_mask = u64::MAX << first;
    let end_mask = if last >= 63 {
        u64::MAX
    } else {
        let high = 1u64 << (last + 1);
        #[cfg(verus_only)]
        proof! { proof::shift_positive((last + 1) as u8); }
        high - 1
    };
    let result = start_mask & end_mask;
    #[cfg(verus_only)]
    proof! { proof::mask_complete(result, start, end, first, last); }
    result
}

/// 将 HK 的最终反向距离转为左侧选中、右侧排除标记。
/// `left_complete` 表示左侧已全部匹配，此时输出独立集为右侧全体。
/// 否则左侧空闲点或其匹配右点不可达时选中；右点不可达时排除。
/// 调用方须建立匹配编号域、数组长度及最终 BFS 的可达语义；本核只证明逐项转换。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires old(selected_left)@.len() == pair_left@.len(),
        old(excluded_right)@.len() == right_distance@.len(),
        proof::pair_indices(pair_left@, right_distance@.len()),
    ensures proof::marks_match(pair_left@, right_distance@, left_complete,
        final(selected_left)@, final(excluded_right)@),
))]
pub(crate) fn mark_independent_set(
    pair_left: &[u16],
    right_distance: &[u16],
    left_complete: bool,
    selected_left: &mut [bool],
    excluded_right: &mut [bool],
) {
    let right_distance = right_distance.split_at(excluded_right.len()).0;
    let pair_left = pair_left.split_at(selected_left.len()).0;
    let mut right = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant excluded_right@.len() == right_distance@.len(),
            right <= excluded_right@.len(),
            forall|i: int| 0 <= i < right ==> #[trigger] excluded_right@[i] ==
                (!left_complete && right_distance@[i] == u16::MAX),
        decreases excluded_right@.len() - right,
    ))]
    while right < excluded_right.len() {
        excluded_right[right] = !left_complete && right_distance[right] == u16::MAX;
        right += 1;
    }
    let mut left = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant selected_left@.len() == pair_left@.len(),
            excluded_right@.len() == right_distance@.len(),
            left <= selected_left@.len(),
            proof::pair_indices(pair_left@, right_distance@.len()),
            forall|i: int| 0 <= i < excluded_right@.len() ==> #[trigger] excluded_right@[i] ==
                (!left_complete && right_distance@[i] == u16::MAX),
            forall|i: int| 0 <= i < left ==> #[trigger] selected_left@[i] ==
                (!left_complete && (pair_left@[i] == u16::MAX
                    || right_distance@[pair_left@[i] as int] == u16::MAX)),
        decreases selected_left@.len() - left,
    ))]
    while left < selected_left.len() {
        let matched = pair_left[left];
        selected_left[left] = !left_complete
            && (matched == u16::MAX || right_distance[usize::from(matched)] == u16::MAX);
        left += 1;
    }
}
