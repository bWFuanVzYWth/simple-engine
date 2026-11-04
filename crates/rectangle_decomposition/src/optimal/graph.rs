//! 稀疏冲突图构建：将 chord 列表转为 CSR 邻接。

use crate::optimal::matching::{
    ConflictFinalizeScratch, ConflictScratch, IMAGE64_AXIS_LIMIT, IMAGE64_MAX_CONFLICT_EDGES,
    SparseAdjacencyRef, SparseEdge, UNMATCHED_U16,
};
use crate::optimal::types::ValuedChord;
use crate::optimal::{copy, get_mut, slice, u16_index, u32_index};

use crate::chord_graph::internal_mask;

#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "../proof/graph.rs"]
pub(crate) mod proof;
#[cfg(verus_only)]
#[path = "../proof/sweep.rs"]
pub(crate) mod sweep;

#[cfg_attr(verus_only, verus_verify)]
const IMAGE64_INTERNAL_MIN: usize = 1;
#[cfg_attr(verus_only, verus_verify)]
const IMAGE64_INTERNAL_MAX: usize = IMAGE64_AXIS_LIMIT - 1;

/// 稳定计数排序右编号；位置等于更小度数总数加同度数的更小编号数。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::degree_domain(right_degrees@), old(order)@.len() == right_degrees@.len(),
        forall|i: int| 0 <= i < old(order)@.len() ==> old(order)@[i] < right_degrees@.len(),
    ensures proof::ordered(right_degrees@, final(order)@),
))]
pub(crate) fn sort_rights(right_degrees: &[u8], order: &mut [u32]) {
    let mut degree_offsets = [0usize; IMAGE64_AXIS_LIMIT];
    let mut right = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::degree_domain(right_degrees@), right <= right_degrees@.len(),
            proof::histogram(right_degrees@, degree_offsets@, right as int),
        decreases right_degrees@.len() - right,
    ))]
    while right < right_degrees.len() {
        proof_decl! { let ghost before = degree_offsets@; }
        let degree = usize::from(copy(right_degrees, right));
        proof! { proof::count_bounds(right_degrees@, right as int, degree as int, degree as int); }
        *get_mut(&mut degree_offsets, degree) += 1;
        proof! { proof::histogram_step(right_degrees@, before, degree_offsets@, right as int); }
        right += 1;
    }
    let mut degree = 0;
    let mut order_start = 0usize;
    proof! { proof::count_bounds(right_degrees@, right_degrees@.len() as int, 0, 0); }
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::degree_domain(right_degrees@), degree <= 64,
            proof::prefix_histogram(right_degrees@, degree_offsets@, degree as int),
            order_start == proof::less(right_degrees@, right_degrees@.len() as int, degree as int),
        decreases 64 - degree,
    ))]
    while degree < degree_offsets.len() {
        proof_decl! { let ghost before = degree_offsets@; }
        proof! { proof::count_bounds(right_degrees@, right_degrees@.len() as int, degree as int, degree as int + 1); }
        let count = copy(&degree_offsets, degree);
        *get_mut(&mut degree_offsets, degree) = order_start;
        proof! { proof::prefix_step(right_degrees@, before, degree_offsets@, degree as int); }
        order_start += count;
        degree += 1;
    }
    let mut right = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::degree_domain(right_degrees@), right <= right_degrees@.len(),
            proof::scatter_counters(right_degrees@, degree_offsets@, right as int),
            proof::order_prefix(right_degrees@, order@, right as int),
        decreases right_degrees@.len() - right,
    ))]
    while right < right_degrees.len() {
        proof_decl! { let ghost before = degree_offsets@; let ghost order_before = order@; }
        let degree = usize::from(copy(right_degrees, right));
        let slot = get_mut(&mut degree_offsets, degree);
        proof! { proof::rank_bound(right_degrees@, right as int); }
        *get_mut(order, *slot) = u32_index(right);
        *slot += 1;
        proof! { proof::scatter_step(right_degrees@, before, degree_offsets@, order_before, order@, right as int); }
        right += 1;
    }
    proof! { proof::complete_order(right_degrees@, order@); }
}

