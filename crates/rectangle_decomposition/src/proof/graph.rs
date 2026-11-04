//! 右度数计数排序及完整 CSR 的组合语义。
use super::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) open spec fn chords_valid(chords: Seq<ValuedChord>, vertical: bool) -> bool {
    (forall|i: int| 0 <= i < chords.len() ==> (#[trigger] chords[i]).well_formed(vertical))
        && forall|i: int, j: int| #![auto] 0 <= i < j < chords.len() && chords[i].line() == chords[j].line()
            ==> chords[i].end() < chords[j].start() || chords[j].end() < chords[i].start()
}

pub(crate) open spec fn graph_chords_valid(horizontal: Seq<ValuedChord>, vertical: Seq<ValuedChord>) -> bool {
    chords_valid(horizontal, false) && chords_valid(vertical, true)
}

pub(crate) open spec fn closed_conflict(horizontal: ValuedChord, vertical: ValuedChord) -> bool {
    horizontal.start() <= vertical.line() <= horizontal.end()
        && vertical.start() <= horizontal.line() <= vertical.end()
}

pub(crate) open spec fn graph_exact(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,offsets: Seq<u16>,edges: Seq<u16>) -> bool {
    offsets.len()==horizontal.len()+1 && forall|h: int,v: int|0<=h<horizontal.len() && 0<=v<vertical.len()
        ==> (#[trigger] crate::optimal::matching::proof::edge_in(offsets,edges,h,v) <==>closed_conflict(horizontal[h],vertical[v]))
}

pub(crate) open spec fn less(degrees: Seq<u8>, end: int, bound: int) -> nat
    recommends 0 <= end <= degrees.len(),
    decreases end,
{
    if end <= 0 { 0 } else {
        less(degrees, end - 1, bound) + if degrees[end - 1] < bound { 1nat } else { 0nat }
    }
}

pub(crate) open spec fn equal(degrees: Seq<u8>, end: int, degree: int) -> nat
    recommends 0 <= end <= degrees.len(),
    decreases end,
{
    if end <= 0 { 0 } else {
        equal(degrees, end - 1, degree) + if degrees[end - 1] == degree { 1nat } else { 0nat }
    }
}

pub(crate) open spec fn degree_domain(degrees: Seq<u8>) -> bool {
    degrees.len() <= 1953 && forall|i: int| 0 <= i < degrees.len() ==> #[trigger] degrees[i] < 64
}

pub(crate) open spec fn rank(degrees: Seq<u8>, right: int) -> int {
    (less(degrees, degrees.len() as int, degrees[right] as int)
        + equal(degrees, right, degrees[right] as int)) as int
}

pub(crate) open spec fn histogram(degrees: Seq<u8>, counts: Seq<usize>, end: int) -> bool {
    0 < counts.len() <= 64 && 0 <= end <= degrees.len()
        && forall|d: int| 0 <= d < counts.len() ==> #[trigger] counts[d] == equal(degrees, end, d)
}

pub(crate) open spec fn prefix_histogram(degrees: Seq<u8>, counts: Seq<usize>, next: int) -> bool {
    0 < counts.len() <= 64 && 0 <= next <= counts.len()
        && forall|d: int| 0 <= d < counts.len() ==> #[trigger] counts[d] == if d < next {
            less(degrees, degrees.len() as int, d)
        } else { equal(degrees, degrees.len() as int, d) }
}

pub(crate) open spec fn scatter_counters(degrees: Seq<u8>, counts: Seq<usize>, end: int) -> bool {
    0 < counts.len() <= 64 && 0 <= end <= degrees.len()
        && forall|d: int| 0 <= d < counts.len() ==> #[trigger] counts[d]
            == less(degrees, degrees.len() as int, d) + equal(degrees, end, d)
}

pub(crate) open spec fn order_prefix(degrees: Seq<u8>, order: Seq<u32>, end: int) -> bool {
    order.len() == degrees.len() && 0 <= end <= degrees.len()
        && (forall|i: int| 0 <= i < order.len() ==> #[trigger] order[i] < degrees.len())
        && (forall|right: int| 0 <= right < end ==> #[trigger] order[rank(degrees, right)] == right)
}

pub(crate) open spec fn ordered(degrees: Seq<u8>, order: Seq<u32>) -> bool {
    order_prefix(degrees, order, degrees.len() as int) && order.no_duplicates()
        && forall|right: int| 0 <= right < degrees.len() ==> #[trigger] order.contains(right as u32)
}

pub(crate) proof fn count_bounds(degrees: Seq<u8>, end: int, first: int, last: int)
    requires 0 <= end <= degrees.len(), first <= last,
    ensures less(degrees, end, first) <= less(degrees, end, last) <= end,
        equal(degrees, end, first) <= end,
        first <= 0 ==> less(degrees, end, first) == 0,
        less(degrees, end, first + 1) == less(degrees, end, first) + equal(degrees, end, first),
    decreases end,
{
    if end > 0 { count_bounds(degrees, end - 1, first, last); }
}

pub(crate) proof fn equal_monotone(degrees: Seq<u8>, start: int, end: int, degree: int)
    requires 0 <= start <= end <= degrees.len(),
    ensures equal(degrees, start, degree) <= equal(degrees, end, degree),
    decreases end - start,
{
    if end > start { equal_monotone(degrees, start, end - 1, degree); }
}

pub(crate) proof fn rank_bound(degrees: Seq<u8>, right: int)
    requires 0 <= right < degrees.len(),
    ensures 0 <= rank(degrees, right) < degrees.len(),
{
    let d = degrees[right] as int;
    equal_monotone(degrees, right + 1, degrees.len() as int, d);
    count_bounds(degrees, degrees.len() as int, d, d + 1);
    reveal_with_fuel(equal, 1);
}

pub(crate) proof fn rank_distinct(degrees: Seq<u8>, first: int, second: int)
    requires 0 <= first < second < degrees.len(),
    ensures rank(degrees, first) != rank(degrees, second),
{
    let a = degrees[first] as int;
    let b = degrees[second] as int;
    if a == b {
        equal_monotone(degrees, first + 1, second, a);
        reveal_with_fuel(equal, 1);
    } else if a < b {
        equal_monotone(degrees, first + 1, degrees.len() as int, a);
        count_bounds(degrees, degrees.len() as int, a, a + 1);
        count_bounds(degrees, degrees.len() as int, a + 1, b);
        reveal_with_fuel(equal, 1);
    } else {
        equal_monotone(degrees, second + 1, degrees.len() as int, b);
        count_bounds(degrees, degrees.len() as int, b, b + 1);
        count_bounds(degrees, degrees.len() as int, b + 1, a);
        reveal_with_fuel(equal, 1);
    }
}

pub(crate) proof fn histogram_step(degrees: Seq<u8>, before: Seq<usize>, after: Seq<usize>, end: int)
    requires histogram(degrees, before, end), end < degrees.len(), degrees[end] < before.len(),
        after == before.update(degrees[end] as int, (before[degrees[end] as int] + 1) as usize),
        before[degrees[end] as int] < usize::MAX,
    ensures histogram(degrees, after, end + 1),
{
    assert forall|d: int| 0 <= d < after.len() implies #[trigger] after[d] == equal(degrees, end + 1, d) by {
        reveal_with_fuel(equal, 1);
    }
}

pub(crate) proof fn prefix_step(degrees: Seq<u8>, before: Seq<usize>, after: Seq<usize>, next: int)
    requires prefix_histogram(degrees, before, next), next < before.len(),
        after == before.update(next, less(degrees, degrees.len() as int, next) as usize),
        degrees.len() <= 1953,
    ensures prefix_histogram(degrees, after, next + 1),
{
    count_bounds(degrees, degrees.len() as int, next, next);
    assert forall|d: int| 0 <= d < after.len() implies #[trigger] after[d] == if d < next + 1 {
        less(degrees, degrees.len() as int, d)
    } else { equal(degrees, degrees.len() as int, d) } by { }
}

pub(crate) proof fn scatter_step(degrees: Seq<u8>, before: Seq<usize>, after: Seq<usize>,
    order_before: Seq<u32>, order_after: Seq<u32>, right: int)
    requires degree_domain(degrees), scatter_counters(degrees, before, right),
        order_prefix(degrees, order_before, right), right < degrees.len(), degrees[right] < before.len(),
        after == before.update(degrees[right] as int, (before[degrees[right] as int] + 1) as usize),
        order_after == order_before.update(rank(degrees, right), right as u32),
        before[degrees[right] as int] < usize::MAX,
    ensures scatter_counters(degrees, after, right + 1), order_prefix(degrees, order_after, right + 1),
{
    rank_bound(degrees, right);
    assert forall|d: int| 0 <= d < after.len() implies #[trigger] after[d]
        == less(degrees, degrees.len() as int, d) + equal(degrees, right + 1, d) by {
        reveal_with_fuel(equal, 1);
    }
    assert forall|i: int| 0 <= i < right + 1 implies #[trigger] order_after[rank(degrees, i)] == i by {
        if i < right { rank_distinct(degrees, i, right); }
    }
}

pub(crate) proof fn complete_order(degrees: Seq<u8>, order: Seq<u32>)
    requires degree_domain(degrees), order_prefix(degrees, order, degrees.len() as int),
    ensures ordered(degrees, order),
{
    assert forall|right: int| 0 <= right < degrees.len() implies #[trigger] order.contains(right as u32) by {
        rank_bound(degrees, right);
        assert(order[rank(degrees, right)] == right as u32);
        assert(exists|i: int| 0 <= i < order.len() && order[i] == right as u32);
    }
    let ids = Seq::new(degrees.len(), |i: int| i as u32);
    assert(ids.no_duplicates());
    ids.unique_seq_to_set();
    assert(order.to_set() =~= ids.to_set()) by {
        assert forall|right: u32| order.to_set().contains(right) implies ids.to_set().contains(right) by {
            let i = choose|i: int| 0 <= i < order.len() && order[i] == right;
            assert(ids[right as int] == right);
        }
        assert forall|right: u32| ids.to_set().contains(right) implies order.to_set().contains(right) by {
            let i = choose|i: int| 0 <= i < ids.len() && ids[i] == right;
            assert(order.contains(right));
        }
    }
    order.lemma_no_dup_set_cardinality();
}

