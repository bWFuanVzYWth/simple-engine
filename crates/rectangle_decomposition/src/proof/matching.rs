//! CSR 双向视图、匹配与反向搜索的组合规范。
use super::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) open spec fn valid_indices(ids: Seq<u16>, limit: nat) -> bool {
    ids.no_duplicates() && forall|i: int| 0 <= i < ids.len() ==> #[trigger] ids[i] < limit
}

pub(crate) open spec fn index_set(ids: Seq<u16>) -> Set<int> {
    Seq::new(ids.len(), |i: int| ids[i] as int).to_set()
}

pub(crate) open spec fn index_independent(horizontal: Seq<ValuedChord>, vertical: Seq<ValuedChord>, left: Set<int>, right: Set<int>) -> bool {
    (forall|i: int| #[trigger] left.contains(i) ==> 0 <= i < horizontal.len())
        && (forall|i: int| #[trigger] right.contains(i) ==> 0 <= i < vertical.len())
        && forall|h: int, v: int| left.contains(h) && right.contains(v)
            ==> !crate::optimal::graph::proof::closed_conflict(horizontal[h], vertical[v])
}

pub(crate) open spec fn maximum_selection(horizontal: Seq<ValuedChord>, vertical: Seq<ValuedChord>, left: Seq<u16>, right: Seq<u16>) -> bool {
    valid_indices(left, horizontal.len()) && valid_indices(right, vertical.len())
        && index_independent(horizontal, vertical, index_set(left), index_set(right))
        && forall|other_left: Set<int>, other_right: Set<int>|
            #[trigger] index_independent(horizontal, vertical, other_left, other_right)
                ==> other_left.len() + other_right.len() <= left.len() + right.len()
}

pub(crate) open spec fn index_domain(size: nat) -> Set<int> { Seq::new(size, |i: int| i).to_set() }

pub(crate) open spec fn index_maximum(horizontal: Seq<ValuedChord>, vertical: Seq<ValuedChord>, left: Set<int>, right: Set<int>) -> bool {
    index_independent(horizontal, vertical, left, right) && forall|other_left: Set<int>, other_right: Set<int>|
        #[trigger] index_independent(horizontal, vertical, other_left, other_right)
            ==> other_left.len() + other_right.len() <= left.len() + right.len()
}

/// 与具体最大匹配无关的实际选择：全 V 已最优时取全 V，否则取所有最优解 V 部分的下界。
pub(crate) open spec fn canonical_selection(horizontal: Seq<ValuedChord>, vertical: Seq<ValuedChord>, left: Seq<u16>, right: Seq<u16>) -> bool {
    maximum_selection(horizontal, vertical, left, right)
        && if index_maximum(horizontal, vertical, Set::empty(), index_domain(vertical.len())) {
            left.len() == 0 && index_set(right) == index_domain(vertical.len())
        } else {
            forall|other_left: Set<int>, other_right: Set<int>| #[trigger] index_maximum(horizontal, vertical, other_left, other_right)
                ==> index_set(right).subset_of(other_right)
        }
}

pub(crate) proof fn maximum_sets(horizontal: Seq<ValuedChord>, vertical: Seq<ValuedChord>, left: Seq<u16>, right: Seq<u16>)
    requires maximum_selection(horizontal, vertical, left, right),
    ensures index_maximum(horizontal, vertical, index_set(left), index_set(right)),
{
    index_set_view(left); index_set_view(right);
}

pub(crate) open spec fn marks_certificate(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, selected: Seq<bool>, excluded: Seq<bool>) -> bool {
    matching_valid(offsets, edges, lefts, rights) && selected.len() == lefts.len() && excluded.len() == rights.len()
        && (forall|l: int| 0 <= l < lefts.len() && lefts[l] == u16::MAX ==> #[trigger] selected[l])
        && (forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> !#[trigger] excluded[r])
        && (forall|l: int| 0 <= l < lefts.len() && lefts[l] != u16::MAX ==> #[trigger] selected[l] == excluded[lefts[l] as int])
        && forall|l: int, r: int| #[trigger] edge_in(offsets, edges, l, r) ==> !(selected[l] && !excluded[r])
}

pub(crate) proof fn index_set_view(ids: Seq<u16>)
    ensures index_set(ids).len() <= ids.len(),
        ids.no_duplicates() ==> index_set(ids).len() == ids.len(),
        forall|id: int| #[trigger] index_set(ids).contains(id) <==> exists|i: int| 0 <= i < ids.len() && ids[i] as int == id,
{
    let mapped = Seq::new(ids.len(), |i: int| ids[i] as int);
    mapped.lemma_cardinality_of_set();
    if ids.no_duplicates() {
        assert(mapped.no_duplicates());
        mapped.unique_seq_to_set();
    }
    assert forall|id: int| #[trigger] index_set(ids).contains(id) <==> exists|i: int| 0 <= i < ids.len() && ids[i] as int == id by {
        if index_set(ids).contains(id) {
            let i = choose|i: int| 0 <= i < mapped.len() && mapped[i] == id;
            assert(ids[i] as int == id);
        }
        if exists|i: int| 0 <= i < ids.len() && ids[i] as int == id {
            let i = choose|i: int| 0 <= i < ids.len() && ids[i] as int == id;
            assert(mapped[i] == id);
            assert(mapped.contains(id));
        }
    }
}

pub(crate) open spec fn flag_ids(flags: Seq<bool>, value: bool, end: int) -> Seq<u16>
    decreases end,
{
    if end <= 0 { Seq::empty() } else {
        let before = flag_ids(flags, value, end - 1);
        if flags[end - 1] == value { before.push((end - 1) as u16) } else { before }
    }
}

pub(crate) proof fn flag_step(flags: Seq<bool>, value: bool, end: int)
    requires 0 <= end < flags.len(),
    ensures flag_ids(flags, value, end + 1) == if flags[end] == value { flag_ids(flags, value, end).push(end as u16) } else { flag_ids(flags, value, end) },
{}

pub(crate) proof fn flag_prefix(flags: Seq<bool>, value: bool, end: int)
    requires 0 <= end <= flags.len(), end <= 1953,
    ensures flag_ids(flags, value, end).len() <= end, valid_indices(flag_ids(flags, value, end), end as nat),
        forall|left: int| 0 <= left < end ==> (flags[left] == value <==> #[trigger] flag_ids(flags, value, end).contains(left as u16)),
        forall|i: int| 0 <= i < flag_ids(flags, value, end).len() ==> flags[#[trigger] flag_ids(flags, value, end)[i] as int] == value,
    decreases end,
{
    if end > 0 {
        flag_prefix(flags, value, end - 1);
        let previous = flag_ids(flags, value, end - 1);
        let current = flag_ids(flags, value, end);
        if flags[end - 1] == value {
            assert(!previous.contains((end - 1) as u16)) by {
                if previous.contains((end - 1) as u16) {
                    let i = choose|i: int| 0 <= i < previous.len() && previous[i] == (end - 1) as u16;
                }
            }
            assert forall|l: int| 0 <= l < end implies (flags[l] == value <==> #[trigger] current.contains(l as u16)) by {
                if l < end - 1 { assert(l as u16 != (end - 1) as u16); }
            }
        }
    }
}

pub(in crate::optimal) proof fn flags_set(flags: Seq<bool>, value: bool)
    requires flags.len() <= 1953,
    ensures valid_indices(flag_ids(flags, value, flags.len() as int), flags.len()),
        index_set(flag_ids(flags, value, flags.len() as int)) == crate::optimal::hk::cover_proof::chosen(flags, value),
{
    flag_prefix(flags, value, flags.len() as int);
    let ids = flag_ids(flags, value, flags.len() as int);
    index_set_view(ids);
    crate::optimal::hk::cover_proof::vertices_view(flags.len());
    assert(index_set(ids) =~= crate::optimal::hk::cover_proof::chosen(flags, value)) by {
        assert forall|l: int| index_set(ids).contains(l) <==> crate::optimal::hk::cover_proof::chosen(flags, value).contains(l) by {
            if index_set(ids).contains(l) {
                let i = choose|i: int| 0 <= i < ids.len() && ids[i] as int == l;
            }
            if 0 <= l < flags.len() && flags[l] == value {
                assert(ids.contains(l as u16));
                let i = choose|i: int| 0 <= i < ids.len() && ids[i] == l as u16;
            }
        }
    }
}

pub(in crate::optimal) proof fn full_indices(size: nat)
    requires size <= 1953,
    ensures valid_indices(Seq::new(size, |i: int| i as u16), size),
        index_set(Seq::new(size, |i: int| i as u16)) == crate::optimal::hk::cover_proof::vertices(size),
{
    let ids = Seq::new(size, |i: int| i as u16);
    assert(ids.no_duplicates());
    index_set_view(ids);
    crate::optimal::hk::cover_proof::vertices_view(size);
    assert(index_set(ids) =~= crate::optimal::hk::cover_proof::vertices(size)) by {
        assert forall|l: int| index_set(ids).contains(l) <==> 0 <= l < size by {
            if index_set(ids).contains(l) {
                let i = choose|i: int| 0 <= i < ids.len() && ids[i] as int == l;
            }
            if 0 <= l < size { assert(ids[l] as int == l); }
        }
    }
}

pub(crate) proof fn full_selection(horizontal: Seq<ValuedChord>, vertical: Seq<ValuedChord>)
    requires horizontal.len() <= 1953, vertical.len() <= 1953,
        forall|h: int, v: int| 0 <= h < horizontal.len() && 0 <= v < vertical.len()
            ==> !crate::optimal::graph::proof::closed_conflict(#[trigger] horizontal[h], #[trigger] vertical[v]),
    ensures maximum_selection(horizontal, vertical, Seq::new(horizontal.len(), |i: int| i as u16), Seq::new(vertical.len(), |i: int| i as u16)),
{
    full_indices(horizontal.len()); full_indices(vertical.len());
    crate::optimal::hk::cover_proof::vertices_view(horizontal.len());
    crate::optimal::hk::cover_proof::vertices_view(vertical.len());
    let left_domain = crate::optimal::hk::cover_proof::vertices(horizontal.len());
    let right_domain = crate::optimal::hk::cover_proof::vertices(vertical.len());
    assert forall|other_left: Set<int>, other_right: Set<int>| #[trigger] index_independent(horizontal, vertical, other_left, other_right)
        implies other_left.len() + other_right.len() <= horizontal.len() + vertical.len() by {
        assert(other_left.subset_of(left_domain)); assert(other_right.subset_of(right_domain));
        vstd::set_lib::lemma_len_subset(other_left, left_domain);
        vstd::set_lib::lemma_len_subset(other_right, right_domain);
    }
}

pub(crate) proof fn full_selection_canonical(horizontal: Seq<ValuedChord>, vertical: Seq<ValuedChord>)
    requires horizontal.len() <= 1953, vertical.len() <= 1953,
        forall|h: int, v: int| 0 <= h < horizontal.len() && 0 <= v < vertical.len()
            ==> !crate::optimal::graph::proof::closed_conflict(#[trigger] horizontal[h], #[trigger] vertical[v]),
    ensures canonical_selection(horizontal, vertical, Seq::new(horizontal.len(), |i: int| i as u16), Seq::new(vertical.len(), |i: int| i as u16)),
{
    full_selection(horizontal, vertical);
    full_indices(horizontal.len()); full_indices(vertical.len());
    crate::optimal::hk::cover_proof::vertices_view(horizontal.len());
    crate::optimal::hk::cover_proof::vertices_view(vertical.len());
    let left = index_domain(horizontal.len()); let right = index_domain(vertical.len());
    maximum_sets(horizontal, vertical, Seq::new(horizontal.len(), |i: int| i as u16), Seq::new(vertical.len(), |i: int| i as u16));
    if index_maximum(horizontal, vertical, Set::empty(), right) {
        assert(horizontal.len() == 0);
    } else {
        assert forall|other_left: Set<int>, other_right: Set<int>| #[trigger] index_maximum(horizontal, vertical, other_left, other_right)
            implies right.subset_of(other_right) by {
            assert(other_left.subset_of(left)); assert(other_right.subset_of(right));
            vstd::set_lib::lemma_len_subset(other_left, left); vstd::set_lib::lemma_len_subset(other_right, right);
            assert(other_right.len() == right.len());
            vstd::set_lib::lemma_subset_equality(other_right, right);
        }
    }
}

pub(crate) proof fn empty_graph(horizontal: Seq<ValuedChord>, vertical: Seq<ValuedChord>, offsets: Seq<u16>, edges: Seq<u16>)
    requires crate::optimal::graph::proof::graph_exact(horizontal, vertical, offsets, edges), rows_valid(offsets, edges.len()), edges.len() == 0,
    ensures forall|h: int, v: int| 0 <= h < horizontal.len() && 0 <= v < vertical.len()
        ==> !crate::optimal::graph::proof::closed_conflict(#[trigger] horizontal[h], #[trigger] vertical[v]),
{
    assert forall|h: int, v: int| 0 <= h < horizontal.len() && 0 <= v < vertical.len()
        implies !crate::optimal::graph::proof::closed_conflict(#[trigger] horizontal[h], #[trigger] vertical[v]) by {
        row_bounds(offsets, edges.len(), h);
        assert(offsets[h] == 0 && offsets[h + 1] == 0);
        assert(edges.subrange(offsets[h] as int, offsets[h + 1] as int) =~= Seq::<u16>::empty());
        assert(!edge_in(offsets, edges, h, v));
    }
}

/// 真实 CSR、最终标记与实际输出编号的组合；不预设算法已经最大。
pub(crate) proof fn maximum_from_marks(horizontal: Seq<ValuedChord>, vertical: Seq<ValuedChord>, offsets: Seq<u16>, edges: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, selected: Seq<bool>, excluded: Seq<bool>)
    requires crate::optimal::graph::proof::graph_exact(horizontal, vertical, offsets, edges),
        horizontal.len() == lefts.len(), vertical.len() == rights.len(),
        marks_certificate(offsets, edges, lefts, rights, selected, excluded),
    ensures maximum_selection(horizontal, vertical, flag_ids(selected, true, selected.len() as int), flag_ids(excluded, false, excluded.len() as int)),
{
    hide(rows_valid); hide(edge_in);
    flags_set(selected, true); flags_set(excluded, false);
    let left_ids = flag_ids(selected, true, selected.len() as int);
    let right_ids = flag_ids(excluded, false, excluded.len() as int);
    let chosen_left = index_set(left_ids);
    let chosen_right = index_set(right_ids);
    index_set_view(left_ids); index_set_view(right_ids);
    crate::optimal::hk::cover_proof::vertices_view(horizontal.len());
    crate::optimal::hk::cover_proof::vertices_view(vertical.len());
    assert(index_independent(horizontal, vertical, chosen_left, chosen_right)) by {
        assert forall|h: int, v: int| chosen_left.contains(h) && chosen_right.contains(v)
            implies !crate::optimal::graph::proof::closed_conflict(horizontal[h], vertical[v]) by {
            assert(selected[h] && !excluded[v]);
            assert(edge_in(offsets, edges, h, v) <==> crate::optimal::graph::proof::closed_conflict(horizontal[h], vertical[v]));
        }
    }
    assert forall|other_left: Set<int>, other_right: Set<int>| #[trigger] index_independent(horizontal, vertical, other_left, other_right)
        implies other_left.len() + other_right.len() <= left_ids.len() + right_ids.len() by {
        assert(other_left.subset_of(crate::optimal::hk::cover_proof::vertices(lefts.len())));
        assert(other_right.subset_of(crate::optimal::hk::cover_proof::vertices(rights.len())));
        assert forall|l: int| 0 <= l < lefts.len() && lefts[l] != u16::MAX
            implies !(other_left.contains(l) && other_right.contains(#[trigger] lefts[l] as int)) by {
            assert(edge_in(offsets, edges, l, lefts[l] as int));
            assert(crate::optimal::graph::proof::closed_conflict(horizontal[l], vertical[lefts[l] as int]));
        }
        crate::optimal::hk::cover_proof::marks_bound(offsets, edges, lefts, rights, selected, excluded, other_left, other_right);
    }
}

impl MatchingScratch {
    pub(crate) open(super) spec fn selection_ready(&self) -> bool {
        buffers_valid(&self.hk) && crate::optimal::graph::proof::conflict_buffers_valid(&self.conflict.finalize)
            && self.selected_horizontal.valid() && self.selected_vertical.valid()
    }

    pub(crate) open(super) spec fn selected_horizontal_view(&self) -> Seq<u16> { self.selected_horizontal@ }
    pub(crate) open(super) spec fn selected_vertical_view(&self) -> Seq<u16> { self.selected_vertical@ }

    pub(crate) proof fn establish(tracked value: &Self)
        ensures value.selection_ready(),
    {
        HkScratch::establish(&value.hk);
        FixedVec::establish(&value.conflict.finalize.right_degrees);
        FixedVec::establish(&value.conflict.finalize.adjacency_offsets);
        FixedVec::establish(&value.conflict.finalize.adjacency_edges);
        FixedVec::establish(&value.conflict.finalize.right_layout);
        FixedVec::establish(&value.conflict.finalize.next_offsets);
        FixedVec::establish(&value.selected_horizontal);
        FixedVec::establish(&value.selected_vertical);
    }
}

impl HkScratch {
    pub(in crate::optimal) proof fn establish(tracked value: &Self)
        ensures buffers_valid(value),
    {
        FixedVec::establish(&value.pair_left); FixedVec::establish(&value.pair_right);
        FixedVec::establish(&value.right_distance); FixedVec::establish(&value.queue);
        FixedVec::establish(&value.unmatched_lefts); FixedVec::establish(&value.shortest_roots);
        FixedVec::establish(&value.reachable_left); FixedVec::establish(&value.reachable_right);
        FixedVec::establish(&value.transpose_offsets); FixedVec::establish(&value.transpose_edges);
        FixedVec::establish(&value.write_offsets); FixedVec::establish(&value.right_order);
        FixedVec::establish(&value.right_degree_counts); FixedVec::establish(&value.dfs_stack);
    }
}

pub(crate) open spec fn rows_valid(offsets: Seq<u16>, length: nat) -> bool {
    0 < offsets.len() <= 1954 && length <= 3969
        && offsets[0] == 0 && offsets[offsets.len() as int - 1] == length
        && forall|i: int, j: int| 0 <= i <= j < offsets.len() ==> #[trigger] offsets[i] <= #[trigger] offsets[j]
}

pub(crate) open(in crate::optimal) spec fn adjacency_valid(graph: &SparseAdjacencyRef, right_size: nat) -> bool {
    rows_valid(graph.offsets@, graph.edges@.len()) && right_size <= 1953
        && forall|i: int| 0 <= i < graph.edges@.len() ==> #[trigger] graph.edges@[i] < right_size
}

pub(crate) open(in crate::optimal) spec fn buffers_valid(scratch: &HkScratch) -> bool {
    scratch.pair_left.valid() && scratch.pair_right.valid() && scratch.right_distance.valid()
        && scratch.queue.valid() && scratch.unmatched_lefts.valid() && scratch.shortest_roots.valid()
        && scratch.reachable_left.valid() && scratch.reachable_right.valid()
        && scratch.transpose_offsets.valid() && scratch.transpose_edges.valid() && scratch.write_offsets.valid()
        && scratch.right_order.valid() && scratch.right_degree_counts.valid() && scratch.dfs_stack.valid()
}

pub(crate) open spec fn value_count(values: Seq<u16>, end: int, value: int) -> int
    recommends 0 <= end <= values.len(),
    decreases end,
{
    if end <= 0 { 0 } else {
        value_count(values, end - 1, value) + if values[end - 1] == value { 1int } else { 0int }
    }
}

pub(crate) open spec fn degrees_exact(values: Seq<u16>, degrees: Seq<u8>) -> bool {
    crate::optimal::graph::proof::degree_domain(degrees)
        && forall|right: int| 0 <= right < degrees.len() ==> #[trigger] degrees[right]
            == value_count(values, values.len() as int, right)
}

pub(crate) proof fn value_monotone(values: Seq<u16>, first: int, last: int, value: int)
    requires 0 <= first <= last <= values.len(),
    ensures 0 <= value_count(values, first, value) <= value_count(values, last, value) <= last,
    decreases last,
{
    if last > 0 {
        if last > first { value_monotone(values, first, last - 1, value); }
        else { value_monotone(values, first - 1, first - 1, value); }
    }
}

pub(crate) open spec fn owner(offsets: Seq<u16>, edge: int) -> int {
    choose|left: int| 0 <= left < offsets.len() - 1 && (#[trigger] offsets[left]) <= edge < offsets[left + 1]
}

pub(crate) proof fn row_exists(offsets: Seq<u16>, length: nat, edge: int, limit: int)
    requires rows_valid(offsets, length), 0 <= limit < offsets.len(), 0 <= edge < offsets[limit],
    ensures exists|left: int| 0 <= left < limit && (#[trigger] offsets[left]) <= edge < offsets[left + 1],
    decreases limit,
{
    assert(limit > 0);
    if edge < offsets[limit - 1] { row_exists(offsets, length, edge, limit - 1); }
}

pub(crate) proof fn owner_of_row(offsets: Seq<u16>, length: nat, edge: int, left: int)
    requires rows_valid(offsets, length), 0 <= left < offsets.len() - 1,
        offsets[left] <= edge < offsets[left + 1],
    ensures owner(offsets, edge) == left, 0 <= owner(offsets, edge) < offsets.len() - 1,
{
    let found = owner(offsets, edge);
    assert(exists|l: int| 0 <= l < offsets.len() - 1 && (#[trigger] offsets[l]) <= edge < offsets[l + 1]);
    if found < left { assert(offsets[found + 1] <= offsets[left]); }
    else if left < found { assert(offsets[left + 1] <= offsets[found]); }
}

pub(crate) proof fn owner_bound(offsets: Seq<u16>, length: nat, edge: int)
    requires rows_valid(offsets, length), 0 <= edge < length,
    ensures 0 <= owner(offsets, edge) < offsets.len() - 1,
        offsets[owner(offsets, edge)] <= edge < offsets[owner(offsets, edge) + 1],
{
    hide(owner);
    row_exists(offsets, length, edge, offsets.len() as int - 1);
    let left = choose|left: int| 0 <= left < offsets.len() - 1 && (#[trigger] offsets[left]) <= edge < offsets[left + 1];
    owner_of_row(offsets, length, edge, left);
}

pub(crate) open spec fn transpose_prefix(offsets: Seq<u16>, edges: Seq<u16>, end: int, right: int) -> Seq<u16>
    recommends rows_valid(offsets, edges.len()), 0 <= end <= edges.len(),
    decreases end,
{
    if end <= 0 { Seq::empty() } else {
        let before = transpose_prefix(offsets, edges, end - 1, right);
        if edges[end - 1] == right { before.push(owner(offsets, end - 1) as u16) } else { before }
    }
}

pub(crate) proof fn transpose_length(offsets: Seq<u16>, edges: Seq<u16>, end: int, right: int)
    requires rows_valid(offsets, edges.len()), 0 <= end <= edges.len(),
    ensures transpose_prefix(offsets, edges, end, right).len() == value_count(edges, end, right),
    decreases end,
{
    if end > 0 { transpose_length(offsets, edges, end - 1, right); }
}

pub(crate) proof fn transpose_contains(offsets: Seq<u16>, edges: Seq<u16>, end: int, right: int, left: u16)
    requires rows_valid(offsets, edges.len()), 0 <= end <= edges.len(),
    ensures transpose_prefix(offsets, edges, end, right).contains(left) <==>
        exists|i: int| 0 <= i < end && (#[trigger] edges[i]) == right && owner(offsets, i) as u16 == left,
    decreases end,
{
    hide(owner);
    if end > 0 {
        transpose_contains(offsets, edges, end - 1, right, left);
        if transpose_prefix(offsets, edges, end, right).contains(left) {
            if transpose_prefix(offsets, edges, end - 1, right).contains(left) {
                let i = choose|i: int| 0 <= i < end - 1 && (#[trigger] edges[i]) == right && owner(offsets, i) as u16 == left;
                assert(exists|j: int| 0 <= j < end && (#[trigger] edges[j]) == right && owner(offsets, j) as u16 == left);
            } else {
                assert(edges[end - 1] == right && owner(offsets, end - 1) as u16 == left);
            }
        }
        if exists|i: int| 0 <= i < end && (#[trigger] edges[i]) == right && owner(offsets, i) as u16 == left {
            let i = choose|i: int| 0 <= i < end && (#[trigger] edges[i]) == right && owner(offsets, i) as u16 == left;
            if i < end - 1 { assert(transpose_prefix(offsets, edges, end - 1, right).contains(left)); }
        }
    }
}

pub(crate) open spec fn counted_offsets(degrees: Seq<u8>, offsets: Seq<u16>, next: int) -> bool {
    offsets.len() == degrees.len() + 1 && 0 <= next <= degrees.len()
        && (forall|i: int| 0 <= i <= next ==> #[trigger] offsets[i]
            == crate::optimal::graph::proof::degree_sum(degrees, i))
        && (forall|i: int| next < i < offsets.len() ==> #[trigger] offsets[i] == 0)
}

pub(crate) open spec fn transpose_progress(offsets: Seq<u16>, edges: Seq<u16>, degrees: Seq<u8>,
    transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>, cursors: Seq<u16>, end: int) -> bool
{
    rows_valid(offsets, edges.len()) && rows_valid(transpose_offsets, edges.len())
        && cursors.len() == degrees.len() && transpose_edges.len() == edges.len() && 0 <= end <= edges.len()
        && counted_offsets(degrees, transpose_offsets, degrees.len() as int)
        && forall|right: int| 0 <= right < degrees.len() ==>
            transpose_offsets[right] <= #[trigger] cursors[right] <= transpose_offsets[right + 1]
            && cursors[right] as int == transpose_offsets[right] as int + value_count(edges, end, right)
            && transpose_edges.subrange(transpose_offsets[right] as int, cursors[right] as int)
                == transpose_prefix(offsets, edges, end, right)
}

pub(crate) open spec fn transposed(offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>,
    transpose_edges: Seq<u16>, right_size: nat) -> bool
{
    rows_valid(offsets, edges.len()) && rows_valid(transpose_offsets, transpose_edges.len())
        && transpose_offsets.len() == right_size + 1 && transpose_edges.len() == edges.len()
        && forall|right: int| 0 <= right < right_size ==> transpose_edges.subrange(
            (#[trigger] transpose_offsets[right]) as int, transpose_offsets[right + 1] as int)
                == transpose_prefix(offsets, edges, edges.len() as int, right)
}

pub(crate) open spec fn value_pairs(values: Seq<u16>) -> Seq<SparseEdge> {
    Seq::new(values.len(), |i: int| SparseEdge { left: 0, right: values[i] })
}

pub(crate) proof fn mapped_count(values: Seq<u16>, end: int, right: int)
    requires 0 <= end <= values.len(),
    ensures crate::optimal::graph::proof::right_count(value_pairs(values), end, right) == value_count(values, end, right),
    decreases end,
{
    if end > 0 { mapped_count(values, end - 1, right); }
}

pub(crate) proof fn sum_counts(values: Seq<u16>, degrees: Seq<u8>, end: int)
    requires degrees_exact(values, degrees), 0 <= end <= degrees.len(),
    ensures crate::optimal::graph::proof::degree_sum(degrees, end)
        == crate::optimal::graph::proof::right_sum(value_pairs(values), values.len() as int, end),
    decreases end,
{
    if end > 0 {
        sum_counts(values, degrees, end - 1);
        mapped_count(values, values.len() as int, end - 1);
    }
}

pub(crate) proof fn degrees_total(values: Seq<u16>, degrees: Seq<u8>)
    requires degrees_exact(values, degrees),
        forall|i: int| 0 <= i < values.len() ==> #[trigger] values[i] < degrees.len(),
    ensures crate::optimal::graph::proof::degree_sum(degrees, degrees.len() as int) == values.len(),
{
    sum_counts(values, degrees, degrees.len() as int);
    crate::optimal::graph::proof::right_sum_all(value_pairs(values), values.len() as int, degrees.len() as int);
}

pub(crate) proof fn degree_sum_monotone(degrees: Seq<u8>, first: int, last: int)
    requires 0 <= first <= last <= degrees.len(),
    ensures 0 <= crate::optimal::graph::proof::degree_sum(degrees, first)
        <= crate::optimal::graph::proof::degree_sum(degrees, last),
    decreases last,
{
    if last > 0 {
        if last > first { degree_sum_monotone(degrees, first, last - 1); }
        else { degree_sum_monotone(degrees, first - 1, first - 1); }
    }
}

pub(crate) proof fn transpose_offset_step(degrees: Seq<u8>, before: Seq<u16>, after: Seq<u16>, next: int)
    requires counted_offsets(degrees, before, next), next < degrees.len(),
        crate::optimal::graph::proof::degree_sum(degrees, next + 1) <= 3969,
        after == before.update(next + 1, (before[next] + degrees[next]) as u16),
    ensures counted_offsets(degrees, after, next + 1),
{
}

pub(crate) proof fn transpose_offsets_valid(values: Seq<u16>, degrees: Seq<u8>, offsets: Seq<u16>)
    requires degrees_exact(values, degrees), values.len() <= 3969,
        forall|i: int| 0 <= i < values.len() ==> #[trigger] values[i] < degrees.len(),
        counted_offsets(degrees, offsets, degrees.len() as int),
    ensures rows_valid(offsets, values.len()),
{
    degrees_total(values, degrees);
    assert forall|i: int, j: int| 0 <= i <= j < offsets.len() implies #[trigger] offsets[i] <= #[trigger] offsets[j] by {
        degree_sum_monotone(degrees, i, j);
    }
}

pub(crate) proof fn transpose_start(offsets: Seq<u16>, edges: Seq<u16>, degrees: Seq<u8>,
    transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>, cursors: Seq<u16>)
    requires rows_valid(offsets, edges.len()), degrees_exact(edges, degrees),
        forall|i: int| 0 <= i < edges.len() ==> #[trigger] edges[i] < degrees.len(),
        counted_offsets(degrees, transpose_offsets, degrees.len() as int),
        transpose_edges.len() == edges.len(), cursors == transpose_offsets.take(degrees.len() as int),
    ensures transpose_progress(offsets, edges, degrees, transpose_offsets, transpose_edges, cursors, 0),
{
    transpose_offsets_valid(edges, degrees, transpose_offsets);
    assert forall|right: int| 0 <= right < degrees.len() implies
        transpose_edges.subrange(transpose_offsets[right] as int, (#[trigger] cursors[right]) as int)
            == transpose_prefix(offsets, edges, 0, right) by {
        assert(transpose_edges.subrange(transpose_offsets[right] as int, cursors[right] as int) =~= Seq::<u16>::empty());
    }
}

pub(crate) proof fn transpose_ready(offsets: Seq<u16>, edges: Seq<u16>, degrees: Seq<u8>,
    transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>, cursors: Seq<u16>, edge: int)
    requires degrees_exact(edges, degrees), transpose_progress(offsets, edges, degrees,
        transpose_offsets, transpose_edges, cursors, edge), edge < edges.len(), edges[edge] < degrees.len(),
    ensures transpose_offsets[edges[edge] as int] <= cursors[edges[edge] as int]
        < transpose_offsets[edges[edge] as int + 1] <= transpose_edges.len(),
        cursors[edges[edge] as int] < 3969,
{
    let right = edges[edge] as int;
    value_monotone(edges, edge + 1, edges.len() as int, right);
    reveal_with_fuel(value_count, 1);
}

pub(crate) proof fn transpose_step(offsets: Seq<u16>, edges: Seq<u16>, degrees: Seq<u8>,
    transpose_offsets: Seq<u16>, previous: Seq<u16>, before: Seq<u16>,
    output: Seq<u16>, cursors: Seq<u16>, edge: int, left: int)
    requires degrees_exact(edges, degrees), transpose_progress(offsets, edges, degrees,
        transpose_offsets, previous, before, edge), edge < edges.len(), edges[edge] < degrees.len(),
        0 <= left < offsets.len() - 1, offsets[left] <= edge < offsets[left + 1],
        output == previous.update(before[edges[edge] as int] as int, left as u16),
        cursors == before.update(edges[edge] as int, (before[edges[edge] as int] + 1) as u16),
        before[edges[edge] as int] < 3969,
    ensures transpose_progress(offsets, edges, degrees, transpose_offsets, output, cursors, edge + 1),
{
    let changed = edges[edge] as int;
    let written = before[changed] as int;
    transpose_ready(offsets, edges, degrees, transpose_offsets, previous, before, edge);
    owner_of_row(offsets, edges.len(), edge, left);
    assert forall|right: int| 0 <= right < degrees.len() implies
        transpose_offsets[right] <= #[trigger] cursors[right] <= transpose_offsets[right + 1]
        && cursors[right] as int == transpose_offsets[right] as int + value_count(edges, edge + 1, right)
        && output.subrange(transpose_offsets[right] as int, cursors[right] as int)
            == transpose_prefix(offsets, edges, edge + 1, right) by {
        if right == changed {
            assert(output.subrange(transpose_offsets[right] as int, cursors[right] as int)
                =~= previous.subrange(transpose_offsets[right] as int, before[right] as int).push(left as u16));
        } else {
            if right < changed { assert(before[right] <= transpose_offsets[changed]); }
            else { assert(written < transpose_offsets[right]); }
            assert(output.subrange(transpose_offsets[right] as int, cursors[right] as int)
                =~= previous.subrange(transpose_offsets[right] as int, before[right] as int));
        }
    }
}

pub(crate) proof fn transpose_complete(offsets: Seq<u16>, edges: Seq<u16>, degrees: Seq<u8>,
    transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>, cursors: Seq<u16>)
    requires degrees_exact(edges, degrees), transpose_progress(offsets, edges, degrees,
        transpose_offsets, transpose_edges, cursors, edges.len() as int),
    ensures transposed(offsets, edges, transpose_offsets, transpose_edges, degrees.len()),
{
    assert forall|right: int| 0 <= right < degrees.len() implies #[trigger] cursors[right] == transpose_offsets[right + 1] by { }
    assert forall|right: int| 0 <= right < degrees.len() implies transpose_edges.subrange(
        (#[trigger] transpose_offsets[right]) as int, transpose_offsets[right + 1] as int)
            == transpose_prefix(offsets, edges, edges.len() as int, right) by {
        assert(cursors[right] == transpose_offsets[right + 1]);
    }
}

pub(crate) open spec fn pair_sum(source: Seq<SparseEdge>, end: int, limit: int, right: int) -> int
    recommends 0 <= end <= source.len(), 0 <= limit,
    decreases limit,
{
    if limit <= 0 { 0 } else { pair_sum(source, end, limit - 1, right)
        + crate::optimal::graph::proof::pair_count(source, end, limit - 1, right) }
}

pub(crate) proof fn pair_sum_step(source: Seq<SparseEdge>, end: int, limit: int, right: int)
    requires 0 <= end < source.len(), 0 <= limit,
    ensures pair_sum(source, end + 1, limit, right) == pair_sum(source, end, limit, right)
        + if source[end].left < limit && source[end].right == right { 1int } else { 0int },
    decreases limit,
{
    if limit > 0 { pair_sum_step(source, end, limit - 1, right); }
}

pub(crate) proof fn pair_sum_zero(source: Seq<SparseEdge>, limit: int, right: int)
    requires 0 <= limit,
    ensures pair_sum(source, 0, limit, right) == 0,
    decreases limit,
{
    if limit > 0 { pair_sum_zero(source, limit - 1, right); }
}

pub(crate) proof fn pair_sum_all(source: Seq<SparseEdge>, end: int, limit: int, right: int)
    requires 0 <= end <= source.len(), 0 <= limit,
        forall|i: int| 0 <= i < end ==> (#[trigger] source[i]).left < limit,
    ensures pair_sum(source, end, limit, right) == crate::optimal::graph::proof::right_count(source, end, right),
    decreases end,
{
    if end > 0 {
        pair_sum_all(source, end - 1, limit, right);
        pair_sum_step(source, end - 1, limit, right);
    } else { pair_sum_zero(source, limit, right); }
}

pub(crate) open spec fn row_sum(offsets: Seq<u16>, edges: Seq<u16>, limit: int, right: int) -> int
    recommends 0 <= limit < offsets.len(),
    decreases limit,
{
    if limit <= 0 { 0 } else { row_sum(offsets, edges, limit - 1, right)
        + crate::optimal::graph::proof::row_count(edges, offsets[limit - 1] as int, offsets[limit] as int, right) }
}

pub(crate) proof fn row_split(values: Seq<u16>, begin: int, middle: int, end: int, right: int)
    requires 0 <= begin <= middle <= end <= values.len(),
    ensures crate::optimal::graph::proof::row_count(values, begin, end, right)
        == crate::optimal::graph::proof::row_count(values, begin, middle, right)
            + crate::optimal::graph::proof::row_count(values, middle, end, right),
    decreases end - middle,
{
    if end > middle { row_split(values, begin, middle, end - 1, right); }
}

pub(crate) proof fn row_sum_all(offsets: Seq<u16>, edges: Seq<u16>, limit: int, right: int)
    requires rows_valid(offsets, edges.len()), 0 <= limit < offsets.len(),
    ensures row_sum(offsets, edges, limit, right)
        == crate::optimal::graph::proof::row_count(edges, 0, offsets[limit] as int, right),
    decreases limit,
{
    if limit > 0 {
        row_sum_all(offsets, edges, limit - 1, right);
        row_split(edges, 0, offsets[limit - 1] as int, offsets[limit] as int, right);
    }
}

pub(crate) proof fn count_from_zero(values: Seq<u16>, end: int, right: int)
    requires 0 <= end <= values.len(),
    ensures value_count(values, end, right) == crate::optimal::graph::proof::row_count(values, 0, end, right),
    decreases end,
{
    if end > 0 { count_from_zero(values, end - 1, right); }
}

pub(crate) proof fn csr_rows(source: Seq<SparseEdge>, offsets: Seq<u16>, edges: Seq<u16>, right_size: nat)
    requires crate::optimal::graph::proof::csr_exact(source, offsets, edges, right_size),
        offsets.len() <= 1954, source.len() <= 3969,
    ensures rows_valid(offsets, edges.len()),
        forall|i: int| 0 <= i < edges.len() ==> #[trigger] edges[i] < right_size,
{
    assert forall|i: int, j: int| 0 <= i <= j < offsets.len() implies #[trigger] offsets[i] <= #[trigger] offsets[j] by {
        crate::chord_graph::proof::count_monotone(source, source.len() as int, i, j);
    }
    assert forall|i: int| 0 <= i < edges.len() implies #[trigger] edges[i] < right_size by {
        owner_bound(offsets, edges.len(), i);
        let left = owner(offsets, i);
        assert(offsets[left] <= i < offsets[left + 1]);
    }
}

pub(crate) proof fn csr_row_sum(source: Seq<SparseEdge>, offsets: Seq<u16>, edges: Seq<u16>,
    right_size: nat, limit: int, right: int)
    requires crate::optimal::graph::proof::csr_exact(source, offsets, edges, right_size),
        0 <= limit < offsets.len(), 0 <= right < right_size,
    ensures row_sum(offsets, edges, limit, right) == pair_sum(source, source.len() as int, limit, right),
    decreases limit,
{
    if limit > 0 { csr_row_sum(source, offsets, edges, right_size, limit - 1, right); }
}

pub(crate) proof fn csr_degrees(source: Seq<SparseEdge>, degrees: Seq<u8>, offsets: Seq<u16>, edges: Seq<u16>)
    requires crate::optimal::graph::proof::source_groups(source, degrees),
        crate::optimal::graph::proof::csr_exact(source, offsets, edges, degrees.len()),
        offsets.len() <= 1954, source.len() <= 3969,
        forall|i: int| 0 <= i < source.len() ==> (#[trigger] source[i]).left as int + 1 < offsets.len(),
    ensures degrees_exact(edges, degrees), rows_valid(offsets, edges.len()),
        forall|i: int| 0 <= i < edges.len() ==> #[trigger] edges[i] < degrees.len(),
{
    csr_rows(source, offsets, edges, degrees.len());
    assert forall|right: int| 0 <= right < degrees.len() implies #[trigger] degrees[right]
        == value_count(edges, edges.len() as int, right) by {
        crate::optimal::graph::proof::group_at(source, degrees, right);
        let start = choose|start: int| crate::optimal::graph::proof::source_run(source, right, start, degrees[right] as int);
        crate::optimal::graph::proof::run_count(source, right, start, degrees[right] as int, source.len() as int);
        pair_sum_all(source, source.len() as int, offsets.len() as int - 1, right);
        csr_row_sum(source, offsets, edges, degrees.len(), offsets.len() as int - 1, right);
        row_sum_all(offsets, edges, offsets.len() as int - 1, right);
        count_from_zero(edges, edges.len() as int, right);
    }
}

pub(crate) open spec fn edge_in(offsets: Seq<u16>, edges: Seq<u16>, left: int, right: int) -> bool {
    0 <= left < offsets.len() - 1 && 0 <= right < 1953
        && edges.subrange(offsets[left] as int, offsets[left + 1] as int).contains(right as u16)
}

pub(crate) proof fn edge_at(offsets: Seq<u16>, edges: Seq<u16>, left: int, edge: int)
    requires rows_valid(offsets, edges.len()), 0 <= left < offsets.len() - 1,
        offsets[left] <= edge < offsets[left + 1], edges[edge] < 1953,
    ensures edge_in(offsets, edges, left, edges[edge] as int),
{
    let row = edges.subrange(offsets[left] as int, offsets[left + 1] as int);
    assert(row[edge - offsets[left]] == edges[edge]);
}

pub(in crate::optimal) proof fn adjacency_edge(graph: &SparseAdjacencyRef, right_size: nat, left: int, right: int)
    requires adjacency_valid(graph, right_size), edge_in(graph.offsets@, graph.edges@, left, right),
    ensures 0 <= left < graph.offsets@.len() - 1, 0 <= right < right_size,
{
    let row = graph.edges@.subrange(graph.offsets@[left] as int, graph.offsets@[left + 1] as int);
    row_bounds(graph.offsets@, graph.edges@.len(), left);
    let i = choose|i: int| 0 <= i < row.len() && row[i] == right as u16;
    assert(graph.edges@[i + graph.offsets@[left]] == right);
}

pub(crate) proof fn transpose_member(offsets: Seq<u16>, edges: Seq<u16>, right: int, left: u16)
    requires rows_valid(offsets, edges.len()), 0 <= right < 1953,
        transpose_prefix(offsets, edges, edges.len() as int, right).contains(left),
    ensures left < offsets.len() - 1, edge_in(offsets, edges, left as int, right),
{
    hide(owner);
    transpose_contains(offsets, edges, edges.len() as int, right, left);
    let edge = choose|edge: int| 0 <= edge < edges.len() && (#[trigger] edges[edge]) == right
        && owner(offsets, edge) as u16 == left;
    owner_bound(offsets, edges.len(), edge);
    let found = owner(offsets, edge);
    assert(found == left);
    edge_at(offsets, edges, found, edge);
}

pub(crate) proof fn row_bounds(offsets: Seq<u16>, length: nat, row: int)
    requires rows_valid(offsets, length), 0 <= row < offsets.len() - 1,
    ensures offsets[row] <= offsets[row + 1] <= length,
{
}

pub(crate) proof fn subrange_member(values: Seq<u16>, begin: int, end: int, index: int)
    requires 0 <= begin <= index < end <= values.len(),
    ensures values.subrange(begin, end).contains(values[index]),
{
    let row = values.subrange(begin, end);
    assert(row[index - begin] == values[index]);
}

pub(crate) proof fn transpose_at(offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>,
    transpose_edges: Seq<u16>, right_size: nat, right: int, slot: int)
    requires transposed(offsets, edges, transpose_offsets, transpose_edges, right_size),
        right_size <= 1953, 0 <= right < right_size,
        transpose_offsets[right] <= slot < transpose_offsets[right + 1],
    ensures transpose_edges[slot] < offsets.len() - 1,
        edge_in(offsets, edges, transpose_edges[slot] as int, right),
{
    hide(owner);
    hide(transpose_prefix);
    hide(edge_in);
    hide(rows_valid);
    let left = transpose_edges[slot];
    row_bounds(transpose_offsets, transpose_edges.len(), right);
    subrange_member(transpose_edges, transpose_offsets[right] as int, transpose_offsets[right + 1] as int, slot);
    transpose_member(offsets, edges, right, left);
}

pub(crate) proof fn transpose_edge(offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>, right_size: nat,
    left: int, right: int)
    requires transposed(offsets, edges, transpose_offsets, transpose_edges, right_size), right_size <= 1953,
        0 <= right < right_size, edge_in(offsets, edges, left, right),
    ensures exists|slot: int| transpose_offsets[right] <= slot < transpose_offsets[right + 1] && (#[trigger] transpose_edges[slot]) == left,
{
    hide(owner); hide(transpose_prefix);
    row_bounds(offsets, edges.len(), left);
    let row = edges.subrange(offsets[left] as int, offsets[left + 1] as int);
    let i = choose|i: int| 0 <= i < row.len() && row[i] == right as u16;
    let slot = i + offsets[left];
    assert(edges[slot] == right);
    owner_of_row(offsets, edges.len(), slot, left);
    transpose_contains(offsets, edges, edges.len() as int, right, left as u16);
    let reverse_row = transpose_edges.subrange(transpose_offsets[right] as int, transpose_offsets[right + 1] as int);
    assert(reverse_row.contains(left as u16));
    let j = choose|j: int| 0 <= j < reverse_row.len() && reverse_row[j] == left as u16;
    assert(transpose_edges[j + transpose_offsets[right]] == left);
}

pub(crate) proof fn reverse_from_transpose(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>)
    requires matching_valid(offsets, edges, lefts, rights), transposed(offsets, edges, transpose_offsets, transpose_edges, rights.len()),
    ensures crate::optimal::hk::levels_proof::reverse_arrays(transpose_offsets, transpose_edges, lefts, rights),
{
    hide(owner);
    hide(transpose_prefix);
    assert forall|i: int| 0 <= i < transpose_edges.len() implies #[trigger] transpose_edges[i] < lefts.len() by {
        owner_bound(transpose_offsets, transpose_edges.len(), i);
        let right = owner(transpose_offsets, i);
        transpose_at(offsets, edges, transpose_offsets, transpose_edges, rights.len(), right, i);
    }
}

pub(crate) open spec fn matching_valid(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>) -> bool {
    rows_valid(offsets, edges.len()) && lefts.len() + 1 == offsets.len() && lefts.len() <= 1953 && rights.len() <= 1953
        && (forall|left: int| 0 <= left < lefts.len() ==> #[trigger] lefts[left] == u16::MAX
            || lefts[left] < rights.len() && rights[lefts[left] as int] == left
                && edge_in(offsets, edges, left, lefts[left] as int))
        && (forall|right: int| 0 <= right < rights.len() ==> #[trigger] rights[right] == u16::MAX
            || rights[right] < lefts.len() && lefts[rights[right] as int] == right)
}

pub(crate) open spec fn matched_count(pairs: Seq<u16>, end: int) -> nat
    recommends 0 <= end <= pairs.len(),
    decreases end,
{
    if end <= 0 { 0 } else { matched_count(pairs, end - 1) + if pairs[end - 1] == u16::MAX { 0nat } else { 1nat } }
}

pub(crate) open spec fn matched_preserved(before: Seq<u16>, after: Seq<u16>) -> bool {
    before.len() == after.len() && forall|i: int| 0 <= i < before.len() && before[i] != u16::MAX
        ==> #[trigger] after[i] != u16::MAX
}

pub(crate) open spec fn free_lefts(pairs: Seq<u16>, ids: Seq<u16>) -> bool {
    ids.no_duplicates()
        && (forall|i: int| 0 <= i < ids.len() ==> (#[trigger] ids[i]) < pairs.len() && pairs[ids[i] as int] == u16::MAX)
        && (forall|left: int| 0 <= left < pairs.len() ==> (pairs[left] == u16::MAX <==> #[trigger] ids.contains(left as u16)))
        && (forall|i: int, j: int| 0 <= i < j < ids.len() ==> #[trigger] ids[i] < #[trigger] ids[j])
}

pub(crate) open spec fn cursors_valid(offsets: Seq<u16>, cursors: Seq<u16>) -> bool {
    offsets.len() == cursors.len() + 1 && forall|left: int| 0 <= left < cursors.len()
        ==> offsets[left] <= #[trigger] cursors[left] <= offsets[left + 1]
}

pub(crate) proof fn matched_added(before: Seq<u16>, after: Seq<u16>, changed: int, value: u16, end: int)
    requires 0 <= changed < before.len(), before[changed] == u16::MAX, value != u16::MAX,
        after == before.update(changed, value), 0 <= end <= before.len(),
    ensures matched_count(after, end) == matched_count(before, end) + if changed < end { 1nat } else { 0nat },
    decreases end,
{
    if end > 0 { matched_added(before, after, changed, value, end - 1); }
}

pub(crate) proof fn free_matched(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    left: int, right: int)
    requires matching_valid(offsets, edges, lefts, rights),
        0 <= left < lefts.len(), 0 <= right < rights.len(),
        lefts[left] == u16::MAX, rights[right] == u16::MAX, edge_in(offsets, edges, left, right),
    ensures matching_valid(offsets, edges, lefts.update(left, right as u16), rights.update(right, left as u16)),
        matched_count(lefts.update(left, right as u16), lefts.len() as int)
            == matched_count(lefts, lefts.len() as int) + 1,
{
    let new_lefts = lefts.update(left, right as u16);
    let new_rights = rights.update(right, left as u16);
    assert forall|l: int| 0 <= l < lefts.len() implies #[trigger] new_lefts[l] == u16::MAX
        || new_lefts[l] < rights.len() && new_rights[new_lefts[l] as int] == l
            && edge_in(offsets, edges, l, new_lefts[l] as int) by {
        if l != left && lefts[l] != u16::MAX { assert(lefts[l] != right); }
    }
    assert forall|r: int| 0 <= r < rights.len() implies #[trigger] new_rights[r] == u16::MAX
        || new_rights[r] < lefts.len() && new_lefts[new_rights[r] as int] == r by {
        if r != right && rights[r] != u16::MAX { assert(rights[r] != left); }
    }
    matched_added(lefts, new_lefts, left, right as u16, lefts.len() as int);
}

pub(crate) proof fn matched_kept(before: Seq<u16>, changed: int, value: u16, end: int)
    requires 0 <= changed < before.len(), before[changed] != u16::MAX, value != u16::MAX,
        0 <= end <= before.len(),
    ensures matched_count(before.update(changed, value), end) == matched_count(before, end),
    decreases end,
{
    if end > 0 { matched_kept(before, changed, value, end - 1); }
}

pub(crate) proof fn three_matched(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    left: int, right: int, middle: int, alternate: int)
    requires matching_valid(offsets, edges, lefts, rights),
        0 <= left < lefts.len(), 0 <= right < rights.len(), 0 <= middle < lefts.len(), 0 <= alternate < rights.len(),
        lefts[left] == u16::MAX, rights[right] == middle, rights[alternate] == u16::MAX,
        edge_in(offsets, edges, left, right), edge_in(offsets, edges, middle, alternate),
    ensures matching_valid(offsets, edges,
        lefts.update(middle, alternate as u16).update(left, right as u16),
        rights.update(alternate, middle as u16).update(right, left as u16)),
        matched_count(lefts.update(middle, alternate as u16).update(left, right as u16), lefts.len() as int)
            == matched_count(lefts, lefts.len() as int) + 1,
{
    assert(left != middle);
    assert(right != alternate);
    assert(lefts[middle] == right);
    let new_lefts = lefts.update(middle, alternate as u16).update(left, right as u16);
    let new_rights = rights.update(alternate, middle as u16).update(right, left as u16);
    assert forall|l: int| 0 <= l < lefts.len() implies #[trigger] new_lefts[l] == u16::MAX
        || new_lefts[l] < rights.len() && new_rights[new_lefts[l] as int] == l
            && edge_in(offsets, edges, l, new_lefts[l] as int) by {
        if l != left && l != middle && lefts[l] != u16::MAX {
            assert(lefts[l] != right);
            assert(lefts[l] != alternate);
        }
    }
    assert forall|r: int| 0 <= r < rights.len() implies #[trigger] new_rights[r] == u16::MAX
        || new_rights[r] < lefts.len() && new_lefts[new_rights[r] as int] == r by {
        if r != right && r != alternate && rights[r] != u16::MAX {
            assert(rights[r] != left);
            assert(rights[r] != middle);
        }
    }
    matched_kept(lefts, middle, alternate as u16, lefts.len() as int);
    matched_added(lefts.update(middle, alternate as u16), new_lefts, left, right as u16, lefts.len() as int);
}

pub(crate) open spec fn order_u32(order: Seq<u16>) -> Seq<u32> {
    Seq::new(order.len(), |i: int| order[i] as u32)
}

pub(crate) open spec fn sorted_rights(degrees: Seq<u8>, order: Seq<u16>) -> bool {
    crate::optimal::graph::proof::ordered(degrees, order_u32(order)) && order.no_duplicates()
        && forall|i: int| 0 <= i < order.len() ==> #[trigger] order[i] < degrees.len()
}

pub(crate) proof fn order_u16_step(degrees: Seq<u8>, before: Seq<usize>, after: Seq<usize>,
    order_before: Seq<u16>, order_after: Seq<u16>, right: int)
    requires crate::optimal::graph::proof::degree_domain(degrees),
        crate::optimal::graph::proof::scatter_counters(degrees, before, right),
        crate::optimal::graph::proof::order_prefix(degrees, order_u32(order_before), right),
        right < degrees.len(), degrees[right] < before.len(),
        after == before.update(degrees[right] as int, (before[degrees[right] as int] + 1) as usize),
        order_after == order_before.update(crate::optimal::graph::proof::rank(degrees, right), right as u16),
        before[degrees[right] as int] < usize::MAX,
    ensures crate::optimal::graph::proof::scatter_counters(degrees, after, right + 1),
        crate::optimal::graph::proof::order_prefix(degrees, order_u32(order_after), right + 1),
{
    crate::optimal::graph::proof::rank_bound(degrees, right);
    assert(order_u32(order_after) =~= order_u32(order_before).update(crate::optimal::graph::proof::rank(degrees, right), right as u32));
    crate::optimal::graph::proof::scatter_step(degrees, before, after, order_u32(order_before), order_u32(order_after), right);
}

pub(crate) proof fn complete_u16_order(degrees: Seq<u8>, order: Seq<u16>)
    requires crate::optimal::graph::proof::degree_domain(degrees),
        crate::optimal::graph::proof::order_prefix(degrees, order_u32(order), degrees.len() as int),
    ensures sorted_rights(degrees, order),
{
    crate::optimal::graph::proof::complete_order(degrees, order_u32(order));
    assert(order.no_duplicates()) by {
        assert forall|i: int, j: int| 0 <= i < j < order.len() implies #[trigger] order[i] != #[trigger] order[j] by {
            assert(order_u32(order)[i] != order_u32(order)[j]);
        }
    }
    assert forall|i: int| 0 <= i < order.len() implies #[trigger] order[i] < degrees.len() by {
        assert(order_u32(order)[i] < degrees.len());
    }
}

pub(crate) open(in crate::optimal) spec fn path_right(edges: Seq<u16>, stack: Seq<DfsFrame>, terminal: u16, index: int) -> u16 {
    if index + 1 < stack.len() { edges[stack[index].next_edge as int - 1] } else { terminal }
}

pub(crate) open(in crate::optimal) spec fn path_links(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    stack: Seq<DfsFrame>, terminal: u16) -> bool
{
    matching_valid(offsets, edges, lefts, rights) && 0 < stack.len() <= lefts.len()
        && terminal < rights.len() && rights[terminal as int] == u16::MAX
        && (forall|i: int| 0 <= i < stack.len() ==> (#[trigger] stack[i]).left < lefts.len())
        && (forall|i: int, j: int| #![auto] 0 <= i < j < stack.len() ==> stack[i].left != stack[j].left)
        && lefts[stack[0].left as int] == u16::MAX
        && edge_in(offsets, edges, stack.last().left as int, terminal as int)
        && forall|i: int| 0 <= i < stack.len() - 1 ==>
            offsets[(#[trigger] stack[i]).left as int] < stack[i].next_edge <= offsets[stack[i].left as int + 1]
            && edges[stack[i].next_edge as int - 1] < rights.len()
            && rights[edges[stack[i].next_edge as int - 1] as int] == stack[i + 1].left
}

pub(crate) open(in crate::optimal) spec fn path_valid(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    stack: Seq<DfsFrame>, terminal: u16) -> bool
{
    path_links(offsets, edges, lefts, rights, stack, terminal)
        && (forall|i: int, j: int| #![auto] 0 <= i < j < stack.len() ==> stack[i].left != stack[j].left)
        && (forall|i: int| 0 <= i < stack.len() ==> stack[i].left < lefts.len()
            && #[trigger] path_right(edges, stack, terminal, i) < rights.len()
            && edge_in(offsets, edges, stack[i].left as int, path_right(edges, stack, terminal, i) as int)
            && (i > 0 ==> lefts[stack[i].left as int] != u16::MAX))
        && forall|i: int, j: int| 0 <= i < j < stack.len() ==>
            #[trigger] path_right(edges, stack, terminal, i) != #[trigger] path_right(edges, stack, terminal, j)
}

pub(in crate::optimal) proof fn path_entry(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    stack: Seq<DfsFrame>, terminal: u16, i: int)
    requires path_links(offsets, edges, lefts, rights, stack, terminal), 0 <= i < stack.len(),
    ensures path_right(edges, stack, terminal, i) < rights.len(),
        edge_in(offsets, edges, stack[i].left as int, path_right(edges, stack, terminal, i) as int),
        i > 0 ==> lefts[stack[i].left as int] != u16::MAX,
        i + 1 < stack.len() ==> rights[path_right(edges, stack, terminal, i) as int] == stack[i + 1].left,
        i + 1 == stack.len() ==> path_right(edges, stack, terminal, i) == terminal,
{
    hide(edge_in);
    hide(rows_valid);
    if i + 1 < stack.len() { edge_at(offsets, edges, stack[i].left as int, stack[i].next_edge as int - 1); }
    if i > 0 {
        let previous = edges[stack[i - 1].next_edge as int - 1] as int;
        assert(previous < rights.len());
        assert(rights[previous] == stack[i].left);
        assert(rights[previous] != u16::MAX);
        assert(lefts[rights[previous] as int] == previous);
        assert(lefts[stack[i].left as int] == edges[stack[i - 1].next_edge as int - 1]);
    }
}

pub(crate) proof fn complete_path_links(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    stack: Seq<DfsFrame>, terminal: u16)
    requires path_links(offsets, edges, lefts, rights, stack, terminal),
    ensures path_valid(offsets, edges, lefts, rights, stack, terminal),
{
    hide(edge_in);
    hide(rows_valid);
    hide(path_right);
    assert forall|i: int| 0 <= i < stack.len() implies stack[i].left < lefts.len()
        && #[trigger] path_right(edges, stack, terminal, i) < rights.len()
        && edge_in(offsets, edges, stack[i].left as int, path_right(edges, stack, terminal, i) as int)
        && (i > 0 ==> lefts[stack[i].left as int] != u16::MAX) by {
        path_entry(offsets, edges, lefts, rights, stack, terminal, i);
    }
    assert forall|i: int, j: int| 0 <= i < j < stack.len() implies
        #[trigger] path_right(edges, stack, terminal, i) != #[trigger] path_right(edges, stack, terminal, j) by {
        path_entry(offsets, edges, lefts, rights, stack, terminal, i);
        path_entry(offsets, edges, lefts, rights, stack, terminal, j);
        if j + 1 < stack.len() {
            assert(stack[i + 1].left != stack[j + 1].left);
        } else { assert(rights[path_right(edges, stack, terminal, i) as int] != rights[terminal as int]); }
    }
}

pub(crate) open(in crate::optimal) spec fn path_has_left(stack: Seq<DfsFrame>, from: int, left: int) -> bool {
    exists|i: int| from <= i < stack.len() && (#[trigger] stack[i]).left == left
}

pub(crate) open(in crate::optimal) spec fn path_has_right(edges: Seq<u16>, stack: Seq<DfsFrame>, terminal: u16, from: int, right: int) -> bool {
    exists|i: int| from <= i < stack.len() && #[trigger] path_right(edges, stack, terminal, i) == right
}

pub(crate) open(in crate::optimal) spec fn path_state(initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, edges: Seq<u16>, stack: Seq<DfsFrame>, terminal: u16, from: int) -> bool
{
    0 <= from <= stack.len() && lefts.len() == initial_left.len() && rights.len() == initial_right.len()
        && (forall|i: int| from <= i < stack.len() ==> #[trigger] lefts[stack[i].left as int] == path_right(edges, stack, terminal, i))
        && (forall|i: int| from <= i < stack.len() ==> #[trigger] rights[path_right(edges, stack, terminal, i) as int] == stack[i].left)
        && (forall|left: int| 0 <= left < lefts.len() && !path_has_left(stack, from, left) ==> #[trigger] lefts[left] == initial_left[left])
        && (forall|right: int| 0 <= right < rights.len() && !path_has_right(edges, stack, terminal, from, right)
            ==> #[trigger] rights[right] == initial_right[right])
}

pub(in crate::optimal) proof fn fold_path_state(initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, edges: Seq<u16>, stack: Seq<DfsFrame>, terminal: u16, from: int)
    requires 0 <= from <= stack.len(), lefts.len() == initial_left.len(), rights.len() == initial_right.len(),
        forall|i: int| from <= i < stack.len() ==> #[trigger] lefts[stack[i].left as int] == path_right(edges, stack, terminal, i),
        forall|i: int| from <= i < stack.len() ==> #[trigger] rights[path_right(edges, stack, terminal, i) as int] == stack[i].left,
        forall|left: int| 0 <= left < lefts.len() && !path_has_left(stack, from, left) ==> #[trigger] lefts[left] == initial_left[left],
        forall|right: int| 0 <= right < rights.len() && !path_has_right(edges, stack, terminal, from, right)
            ==> #[trigger] rights[right] == initial_right[right],
    ensures path_state(initial_left, initial_right, lefts, rights, edges, stack, terminal, from),
{
}

pub(in crate::optimal) proof fn path_advance(offsets: Seq<u16>, edges: Seq<u16>, initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, stack: Seq<DfsFrame>, terminal: u16, next: int)
    requires path_valid(offsets, edges, initial_left, initial_right, stack, terminal),
        0 <= next < stack.len(), path_state(initial_left, initial_right, lefts, rights, edges, stack, terminal, next + 1),
    ensures path_state(initial_left, initial_right,
        lefts.update(stack[next].left as int, path_right(edges, stack, terminal, next)),
        rights.update(path_right(edges, stack, terminal, next) as int, stack[next].left),
        edges, stack, terminal, next),
{
    hide(path_links);
    hide(path_right);
    hide(edge_in);
    hide(rows_valid);
    let left = stack[next].left as int;
    let right = path_right(edges, stack, terminal, next) as int;
    assert(0 <= left < lefts.len());
    assert(0 <= right < rights.len());
    let new_lefts = lefts.update(left, right as u16);
    let new_rights = rights.update(right, left as u16);
    assert forall|i: int| next <= i < stack.len() implies #[trigger] new_lefts[stack[i].left as int] == path_right(edges, stack, terminal, i) by {
        if i > next {
            assert(stack[i].left != left);
            assert(lefts[stack[i].left as int] == path_right(edges, stack, terminal, i));
        } else {
            assert(new_lefts[left] == path_right(edges, stack, terminal, next));
        }
    }
    assert forall|i: int| next <= i < stack.len() implies #[trigger] new_rights[path_right(edges, stack, terminal, i) as int] == stack[i].left by {
        if i > next {
            assert(path_right(edges, stack, terminal, i) != right);
            assert(rights[path_right(edges, stack, terminal, i) as int] == stack[i].left);
        } else { assert(new_rights[right] == stack[next].left); }
    }
    assert forall|l: int| 0 <= l < lefts.len() && !path_has_left(stack, next, l) implies #[trigger] new_lefts[l] == initial_left[l] by {
        assert(l != left);
        assert(!path_has_left(stack, next + 1, l));
    }
    assert forall|r: int| 0 <= r < rights.len() && !path_has_right(edges, stack, terminal, next, r)
        implies #[trigger] new_rights[r] == initial_right[r] by {
        assert(r != right);
        assert(!path_has_right(edges, stack, terminal, next + 1, r));
    }
    fold_path_state(initial_left, initial_right, new_lefts, new_rights, edges, stack, terminal, next);
}

pub(crate) proof fn matched_one_new(before: Seq<u16>, after: Seq<u16>, changed: int, end: int)
    requires before.len() == after.len(), 0 <= changed < before.len(),
        before[changed] == u16::MAX, after[changed] != u16::MAX, 0 <= end <= before.len(),
        forall|i: int| 0 <= i < before.len() && i != changed ==> ((#[trigger] before[i]) == u16::MAX <==> (#[trigger] after[i]) == u16::MAX),
    ensures matched_count(after, end) == matched_count(before, end) + if changed < end { 1nat } else { 0nat },
    decreases end,
{
    if end > 0 { matched_one_new(before, after, changed, end - 1); }
}

pub(crate) proof fn path_complete(offsets: Seq<u16>, edges: Seq<u16>, initial_left: Seq<u16>, initial_right: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, stack: Seq<DfsFrame>, terminal: u16)
    requires path_valid(offsets, edges, initial_left, initial_right, stack, terminal),
        path_state(initial_left, initial_right, lefts, rights, edges, stack, terminal, 0),
    ensures matching_valid(offsets, edges, lefts, rights),
        matched_preserved(initial_left, lefts), matched_preserved(initial_right, rights),
        matched_count(lefts, lefts.len() as int) == matched_count(initial_left, initial_left.len() as int) + 1,
{
    hide(edge_in);
    hide(rows_valid);
    hide(path_right);
    hide(matched_count);
    assert forall|left: int| 0 <= left < lefts.len() implies #[trigger] lefts[left] == u16::MAX
        || lefts[left] < rights.len() && rights[lefts[left] as int] == left && edge_in(offsets, edges, left, lefts[left] as int) by {
        if path_has_left(stack, 0, left) {
            let i = choose|i: int| 0 <= i < stack.len() && (#[trigger] stack[i]).left == left;
        } else if initial_left[left] != u16::MAX {
            let right = initial_left[left] as int;
            assert(initial_right[right] == left);
            if path_has_right(edges, stack, terminal, 0, right) {
                let i = choose|i: int| 0 <= i < stack.len() && #[trigger] path_right(edges, stack, terminal, i) == right;
                path_entry(offsets, edges, initial_left, initial_right, stack, terminal, i);
                if i + 1 < stack.len() {
                    assert(initial_right[right] == stack[i + 1].left);
                    assert(path_has_left(stack, 0, stack[i + 1].left as int));
                }
            }
            assert(!path_has_right(edges, stack, terminal, 0, right));
        }
    }
    assert forall|right: int| 0 <= right < rights.len() implies #[trigger] rights[right] == u16::MAX
        || rights[right] < lefts.len() && lefts[rights[right] as int] == right by {
        if path_has_right(edges, stack, terminal, 0, right) {
            let i = choose|i: int| 0 <= i < stack.len() && #[trigger] path_right(edges, stack, terminal, i) == right;
            assert(lefts[stack[i].left as int] == path_right(edges, stack, terminal, i));
            assert(rights[right] == stack[i].left);
        } else if initial_right[right] != u16::MAX {
            let left = initial_right[right] as int;
            assert(initial_left[left] == right);
            if path_has_left(stack, 0, left) {
                let i = choose|i: int| 0 <= i < stack.len() && (#[trigger] stack[i]).left == left;
                if i > 0 {
                    path_entry(offsets, edges, initial_left, initial_right, stack, terminal, i - 1);
                    assert(initial_left[left] == path_right(edges, stack, terminal, i - 1));
                    assert(path_has_right(edges, stack, terminal, 0, right));
                }
            }
            assert(!path_has_left(stack, 0, left));
        }
    }
    assert forall|left: int| 0 <= left < lefts.len() && initial_left[left] != u16::MAX implies #[trigger] lefts[left] != u16::MAX by {
        if path_has_left(stack, 0, left) { let i = choose|i: int| 0 <= i < stack.len() && (#[trigger] stack[i]).left == left; }
    }
    assert forall|right: int| 0 <= right < rights.len() && initial_right[right] != u16::MAX implies #[trigger] rights[right] != u16::MAX by {
        if path_has_right(edges, stack, terminal, 0, right) {
            let i = choose|i: int| 0 <= i < stack.len() && #[trigger] path_right(edges, stack, terminal, i) == right;
            assert(lefts[stack[i].left as int] == path_right(edges, stack, terminal, i));
            assert(rights[right] == stack[i].left);
        }
    }
    let root = stack[0].left as int;
    assert forall|left: int| 0 <= left < lefts.len() && left != root implies
        ((#[trigger] initial_left[left]) == u16::MAX <==> (#[trigger] lefts[left]) == u16::MAX) by {
        if path_has_left(stack, 0, left) {
            let i = choose|i: int| 0 <= i < stack.len() && (#[trigger] stack[i]).left == left;
            assert(i > 0);
        }
    }
    matched_one_new(initial_left, lefts, root, lefts.len() as int);
}

proof fn transpose_fixture_result(target_offsets: Seq<u16>, target_edges: Seq<u16>)
    requires transposed(seq![0u16, 2, 3], seq![1u16, 0, 1], target_offsets, target_edges, 3),
    ensures target_offsets.len() == 4, target_offsets[2] == 3, target_edges[2] == 1,
{
    let offsets = seq![0u16, 2, 3];
    let edges = seq![1u16, 0, 1];
    owner_of_row(offsets, 3, 0, 0);
    owner_of_row(offsets, 3, 1, 0);
    owner_of_row(offsets, 3, 2, 1);
    reveal_with_fuel(transpose_prefix, 4);
    assert(target_offsets.len() == 4);
    assert(target_offsets[0] == 0);
    assert(target_edges.subrange(target_offsets[0] as int, target_offsets[1] as int) == seq![0u16]);
    assert(target_offsets[1] == 1);
    assert(target_edges.subrange(target_offsets[1] as int, target_offsets[2] as int) == seq![0u16, 1]);
}

fn transpose_three_edges_with_unused_right() {
    let offsets = [0u16, 2, 3];
    let edges = [1u16, 0, 1];
    let mut degrees = [1u8, 2, 0];
    #[cfg(reject_transpose_degree)]
    { degrees[1] = 1; }
    let adjacency = SparseAdjacencyRef { offsets: &offsets, edges: &edges };
    let mut target_offsets = FixedVec::<u16, 1954>::new(0);
    let mut target_edges = FixedVec::<u16, 3969>::new(0);
    let mut cursors = FixedVec::<u16, 1953>::new(0);
    reveal_with_fuel(value_count, 4);
    crate::optimal::greedy::build_transpose(&adjacency, &degrees,
        &mut target_offsets, &mut target_edges, &mut cursors);
    proof { transpose_fixture_result(target_offsets@, target_edges@); }
    #[cfg(reject_transpose_row_start)]
    assert(target_offsets@[2] == 2);
    #[cfg(reject_transpose_edge)]
    assert(target_edges@[2] == 0);
}

fn matching_three_edges_preserves_unvisited_left() {
    let offsets = [0u16, 1, 3, 3];
    let edges = [0u16, 0, 1];
    let adjacency = SparseAdjacencyRef { offsets: &offsets, edges: &edges };
    let mut lefts = [u16::MAX; 3];
    let mut rights = [u16::MAX; 2];
    #[cfg(reject_match_nonedge)]
    match_free(&adjacency, 0, 1, &mut lefts, &mut rights);
    match_free(&adjacency, 1, 0, &mut lefts, &mut rights);
    #[cfg(reject_match_occupied)]
    match_free(&adjacency, 0, 0, &mut lefts, &mut rights);
    match_three(&adjacency, 0, 0, 1, 1, &mut lefts, &mut rights);
    assert(lefts@ =~= seq![0u16, 1, u16::MAX]);
    assert(rights@ =~= seq![0u16, 1]);
    reveal_with_fuel(matched_count, 4);
    assert(matched_count(lefts@, 3) == 2);
    #[cfg(reject_match_flip_loss)]
    assert(lefts[1] == 0);
    #[cfg(reject_match_frame)]
    assert(lefts[2] != u16::MAX);
}

#[cfg(reject_right_order_duplicate)]
fn reject_right_order_duplicate() {
    let degrees = [2u8, 1, 2];
    let mut order = FixedVec::<u16, 1953>::new(0);
    let mut counts = FixedVec::<usize, 64>::new(0);
    crate::optimal::greedy::counting_sort_right_order_u16(&degrees, &mut order, &mut counts);
    assert(order@[0] == order@[1]);
}

fn commit_five_edge_path_preserves_other_left() {
    let offsets = [0u16, 1, 3, 5, 5];
    let edges = [0u16, 0, 1, 1, 2];
    let adjacency = SparseAdjacencyRef { offsets: &offsets, edges: &edges };
    let mut lefts = [u16::MAX; 4];
    let mut rights = [u16::MAX; 3];
    match_free(&adjacency, 1, 0, &mut lefts, &mut rights);
    match_free(&adjacency, 2, 1, &mut lefts, &mut rights);
    let mut stack = [DfsFrame { left: 0, next_edge: 1 },
        DfsFrame { left: 1, next_edge: 3 }, DfsFrame { left: 2, next_edge: 3 }];
    #[cfg(reject_path_link)]
    { stack[1].next_edge = 2; }
    commit_path(&adjacency, &stack, 2, &mut lefts, &mut rights);
    assert(lefts[0] == path_right(edges@, stack@, 2, 0));
    assert(lefts[1] == path_right(edges@, stack@, 2, 1));
    assert(lefts[2] == path_right(edges@, stack@, 2, 2));
    assert(!path_has_left(stack@, 0, 3));
    assert(lefts@ =~= seq![0u16, 1, 2, u16::MAX]);
    assert(rights@ =~= seq![0u16, 1, 2]);
    reveal_with_fuel(matched_count, 5);
    assert(matched_count(lefts@, 4) == 3);
    #[cfg(reject_path_count)]
    assert(matched_count(lefts@, 4) == 2);
    #[cfg(reject_path_frame)]
    assert(lefts[3] != u16::MAX);
}

fn real_single_conflict_selection() {
    let horizontal = [ValuedChord::on_axis::<true>(2, 1, 3, 1)];
    let vertical = [ValuedChord::on_axis::<false>(2, 1, 3, 1)];
    let offsets = [0u16, 1];
    let edges = [0u16];
    let degrees = [1u8];
    let adjacency = SparseAdjacencyRef { offsets: &offsets, edges: &edges };
    let mut scratch = HkScratch::new();
    let mut left = ChordBuffer::<u16>::new(0);
    let mut right = ChordBuffer::<u16>::new(0);
    reveal_with_fuel(value_count, 2);
    select_from_graph(&horizontal, &vertical, &adjacency, &degrees, &mut scratch, &mut left, &mut right);
    proof {
    crate::optimal::hk::levels_proof::ids_bound(left@, 1nat);
    crate::optimal::hk::levels_proof::ids_bound(right@, 1nat);
    index_set_view(left@); index_set_view(right@);
    assert(index_independent(horizontal@, vertical@, set![0int], Set::empty()));
    assert(left@.len() + right@.len() >= 1);
    if left@.len() > 0 && right@.len() > 0 {
        assert(left@[0] == 0 && right@[0] == 0);
        assert(index_set(left@).contains(0) && index_set(right@).contains(0));
        assert(!crate::optimal::graph::proof::closed_conflict(horizontal@[0], vertical@[0]));
        assert(false);
    }
    assert(left@.len() + right@.len() == 1);
    assert(index_independent(horizontal@, vertical@, Set::empty(), index_domain(1)));
    crate::optimal::hk::cover_proof::vertices_view(1);
    assert(index_maximum(horizontal@, vertical@, Set::empty(), index_domain(1)));
    assert(left@.len() == 0 && right@.len() == 1);
    #[cfg(reject_maximum_empty)]
    assert(left@.len() + right@.len() == 0);
    #[cfg(reject_maximum_conflict)]
    assert(left@.len() + right@.len() == 2);
    #[cfg(reject_canonical_left_tie)]
    assert(left@.len() == 1);
    }
}

fn real_incomplete_left_chooses_minimum_right() {
    let horizontal = [ValuedChord::on_axis::<true>(2, 1, 3, 1), ValuedChord::on_axis::<true>(5, 1, 3, 1)];
    let vertical = [ValuedChord::on_axis::<false>(2, 1, 3, 1)];
    let offsets = [0u16, 1, 1];
    let edges = [0u16];
    let degrees = [1u8];
    let adjacency = SparseAdjacencyRef { offsets: &offsets, edges: &edges };
    let mut scratch = HkScratch::new();
    let mut left = ChordBuffer::<u16>::new(0);
    let mut right = ChordBuffer::<u16>::new(0);
    reveal_with_fuel(value_count, 2);
    select_from_graph(&horizontal, &vertical, &adjacency, &degrees, &mut scratch, &mut left, &mut right);
    proof {
        crate::optimal::hk::cover_proof::vertices_view(2); crate::optimal::hk::cover_proof::vertices_view(1);
        index_set_view(left@); index_set_view(right@);
        assert(index_independent(horizontal@, vertical@, index_domain(2), Set::empty()));
        assert(left@.len() + right@.len() >= 2);
        assert(!index_maximum(horizontal@, vertical@, Set::empty(), index_domain(1)));
        assert forall|other_left: Set<int>, other_right: Set<int>| #[trigger] index_independent(horizontal@, vertical@, other_left, other_right)
            implies other_left.len() + other_right.len() <= 2 by {
            assert(other_left.subset_of(index_domain(2)) && other_right.subset_of(index_domain(1)));
            vstd::set_lib::lemma_len_subset(other_left, index_domain(2));
            vstd::set_lib::lemma_len_subset(other_right, index_domain(1));
            if other_left.len() == 2 && other_right.len() == 1 {
                vstd::set_lib::lemma_subset_equality(other_left, index_domain(2));
                vstd::set_lib::lemma_subset_equality(other_right, index_domain(1));
                assert(other_left.contains(0) && other_right.contains(0));
                assert(!crate::optimal::graph::proof::closed_conflict(horizontal@[0], vertical@[0]));
                assert(false);
            }
        }
        assert(index_maximum(horizontal@, vertical@, index_domain(2), Set::empty()));
        assert(index_set(right@).subset_of(Set::empty()));
        if right@.len() > 0 { assert(index_set(right@).contains(right@[0] as int)); assert(false); }
        assert(right@.len() == 0);
        #[cfg(reject_canonical_extra_right)]
        assert(right@.len() > 0);
    }
}

}