/// 按完整右编号排列散布源组，得到逐对出现次数精确的左 CSR。
/// 只要求源组/度数、已计算 offsets 与组起点正确，不要求目的边内容正确。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::csr_shape(edges@, right_degrees@, starts@, order@, offsets@, old(output)@.len()),
        old(cursors)@ == offsets@.take(offsets@.len() as int - 1),
    ensures proof::csr_exact(edges@, offsets@, final(output)@, right_degrees@.len()),
        final(cursors)@.len() == old(cursors)@.len(),
        forall|left: int| 0 <= left < final(cursors)@.len()
            ==> final(cursors)@[left] == offsets@[left + 1],
))]
pub(crate) fn scatter_right_groups(
    edges: &[SparseEdge],
    right_degrees: &[u8],
    starts: &[u32],
    order: &[u32],
    offsets: &[u16],
    cursors: &mut [u16],
    output: &mut [u16],
) {
    proof! { proof::scatter_begin(edges@, right_degrees@, starts@, order@, offsets@, cursors@, output@); }
    let mut next = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::csr_shape(edges@, right_degrees@, starts@, order@, offsets@, output@.len()),
            proof::scattered(edges@, order@, offsets@, cursors@, output@, next as int, right_degrees@.len()),
        decreases order@.len() - next,
    ))]
    while next < order.len() {
        proof_decl! { let ghost before = cursors@; let ghost previous = output@; }
        let right = copy(order, next) as usize;
        let start = copy(starts, right) as usize;
        let end = start + usize::from(copy(right_degrees, right));
        proof! { proof::scatter_ready(edges@, right_degrees@, starts@, order@, offsets@, cursors@, output@, next as int); }
        crate::chord_graph::scatter_group(
            slice(edges, start..end),
            u16_index(right),
            offsets,
            cursors,
            output,
        );
        proof! { proof::scatter_next(edges@, right_degrees@, starts@, order@, offsets@, before, previous, cursors@, output@, next as int); }
        next += 1;
    }
    proof! { proof::scatter_complete(edges@, right_degrees@, starts@, order@, offsets@, cursors@, output@); }
}

/// 在同一借用内排序并散布，布局前半段的源组起点保持不变。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::csr_source(edges@, right_degrees@, old(layout)@, offsets@, old(output)@.len()),
        old(cursors)@ == offsets@.take(offsets@.len() as int - 1),
    ensures proof::csr_exact(edges@, offsets@, final(output)@, right_degrees@.len()),
        crate::optimal::matching::proof::rows_valid(offsets@, final(output)@.len()),
        crate::optimal::matching::proof::degrees_exact(final(output)@, right_degrees@),
        forall|i: int| 0 <= i < final(output)@.len() ==> final(output)@[i] < right_degrees@.len(),
        final(layout)@.len() == old(layout)@.len(),
        final(cursors)@.len() == old(cursors)@.len(),
))]
fn sort_and_scatter(
    edges: &[SparseEdge],
    right_degrees: &[u8],
    layout: &mut [u32],
    offsets: &[u16],
    cursors: &mut [u16],
    output: &mut [u16],
) {
    let (starts, order) = layout.split_at_mut(right_degrees.len());
    proof! { proof::starts_complete(edges@, right_degrees@, starts@); }
    sort_rights(right_degrees, order);
    scatter_right_groups(
        edges,
        right_degrees,
        starts,
        order,
        offsets,
        cursors,
        output,
    );
    proof! { crate::optimal::matching::proof::csr_degrees(edges@, right_degrees@, offsets@, output@); }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::degree_domain(degrees@),
    ensures result == proof::degree_sum(degrees@, degrees@.len() as int),
))]
fn sum_right_degrees(degrees: &[u8]) -> usize {
    let mut end = 0;
    let mut sum = 0usize;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::degree_domain(degrees@), end <= degrees@.len(),
            sum == proof::degree_sum(degrees@, end as int),
        decreases degrees@.len() - end,
    ))]
    while end < degrees.len() {
        proof! { proof::degree_sum_bound(degrees@, end as int); }
        sum += usize::from(copy(degrees, end));
        end += 1;
    }
    sum
}