pub(crate) open spec fn source_run(edges: Seq<SparseEdge>, right: int, start: int, count: int) -> bool {
    0 <= start && 0 <= count && start + count <= edges.len() && (count == 0 ==> start == 0)
        && forall|i: int| 0 <= i < edges.len() ==> ((#[trigger] edges[i]).right == right
            <==> start <= i < start + count)
}

pub(crate) open(super) spec fn finalize_input(scratch: &ConflictFinalizeScratch, left: usize, right: usize) -> bool {
    left <= 1953 && right <= 1953 && scratch.edge_count <= 3969
        && conflict_buffers_valid(scratch) && scratch.right_degrees@.len() == right
        && source_groups(scratch.edge_buffer@.take(scratch.edge_count as int), scratch.right_degrees@)
        && forall|i: int| 0 <= i < scratch.edge_count ==> (#[trigger] scratch.edge_buffer@[i]).left < left
}

pub(crate) open(in crate::optimal) spec fn conflict_buffers_valid(scratch: &ConflictFinalizeScratch) -> bool {
    scratch.right_degrees.valid() && scratch.adjacency_offsets.valid() && scratch.adjacency_edges.valid()
        && scratch.right_layout.valid() && scratch.next_offsets.valid()
}

pub(crate) open spec fn run_start(edges: Seq<SparseEdge>,right: int,count: int) -> int {
    choose|start: int|source_run(edges,right,start,count)
}

pub(crate) proof fn run_start_exists(edges: Seq<SparseEdge>,right: int,count: int)
    requires exists|start: int|#[trigger] source_run(edges,right,start,count),
    ensures source_run(edges,right,run_start(edges,right,count),count),
{}

pub(crate) open spec fn source_groups(edges: Seq<SparseEdge>, degrees: Seq<u8>) -> bool {
    degree_domain(degrees)
        && (forall|i: int| 0 <= i < edges.len() ==> (#[trigger] edges[i]).right < degrees.len())
        && forall|right: int| 0 <= right < degrees.len() ==>
            #[trigger] source_run(edges, right, run_start(edges,right,degrees[right] as int), (#[trigger] degrees[right]) as int)
}

/// 选择规范起点与原存在式等价；不添加连续组假设。
pub(crate) proof fn run_start_equivalent(edges: Seq<SparseEdge>,right: int,count: int)
    ensures source_run(edges,right,run_start(edges,right,count),count) <==> exists|start: int|#[trigger] source_run(edges,right,start,count),
{
    hide(source_run);hide(run_start);
    if exists|start: int|#[trigger] source_run(edges,right,start,count) {run_start_exists(edges,right,count);}
}

pub(crate) proof fn group_at(edges: Seq<SparseEdge>,degrees: Seq<u8>,right: int)
    requires source_groups(edges,degrees),0<=right<degrees.len(),
    ensures source_run(edges,right,run_start(edges,right,degrees[right] as int),degrees[right] as int),
{
    hide(run_start);hide(source_run);
    let count=degrees[right] as int;assert(count<64);
}

pub(crate) open spec fn group_starts(edges: Seq<SparseEdge>, degrees: Seq<u8>, starts: Seq<u32>) -> bool {
    starts.len() == degrees.len() && forall|right: int| 0 <= right < degrees.len()
        ==> source_run(edges, right, (#[trigger] starts[right]) as int, degrees[right] as int)
}

pub(crate) proof fn starts_complete(edges: Seq<SparseEdge>, degrees: Seq<u8>, starts: Seq<u32>)
    requires source_groups(edges, degrees), starts.len() == degrees.len(),
        crate::chord_graph::proof::starts_match(edges, starts, edges.len() as int),
    ensures group_starts(edges, degrees, starts),
{
    assert forall|right: int| 0 <= right < degrees.len() implies
        source_run(edges, right, #[trigger] starts[right] as int, degrees[right] as int) by {
        let start = run_start(edges,right,degrees[right] as int);
        group_at(edges,degrees,right);
        if degrees[right] > 0 {
            assert(edges[start].right == right);
            assert(starts[right] <= start);
            assert(edges[starts[right] as int].right == right);
            assert(start <= starts[right]);
        } else {
            assert(forall|i: int| 0 <= i < edges.len() ==> (#[trigger] edges[i]).right != right);
            assert(starts[right] == 0);
        }
    }
}

pub(crate) proof fn groups_contiguous(edges: Seq<SparseEdge>, degrees: Seq<u8>)
    requires source_groups(edges, degrees),
    ensures crate::chord_graph::proof::grouped_right(edges),
{
    assert forall|i: int, j: int, k: int| #![auto] 0 <= i < j < k < edges.len()
        && edges[i].right == edges[k].right implies edges[j].right == edges[i].right by {
        let r = edges[i].right as int;
        let start = run_start(edges,r,degrees[r] as int);
        group_at(edges,degrees,r);
    }
}

pub(crate) open spec fn pair_count(edges: Seq<SparseEdge>, end: int, left: int, right: int) -> int
    recommends 0 <= end <= edges.len(),
    decreases end,
{
    if end <= 0 { 0 } else {
        pair_count(edges, end - 1, left, right)
            + if edges[end - 1].left == left && edges[end - 1].right == right { 1int } else { 0int }
    }
}

pub(crate) open spec fn picked(edges: Seq<SparseEdge>, end: int, done: Set<u32>, left: int) -> int
    recommends 0 <= end <= edges.len(),
    decreases end,
{
    if end <= 0 { 0 } else {
        picked(edges, end - 1, done, left)
            + if edges[end - 1].left == left && done.contains(edges[end - 1].right as u32) { 1int } else { 0int }
    }
}

pub(crate) proof fn picked_bound(edges: Seq<SparseEdge>, end: int, done: Set<u32>, left: int)
    requires 0 <= end <= edges.len(),
    ensures 0 <= picked(edges, end, done, left) <= crate::chord_graph::proof::equal_count(edges, end, left),
    decreases end,
{
    if end > 0 { picked_bound(edges, end - 1, done, left); }
    reveal_with_fuel(crate::chord_graph::proof::count_below, 1);
}

pub(crate) proof fn picked_empty(edges: Seq<SparseEdge>, end: int, left: int)
    requires 0 <= end <= edges.len(),
    ensures picked(edges, end, Set::empty(), left) == 0,
    decreases end,
{
    if end > 0 { picked_empty(edges, end - 1, left); }
}

pub(crate) proof fn picked_insert(edges: Seq<SparseEdge>, end: int, done: Set<u32>, left: int, right: u32)
    requires 0 <= end <= edges.len(), !done.contains(right),
    ensures picked(edges, end, done.insert(right), left)
        == picked(edges, end, done, left) + pair_count(edges, end, left, right as int),
    decreases end,
{
    if end > 0 { picked_insert(edges, end - 1, done, left, right); }
}

pub(crate) proof fn picked_all(edges: Seq<SparseEdge>, end: int, done: Set<u32>, left: int)
    requires 0 <= end <= edges.len(),
        forall|i: int| 0 <= i < end ==> done.contains((#[trigger] edges[i]).right as u32),
    ensures picked(edges, end, done, left) == crate::chord_graph::proof::equal_count(edges, end, left),
    decreases end,
{
    if end > 0 { picked_all(edges, end - 1, done, left); }
    reveal_with_fuel(crate::chord_graph::proof::count_below, 1);
}

pub(crate) proof fn pair_prefix(first: Seq<SparseEdge>, second: Seq<SparseEdge>, end: int, left: int, right: int)
    requires 0 <= end <= first.len(), end <= second.len(),
        forall|i: int| 0 <= i < end ==> #[trigger] first[i] == #[trigger] second[i],
    ensures pair_count(first, end, left, right) == pair_count(second, end, left, right),
    decreases end,
{
    if end > 0 { pair_prefix(first, second, end - 1, left, right); }
}

pub(crate) proof fn pair_split(edges: Seq<SparseEdge>, end: int, split: int, left: int, right: int)
    requires 0 <= split <= end <= edges.len(),
    ensures pair_count(edges, end, left, right) == pair_count(edges, split, left, right)
        + pair_count(edges.subrange(split, end), end - split, left, right),
    decreases end - split,
{
    if end > split {
        pair_split(edges, end - 1, split, left, right);
        let before = edges.subrange(split, end - 1);
        let current = edges.subrange(split, end);
        pair_prefix(before, current, end - split - 1, left, right);
        assert(current[end - split - 1] == edges[end - 1]);
    }
}

pub(crate) proof fn pair_same(edges: Seq<SparseEdge>, end: int, left: int, right: int, same: bool)
    requires 0 <= end <= edges.len(),
        forall|i: int| 0 <= i < end ==> ((#[trigger] edges[i]).right == right) == same,
    ensures pair_count(edges, end, left, right) == if same {
        crate::chord_graph::proof::equal_count(edges, end, left)
    } else { 0 },
    decreases end,
{
    if end > 0 { pair_same(edges, end - 1, left, right, same); }
    reveal_with_fuel(crate::chord_graph::proof::count_below, 1);
}

pub(crate) proof fn group_count(edges: Seq<SparseEdge>, left: int, right: int, start: int, count: int)
    requires source_run(edges, right, start, count),
    ensures pair_count(edges, edges.len() as int, left, right)
        == crate::chord_graph::proof::equal_count(edges.subrange(start, start + count), count, left),
{
    let rest = edges.subrange(start, edges.len() as int);
    let group = edges.subrange(start, start + count);
    let before = edges.subrange(0, start);
    let after = edges.subrange(start + count, edges.len() as int);
    pair_split(edges, edges.len() as int, start, left, right);
    pair_split(rest, rest.len() as int, count, left, right);
    pair_split(edges, start, 0, left, right);
    assert(rest.subrange(0, count) =~= group);
    assert(rest.subrange(count, rest.len() as int) =~= after);
    pair_prefix(edges, before, start, left, right);
    pair_prefix(rest, group, count, left, right);
    assert forall|i: int| 0 <= i < before.len() implies (#[trigger] before[i]).right != right by { }
    assert forall|i: int| 0 <= i < group.len() implies (#[trigger] group[i]).right == right by { }
    assert forall|i: int| 0 <= i < after.len() implies (#[trigger] after[i]).right != right by { }
    pair_same(before, before.len() as int, left, right, false);
    pair_same(group, group.len() as int, left, right, true);
    pair_same(after, after.len() as int, left, right, false);
}

pub(crate) open spec fn row_count(values: Seq<u16>, begin: int, end: int, right: int) -> int
    recommends 0 <= begin <= end <= values.len(),
    decreases end - begin,
{
    if end <= begin { 0 } else {
        row_count(values, begin, end - 1, right) + if values[end - 1] == right { 1int } else { 0int }
    }
}

pub(crate) proof fn row_prefix(before: Seq<u16>, after: Seq<u16>, begin: int, end: int, right: int)
    requires 0 <= begin <= end <= before.len(), end <= after.len(),
        forall|i: int| begin <= i < end ==> #[trigger] before[i] == #[trigger] after[i],
    ensures row_count(before, begin, end, right) == row_count(after, begin, end, right),
    decreases end - begin,
{
    if end > begin { row_prefix(before, after, begin, end - 1, right); }
}

pub(crate) proof fn row_append(before: Seq<u16>, after: Seq<u16>, begin: int, middle: int,
    end: int, value: int, right: int)
    requires 0 <= begin <= middle <= end <= before.len(), end <= after.len(),
        forall|i: int| begin <= i < end ==> #[trigger] after[i]
            == if middle <= i { value } else { before[i] as int },
    ensures row_count(after, begin, end, right) == row_count(before, begin, middle, right)
        + if right == value { end - middle } else { 0 },
    decreases end - middle,
{
    if end > middle {
        row_append(before, after, begin, middle, end - 1, value, right);
    } else {
        row_prefix(before, after, begin, middle, right);
    }
}

pub(crate) open spec fn csr_shape(edges: Seq<SparseEdge>, degrees: Seq<u8>, starts: Seq<u32>,
    order: Seq<u32>, offsets: Seq<u16>, length: nat) -> bool
{
    edges.len() <= 3969 && offsets.len() > 0 && offsets.len() <= 1954
        && length == edges.len() && source_groups(edges, degrees) && group_starts(edges, degrees, starts)
        && ordered(degrees, order)
        && (forall|i: int| 0 <= i < edges.len() ==> (#[trigger] edges[i]).left as int + 1 < offsets.len())
        && crate::chord_graph::proof::offsets_match(edges, offsets)
        && crate::chord_graph::proof::offsets_shape(edges, offsets)
}

pub(crate) open spec fn csr_source(edges: Seq<SparseEdge>, degrees: Seq<u8>, layout: Seq<u32>,
    offsets: Seq<u16>, length: nat) -> bool
{
    edges.len() <= 3969 && offsets.len() > 0 && offsets.len() <= 1954
        && length == edges.len() && source_groups(edges, degrees) && layout.len() == 2 * degrees.len()
        && crate::chord_graph::proof::starts_match(edges, layout.take(degrees.len() as int), edges.len() as int)
        && (forall|i: int| degrees.len() <= i < layout.len() ==> #[trigger] layout[i] < degrees.len())
        && (forall|i: int| 0 <= i < edges.len() ==> (#[trigger] edges[i]).left as int + 1 < offsets.len())
        && crate::chord_graph::proof::offsets_match(edges, offsets)
        && crate::chord_graph::proof::offsets_shape(edges, offsets)
}

pub(crate) open spec fn scattered(edges: Seq<SparseEdge>, order: Seq<u32>, offsets: Seq<u16>,
    cursors: Seq<u16>, output: Seq<u16>, next: int, right_size: nat) -> bool
{
    0 <= next <= order.len() && offsets.len() == cursors.len() + 1 && output.len() == edges.len()
        && forall|left: int| 0 <= left < cursors.len() ==>
            offsets[left] <= #[trigger] cursors[left] <= offsets[left + 1]
            && cursors[left] as int == offsets[left] as int
                + picked(edges, edges.len() as int, order.take(next).to_set(), left)
            && (forall|right: int| 0 <= right < right_size ==> row_count(output,
                offsets[left] as int, cursors[left] as int, right) == if order.take(next).contains(right as u32) {
                    pair_count(edges, edges.len() as int, left, right)
                } else { 0 })
            && (forall|i: int| offsets[left] <= i < cursors[left] ==> #[trigger] output[i] < right_size)
}

pub(crate) open spec fn csr_exact(source: Seq<SparseEdge>, offsets: Seq<u16>, output: Seq<u16>, right_size: nat) -> bool {
    crate::chord_graph::proof::offsets_match(source, offsets)
        && crate::chord_graph::proof::offsets_shape(source, offsets) && output.len() == source.len()
        && forall|left: int| #![trigger offsets[left]] 0 <= left < offsets.len() - 1 ==>
            (forall|right: int| 0 <= right < right_size ==> row_count(output,
                offsets[left] as int, offsets[left + 1] as int, right)
                    == pair_count(source, source.len() as int, left, right))
            && (forall|i: int| offsets[left] <= i < offsets[left + 1] ==> #[trigger] output[i] < right_size)
}

pub(crate) proof fn offsets_bounded(edges: Seq<SparseEdge>, offsets: Seq<u16>)
    requires crate::chord_graph::proof::offsets_match(edges, offsets),
    ensures forall|i: int| 0 <= i < offsets.len() ==> #[trigger] offsets[i] <= edges.len(),
{
    assert forall|i: int| 0 <= i < offsets.len() implies #[trigger] offsets[i] <= edges.len() by {
        crate::chord_graph::proof::count_bound(edges, edges.len() as int, i);
    }
}

pub(crate) proof fn scatter_begin(edges: Seq<SparseEdge>, degrees: Seq<u8>, starts: Seq<u32>, order: Seq<u32>,
    offsets: Seq<u16>, cursors: Seq<u16>, output: Seq<u16>)
    requires csr_shape(edges, degrees, starts, order, offsets, output.len()),
        cursors == offsets.take(offsets.len() as int - 1),
    ensures scattered(edges, order, offsets, cursors, output, 0, degrees.len()),
{
    assert(order.take(0).to_set() =~= Set::<u32>::empty());
    assert forall|left: int| 0 <= left < cursors.len() implies
        cursors[left] as int == offsets[left] as int
            + picked(edges, edges.len() as int, order.take(0).to_set(), left) by {
        picked_empty(edges, edges.len() as int, left);
    }
}

pub(crate) proof fn scatter_ready(edges: Seq<SparseEdge>, degrees: Seq<u8>, starts: Seq<u32>, order: Seq<u32>,
    offsets: Seq<u16>, cursors: Seq<u16>, output: Seq<u16>, next: int)
    requires csr_shape(edges, degrees, starts, order, offsets, output.len()),
        scattered(edges, order, offsets, cursors, output, next, degrees.len()), next < order.len(),
    ensures crate::chord_graph::proof::scatter_inputs(
        edges.subrange(starts[order[next] as int] as int,
            starts[order[next] as int] as int + degrees[order[next] as int] as int),
        order[next] as u16, offsets, cursors, output.len()),
{
    let right = order[next] as int;
    let start = starts[right] as int;
    let count = degrees[right] as int;
    let done = order.take(next).to_set();
    let group = edges.subrange(start, start + count);
    offsets_bounded(edges, offsets);
    assert(!done.contains(right as u32));
    assert forall|i: int, j: int| 0 <= i <= j < offsets.len() implies
        #[trigger] offsets[i] <= #[trigger] offsets[j] by {
        crate::chord_graph::proof::count_monotone(edges, edges.len() as int, i, j);
    }
    assert forall|left: int| 0 <= left < cursors.len() implies offsets[left] <= #[trigger] cursors[left]
        && cursors[left] as int + crate::chord_graph::proof::equal_count(group, count, left) <= offsets[left + 1] by {
        picked_insert(edges, edges.len() as int, done, left, right as u32);
        picked_bound(edges, edges.len() as int, done.insert(right as u32), left);
        group_count(edges, left, right, start, count);
    }
}

pub(crate) proof fn scatter_next(edges: Seq<SparseEdge>, degrees: Seq<u8>, starts: Seq<u32>, order: Seq<u32>,
    offsets: Seq<u16>, before: Seq<u16>, previous: Seq<u16>, cursors: Seq<u16>, output: Seq<u16>, next: int)
    requires csr_shape(edges, degrees, starts, order, offsets, previous.len()),
        scattered(edges, order, offsets, before, previous, next, degrees.len()), next < order.len(),
        crate::chord_graph::proof::scatter_state(
            edges.subrange(starts[order[next] as int] as int,
                starts[order[next] as int] as int + degrees[order[next] as int] as int),
            order[next] as u16, offsets, before, previous, cursors, output,
            degrees[order[next] as int] as int),
    ensures scattered(edges, order, offsets, cursors, output, next + 1, degrees.len()),
{
    let right = order[next] as int;
    let start = starts[right] as int;
    let count = degrees[right] as int;
    let done = order.take(next).to_set();
    let group = edges.subrange(start, start + count);
    offsets_bounded(edges, offsets);
    assert(order.take(next + 1) =~= order.take(next).push(order[next]));
    assert(order.take(next + 1).to_set() =~= done.insert(right as u32));
    assert(!done.contains(right as u32));
    assert forall|left: int| 0 <= left < cursors.len() implies offsets[left] <= #[trigger] cursors[left] <= offsets[left + 1]
        && before[left] <= cursors[left]
        && cursors[left] as int == offsets[left] as int
            + picked(edges, edges.len() as int, order.take(next + 1).to_set(), left) by {
        group_count(edges, left, right, start, count);
        crate::chord_graph::proof::equal_count_monotone(group, 0, count, left);
        picked_insert(edges, edges.len() as int, done, left, right as u32);
        picked_bound(edges, edges.len() as int, done.insert(right as u32), left);
    }
    assert forall|left: int, query: int| 0 <= left < cursors.len() && 0 <= query < degrees.len() implies
        #[trigger] row_count(output, offsets[left] as int, cursors[left] as int, query)
            == if order.take(next + 1).contains(query as u32) { pair_count(edges, edges.len() as int, left, query) } else { 0 } by {
        group_count(edges, left, right, start, count);
        assert(0 <= offsets[left] <= before[left] <= cursors[left] <= previous.len());
        assert forall|i: int| offsets[left] <= i < cursors[left] implies #[trigger] output[i]
            == if before[left] <= i { right } else { previous[i] as int } by { }
        row_append(previous, output, offsets[left] as int, before[left] as int, cursors[left] as int, right, query);
    }
    assert forall|left: int, i: int| #![trigger cursors[left], output[i]]
        0 <= left < cursors.len() && offsets[left] <= i < cursors[left]
        implies output[i] < degrees.len() by { }
}

pub(crate) proof fn scatter_complete(edges: Seq<SparseEdge>, degrees: Seq<u8>, starts: Seq<u32>, order: Seq<u32>,
    offsets: Seq<u16>, cursors: Seq<u16>, output: Seq<u16>)
    requires csr_shape(edges, degrees, starts, order, offsets, output.len()),
        scattered(edges, order, offsets, cursors, output, order.len() as int, degrees.len()),
    ensures csr_exact(edges, offsets, output, degrees.len()),
        forall|left: int| 0 <= left < cursors.len() ==> #[trigger] cursors[left] == offsets[left + 1],
{
    assert(order.take(order.len() as int) == order);
    assert forall|left: int| 0 <= left < cursors.len() implies #[trigger] cursors[left] == offsets[left + 1] by {
        assert forall|i: int| 0 <= i < edges.len() implies order.to_set().contains((#[trigger] edges[i]).right as u32) by { }
        picked_all(edges, edges.len() as int, order.to_set(), left);
    }
    assert forall|left: int, right: int| 0 <= left < offsets.len() - 1 && 0 <= right < degrees.len() implies
        #[trigger] row_count(output, offsets[left] as int, offsets[left + 1] as int, right)
            == pair_count(edges, edges.len() as int, left, right) by {
        assert(order.contains(right as u32));
        assert(cursors[left] == offsets[left + 1]);
        assert(row_count(output, offsets[left] as int, cursors[left] as int, right)
            == pair_count(edges, edges.len() as int, left, right));
    }
    assert forall|left: int, i: int| #![trigger offsets[left], output[i]]
        0 <= left < offsets.len() - 1 && offsets[left] <= i < offsets[left + 1]
        implies output[i] < degrees.len() by { assert(cursors[left] == offsets[left + 1]); }
}

pub(crate) open spec fn degree_sum(degrees: Seq<u8>, end: int) -> int
    recommends 0 <= end <= degrees.len(),
    decreases end,
{
    if end <= 0 { 0 } else { degree_sum(degrees, end - 1) + degrees[end - 1] as int }
}

pub(crate) proof fn degree_sum_bound(degrees: Seq<u8>, end: int)
    requires degree_domain(degrees), 0 <= end <= degrees.len(),
    ensures 0 <= degree_sum(degrees, end) <= 63 * end,
    decreases end,
{
    if end > 0 { degree_sum_bound(degrees, end - 1); }
}

pub(crate) open spec fn right_count(edges: Seq<SparseEdge>, end: int, right: int) -> int
    recommends 0 <= end <= edges.len(),
    decreases end,
{
    if end <= 0 { 0 } else {
        right_count(edges, end - 1, right) + if edges[end - 1].right == right { 1int } else { 0int }
    }
}

pub(crate) proof fn run_count(edges: Seq<SparseEdge>, right: int, start: int, count: int, end: int)
    requires source_run(edges, right, start, count), 0 <= end <= edges.len(),
    ensures right_count(edges, end, right) == if end <= start { 0 }
        else if end <= start + count { end - start } else { count },
    decreases end,
{
    if end > 0 { run_count(edges, right, start, count, end - 1); }
}

pub(crate) open spec fn right_sum(edges: Seq<SparseEdge>, end: int, limit: int) -> int
    recommends 0 <= end <= edges.len(), 0 <= limit,
    decreases limit,
{
    if limit <= 0 { 0 } else { right_sum(edges, end, limit - 1) + right_count(edges, end, limit - 1) }
}

pub(crate) proof fn right_sum_step(edges: Seq<SparseEdge>, end: int, limit: int)
    requires 0 <= end < edges.len(), 0 <= limit,
    ensures right_sum(edges, end + 1, limit) == right_sum(edges, end, limit)
        + if edges[end].right < limit { 1int } else { 0int },
    decreases limit,
{
    if limit > 0 { right_sum_step(edges, end, limit - 1); }
}

pub(crate) proof fn right_sum_all(edges: Seq<SparseEdge>, end: int, limit: int)
    requires 0 <= end <= edges.len(), 0 <= limit,
        forall|i: int| 0 <= i < end ==> (#[trigger] edges[i]).right < limit,
    ensures right_sum(edges, end, limit) == end,
    decreases end,
{
    if end > 0 {
        right_sum_all(edges, end - 1, limit);
        right_sum_step(edges, end - 1, limit);
    } else {
        right_sum_zero(edges, limit);
    }
}

pub(crate) proof fn right_sum_zero(edges: Seq<SparseEdge>, limit: int)
    requires 0 <= limit,
    ensures right_sum(edges, 0, limit) == 0,
    decreases limit,
{
    if limit > 0 { right_sum_zero(edges, limit - 1); }
}

pub(crate) proof fn degree_sum_source(edges: Seq<SparseEdge>, degrees: Seq<u8>, end: int)
    requires source_groups(edges, degrees), 0 <= end <= degrees.len(),
    ensures degree_sum(degrees, end) == right_sum(edges, edges.len() as int, end),
    decreases end,
{
    if end > 0 {
        degree_sum_source(edges, degrees, end - 1);
        let start = run_start(edges,end-1,degrees[end-1] as int);
        run_count(edges, end - 1, start, degrees[end - 1] as int, edges.len() as int);
    }
}

pub(crate) proof fn groups_total(edges: Seq<SparseEdge>, degrees: Seq<u8>)
    requires source_groups(edges, degrees),
    ensures degree_sum(degrees, degrees.len() as int) == edges.len(),
{
    degree_sum_source(edges, degrees, degrees.len() as int);
    right_sum_all(edges, edges.len() as int, degrees.len() as int);
}

#[cfg(reject_csr_order_duplicate)]
fn reject_csr_order_duplicate() {
    let degrees = [2u8, 1, 2];
    let mut order = [0u32; 3];
    sort_rights(&degrees, &mut order);
    assert(order[0] == order[1]);
}

fn csr_two_groups_with_unused_right() {
    let mut scratch = ConflictFinalizeScratch::new();
    scratch.right_degrees.push(1);
    scratch.right_degrees.push(0);
    scratch.right_degrees.push(1);
    scratch.edge_buffer[0] = SparseEdge { left: 0, right: 2 };
    scratch.edge_buffer[1] = SparseEdge { left: 1, right: 0 };
    scratch.edge_count = 2;
    #[cfg(reject_csr_wrong_degree)]
    { *get_mut(&mut scratch.right_degrees, 2) = 2; }
    let ghost source = scratch.edge_buffer@.take(2);
    assert(source_run(source, 0, 1, 1));
    assert(source_run(source, 1, 0, 0));
    assert(source_run(source, 2, 0, 1));
    assert forall|right: int| 0 <= right < 3 implies exists|start: int|
        #[trigger] source_run(source, right, start, (#[trigger] scratch.right_degrees@[right]) as int) by {
        if right == 0 { assert(source_run(source, right, 1, scratch.right_degrees@[right] as int)); }
        else { assert(source_run(source, right, 0, scratch.right_degrees@[right] as int)); }
    }
    assert(scratch.right_degrees@ =~= seq![1u8, 0, 1]);
    assert(source_groups(source, scratch.right_degrees@));
    assert(scratch.right_degrees.valid());
    assert(scratch.adjacency_offsets.valid());
    assert(scratch.adjacency_edges.valid());
    assert(scratch.right_layout.valid());
    assert(scratch.next_offsets.valid());
    assert(forall|i: int| 0 <= i < 2 ==> (#[trigger] scratch.edge_buffer@[i]).left < 2);
    assert(finalize_input(&scratch, 2, 3));
    let (csr, degrees) = build_sparse_adjacency_in_scratch(2, 3, &mut scratch);
    reveal_with_fuel(crate::chord_graph::proof::count_below, 3);
    assert(csr.offsets@.len() == 3);
    assert(csr.offsets[0] == 0);
    assert(csr.offsets[1] == 1);
    assert(csr.offsets[2] == 2);
    assert(csr.offsets@ =~= seq![0u16, 1, 2]);
    assert(degrees@ =~= seq![1u8, 0, 1]);
    assert(row_count(csr.edges@, 0, 1, 2) == pair_count(source, 2, 0, 2));
    assert(row_count(csr.edges@, 1, 2, 0) == pair_count(source, 2, 1, 0));
    reveal_with_fuel(pair_count, 3);
    reveal_with_fuel(row_count, 2);
    assert(csr.edges[0] == 2);
    assert(csr.edges[1] == 0);
    #[cfg(reject_csr_pair_loss)]
    assert(csr.edges[0] == 0);
    #[cfg(reject_csr_source_frame)]
    assert(scratch.edge_count == 0);
}

}
