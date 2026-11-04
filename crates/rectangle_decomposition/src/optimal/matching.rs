//! 稀疏 chord 匹配后端。
//!
//! 对全部水平/垂直 chord 构建二分冲突图 CSR，再用
//! HKDW 的分层及追加增广阶段求最大独立集。

use crate::optimal::fixed::FixedVec;
use crate::optimal::types::ValuedChord;

use crate::optimal::copy;
#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "../proof/matching.rs"]
pub(super) mod proof;

#[cfg_attr(verus_only, verus_verify)]
pub const IMAGE64_AXIS_LIMIT: usize = 64;
#[cfg_attr(verus_only, verus_verify)]
pub const IMAGE64_AXIS_LEN: usize = IMAGE64_AXIS_LIMIT + 1;
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::nonlinear)]
pub const IMAGE64_MAX_CONFLICT_EDGES: usize = (IMAGE64_AXIS_LIMIT - 1) * (IMAGE64_AXIS_LIMIT - 1);
/// 每条内部格线至多有 floor(63 / 2) 条端点互异的同向 chord。
/// 验证义务：每条 chord 占用至少两个互异内部端点，同线闭段互不相交，故每向至多 1953 条。
#[cfg_attr(verus_only, verus_verify)]
pub const IMAGE64_MAX_CHORDS: usize = (IMAGE64_AXIS_LIMIT - 1) * ((IMAGE64_AXIS_LIMIT - 1) / 2);
// 图的固定容量同时约束 CSR 末尾 offset、扫描游标和 DFS 顶点编号。
const _: () = assert!(IMAGE64_MAX_CONFLICT_EDGES < 65_535);
const _: () = assert!(IMAGE64_MAX_CHORDS < 65_535);
pub type ChordBuffer<T> = FixedVec<T, IMAGE64_MAX_CHORDS>;

pub use crate::chord_graph::SparseEdge;

#[derive(Clone, Copy, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct SparseAdjacencyRef<'a> {
    pub(super) offsets: &'a [u16],
    pub(super) edges: &'a [u16],
}

#[derive(Clone, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct ConflictFinalizeScratch {
    /// 构图时暂存横向起止事件的 next 和纵向查询的 next；CSR 阶段复用为
    /// 右顶点的边段起点及按 (度数, 编号) 排序的右顶点。
    pub(super) right_layout: FixedVec<u32, { 2 * IMAGE64_MAX_CHORDS }>,
    pub(super) next_offsets: ChordBuffer<u16>,
    pub(super) edge_buffer: [SparseEdge; IMAGE64_MAX_CONFLICT_EDGES],
    pub(super) edge_count: usize,
    pub(super) right_degrees: ChordBuffer<u8>,
    pub(super) adjacency_offsets: FixedVec<u16, { IMAGE64_MAX_CHORDS + 1 }>,
    pub(super) adjacency_edges: FixedVec<u16, IMAGE64_MAX_CONFLICT_EDGES>,
    pub(super) horizontal_start_heads: [u16; IMAGE64_AXIS_LEN],
    pub(super) horizontal_end_heads: [u16; IMAGE64_AXIS_LEN],
    pub(super) vertical_query_heads: [u16; IMAGE64_AXIS_LEN],
    /// 活跃 y 位由扫线的局部 u64 保存，只读取活跃行的水平 chord 编号。
    pub(super) active_horizontal: [u16; IMAGE64_AXIS_LEN],
}