/// 利用按右顶点连续分组的 `edge_buffer`，在线性时间内构建有序左 CSR。
/// 每个左邻接表仍按 (右顶点度数, 右顶点编号) 排序。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::spinoff_prover)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::finalize_input(old(scratch), left_size, right_size),
    ensures proof::csr_exact(old(scratch).edge_buffer@.take(old(scratch).edge_count as int),
        result.0.offsets@, result.0.edges@, right_size as nat),
        result.0.offsets@.len() == left_size + 1,
        result.0.edges@.len() == old(scratch).edge_count,
        crate::optimal::matching::proof::adjacency_valid(&result.0, right_size as nat),
        crate::optimal::matching::proof::degrees_exact(result.0.edges@, result.1@),
        result.1@ == old(scratch).right_degrees@,
        final(scratch).right_degrees@ == old(scratch).right_degrees@,
        final(scratch).edge_buffer == old(scratch).edge_buffer,
        final(scratch).edge_count == old(scratch).edge_count,
        final(scratch).horizontal_start_heads == old(scratch).horizontal_start_heads,
        final(scratch).horizontal_end_heads == old(scratch).horizontal_end_heads,
        final(scratch).vertical_query_heads == old(scratch).vertical_query_heads,
        final(scratch).active_horizontal == old(scratch).active_horizontal,
        proof::conflict_buffers_valid(final(scratch)),
))]
pub(super) fn build_sparse_adjacency_in_scratch(
    left_size: usize,
    right_size: usize,
    scratch: &mut ConflictFinalizeScratch,
) -> (SparseAdjacencyRef<'_>, &[u8]) {
    let edges = slice(&scratch.edge_buffer, 0..scratch.edge_count);
    let right_degrees = scratch.right_degrees.as_slice();
    debug_assert_eq!(right_degrees.len(), right_size);
    scratch.adjacency_offsets.clear();
    scratch.adjacency_offsets.resize(left_size + 1, 0);
    scratch.right_layout.clear();
    scratch.right_layout.resize(2 * right_size, 0);
    let layout = &mut scratch.right_layout;
    let offsets = &mut scratch.adjacency_offsets;
    proof! { proof::groups_contiguous(edges@, right_degrees@); }
    crate::chord_graph::build_csr_layout(
        edges,
        offsets.as_mut_slice(),
        crate::optimal::slice_mut(layout.as_mut_slice(), 0..right_size),
    );

    scratch.next_offsets.clear();
    scratch
        .next_offsets
        .extend_from_slice(slice(offsets, 0..left_size));
    scratch.adjacency_edges.clear();
    scratch.adjacency_edges.resize(edges.len(), 0);
    let compact_edges = &mut scratch.adjacency_edges;
    proof! { proof::groups_total(edges@, right_degrees@); }
    debug_assert_eq!(sum_right_degrees(right_degrees), edges.len());

    // 稳定计数排序给出全局右顶点顺序；散布后，各左邻接表自然有序。
    sort_and_scatter(
        edges,
        right_degrees,
        layout.as_mut_slice(),
        offsets.as_slice(),
        scratch.next_offsets.as_mut_slice(),
        compact_edges.as_mut_slice(),
    );
    proof! {
        assert(offsets@.len() == left_size + 1);
        assert(compact_edges@.len() == edges@.len());
        crate::optimal::fixed::FixedVec::establish(&*offsets);
        crate::optimal::fixed::FixedVec::establish(&*compact_edges);
        crate::optimal::fixed::FixedVec::establish(&*layout);
        crate::optimal::fixed::FixedVec::establish(&scratch.next_offsets);
    }
    let adjacency = SparseAdjacencyRef {
        offsets: offsets.as_slice(),
        edges: compact_edges.as_slice(),
    };
    proof! { assert(crate::optimal::matching::proof::adjacency_valid(&adjacency, right_size as nat)); }
    (adjacency, right_degrees)
}

/// 验证目标：输入来自完整 chord 提取，同线同向闭段互不相交，端点均为内部格点。
/// 输出边当且仅当 `h.x1 <= v.x1 <= h.x2 ∧ v.y1 <= h.y1 <= v.y2`，无重复或遗漏。
/// CSR 满足 offsets 长度为 `|H|+1`、首项为 0、末项为 `|E|`、单调不减，
/// 邻接编号均小于 `|V|`，right_degrees 与实际右度数相同。
/// 几何上每条边唯一对应内部交点，故 `|E| <= 63² = 3969`、每个右度数至多 63；
/// 这些须由调用链证明，本接口不接受任意重叠线段构成的图。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires proof::graph_chords_valid(horizontal_edges@,vertical_edges@),proof::conflict_buffers_valid(&old(scratch).finalize),
    ensures proof::conflict_buffers_valid(&final(scratch).finalize),proof::graph_exact(horizontal_edges@,vertical_edges@,result.0.offsets@,result.0.edges@),
        result.1@.len()==vertical_edges@.len(),crate::optimal::matching::proof::adjacency_valid(&result.0,vertical_edges@.len()),
        crate::optimal::matching::proof::degrees_exact(result.0.edges@,result.1@),
        forall|h: int,v: int|0<=h<horizontal_edges@.len() && 0<=v<vertical_edges@.len() ==>
            proof::row_count(result.0.edges@,result.0.offsets@[h] as int,result.0.offsets@[h+1] as int,v)==if proof::closed_conflict(horizontal_edges@[h],vertical_edges@[v]){1int}else{0int},
))]
#[inline(always)]
pub(super) fn build_sparse_conflict_graph_csr<'a>(
    horizontal_edges: &[ValuedChord],
    vertical_edges: &[ValuedChord],
    scratch: &'a mut ConflictScratch,
) -> (SparseAdjacencyRef<'a>, &'a [u8]) {
    build_sparse_conflict_graph_sweep(horizontal_edges, vertical_edges, &mut scratch.finalize);
    proof_decl! {let ghost source=sweep::source(&scratch.finalize);}
    let result = build_sparse_adjacency_in_scratch(
        horizontal_edges.len(),
        vertical_edges.len(),
        &mut scratch.finalize,
    );
    proof! {sweep::csr_complete(horizontal_edges@,vertical_edges@,source,result.0.offsets@,result.0.edges@);}
    result
}

