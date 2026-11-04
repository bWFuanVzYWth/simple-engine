//! 反向 BFS 的唯一队列、行扫描预算与层次证明。
use super::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) open spec fn bounded_ids(ids: Seq<u16>, limit: nat) -> bool {
    ids.no_duplicates() && forall|i: int| 0 <= i < ids.len() ==> #[trigger] ids[i] < limit
}

pub(crate) proof fn ids_subset(ids: Seq<u16>, limit: nat)
    requires bounded_ids(ids, limit), limit <= 1953,
    ensures Seq::new(limit, |i: int| i as u16).no_duplicates(),
        ids.to_set().subset_of(Seq::new(limit, |i: int| i as u16).to_set()),
{
    let all = Seq::new(limit, |i: int| i as u16);
    assert(all.no_duplicates());
    assert(ids.to_set().subset_of(all.to_set())) by {
        assert forall|id: u16| ids.to_set().contains(id) implies all.to_set().contains(id) by {
            let i = choose|i: int| 0 <= i < ids.len() && ids[i] == id;
            assert(all[id as int] == id);
        }
    }
}

pub(crate) proof fn ids_bound(ids: Seq<u16>, limit: nat)
    requires bounded_ids(ids, limit), limit <= 1953,
    ensures ids.len() <= limit,
{
    ids_subset(ids, limit);
    let all = Seq::new(limit, |i: int| i as u16);
    ids.unique_seq_to_set();
    all.unique_seq_to_set();
    vstd::set_lib::lemma_len_subset(ids.to_set(), all.to_set());
}

pub(crate) proof fn ids_room(ids: Seq<u16>, limit: nat, id: u16)
    requires bounded_ids(ids, limit), limit <= 1953, id < limit, !ids.contains(id),
    ensures ids.len() < limit,
{
    ids_subset(ids, limit);
    let all = Seq::new(limit, |i: int| i as u16);
    ids.unique_seq_to_set();
    all.unique_seq_to_set();
    assert(all[id as int] == id);
    ids.to_set().lemma_subset_not_in_lt(all.to_set(), id);
}

pub(crate) open spec fn weights_sum(weights: Seq<int>, end: int) -> int
    recommends 0 <= end <= weights.len(),
    decreases end,
{
    if end <= 0 { 0 } else { weights_sum(weights, end - 1) + weights[end - 1] }
}

pub(crate) open spec fn selected_sum(weights: Seq<int>, ids: Seq<u16>, end: int) -> int
    recommends 0 <= end <= ids.len(),
    decreases end,
{
    if end <= 0 { 0 } else { selected_sum(weights, ids, end - 1) + weights[ids[end - 1] as int] }
}

pub(crate) proof fn weights_nonnegative(weights: Seq<int>, end: int)
    requires 0 <= end <= weights.len(), forall|i: int| 0 <= i < end ==> #[trigger] weights[i] >= 0,
    ensures weights_sum(weights, end) >= 0,
    decreases end,
{
    if end > 0 { weights_nonnegative(weights, end - 1); }
}

pub(crate) proof fn weight_removed(weights: Seq<int>, changed: int, end: int)
    requires 0 <= changed < weights.len(), 0 <= end <= weights.len(),
    ensures weights_sum(weights.update(changed, 0), end)
        == weights_sum(weights, end) - if changed < end { weights[changed] } else { 0 },
    decreases end,
{
    if end > 0 { weight_removed(weights, changed, end - 1); }
}