impl Default for ConflictFinalizeScratch {
    fn default() -> Self {
        const { Self::new() }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl ConflictFinalizeScratch {
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result.edge_count == 0,
            result.right_degrees.valid(), result.right_degrees@ == Seq::<u8>::empty(),
            result.adjacency_offsets.valid(), result.adjacency_offsets@ == Seq::<u16>::empty(),
            result.adjacency_edges.valid(), result.adjacency_edges@ == Seq::<u16>::empty(),
            result.right_layout.valid(), result.right_layout@ == Seq::<u32>::empty(),
            result.next_offsets.valid(), result.next_offsets@ == Seq::<u16>::empty(),
    ))]
    pub(super) const fn new() -> Self {
        Self {
            right_layout: FixedVec::new(0),
            next_offsets: FixedVec::new(0),
            edge_buffer: [SparseEdge { left: 0, right: 0 }; IMAGE64_MAX_CONFLICT_EDGES],
            edge_count: 0,
            right_degrees: FixedVec::new(0),
            adjacency_offsets: FixedVec::new(0),
            adjacency_edges: FixedVec::new(0),
            // 事件头在每轮构图时重置；仅通过活跃位读取 owner。
            // 全零初值使整个栈 scratch 可由一次 memset 初始化。
            horizontal_start_heads: [0; IMAGE64_AXIS_LEN],
            horizontal_end_heads: [0; IMAGE64_AXIS_LEN],
            vertical_query_heads: [0; IMAGE64_AXIS_LEN],
            active_horizontal: [0; IMAGE64_AXIS_LEN],
        }
    }
}

#[derive(Clone, Debug, Default)]
#[cfg_attr(verus_only, verus_verify)]
pub struct ConflictScratch {
    pub(super) finalize: ConflictFinalizeScratch,
}

/// Hopcroft-Karp 阶段的复用 scratch。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub(super) struct DfsFrame {
    pub(super) left: u16,
    pub(super) next_edge: u16,
}

const _: () = assert!(core::mem::size_of::<DfsFrame>() == 4);

/// Hopcroft-Karp 阶段的复用 scratch。
#[derive(Clone, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct HkScratch {
    pub(super) pair_left: ChordBuffer<u16>,
    pub(super) pair_right: ChordBuffer<u16>,
    pub(super) right_distance: ChordBuffer<u16>,
    pub(super) queue: ChordBuffer<u16>,
    pub(super) unmatched_lefts: ChordBuffer<u16>,
    /// 最短层上的空闲左顶点，可重复；长度至多为冲突边数。
    pub(super) shortest_roots: FixedVec<u16, IMAGE64_MAX_CONFLICT_EDGES>,
    /// 搜索时保存本轮访问标记，结束后编码左侧选中、右侧排除状态。
    pub(super) reachable_left: ChordBuffer<bool>,
    pub(super) reachable_right: ChordBuffer<bool>,
    pub(super) transpose_offsets: FixedVec<u16, { IMAGE64_MAX_CHORDS + 1 }>,
    pub(super) transpose_edges: FixedVec<u16, IMAGE64_MAX_CONFLICT_EDGES>,
    /// 转置 CSR 写入游标；构造完成后复用为长度 3 增广的左邻接扫描游标。
    pub(super) write_offsets: ChordBuffer<u16>,
    pub(super) right_order: ChordBuffer<u16>,
    pub(super) right_degree_counts: FixedVec<usize, IMAGE64_AXIS_LIMIT>,
    pub(super) dfs_stack: ChordBuffer<DfsFrame>,
}

#[cfg_attr(verus_only, verus_verify)]
impl HkScratch {
    #[cfg_attr(verus_only, verus_spec(result => ensures proof::buffers_valid(&result)))]
    pub(super) const fn new() -> Self {
        Self {
            pair_left: FixedVec::new(0),
            pair_right: FixedVec::new(0),
            right_distance: FixedVec::new(0),
            queue: FixedVec::new(0),
            unmatched_lefts: FixedVec::new(0),
            shortest_roots: FixedVec::new(0),
            reachable_left: FixedVec::new(false),
            reachable_right: FixedVec::new(false),
            transpose_offsets: FixedVec::new(0),
            transpose_edges: FixedVec::new(0),
            write_offsets: FixedVec::new(0),
            right_order: FixedVec::new(0),
            right_degree_counts: FixedVec::new(0),
            dfs_stack: FixedVec::new(DfsFrame {
                left: 0,
                next_edge: 0,
            }),
        }
    }
}