/// x 事件桶和活跃 y 位图避免逐格展开水平 chord；总工作为 O(n + C + E)。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::graph_chords_valid(horizontal_edges@,vertical_edges@),proof::conflict_buffers_valid(old(scratch)),
    ensures proof::conflict_buffers_valid(final(scratch)),proof::finalize_input(final(scratch),horizontal_edges@.len() as usize,vertical_edges@.len() as usize),
        sweep::complete_edges(horizontal_edges@,vertical_edges@,sweep::source(final(scratch))),
))]
fn build_sparse_conflict_graph_sweep(
    horizontal_edges: &[ValuedChord],
    vertical_edges: &[ValuedChord],
    scratch: &mut ConflictFinalizeScratch,
) {
    build_conflict_events(horizontal_edges, vertical_edges, scratch);
    sweep_conflict_events(horizontal_edges, vertical_edges, scratch);
    debug_assert!(scratch.edge_count <= IMAGE64_MAX_CONFLICT_EDGES);
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires proof::graph_chords_valid(horizontal_edges@,vertical_edges@),proof::conflict_buffers_valid(old(scratch)),
    ensures sweep::events(horizontal_edges@,vertical_edges@,final(scratch)),proof::conflict_buffers_valid(final(scratch)),
        final(scratch).edge_count==0,final(scratch).right_degrees@==Seq::new(vertical_edges@.len(),|_: int|0u8),
))]
fn build_conflict_events(
    horizontal_edges: &[ValuedChord],
    vertical_edges: &[ValuedChord],
    scratch: &mut ConflictFinalizeScratch,
) {
    proof! { sweep::chord_capacity(horizontal_edges@,false);sweep::chord_capacity(vertical_edges@,true); }
    crate::std_adapter::fill_u16(&mut scratch.horizontal_start_heads, UNMATCHED_U16);
    crate::std_adapter::fill_u16(&mut scratch.horizontal_end_heads, UNMATCHED_U16);
    crate::std_adapter::fill_u16(&mut scratch.vertical_query_heads, UNMATCHED_U16);
    scratch.right_layout.clear();
    scratch
        .right_layout
        .resize(horizontal_edges.len() + vertical_edges.len(), u32::MAX);
    scratch.edge_count = 0;
    scratch.right_degrees.clear();
    scratch.right_degrees.resize(vertical_edges.len(), 0);

    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::graph_chords_valid(horizontal_edges@,vertical_edges@),horizontal_edges@.len()<=1953,vertical_edges@.len()<=1953,
            proof::conflict_buffers_valid(scratch),scratch.edge_count==0,scratch.right_degrees@==Seq::new(vertical_edges@.len(),|_: int|0u8),
            index<=horizontal_edges@.len(),sweep::heads(horizontal_edges@,0,index as int,scratch.horizontal_start_heads@),
            sweep::heads(horizontal_edges@,1,index as int,scratch.horizontal_end_heads@),
            sweep::heads(vertical_edges@,2,0,scratch.vertical_query_heads@),sweep::links(horizontal_edges@,vertical_edges@,scratch.right_layout@,index as int,0),
        decreases horizontal_edges@.len()-index,
    ))]
    while index < horizontal_edges.len() {
        proof_decl! { let ghost previous=scratch.right_layout@; let ghost starts=scratch.horizontal_start_heads@;let ghost ends=scratch.horizontal_end_heads@; }
        proof! { assert(horizontal_edges@[index as int].well_formed(false)); }
        let horizontal = crate::optimal::get(horizontal_edges, index);
        let horizontal = horizontal.chord();
        debug_assert_eq!(horizontal.y1, horizontal.y2);
        let y = usize::from(horizontal.y1);
        debug_assert!((IMAGE64_INTERNAL_MIN..=IMAGE64_INTERNAL_MAX).contains(&y));
        let start = usize::from(horizontal.x1).max(IMAGE64_INTERNAL_MIN);
        let end = usize::from(horizontal.x2).min(IMAGE64_INTERNAL_MAX);
        if start > end {
            index += 1;
            continue;
        }
        // 两个横向事件的 next 编号打包到一个 u32，后续 CSR 复用这块存储。
        let start_next = copy(&scratch.horizontal_start_heads, start);
        let end_next = copy(&scratch.horizontal_end_heads, end);
        *get_mut(&mut scratch.right_layout, index) =
            u32::from(start_next) | (u32::from(end_next) << 16);
        *get_mut(&mut scratch.horizontal_start_heads, start) = u16_index(index);
        *get_mut(&mut scratch.horizontal_end_heads, end) = u16_index(index);
        proof! {
            crate::optimal::fixed::FixedVec::establish(&scratch.right_layout);
            sweep::horizontal_link(horizontal_edges@,vertical_edges@,previous,scratch.right_layout@,index as int,scratch.right_layout@[index as int]);
            sweep::heads_step(horizontal_edges@,0,index as int,starts,scratch.horizontal_start_heads@);
            sweep::heads_step(horizontal_edges@,1,index as int,ends,scratch.horizontal_end_heads@);
        }
        index += 1;
    }

    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant proof::graph_chords_valid(horizontal_edges@,vertical_edges@),horizontal_edges@.len()<=1953,vertical_edges@.len()<=1953,
            proof::conflict_buffers_valid(scratch),scratch.edge_count==0,scratch.right_degrees@==Seq::new(vertical_edges@.len(),|_: int|0u8),
            index<=vertical_edges@.len(),sweep::heads(horizontal_edges@,0,horizontal_edges@.len() as int,scratch.horizontal_start_heads@),
            sweep::heads(horizontal_edges@,1,horizontal_edges@.len() as int,scratch.horizontal_end_heads@),
            sweep::heads(vertical_edges@,2,index as int,scratch.vertical_query_heads@),sweep::links(horizontal_edges@,vertical_edges@,scratch.right_layout@,horizontal_edges@.len() as int,index as int),
        decreases vertical_edges@.len()-index,
    ))]
    while index < vertical_edges.len() {
        proof_decl! {let ghost previous=scratch.right_layout@;let ghost queries=scratch.vertical_query_heads@;}
        proof! {assert(vertical_edges@[index as int].well_formed(true));}
        let vertical = crate::optimal::get(vertical_edges, index);
        let vertical = vertical.chord();
        debug_assert_eq!(vertical.x1, vertical.x2);
        let x = usize::from(vertical.x1);
        debug_assert!((IMAGE64_INTERNAL_MIN..=IMAGE64_INTERNAL_MAX).contains(&x));
        *get_mut(&mut scratch.right_layout, horizontal_edges.len() + index) =
            u32::from(copy(&scratch.vertical_query_heads, x));
        *get_mut(&mut scratch.vertical_query_heads, x) = u16_index(index);
        proof! {
            crate::optimal::fixed::FixedVec::establish(&scratch.right_layout);
            sweep::vertical_link(horizontal_edges@,vertical_edges@,previous,scratch.right_layout@,index as int,scratch.right_layout@[horizontal_edges@.len() as int+index as int]);
            sweep::heads_step(vertical_edges@,2,index as int,queries,scratch.vertical_query_heads@);
        }
        index += 1;
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires sweep::events(horizontal_edges@,vertical_edges@,old(scratch)),proof::conflict_buffers_valid(old(scratch)),
        current<horizontal_edges@.len(),x<=63,horizontal_edges@[current as int].line()<64,
        sweep::coordinate(horizontal_edges@[current as int],if START{0}else{1})==x,
        sweep::active(horizontal_edges@,old(scratch).active_horizontal@,*old(active_y),x as int,if START{0}else{2},current as int),
    ensures sweep::events(horizontal_edges@,vertical_edges@,final(scratch)),proof::conflict_buffers_valid(final(scratch)),sweep::active_frame(old(scratch),final(scratch)),
        sweep::cursor(result)==sweep::last(horizontal_edges@,if START{0}else{1},current as int,x as int),sweep::cursor(result)<current,
        result!=65535 ==> sweep::coordinate(horizontal_edges@[result as int],if START{0}else{1})==x,
        sweep::active(horizontal_edges@,final(scratch).active_horizontal@,*final(active_y),x as int,if START{0}else{2},sweep::cursor(result)),
))]
fn horizontal_event<const START: bool>(
    horizontal_edges: &[ValuedChord],
    #[cfg(verus_only)] vertical_edges: &[ValuedChord],
    scratch: &mut ConflictFinalizeScratch,
    active_y: &mut u64,
    #[cfg(verus_only)] x: usize,
    current: u16,
) -> u16 {
    let index = usize::from(current);
    proof! {
        assert(horizontal_edges@[index as int].well_formed(false));
        sweep::packed_next(horizontal_edges@,vertical_edges@,scratch,index as int,START);
        sweep::next_cursor(horizontal_edges@,if START{0}else{1},index as int,x as int);
        if START{sweep::activation(horizontal_edges@,scratch.active_horizontal@,*active_y,x as int,index as int);}
        else{sweep::deactivation(horizontal_edges@,scratch.active_horizontal@,*active_y,x as int,index as int);}
        sweep::bit_and(*active_y,horizontal_edges@[index as int].line() as u32);
    }
    let y = usize::from(crate::optimal::get(horizontal_edges, index).chord().y1);
    if START {
        debug_assert_eq!(*active_y & (1u64 << y), 0);
        *active_y |= 1u64 << y;
        *get_mut(&mut scratch.active_horizontal, y) = current;
    } else {
        debug_assert_eq!(copy(&scratch.active_horizontal, y), current);
        *active_y &= !(1u64 << y);
    }
    let packed = copy(&scratch.right_layout, index);
    proof! {assert((packed&0xffff)<=65535 && (packed>>16)<=65535) by(bit_vector);}
    if START {
        u16_index((packed & 0xffff) as usize)
    } else {
        u16_index((packed >> 16) as usize)
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires sweep::events(horizontal_edges@,vertical_edges@,old(scratch)),proof::conflict_buffers_valid(old(scratch)),old(scratch).edge_count<=3969,
        current<vertical_edges@.len(),vertical_edges@[current as int].line()==x,
        sweep::query_mask(horizontal_edges@,vertical_edges@,old(scratch).active_horizontal@,mask,x as int,current as int),
        sweep::partial(horizontal_edges@,vertical_edges@,sweep::source(old(scratch)),x as int,current as int,*old(remaining)),
        sweep::partial_groups(sweep::source(old(scratch)),old(scratch).right_degrees@,current as int,start as int),*old(remaining)!=0,
        forall|y: int|0<=y<64 && sweep::bit(*old(remaining),y) ==> sweep::bit(mask,y),
    ensures sweep::events(horizontal_edges@,vertical_edges@,final(scratch)),proof::conflict_buffers_valid(final(scratch)),sweep::query_frame(old(scratch),final(scratch)),
        final(scratch).right_degrees==old(scratch).right_degrees,final(scratch).edge_count==old(scratch).edge_count+1,final(scratch).edge_count<=3969,
        *final(remaining)==*old(remaining)&((*old(remaining)-1) as u64),*final(remaining)<*old(remaining),
        sweep::partial(horizontal_edges@,vertical_edges@,sweep::source(final(scratch)),x as int,current as int,*final(remaining)),
        sweep::partial_groups(sweep::source(final(scratch)),final(scratch).right_degrees@,current as int,start as int),
        forall|y: int|0<=y<64 && sweep::bit(*final(remaining),y) ==> sweep::bit(mask,y),
))]
fn emit_conflict(
    #[cfg(verus_only)] horizontal_edges: &[ValuedChord],
    #[cfg(verus_only)] vertical_edges: &[ValuedChord],
    scratch: &mut ConflictFinalizeScratch,
    remaining: &mut u64,
    #[cfg(verus_only)] x: usize,
    current: u16,
    #[cfg(verus_only)] start: usize,
    #[cfg(verus_only)] mask: u64,
) {
    proof_decl! {let ghost before=sweep::source(scratch);}
    proof! {sweep::clear_lowest(*remaining);}
    let y = remaining.trailing_zeros() as usize;
    let left = copy(&scratch.active_horizontal, y);
    proof! {
        sweep::append_edge(horizontal_edges@,vertical_edges@,scratch.active_horizontal@,mask,*remaining,before,x as int,current as int,y as int);
        sweep::group_append(before,scratch.right_degrees@,current as int,start as int,SparseEdge{left,right:current});
    }
    *get_mut(&mut scratch.edge_buffer, scratch.edge_count) = SparseEdge {
        left,
        right: current,
    };
    scratch.edge_count += 1;
    proof! {assert(sweep::source(scratch) =~=before.push(SparseEdge{left,right:current}));}
    *remaining &= *remaining - 1;
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires sweep::events(horizontal_edges@,vertical_edges@,old(scratch)),proof::conflict_buffers_valid(old(scratch)),old(scratch).edge_count<=3969,
        current<vertical_edges@.len(),vertical_edges@[current as int].line()==x,
        sweep::active(horizontal_edges@,old(scratch).active_horizontal@,active_y,x as int,1,0),
        sweep::between(horizontal_edges@,vertical_edges@,sweep::source(old(scratch)),x as int,current as int),
        proof::source_groups(sweep::source(old(scratch)),old(scratch).right_degrees@),old(scratch).right_degrees@.len()==vertical_edges@.len(),
    ensures sweep::events(horizontal_edges@,vertical_edges@,final(scratch)),proof::conflict_buffers_valid(final(scratch)),sweep::query_frame(old(scratch),final(scratch)),
        final(scratch).edge_count<=3969,final(scratch).right_degrees@.len()==vertical_edges@.len(),
        sweep::between(horizontal_edges@,vertical_edges@,sweep::source(final(scratch)),x as int,sweep::cursor(result)),
        proof::source_groups(sweep::source(final(scratch)),final(scratch).right_degrees@),
        sweep::cursor(result)==sweep::last(vertical_edges@,2,current as int,x as int),sweep::cursor(result)<current,
        result!=65535 ==> vertical_edges@[result as int].line()==x,
))]
fn query_vertical(
    horizontal_edges: &[ValuedChord],
    vertical_edges: &[ValuedChord],
    scratch: &mut ConflictFinalizeScratch,
    active_y: u64,
    #[cfg(verus_only)] x: usize,
    current: u16,
) -> u16 {
    let index = usize::from(current);
    proof! {assert(vertical_edges@[index as int].well_formed(true));}
    let vertical = crate::optimal::get(vertical_edges, index).chord();
    let range = internal_mask(vertical.y1, vertical.y2);
    let mask = active_y & range;
    let mut conflicts = mask;
    let start = scratch.edge_count;
    proof! {
        sweep::query_begin(horizontal_edges@,vertical_edges@,scratch.active_horizontal@,active_y,range,sweep::source(scratch),x as int,index as int);
        sweep::unseen_query(horizontal_edges@,vertical_edges@,sweep::source(scratch),x as int,index as int);
        sweep::group_begin(sweep::source(scratch),scratch.right_degrees@,index as int);
    }
    #[cfg_attr(verus_only, verus_spec(
        invariant sweep::events(horizontal_edges@,vertical_edges@,scratch),proof::conflict_buffers_valid(scratch),scratch.edge_count<=3969,
            current<vertical_edges@.len(),vertical_edges@[current as int].line()==x,
            sweep::query_mask(horizontal_edges@,vertical_edges@,scratch.active_horizontal@,mask,x as int,current as int),
            sweep::partial(horizontal_edges@,vertical_edges@,sweep::source(scratch),x as int,current as int,conflicts),
            sweep::partial_groups(sweep::source(scratch),scratch.right_degrees@,current as int,start as int),
            scratch.right_degrees@.len()==vertical_edges@.len(),sweep::query_frame(old(scratch),scratch),
            forall|y: int|0<=y<64 && sweep::bit(conflicts,y) ==> sweep::bit(mask,y),
        decreases conflicts,
    ))]
    while conflicts != 0 {
        emit_conflict(
            #[cfg(verus_only)]
            horizontal_edges,
            #[cfg(verus_only)]
            vertical_edges,
            scratch,
            &mut conflicts,
            #[cfg(verus_only)]
            x,
            current,
            #[cfg(verus_only)]
            start,
            #[cfg(verus_only)]
            mask,
        );
    }
    proof! {
        sweep::group_capacity(horizontal_edges@,vertical_edges@,sweep::source(scratch),index as int,start as int);
        sweep::group_finish(sweep::source(scratch),scratch.right_degrees@,index as int,start as int);
        sweep::query_end(horizontal_edges@,vertical_edges@,sweep::source(scratch),x as int,index as int);
        sweep::next_cursor(vertical_edges@,2,index as int,x as int);
    }
    *get_mut(&mut scratch.right_degrees, index) = (scratch.edge_count - start) as u8;
    proof! {
        crate::optimal::fixed::FixedVec::establish(&scratch.right_degrees);
        sweep::vertical_next(horizontal_edges@,vertical_edges@,scratch,index as int);
    }
    u16_index(copy(&scratch.right_layout, horizontal_edges.len() + index) as usize)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(
    requires sweep::events(horizontal_edges@,vertical_edges@,old(scratch)),proof::conflict_buffers_valid(old(scratch)),old(scratch).edge_count==0,
        old(scratch).right_degrees@==Seq::new(vertical_edges@.len(),|_: int|0u8),
    ensures sweep::events(horizontal_edges@,vertical_edges@,final(scratch)),proof::conflict_buffers_valid(final(scratch)),
        proof::finalize_input(final(scratch),horizontal_edges@.len() as usize,vertical_edges@.len() as usize),
        sweep::complete_edges(horizontal_edges@,vertical_edges@,sweep::source(final(scratch))),
))]
fn sweep_conflict_events(
    horizontal_edges: &[ValuedChord],
    vertical_edges: &[ValuedChord],
    scratch: &mut ConflictFinalizeScratch,
) {
    let mut active_y = 0u64;
    let mut x = IMAGE64_INTERNAL_MIN;
    proof! {
        sweep::sweep_started(horizontal_edges@,vertical_edges@,scratch.active_horizontal@);
        assert(sweep::source(scratch) =~=Seq::<SparseEdge>::empty());
        assert(scratch.right_degrees@==Seq::new(vertical_edges@.len(),|_: int|0u8));
        assert(proof::source_groups(sweep::source(scratch),scratch.right_degrees@));
    }
    #[cfg_attr(verus_only, verus_spec(
        invariant 1<=x<=64,sweep::running(horizontal_edges@,vertical_edges@,scratch),
            sweep::active(horizontal_edges@,scratch.active_horizontal@,active_y,x as int,0,1953),
            sweep::between(horizontal_edges@,vertical_edges@,sweep::source(scratch),x as int,1953),
        decreases 64-x,
    ))]
    while x <= IMAGE64_INTERNAL_MAX {
        // 闭端点：同一 x 的 start 先于 query，end 后于 query。
        let mut horizontal_index = copy(&scratch.horizontal_start_heads, x);
        proof! {
            sweep::head_cursor(horizontal_edges@,0,horizontal_edges@.len() as int,x as int,horizontal_index);
            sweep::phase_begin(horizontal_edges@,scratch.active_horizontal@,active_y,x as int,horizontal_index,true);
        }
        #[cfg_attr(verus_only, verus_spec(
            invariant 1<=x<=63,sweep::running(horizontal_edges@,vertical_edges@,scratch),
                sweep::active(horizontal_edges@,scratch.active_horizontal@,active_y,x as int,0,sweep::cursor(horizontal_index)),
                sweep::between(horizontal_edges@,vertical_edges@,sweep::source(scratch),x as int,1953),
                -1<=sweep::cursor(horizontal_index)<horizontal_edges@.len(),horizontal_index!=65535 ==> horizontal_edges@[horizontal_index as int].start()==x,
            decreases sweep::cursor(horizontal_index)+1,
        ))]
        while horizontal_index != UNMATCHED_U16 {
            horizontal_index = horizontal_event::<true>(
                horizontal_edges,
                #[cfg(verus_only)]
                vertical_edges,
                scratch,
                &mut active_y,
                #[cfg(verus_only)]
                x,
                horizontal_index,
            );
        }
        proof! {sweep::phase_end(horizontal_edges@,scratch.active_horizontal@,active_y,x as int,true);}

        let mut vertical_index = copy(&scratch.vertical_query_heads, x);
        proof! {
            sweep::head_cursor(vertical_edges@,2,vertical_edges@.len() as int,x as int,vertical_index);
            sweep::queries_begin(horizontal_edges@,vertical_edges@,sweep::source(scratch),x as int,vertical_index);
        }
        #[cfg_attr(verus_only, verus_spec(
            invariant 1<=x<=63,sweep::running(horizontal_edges@,vertical_edges@,scratch),
                sweep::active(horizontal_edges@,scratch.active_horizontal@,active_y,x as int,1,0),
                sweep::between(horizontal_edges@,vertical_edges@,sweep::source(scratch),x as int,sweep::cursor(vertical_index)),
                -1<=sweep::cursor(vertical_index)<vertical_edges@.len(),vertical_index!=65535 ==> vertical_edges@[vertical_index as int].line()==x,
            decreases sweep::cursor(vertical_index)+1,
        ))]
        while vertical_index != UNMATCHED_U16 {
            vertical_index = query_vertical(
                horizontal_edges,
                vertical_edges,
                scratch,
                active_y,
                #[cfg(verus_only)]
                x,
                vertical_index,
            );
        }
        proof! {sweep::queries_end(horizontal_edges@,vertical_edges@,sweep::source(scratch),x as int);}

        horizontal_index = copy(&scratch.horizontal_end_heads, x);
        proof! {
            sweep::head_cursor(horizontal_edges@,1,horizontal_edges@.len() as int,x as int,horizontal_index);
            sweep::phase_begin(horizontal_edges@,scratch.active_horizontal@,active_y,x as int,horizontal_index,false);
        }
        #[cfg_attr(verus_only, verus_spec(
            invariant 1<=x<=63,sweep::running(horizontal_edges@,vertical_edges@,scratch),
                sweep::active(horizontal_edges@,scratch.active_horizontal@,active_y,x as int,2,sweep::cursor(horizontal_index)),
                sweep::between(horizontal_edges@,vertical_edges@,sweep::source(scratch),x as int+1,1953),
                -1<=sweep::cursor(horizontal_index)<horizontal_edges@.len(),horizontal_index!=65535 ==> horizontal_edges@[horizontal_index as int].end()==x,
            decreases sweep::cursor(horizontal_index)+1,
        ))]
        while horizontal_index != UNMATCHED_U16 {
            horizontal_index = horizontal_event::<false>(
                horizontal_edges,
                #[cfg(verus_only)]
                vertical_edges,
                scratch,
                &mut active_y,
                #[cfg(verus_only)]
                x,
                horizontal_index,
            );
        }
        proof! {sweep::phase_end(horizontal_edges@,scratch.active_horizontal@,active_y,x as int,false);}
        x += 1;
    }
    proof! {
        sweep::sweep_finished(horizontal_edges@,vertical_edges@,sweep::source(scratch),scratch.active_horizontal@,active_y);
        assert forall|p: int|0<=p<scratch.edge_count implies (#[trigger] scratch.edge_buffer@[p]).left<horizontal_edges@.len() by {
            assert(sweep::source(scratch)[p]==scratch.edge_buffer@[p]);
        }
    }
    debug_assert_eq!(active_y, 0);
}