pub(crate) proof fn selected_removed(weights: Seq<int>, ids: Seq<u16>, changed: int, end: int)
    requires 0 <= changed < weights.len(), 0 <= end <= ids.len(),
        forall|i: int| 0 <= i < end ==> (#[trigger] ids[i]) < weights.len() && ids[i] != changed,
    ensures selected_sum(weights.update(changed, 0), ids, end) == selected_sum(weights, ids, end),
    decreases end,
{
    if end > 0 { selected_removed(weights, ids, changed, end - 1); }
}

pub(crate) proof fn selected_bound(weights: Seq<int>, ids: Seq<u16>, end: int)
    requires 0 <= end <= ids.len(), ids.no_duplicates(),
        forall|i: int| 0 <= i < weights.len() ==> #[trigger] weights[i] >= 0,
        forall|i: int| 0 <= i < ids.len() ==> #[trigger] ids[i] < weights.len(),
    ensures 0 <= selected_sum(weights, ids, end) <= weights_sum(weights, weights.len() as int),
    decreases end,
{
    if end > 0 {
        let id = ids[end - 1] as int;
        let reduced = weights.update(id, 0);
        selected_bound(reduced, ids, end - 1);
        assert(forall|i: int| 0 <= i < end - 1 ==> (#[trigger] ids[i]) != id);
        selected_removed(weights, ids, id, end - 1);
        weight_removed(weights, id, weights.len() as int);
    } else { weights_nonnegative(weights, weights.len() as int); }
}

pub(crate) open spec fn row_weights(offsets: Seq<u16>) -> Seq<int> {
    Seq::new((offsets.len() - 1) as nat, |i: int| offsets[i + 1] as int - offsets[i] as int)
}

pub(crate) proof fn row_weights_sum(offsets: Seq<u16>, length: nat, end: int)
    requires proof::rows_valid(offsets, length), 0 <= end < offsets.len(),
    ensures weights_sum(row_weights(offsets), end) == offsets[end],
    decreases end,
{
    if end > 0 { row_weights_sum(offsets, length, end - 1); }
}

pub(crate) proof fn queue_budget(offsets: Seq<u16>, length: nat, ids: Seq<u16>, end: int)
    requires proof::rows_valid(offsets, length), bounded_ids(ids, (offsets.len() - 1) as nat), 0 <= end <= ids.len(),
    ensures 0 <= selected_sum(row_weights(offsets), ids, end) <= length,
{
    let weights = row_weights(offsets);
    assert(forall|i: int| 0 <= i < weights.len() ==> #[trigger] weights[i] >= 0);
    row_weights_sum(offsets, length, offsets.len() as int - 1);
    selected_bound(weights, ids, end);
}

pub(crate) proof fn selected_pushed(weights: Seq<int>, ids: Seq<u16>, end: int, id: u16)
    requires 0 <= end <= ids.len(),
    ensures selected_sum(weights, ids.push(id), end) == selected_sum(weights, ids, end),
    decreases end,
{
    if end > 0 { selected_pushed(weights, ids, end - 1, id); }
}

pub(crate) proof fn roots_room(offsets: Seq<u16>, length: nat, ids: Seq<u16>, head: int, edge: int, roots: int)
    requires proof::rows_valid(offsets, length), bounded_ids(ids, (offsets.len() - 1) as nat),
        0 < head <= ids.len(), offsets[ids[head - 1] as int] <= edge < offsets[ids[head - 1] as int + 1],
        roots <= selected_sum(row_weights(offsets), ids, head - 1) + edge - offsets[ids[head - 1] as int],
    ensures roots < length <= 3969,
{
    queue_budget(offsets, length, ids, head);
}

pub(crate) open spec fn queued(distance: Seq<u16>, queue: Seq<u16>) -> bool {
    distance.len() <= 1953 && bounded_ids(queue, distance.len()) && queue.len() <= distance.len()
        && (forall|right: int| 0 <= right < distance.len() ==> ((#[trigger] distance[right]) != u16::MAX
            <==> queue.contains(right as u16)))
        && forall|i: int| 0 <= i < queue.len() ==> #[trigger] distance[queue[i] as int] <= i
}

pub(crate) proof fn queue_pushed(distance: Seq<u16>, queue: Seq<u16>, right: u16, depth: u16)
    requires queued(distance, queue), right < distance.len(), distance[right as int] == u16::MAX,
        depth <= queue.len(),
    ensures queued(distance.update(right as int, depth), queue.push(right)), queue.len() < distance.len(),
{
    assert(!queue.contains(right));
    ids_room(queue, distance.len(), right);
    assert forall|r: int| 0 <= r < distance.len() implies (#[trigger] distance.update(right as int, depth)[r]) != u16::MAX
        <==> queue.push(right).contains(r as u16) by { }
    assert(queue.push(right).no_duplicates());
    assert forall|i: int| 0 <= i < queue.push(right).len() implies #[trigger] queue.push(right)[i] < distance.len() by { }
    assert(bounded_ids(queue.push(right), distance.len()));
    assert forall|i: int| 0 <= i < queue.push(right).len() implies
        #[trigger] distance.update(right as int, depth)[queue.push(right)[i] as int] <= i by {
        if i < queue.len() { assert(queue[i] != right); }
    }
}

pub(crate) open spec fn reverse_arrays(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>) -> bool {
    proof::rows_valid(offsets, edges.len()) && offsets.len() == rights.len() + 1
        && lefts.len() <= 1953 && rights.len() <= 1953
        && (forall|i: int| 0 <= i < edges.len() ==> #[trigger] edges[i] < lefts.len())
        && (forall|left: int| 0 <= left < lefts.len() ==> (#[trigger] lefts[left]) == u16::MAX || lefts[left] < rights.len())
}

pub(crate) open(in crate::optimal) spec fn reverse_domain(transpose: &SparseAdjacencyRef, lefts: Seq<u16>, rights: Seq<u16>) -> bool {
    reverse_arrays(transpose.offsets@, transpose.edges@, lefts, rights)
}

pub(crate) open spec fn roots_free(roots: Seq<u16>, lefts: Seq<u16>) -> bool {
    forall|i: int| 0 <= i < roots.len() ==> (#[trigger] roots[i]) < lefts.len() && lefts[roots[i] as int] == u16::MAX
}

pub(crate) open spec fn seed_distance(distance: Seq<u16>, rights: Seq<u16>, end: int) -> bool {
    distance.len() == rights.len() && 0 <= end <= rights.len()
        && forall|right: int| 0 <= right < distance.len() ==> #[trigger] distance[right]
            == if right < end && rights[right] == u16::MAX { 0u16 } else { u16::MAX }
}

pub(crate) open spec fn sorted_queue(distance: Seq<u16>, queue: Seq<u16>) -> bool {
    forall|i: int, j: int| 0 <= i < j < queue.len()
        ==> #[trigger] distance[queue[i] as int] <= #[trigger] distance[queue[j] as int]
}

pub(crate) open spec fn queue_ceiling(distance: Seq<u16>, queue: Seq<u16>, ceiling: int) -> bool {
    forall|i: int| 0 <= i < queue.len() ==> #[trigger] distance[queue[i] as int] <= ceiling
}

pub(crate) open spec fn layered(distance: Seq<u16>, queue: Seq<u16>, head: int) -> bool {
    sorted_queue(distance, queue) && 0 <= head <= queue.len()
        && (head < queue.len() ==> forall|i: int| head <= i < queue.len()
            ==> #[trigger] distance[queue[i] as int] <= distance[queue[head] as int] as int + 1)
}

pub(crate) proof fn seed_layered(distance: Seq<u16>, queue: Seq<u16>, rights: Seq<u16>)
    requires queued(distance, queue), seed_distance(distance, rights, rights.len() as int),
    ensures layered(distance, queue, 0),
{
    assert forall|i: int| 0 <= i < queue.len() implies #[trigger] distance[queue[i] as int] == 0 by { }
}

pub(crate) proof fn current_layer(distance: Seq<u16>, queue: Seq<u16>, head: int)
    requires queued(distance, queue), layered(distance, queue, head), head < queue.len(),
    ensures queue_ceiling(distance, queue, distance[queue[head] as int] as int + 1),
{
    assert forall|i: int| 0 <= i < queue.len() implies #[trigger] distance[queue[i] as int]
        <= distance[queue[head] as int] as int + 1 by {
        if i < head { assert(distance[queue[i] as int] <= distance[queue[head] as int]); }
    }
}

pub(crate) proof fn finish_layer(distance: Seq<u16>, queue: Seq<u16>, parent: int)
    requires queued(distance, queue), sorted_queue(distance, queue), 0 <= parent < queue.len(),
        queue_ceiling(distance, queue, distance[queue[parent] as int] as int + 1),
    ensures layered(distance, queue, parent + 1),
{
    if parent + 1 < queue.len() { assert(distance[queue[parent] as int] <= distance[queue[parent + 1] as int]); }
}

pub(crate) proof fn pushed_layers(distance: Seq<u16>, queue: Seq<u16>, right: u16, depth: u16)
    requires queued(distance, queue), sorted_queue(distance, queue), queue_ceiling(distance, queue, depth as int),
        right < distance.len(), distance[right as int] == u16::MAX,
    ensures sorted_queue(distance.update(right as int, depth), queue.push(right)),
        queue_ceiling(distance.update(right as int, depth), queue.push(right), depth as int),
{
    assert(!queue.contains(right));
    let new = distance.update(right as int, depth);
    assert forall|i: int| 0 <= i < queue.len() implies #[trigger] new[queue[i] as int] == distance[queue[i] as int] by {
        assert(queue[i] != right);
    }
    assert forall|i: int, j: int| 0 <= i < j < queue.push(right).len() implies
        #[trigger] new[queue.push(right)[i] as int] <= #[trigger] new[queue.push(right)[j] as int] by {
        if j < queue.len() { assert(distance[queue[i] as int] <= distance[queue[j] as int]); }
    }
}

pub(crate) proof fn queued_ceiling(distance: Seq<u16>, queue: Seq<u16>, ceiling: int, right: int)
    requires queued(distance, queue), queue_ceiling(distance, queue, ceiling),
        0 <= right < distance.len(), distance[right] != u16::MAX,
    ensures distance[right] <= ceiling,
{
    assert(queue.contains(right as u16));
    let i = choose|i: int| 0 <= i < queue.len() && queue[i] == right;
}

pub(crate) open spec fn reverse_arc(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, from: int, target: int) -> bool {
    0 <= from < offsets.len() - 1 && 0 <= target < offsets.len() - 1 && target != u16::MAX
        && exists|edge: int| offsets[from] <= edge < offsets[from + 1]
            && (#[trigger] edges[edge]) < lefts.len() && lefts[edges[edge] as int] == target
}

pub(crate) open spec fn reverse_walk(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    path: Seq<int>, right: int, depth: int) -> bool
{
    0 <= depth && path.len() == depth + 1 && path.last() == right
        && (forall|i: int| 0 <= i < path.len() ==> 0 <= (#[trigger] path[i]) < rights.len())
        && rights[path[0]] == u16::MAX
        && forall|i: int| 0 <= i < depth ==> reverse_arc(offsets, edges, lefts, (#[trigger] path[i]), path[i + 1])
}

pub(crate) open spec fn reach_at(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, right: int, depth: int) -> bool {
    exists|path: Seq<int>| #[trigger] reverse_walk(offsets, edges, lefts, rights, path, right, depth)
}

#[verifier::opaque]
pub(crate) open spec fn distance_witness(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>) -> bool {
    distance.len() == rights.len() && forall|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) != u16::MAX
        ==> reach_at(offsets, edges, lefts, rights, right, distance[right] as int)
}

pub(crate) proof fn reach_step(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, parent: int, target: int, depth: int)
    requires 0 <= depth, 0 <= target < rights.len(), reach_at(offsets, edges, lefts, rights, parent, depth),
        reverse_arc(offsets, edges, lefts, parent, target),
    ensures reach_at(offsets, edges, lefts, rights, target, depth + 1),
{
    let path = choose|path: Seq<int>| reverse_walk(offsets, edges, lefts, rights, path, parent, depth);
    let extended = path.push(target);
    assert forall|i: int| 0 <= i < depth + 1 implies reverse_arc(offsets, edges, lefts, (#[trigger] extended[i]), extended[i + 1]) by {
        if i < depth { assert(reverse_arc(offsets, edges, lefts, path[i], path[i + 1])); }
    }
    assert(reverse_walk(offsets, edges, lefts, rights, extended, target, depth + 1));
}

pub(crate) proof fn seed_witness(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>)
    requires seed_distance(distance, rights, rights.len() as int),
    ensures distance_witness(offsets, edges, lefts, rights, distance),
{
    reveal(distance_witness);
    assert forall|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) != u16::MAX
        implies reach_at(offsets, edges, lefts, rights, right, distance[right] as int) by {
        assert(reverse_walk(offsets, edges, lefts, rights, seq![right], right, 0));
    }
}

pub(crate) proof fn witness_pushed(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    distance: Seq<u16>, parent: int, edge: int, target: u16, depth: u16)
    requires distance_witness(offsets, edges, lefts, rights, distance), offsets.len() == rights.len() + 1,
        0 <= parent < rights.len(), distance[parent] != u16::MAX,
        offsets[parent] <= edge < offsets[parent + 1], edges[edge] < lefts.len(),
        lefts[edges[edge] as int] == target, target != u16::MAX, target < rights.len(),
        distance[target as int] == u16::MAX, depth as int == distance[parent] as int + 1,
    ensures distance_witness(offsets, edges, lefts, rights, distance.update(target as int, depth)),
{
    hide(reach_at);
    reveal(distance_witness);
    assert(reverse_arc(offsets, edges, lefts, parent, target as int));
    assert(reach_at(offsets, edges, lefts, rights, parent, distance[parent] as int));
    reach_step(offsets, edges, lefts, rights, parent, target as int, distance[parent] as int);
    assert forall|right: int| 0 <= right < distance.len() && (#[trigger] distance.update(target as int, depth)[right]) != u16::MAX
        implies reach_at(offsets, edges, lefts, rights, right, distance.update(target as int, depth)[right] as int) by {
        if right != target { assert(distance[right] != u16::MAX); }
    }
}

pub(crate) open spec fn free_incident(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, right: int, left: u16) -> bool {
    0 <= right < offsets.len() - 1 && left < lefts.len() && lefts[left as int] == u16::MAX
        && offsets[right] <= offsets[right + 1] <= edges.len()
        && edges.subrange(offsets[right] as int, offsets[right + 1] as int).contains(left)
}

#[verifier::opaque]
pub(crate) open spec fn roots_sound(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, cut: u16) -> bool {
    forall|i: int| #![trigger roots[i]] 0 <= i < roots.len() ==> (roots[i] < lefts.len() && lefts[roots[i] as int] == u16::MAX
        && cut != u16::MAX && (exists|right: int| 0 <= right < distance.len()
        && (#[trigger] distance[right]) == cut && free_incident(offsets, edges, lefts, right, roots[i])))
}

pub(crate) proof fn roots_start(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>)
    requires roots.len() == 0,
    ensures roots_sound(offsets, edges, lefts, distance, roots, u16::MAX),
{
    reveal(roots_sound);
}

pub(crate) proof fn root_witness(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, cut: u16, i: int)
    requires roots_sound(offsets, edges, lefts, distance, roots, cut), 0 <= i < roots.len(),
    ensures roots[i] < lefts.len(), lefts[roots[i] as int] == u16::MAX, cut != u16::MAX,
        exists|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) == cut
            && free_incident(offsets, edges, lefts, right, roots[i]),
{
    reveal(roots_sound);
    assert(roots[i] < lefts.len());
    assert(lefts[roots[i] as int] == u16::MAX);
    assert(cut != u16::MAX);
}

pub(crate) proof fn roots_pushed(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>,
    roots: Seq<u16>, cut: u16, parent: int, edge: int)
    requires proof::rows_valid(offsets, edges.len()), offsets.len() == distance.len() + 1,
        roots_sound(offsets, edges, lefts, distance, roots, cut), cut == u16::MAX ==> roots.len() == 0,
        0 <= parent < distance.len(), distance[parent] != u16::MAX,
        cut == u16::MAX || cut == distance[parent], offsets[parent] <= edge < offsets[parent + 1],
        edges[edge] < lefts.len(), lefts[edges[edge] as int] == u16::MAX,
    ensures roots_sound(offsets, edges, lefts, distance, roots.push(edges[edge]), distance[parent]),
{
    reveal(roots_sound);
    proof::row_bounds(offsets, edges.len(), parent);
    proof::subrange_member(edges, offsets[parent] as int, offsets[parent + 1] as int, edge);
    assert(free_incident(offsets, edges, lefts, parent, edges[edge]));
    assert forall|i: int| #![trigger roots.push(edges[edge])[i]] 0 <= i < roots.push(edges[edge]).len() implies distance[parent] != u16::MAX
        && exists|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) == distance[parent]
            && free_incident(offsets, edges, lefts, right, roots.push(edges[edge])[i]) by {
        if i < roots.len() {
            assert(cut != u16::MAX);
        }
    }
}

pub(crate) proof fn roots_distance_changed(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>,
    roots: Seq<u16>, cut: u16, target: u16, depth: u16)
    requires roots_sound(offsets, edges, lefts, distance, roots, cut), target < distance.len(), distance[target as int] == u16::MAX,
    ensures roots_sound(offsets, edges, lefts, distance.update(target as int, depth), roots, cut),
{
    reveal(roots_sound);
    assert forall|i: int| #![trigger roots[i]] 0 <= i < roots.len() implies roots[i] < lefts.len()
        && lefts[roots[i] as int] == u16::MAX && cut != u16::MAX && exists|right: int| 0 <= right < distance.len()
        && (#[trigger] distance.update(target as int, depth)[right]) == cut && free_incident(offsets, edges, lefts, right, roots[i]) by {
        root_witness(offsets, edges, lefts, distance, roots, cut, i);
        assert(cut != u16::MAX);
        let right = choose|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) == cut
            && free_incident(offsets, edges, lefts, right, roots[i]);
        assert(right != target);
        assert(distance.update(target as int, depth)[right] == cut);
        assert(exists|r: int| 0 <= r < distance.len() && (#[trigger] distance.update(target as int, depth)[r]) == cut
            && free_incident(offsets, edges, lefts, r, roots[i]));
    }
}

pub(crate) open spec fn closed_slot(edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, cut: u16, right: int, edge: int) -> bool {
    0 <= right < distance.len() && distance[right] != u16::MAX && 0 <= edge < edges.len() && edges[edge] < lefts.len()
        && if lefts[edges[edge] as int] == u16::MAX {
            distance[right] >= cut && (distance[right] == cut ==> roots.contains(edges[edge]))
        } else {
            lefts[edges[edge] as int] < distance.len() && (distance[right] < cut
                ==> distance[lefts[edges[edge] as int] as int] <= distance[right] as int + 1)
        }
}

pub(crate) open spec fn scanned_rows(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>,
    queue: Seq<u16>, roots: Seq<u16>, cut: u16, head: int) -> bool
{
    0 <= head <= queue.len() && forall|i: int| 0 <= i < head ==> (#[trigger] queue[i]) < offsets.len() - 1
        && forall|edge: int| offsets[queue[i] as int] <= edge < offsets[queue[i] as int + 1]
            ==> #[trigger] closed_slot(edges, lefts, distance, roots, cut, queue[i] as int, edge)
}

pub(crate) proof fn free_slot(edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, right: int, edge: int)
    requires 0 <= right < distance.len(), distance[right] != u16::MAX,
        0 <= edge < edges.len(), edges[edge] < lefts.len(), lefts[edges[edge] as int] == u16::MAX,
    ensures closed_slot(edges, lefts, distance, roots.push(edges[edge]), distance[right], right, edge),
{
    assert(roots.push(edges[edge]).contains(edges[edge]));
}

pub(crate) open spec fn scanned_prefix(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>,
    queue: Seq<u16>, roots: Seq<u16>, cut: u16, head: int, edge: int) -> bool
{
    0 < head <= queue.len() && queue[head - 1] < offsets.len() - 1
        && scanned_rows(offsets, edges, lefts, distance, queue, roots, cut, head - 1)
        && offsets[queue[head - 1] as int] <= edge <= offsets[queue[head - 1] as int + 1]
        && forall|slot: int| offsets[queue[head - 1] as int] <= slot < edge
            ==> #[trigger] closed_slot(edges, lefts, distance, roots, cut, queue[head - 1] as int, slot)
}

pub(crate) open spec fn search_monotone(distance: Seq<u16>, updated: Seq<u16>, queue: Seq<u16>, new_queue: Seq<u16>,
    roots: Seq<u16>, new_roots: Seq<u16>, cut: u16, new_cut: u16, head: int) -> bool
{
    0 <= head <= queue.len() && head <= new_queue.len() && new_queue.take(head) == queue.take(head)
        && updated.len() == distance.len() && new_cut <= cut
        && (forall|r: int| 0 <= r < distance.len() ==> #[trigger] updated[r] <= distance[r])
        && (forall|i: int| 0 <= i < head ==> #[trigger] updated[queue[i] as int] == distance[queue[i] as int])
        && (forall|left: u16| roots.contains(left) ==> #[trigger] new_roots.contains(left))
}

pub(crate) proof fn monotone_root(distance: Seq<u16>, queue: Seq<u16>, roots: Seq<u16>, cut: u16, depth: u16, head: int, left: u16)
    requires 0 <= head <= queue.len(), depth <= cut,
    ensures search_monotone(distance, distance, queue, queue, roots, roots.push(left), cut, depth, head),
{
}

pub(crate) proof fn monotone_discovery(distance: Seq<u16>, queue: Seq<u16>, roots: Seq<u16>, cut: u16, head: int, target: u16, depth: u16)
    requires queued(distance, queue), 0 <= head <= queue.len(), target < distance.len(), distance[target as int] == u16::MAX,
    ensures search_monotone(distance, distance.update(target as int, depth), queue, queue.push(target), roots, roots, cut, cut, head),
{
    assert(!queue.contains(target));
    assert(queue.push(target).take(head) =~= queue.take(head));
    assert forall|i: int| 0 <= i < head implies #[trigger] distance.update(target as int, depth)[queue[i] as int] == distance[queue[i] as int] by {
        assert(queue[i] != target);
    }
}

pub(crate) proof fn slot_monotone(edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, updated: Seq<u16>,
    roots: Seq<u16>, new_roots: Seq<u16>, cut: u16, new_cut: u16, right: int, edge: int)
    requires closed_slot(edges, lefts, distance, roots, cut, right, edge),
        updated.len() == distance.len(), updated[right] == distance[right], new_cut <= cut,
        forall|r: int| 0 <= r < distance.len() ==> #[trigger] updated[r] <= distance[r],
        forall|left: u16| roots.contains(left) ==> #[trigger] new_roots.contains(left),
    ensures closed_slot(edges, lefts, updated, new_roots, new_cut, right, edge),
{
}

pub(crate) proof fn scanned_step(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, updated: Seq<u16>,
    queue: Seq<u16>, new_queue: Seq<u16>, roots: Seq<u16>, new_roots: Seq<u16>, cut: u16, new_cut: u16, head: int, edge: int)
    requires scanned_prefix(offsets, edges, lefts, distance, queue, roots, cut, head, edge),
        edge < offsets[queue[head - 1] as int + 1],
        search_monotone(distance, updated, queue, new_queue, roots, new_roots, cut, new_cut, head),
        closed_slot(edges, lefts, updated, new_roots, new_cut, queue[head - 1] as int, edge),
    ensures scanned_prefix(offsets, edges, lefts, updated, new_queue, new_roots, new_cut, head, edge + 1),
{
    assert forall|i: int| 0 <= i < head implies #[trigger] new_queue[i] == queue[i] by {
        assert(new_queue.take(head)[i] == queue.take(head)[i]);
    }
    assert forall|i: int| 0 <= i < head - 1 implies (#[trigger] new_queue[i]) < offsets.len() - 1
        && forall|slot: int| offsets[new_queue[i] as int] <= slot < offsets[new_queue[i] as int + 1]
            ==> #[trigger] closed_slot(edges, lefts, updated, new_roots, new_cut, new_queue[i] as int, slot) by {
        assert forall|slot: int| offsets[new_queue[i] as int] <= slot < offsets[new_queue[i] as int + 1] implies
            #[trigger] closed_slot(edges, lefts, updated, new_roots, new_cut, new_queue[i] as int, slot) by {
            slot_monotone(edges, lefts, distance, updated, roots, new_roots, cut, new_cut, queue[i] as int, slot);
        }
    }
    assert forall|slot: int| offsets[new_queue[head - 1] as int] <= slot < edge + 1 implies
        #[trigger] closed_slot(edges, lefts, updated, new_roots, new_cut, new_queue[head - 1] as int, slot) by {
        if slot < edge { slot_monotone(edges, lefts, distance, updated, roots, new_roots, cut, new_cut, queue[head - 1] as int, slot); }
    }
}

pub(crate) proof fn scanned_finished(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>,
    queue: Seq<u16>, roots: Seq<u16>, cut: u16, head: int)
    requires scanned_prefix(offsets, edges, lefts, distance, queue, roots, cut, head, offsets[queue[head - 1] as int + 1] as int),
    ensures scanned_rows(offsets, edges, lefts, distance, queue, roots, cut, head),
{
    assert forall|i: int| 0 <= i < head implies (#[trigger] queue[i]) < offsets.len() - 1
        && forall|edge: int| offsets[queue[i] as int] <= edge < offsets[queue[i] as int + 1]
            ==> #[trigger] closed_slot(edges, lefts, distance, roots, cut, queue[i] as int, edge) by { }
}

pub(in crate::optimal) proof fn scanned_skipped(transpose: &SparseAdjacencyRef, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>,
    queue: Seq<u16>, roots: Seq<u16>, cut: u16, head: int)
    requires reverse_domain(transpose, lefts, rights), queued(distance, queue), distance.len() == rights.len(),
        0 < head <= queue.len(), distance[queue[head - 1] as int] > cut,
        scanned_rows(transpose.offsets@, transpose.edges@, lefts, distance, queue, roots, cut, head - 1),
    ensures scanned_rows(transpose.offsets@, transpose.edges@, lefts, distance, queue, roots, cut, head),
{
    let right = queue[head - 1] as int;
    proof::row_bounds(transpose.offsets@, transpose.edges@.len(), right);
    assert forall|i: int| 0 <= i < head implies (#[trigger] queue[i]) < transpose.offsets@.len() - 1
        && forall|edge: int| transpose.offsets@[queue[i] as int] <= edge < transpose.offsets@[queue[i] as int + 1]
            ==> #[trigger] closed_slot(transpose.edges@, lefts, distance, roots, cut, queue[i] as int, edge) by {
        if i == head - 1 {
            assert forall|edge: int| transpose.offsets@[right] <= edge < transpose.offsets@[right + 1] implies
                #[trigger] closed_slot(transpose.edges@, lefts, distance, roots, cut, right, edge) by { }
        }
    }
}

pub(crate) open spec fn global_closed(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, cut: u16) -> bool {
    offsets.len() == distance.len() + 1 && forall|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) != u16::MAX
        ==> forall|edge: int| offsets[right] <= edge < offsets[right + 1]
            ==> #[trigger] closed_slot(edges, lefts, distance, roots, cut, right, edge)
}

pub(crate) proof fn final_scanned(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, queue: Seq<u16>, roots: Seq<u16>, cut: u16)
    requires queued(distance, queue), offsets.len() == distance.len() + 1,
        scanned_rows(offsets, edges, lefts, distance, queue, roots, cut, queue.len() as int),
    ensures global_closed(offsets, edges, lefts, distance, roots, cut),
        forall|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) != u16::MAX ==> distance[right] < distance.len(),
{
    assert forall|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) != u16::MAX implies
        distance[right] < distance.len() && (forall|edge: int| offsets[right] <= edge < offsets[right + 1]
            ==> #[trigger] closed_slot(edges, lefts, distance, roots, cut, right, edge)) by {
        assert(queue.contains(right as u16));
        let i = choose|i: int| 0 <= i < queue.len() && queue[i] == right;
        assert(distance[queue[i] as int] <= i);
    }
}

pub(crate) proof fn final_cut_bound(distance: Seq<u16>, queue: Seq<u16>, cut: u16)
    requires queued(distance, queue), cut != u16::MAX ==> queue_ceiling(distance, queue, cut as int + 1),
    ensures forall|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) != u16::MAX && cut != u16::MAX
        ==> distance[right] <= cut as int + 1,
{
    assert forall|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) != u16::MAX && cut != u16::MAX
        implies distance[right] <= cut as int + 1 by { queued_ceiling(distance, queue, cut as int + 1, right); }
}

pub(crate) proof fn distance_has_walk(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, right: int)
    requires distance_witness(offsets, edges, lefts, rights, distance), 0 <= right < distance.len(), distance[right] != u16::MAX,
    ensures reach_at(offsets, edges, lefts, rights, right, distance[right] as int),
{
    reveal(distance_witness);
    assert(distance[right] != u16::MAX);
}

pub(crate) proof fn relaxed_arc(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>,
    cut: u16, from: int, target: int)
    requires global_closed(offsets, edges, lefts, distance, roots, cut),
        0 <= from < distance.len(), distance[from] != u16::MAX, distance[from] < cut,
        reverse_arc(offsets, edges, lefts, from, target),
    ensures 0 <= target < distance.len(), distance[target] <= distance[from] as int + 1,
{
    let edge = choose|edge: int| offsets[from] <= edge < offsets[from + 1]
        && (#[trigger] edges[edge]) < lefts.len() && lefts[edges[edge] as int] == target;
    assert(closed_slot(edges, lefts, distance, roots, cut, from, edge));
}

pub(crate) proof fn incident_edge(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, right: int, left: u16)
    requires free_incident(offsets, edges, lefts, right, left),
    ensures exists|edge: int| offsets[right] <= edge < offsets[right + 1] && (#[trigger] edges[edge]) == left,
{
    let row = edges.subrange(offsets[right] as int, offsets[right + 1] as int);
    let index = choose|i: int| 0 <= i < row.len() && row[i] == left;
    assert(edges[index + offsets[right]] == left);
}

pub(crate) proof fn terminal_impossible(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>,
    cut: u16, right: int, left: u16)
    requires global_closed(offsets, edges, lefts, distance, roots, cut),
        0 <= right < distance.len(), distance[right] != u16::MAX, distance[right] < cut,
        free_incident(offsets, edges, lefts, right, left),
    ensures false,
{
    incident_edge(offsets, edges, lefts, right, left);
    let edge = choose|edge: int| offsets[right] <= edge < offsets[right + 1] && (#[trigger] edges[edge]) == left;
    assert(closed_slot(edges, lefts, distance, roots, cut, right, edge));
}

pub(crate) proof fn walk_prefix_bound(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>,
    roots: Seq<u16>, cut: u16, path: Seq<int>, end: int)
    requires global_closed(offsets, edges, lefts, distance, roots, cut), distance.len() == rights.len(),
        reverse_walk(offsets, edges, lefts, rights, path, path.last(), path.len() as int - 1),
        0 <= end < path.len(), end <= cut, end < u16::MAX,
        forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> distance[r] == 0,
    ensures distance[path[end]] <= end, distance[path[end]] != u16::MAX,
    decreases end,
{
    if end > 0 {
        walk_prefix_bound(offsets, edges, lefts, rights, distance, roots, cut, path, end - 1);
        assert(reverse_arc(offsets, edges, lefts, path[end - 1], path[end]));
        relaxed_arc(offsets, edges, lefts, distance, roots, cut, path[end - 1], path[end]);
    }
}

pub(crate) proof fn reachable_bound(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>,
    roots: Seq<u16>, cut: u16, right: int, depth: int)
    requires global_closed(offsets, edges, lefts, distance, roots, cut), distance.len() == rights.len(),
        reach_at(offsets, edges, lefts, rights, right, depth), 0 <= depth <= cut, depth < u16::MAX,
        forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> distance[r] == 0,
    ensures 0 <= right < distance.len(), distance[right] <= depth, distance[right] != u16::MAX,
{
    let path = choose|path: Seq<int>| reverse_walk(offsets, edges, lefts, rights, path, right, depth);
    walk_prefix_bound(offsets, edges, lefts, rights, distance, roots, cut, path, depth);
}

pub(crate) proof fn assigned_minimal(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>,
    roots: Seq<u16>, cut: u16, right: int)
    requires distance_witness(offsets, edges, lefts, rights, distance), global_closed(offsets, edges, lefts, distance, roots, cut),
        distance.len() == rights.len(), 0 <= right < distance.len(), distance[right] != u16::MAX,
        cut == u16::MAX || distance[right] <= cut as int + 1,
        forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> distance[r] == 0,
    ensures reach_at(offsets, edges, lefts, rights, right, distance[right] as int),
        forall|depth: int| 0 <= depth < distance[right] ==> !reach_at(offsets, edges, lefts, rights, right, depth),
{
    distance_has_walk(offsets, edges, lefts, rights, distance, right);
    assert forall|depth: int| 0 <= depth < distance[right] implies !reach_at(offsets, edges, lefts, rights, right, depth) by {
        if reach_at(offsets, edges, lefts, rights, right, depth) { reachable_bound(offsets, edges, lefts, rights, distance, roots, cut, right, depth); }
    }
}

pub(crate) proof fn root_complete(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, cut: u16, right: int, left: u16)
    requires global_closed(offsets, edges, lefts, distance, roots, cut), cut != u16::MAX,
        0 <= right < distance.len(), distance[right] == cut, free_incident(offsets, edges, lefts, right, left),
    ensures roots.contains(left),
{
    incident_edge(offsets, edges, lefts, right, left);
    let edge = choose|edge: int| offsets[right] <= edge < offsets[right + 1] && (#[trigger] edges[edge]) == left;
    assert(closed_slot(edges, lefts, distance, roots, cut, right, edge));
}

pub(crate) proof fn shortest_root_complete(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>,
    roots: Seq<u16>, depth: u16, right: int, left: u16)
    requires global_closed(offsets, edges, lefts, distance, roots, depth), depth != u16::MAX,
        distance.len() == rights.len(), reach_at(offsets, edges, lefts, rights, right, depth as int),
        free_incident(offsets, edges, lefts, right, left),
        forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> distance[r] == 0,
    ensures roots.contains(left),
{
    reachable_bound(offsets, edges, lefts, rights, distance, roots, depth, right, depth as int);
    if distance[right] < depth { terminal_impossible(offsets, edges, lefts, distance, roots, depth, right, left); }
    root_complete(offsets, edges, lefts, distance, roots, depth, right, left);
}

pub(crate) proof fn shortest_no_smaller(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>,
    roots: Seq<u16>, cut: u16, right: int, depth: int, left: u16)
    requires global_closed(offsets, edges, lefts, distance, roots, cut), distance.len() == rights.len(),
        0 <= depth < cut, reach_at(offsets, edges, lefts, rights, right, depth), free_incident(offsets, edges, lefts, right, left),
        forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> distance[r] == 0,
    ensures false,
{
    reachable_bound(offsets, edges, lefts, rights, distance, roots, cut, right, depth);
    terminal_impossible(offsets, edges, lefts, distance, roots, cut, right, left);
}

pub(crate) proof fn walk_closed(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>,
    roots: Seq<u16>, path: Seq<int>, end: int)
    requires global_closed(offsets, edges, lefts, distance, roots, u16::MAX), distance.len() == rights.len(), distance.len() <= 1953,
        reverse_walk(offsets, edges, lefts, rights, path, path.last(), path.len() as int - 1), 0 <= end < path.len(),
        forall|r: int| 0 <= r < distance.len() && (#[trigger] distance[r]) != u16::MAX ==> distance[r] < distance.len(),
        forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> distance[r] == 0,
    ensures distance[path[end]] != u16::MAX,
    decreases end,
{
    if end > 0 {
        walk_closed(offsets, edges, lefts, rights, distance, roots, path, end - 1);
        assert(reverse_arc(offsets, edges, lefts, path[end - 1], path[end]));
        relaxed_arc(offsets, edges, lefts, distance, roots, u16::MAX, path[end - 1], path[end]);
    }
}

pub(crate) proof fn failed_reachability(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, right: int)
    requires global_closed(offsets, edges, lefts, distance, roots, u16::MAX), distance_witness(offsets, edges, lefts, rights, distance),
        distance.len() == rights.len(), distance.len() <= 1953, 0 <= right < distance.len(),
        forall|r: int| 0 <= r < distance.len() && (#[trigger] distance[r]) != u16::MAX ==> distance[r] < distance.len(),
        forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> distance[r] == 0,
    ensures distance[right] != u16::MAX <==> exists|depth: int| 0 <= depth && #[trigger] reach_at(offsets, edges, lefts, rights, right, depth),
{
    if distance[right] != u16::MAX {
        distance_has_walk(offsets, edges, lefts, rights, distance, right);
    }
    if exists|depth: int| 0 <= depth && #[trigger] reach_at(offsets, edges, lefts, rights, right, depth) {
        let depth = choose|depth: int| 0 <= depth && #[trigger] reach_at(offsets, edges, lefts, rights, right, depth);
        let path = choose|path: Seq<int>| reverse_walk(offsets, edges, lefts, rights, path, right, depth);
        walk_closed(offsets, edges, lefts, rights, distance, roots, path, depth);
    }
}

pub(crate) proof fn shortest_has_terminal(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, roots: Seq<u16>, depth: u16)
    requires distance_witness(offsets, edges, lefts, rights, distance), roots_sound(offsets, edges, lefts, distance, roots, depth), roots.len() > 0,
    ensures exists|right: int, left: u16| #[trigger] free_incident(offsets, edges, lefts, right, left)
        && reach_at(offsets, edges, lefts, rights, right, depth as int),
{
    root_witness(offsets, edges, lefts, distance, roots, depth, 0);
    let right = choose|right: int| 0 <= right < distance.len() && (#[trigger] distance[right]) == depth
        && free_incident(offsets, edges, lefts, right, roots[0]);
    distance_has_walk(offsets, edges, lefts, rights, distance, right);
}

fn bfs_one_edge_keeps_seed_and_finds_root() {
    let offsets = [0u16, 1];
    let edges = [0u16];
    let transpose = SparseAdjacencyRef { offsets: &offsets, edges: &edges };
    let lefts = [u16::MAX];
    let rights = [u16::MAX];
    let mut distance = [17u16];
    let mut seen = [true];
    let mut queue = FixedVec::<u16, 1953>::new(0);
    let mut roots = FixedVec::<u16, 3969>::new(0);
    let result = reverse_levels(&transpose, &lefts, &rights, &mut distance, &mut seen, &mut queue, &mut roots);
    assert(distance[0] == 0);
    match result { None => { assert(closed_slot(edges@, lefts@, distance@, roots@, u16::MAX, 0, 0)); }, Some(_) => {} }
    assert(result == Some(0));
    assert(roots@.len() == 1);
    assert(roots@[0] == 0);
    assert(seen[0] == false);
    #[cfg(reject_bfs_seed)]
    assert(distance[0] != 0);
    #[cfg(reject_bfs_depth)]
    assert(result == Some(1));
    #[cfg(reject_bfs_missing_root)]
    assert(roots@.len() == 0);
}

}