impl Default for HkScratch {
    fn default() -> Self {
        const { Self::new() }
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub struct MatchingScratch {
    pub(super) hk: HkScratch,
    pub(super) conflict: ConflictScratch,
    selected_horizontal: ChordBuffer<u16>,
    selected_vertical: ChordBuffer<u16>,
}

impl Default for MatchingScratch {
    fn default() -> Self {
        const { Self::new() }
    }
}

#[cfg_attr(verus_only, verus_verify)]
pub const UNMATCHED_U16: u16 = u16::MAX;

/// 连接一条边的两个空闲端点；其它配对逐项保留。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::matching_valid(_adjacency.offsets@, _adjacency.edges@, old(pair_left)@, old(pair_right)@),
        left < old(pair_left)@.len(), right < old(pair_right)@.len(),
        old(pair_left)@[left as int] == u16::MAX, old(pair_right)@[right as int] == u16::MAX,
        proof::edge_in(_adjacency.offsets@, _adjacency.edges@, left as int, right as int),
    ensures proof::matching_valid(_adjacency.offsets@, _adjacency.edges@, final(pair_left)@, final(pair_right)@),
        proof::matched_preserved(old(pair_left)@, final(pair_left)@),
        proof::matched_preserved(old(pair_right)@, final(pair_right)@),
        final(pair_left)@ == old(pair_left)@.update(left as int, right as u16),
        final(pair_right)@ == old(pair_right)@.update(right as int, left as u16),
        proof::matched_count(final(pair_left)@, final(pair_left)@.len() as int)
            == proof::matched_count(old(pair_left)@, old(pair_left)@.len() as int) + 1,
))]
pub(super) fn match_free(
    _adjacency: &SparseAdjacencyRef<'_>,
    left: usize,
    right: usize,
    pair_left: &mut [u16],
    pair_right: &mut [u16],
) {
    proof_decl! { let ghost before_left = pair_left@; let ghost before_right = pair_right@; }
    *crate::optimal::get_mut(pair_left, left) = crate::optimal::u16_index(right);
    *crate::optimal::get_mut(pair_right, right) = crate::optimal::u16_index(left);
    proof! { proof::free_matched(_adjacency.offsets@, _adjacency.edges@, before_left, before_right, left as int, right as int); }
}

/// 翻转长度 3 的交替路径，原有已匹配顶点仍匹配。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::matching_valid(_adjacency.offsets@, _adjacency.edges@, old(pair_left)@, old(pair_right)@),
        left < old(pair_left)@.len(), middle < old(pair_left)@.len(),
        right < old(pair_right)@.len(), alternate < old(pair_right)@.len(),
        old(pair_left)@[left as int] == u16::MAX, old(pair_right)@[right as int] == middle,
        old(pair_right)@[alternate as int] == u16::MAX,
        proof::edge_in(_adjacency.offsets@, _adjacency.edges@, left as int, right as int),
        proof::edge_in(_adjacency.offsets@, _adjacency.edges@, middle as int, alternate as int),
    ensures proof::matching_valid(_adjacency.offsets@, _adjacency.edges@, final(pair_left)@, final(pair_right)@),
        proof::matched_preserved(old(pair_left)@, final(pair_left)@),
        proof::matched_preserved(old(pair_right)@, final(pair_right)@),
        final(pair_left)@ == old(pair_left)@.update(middle as int, alternate as u16).update(left as int, right as u16),
        final(pair_right)@ == old(pair_right)@.update(alternate as int, middle as u16).update(right as int, left as u16),
        proof::matched_count(final(pair_left)@, final(pair_left)@.len() as int)
            == proof::matched_count(old(pair_left)@, old(pair_left)@.len() as int) + 1,
))]
pub(super) fn match_three(
    _adjacency: &SparseAdjacencyRef<'_>,
    left: usize,
    right: usize,
    middle: usize,
    alternate: usize,
    pair_left: &mut [u16],
    pair_right: &mut [u16],
) {
    proof_decl! { let ghost before_left = pair_left@; let ghost before_right = pair_right@; }
    *crate::optimal::get_mut(pair_left, middle) = crate::optimal::u16_index(alternate);
    *crate::optimal::get_mut(pair_right, alternate) = crate::optimal::u16_index(middle);
    *crate::optimal::get_mut(pair_left, left) = crate::optimal::u16_index(right);
    *crate::optimal::get_mut(pair_right, right) = crate::optimal::u16_index(left);
    proof! { proof::three_matched(_adjacency.offsets@, _adjacency.edges@, before_left, before_right, left as int, right as int, middle as int, alternate as int); }
}

/// 反向提交显式栈中的简单增广路径；路径外的配对逐项保留。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires terminal < old(pair_right)@.len(),
        proof::path_links(adjacency.offsets@, adjacency.edges@, old(pair_left)@, old(pair_right)@, stack@, terminal as u16),
    ensures proof::matching_valid(adjacency.offsets@, adjacency.edges@, final(pair_left)@, final(pair_right)@),
        proof::path_state(old(pair_left)@, old(pair_right)@, final(pair_left)@, final(pair_right)@,
            adjacency.edges@, stack@, terminal as u16, 0),
        proof::matched_preserved(old(pair_left)@, final(pair_left)@),
        proof::matched_preserved(old(pair_right)@, final(pair_right)@),
        proof::matched_count(final(pair_left)@, final(pair_left)@.len() as int)
            == proof::matched_count(old(pair_left)@, old(pair_left)@.len() as int) + 1,
))]
pub(super) fn commit_path(
    adjacency: &SparseAdjacencyRef<'_>,
    stack: &[DfsFrame],
    terminal: usize,
    pair_left: &mut [u16],
    pair_right: &mut [u16],
) {
    proof_decl! { let ghost initial_left = pair_left@; let ghost initial_right = pair_right@; }
    let terminal = crate::optimal::u16_index(terminal);
    proof! {
        proof::complete_path_links(adjacency.offsets@, adjacency.edges@, initial_left, initial_right, stack@, terminal);
        assert(proof::path_state(initial_left, initial_right, pair_left@, pair_right@,
            adjacency.edges@, stack@, terminal, stack@.len() as int));
    }
    let last = copy(stack, stack.len() - 1);
    proof! { proof::path_advance(adjacency.offsets@, adjacency.edges@, initial_left, initial_right,
    pair_left@, pair_right@, stack@, terminal, stack@.len() as int - 1); }
    *crate::optimal::get_mut(pair_left, usize::from(last.left)) = terminal;
    *crate::optimal::get_mut(pair_right, usize::from(terminal)) = last.left;
    let mut next = stack.len() - 1;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::path_valid(adjacency.offsets@, adjacency.edges@, initial_left, initial_right, stack@, terminal),
            proof::path_state(initial_left, initial_right, pair_left@, pair_right@,
                adjacency.edges@, stack@, terminal, next as int), next < stack@.len(),
        decreases next,
    ))]
    while next > 0 {
        next -= 1;
        let frame = copy(stack, next);
        let right = copy(adjacency.edges, usize::from(frame.next_edge - 1));
        proof! { proof::path_advance(adjacency.offsets@, adjacency.edges@, initial_left, initial_right,
        pair_left@, pair_right@, stack@, terminal, next as int); }
        *crate::optimal::get_mut(pair_left, usize::from(frame.left)) = right;
        *crate::optimal::get_mut(pair_right, usize::from(right)) = frame.left;
    }
    proof! { proof::path_complete(adjacency.offsets@, adjacency.edges@, initial_left, initial_right,
    pair_left@, pair_right@, stack@, terminal); }
}

#[cfg_attr(verus_only, verus_verify)]
impl MatchingScratch {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.selection_ready(),
        result.selected_horizontal_view() == Seq::<u16>::empty(), result.selected_vertical_view() == Seq::<u16>::empty(),
    ))]
    pub(crate) const fn new() -> Self {
        Self {
            hk: HkScratch::new(),
            conflict: ConflictScratch {
                finalize: ConflictFinalizeScratch::new(),
            },
            selected_horizontal: FixedVec::new(0),
            selected_vertical: FixedVec::new(0),
        }
    }
}

impl MatchingScratch {
    /// 验证目标：输出 I 为冲突图顶点子集，编号有效且无重复，任意边不同时选中两端。
    /// 对所有独立集 J 有 `|J| <= |I|`，而不只是 I 无法再加点。
    /// 可用对偶证书证明：匹配 M 合法，`C = (H ∪ V) \ I` 覆盖每条边，且 `|C| = |M|`；
    /// 推出 `|I| = |H| + |V| - |M|` 为最大值。单侧为空或没有冲突边时应选中全部顶点。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(
        requires old(self).selection_ready(), crate::optimal::graph::proof::graph_chords_valid(horizontal_edges@, vertical_edges@),
        ensures final(self).selection_ready(),
            proof::maximum_selection(horizontal_edges@, vertical_edges@, final(self).selected_horizontal_view(), final(self).selected_vertical_view()),
            proof::canonical_selection(horizontal_edges@, vertical_edges@, final(self).selected_horizontal_view(), final(self).selected_vertical_view()),
            final(self).selected_horizontal_view().len() <= 1953, final(self).selected_vertical_view().len() <= 1953,
    ))]
    pub(crate) fn select_maximum_independent_set(
        &mut self,
        horizontal_edges: &[ValuedChord],
        vertical_edges: &[ValuedChord],
    ) {
        proof! {
            crate::optimal::graph::sweep::chord_capacity(horizontal_edges@, false);
            crate::optimal::graph::sweep::chord_capacity(vertical_edges@, true);
        }
        self.selected_horizontal.clear();
        self.selected_vertical.clear();
        if horizontal_edges.is_empty() || vertical_edges.is_empty() {
            select_full_independent_set(
                horizontal_edges.len(),
                vertical_edges.len(),
                &mut self.selected_horizontal,
                &mut self.selected_vertical,
            );
            proof! { proof::full_selection_canonical(horizontal_edges@, vertical_edges@); }
            return;
        }

        {
            let (sparse_adjacency, right_degrees) =
                crate::optimal::graph::build_sparse_conflict_graph_csr(
                    horizontal_edges,
                    vertical_edges,
                    &mut self.conflict,
                );
            if sparse_adjacency.edges.is_empty() {
                select_full_independent_set(
                    horizontal_edges.len(),
                    vertical_edges.len(),
                    &mut self.selected_horizontal,
                    &mut self.selected_vertical,
                );
                proof! {
                    proof::empty_graph(horizontal_edges@, vertical_edges@, sparse_adjacency.offsets@, sparse_adjacency.edges@);
                    proof::full_selection_canonical(horizontal_edges@, vertical_edges@);
                }
            } else {
                select_from_graph(
                    horizontal_edges,
                    vertical_edges,
                    &sparse_adjacency,
                    right_degrees,
                    &mut self.hk,
                    &mut self.selected_horizontal,
                    &mut self.selected_vertical,
                );
            }
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl MatchingScratch {
    #[cfg_attr(verus_only, verus_spec(result => requires self.selection_ready(), ensures result@ == self.selected_horizontal_view()))]
    pub(crate) const fn selected_horizontal(&self) -> &[u16] {
        self.selected_horizontal.as_slice()
    }

    #[cfg_attr(verus_only, verus_spec(result => requires self.selection_ready(), ensures result@ == self.selected_vertical_view()))]
    pub(crate) const fn selected_vertical(&self) -> &[u16] {
        self.selected_vertical.as_slice()
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires horizontal_len <= 1953, vertical_len <= 1953, old(horizontal_out).valid(), old(vertical_out).valid(),
        old(horizontal_out)@.len() == 0, old(vertical_out)@.len() == 0,
    ensures final(horizontal_out).valid(), final(vertical_out).valid(),
        final(horizontal_out)@ == Seq::new(horizontal_len as nat, |i: int| i as u16),
        final(vertical_out)@ == Seq::new(vertical_len as nat, |i: int| i as u16),
))]
fn select_full_independent_set(
    horizontal_len: usize,
    vertical_len: usize,
    horizontal_out: &mut ChordBuffer<u16>,
    vertical_out: &mut ChordBuffer<u16>,
) {
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= horizontal_len <= 1953, horizontal_out.valid(),
            horizontal_out@ == Seq::new(index as nat, |i: int| i as u16),
        decreases horizontal_len - index,
    ))]
    while index < horizontal_len {
        horizontal_out.push(crate::optimal::u16_index(index));
        proof! { assert(horizontal_out@ =~= Seq::new(index as nat + 1, |i: int| i as u16)); }
        index += 1;
    }
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= vertical_len <= 1953, vertical_out.valid(),
            vertical_out@ == Seq::new(index as nat, |i: int| i as u16),
        decreases vertical_len - index,
    ))]
    while index < vertical_len {
        vertical_out.push(crate::optimal::u16_index(index));
        proof! { assert(vertical_out@ =~= Seq::new(index as nat + 1, |i: int| i as u16)); }
        index += 1;
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::buffers_valid(old(scratch)), old(horizontal_out).valid(), old(vertical_out).valid(),
        old(horizontal_out)@.len() == 0, old(vertical_out)@.len() == 0,
        crate::optimal::graph::proof::graph_exact(_horizontal@, vertical@, adjacency.offsets@, adjacency.edges@),
        proof::adjacency_valid(adjacency, vertical@.len()), right_degrees@.len() == vertical@.len(),
        proof::degrees_exact(adjacency.edges@, right_degrees@),
    ensures proof::buffers_valid(final(scratch)), final(horizontal_out).valid(), final(vertical_out).valid(),
        proof::maximum_selection(_horizontal@, vertical@, final(horizontal_out)@, final(vertical_out)@),
        proof::canonical_selection(_horizontal@, vertical@, final(horizontal_out)@, final(vertical_out)@),
        final(horizontal_out)@.len() <= 1953, final(vertical_out)@.len() <= 1953,
))]
fn select_from_graph(
    _horizontal: &[ValuedChord],
    vertical: &[ValuedChord],
    adjacency: &SparseAdjacencyRef<'_>,
    right_degrees: &[u8],
    scratch: &mut HkScratch,
    horizontal_out: &mut ChordBuffer<u16>,
    vertical_out: &mut ChordBuffer<u16>,
) {
    crate::optimal::hk::hopcroft_karp_dw_sparse_csr_u16(
        adjacency,
        vertical.len(),
        right_degrees,
        scratch,
    );
    collect_flags(scratch.reachable_left.as_slice(), true, horizontal_out);
    collect_flags(scratch.reachable_right.as_slice(), false, vertical_out);
    proof! { proof::maximum_from_marks(_horizontal@, vertical@, adjacency.offsets@, adjacency.edges@,
    scratch.pair_left@, scratch.pair_right@, scratch.reachable_left@, scratch.reachable_right@);
    crate::optimal::hk::choice_proof::from_completed(_horizontal@, vertical@, adjacency, scratch); }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires flags@.len() <= 1953, old(output).valid(), old(output)@.len() == 0,
    ensures final(output).valid(), final(output)@ == proof::flag_ids(flags@, value, flags@.len() as int), final(output)@.len() <= flags@.len(),
))]
fn collect_flags(flags: &[bool], value: bool, output: &mut ChordBuffer<u16>) {
    let mut next = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant next <= flags@.len() <= 1953, output.valid(), output@ == proof::flag_ids(flags@, value, next as int),
            output@.len() <= next,
        decreases flags@.len() - next,
    ))]
    while next < flags.len() {
        proof! { proof::flag_step(flags@, value, next as int); }
        if copy(flags, next) == value {
            output.push(crate::optimal::u16_index(next));
        }
        next += 1;
    }
}
