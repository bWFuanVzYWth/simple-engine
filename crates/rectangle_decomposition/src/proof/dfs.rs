//! 分层及追加 DFS 的栈所有权、逐边耗尽与路径见证。
use super::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

#[verifier::opaque]
pub(in crate::optimal) open spec fn domain(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16) -> bool {
    proof::matching_valid(offsets, edges, lefts, rights) && distance.len() == rights.len() && depth < rights.len()
        && (forall|e: int| 0 <= e < edges.len() ==> #[trigger] edges[e] < rights.len())
        && (forall|r: int| 0 <= r < distance.len() ==> #[trigger] distance[r] == u16::MAX || distance[r] < rights.len())
        && forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> #[trigger] distance[r] == 0
}

pub(in crate::optimal) open spec fn rank(lefts: Seq<u16>, distance: Seq<u16>, depth: u16, left: int) -> int {
    if lefts[left] == u16::MAX { depth as int + 1 } else { distance[lefts[left] as int] as int }
}

pub(in crate::optimal) open spec fn allowed(lefts: Seq<u16>, distance: Seq<u16>, depth: u16, left: int, right: int) -> bool {
    distance[right] as int + 1 == rank(lefts, distance, depth, left)
}

pub(in crate::optimal) open spec fn increased(before: Seq<bool>, after: Seq<bool>) -> bool {
    before.len() == after.len() && forall|l: int| 0 <= l < before.len() && before[l] ==> #[trigger] after[l]
}

pub(in crate::optimal) open spec fn nodes(stack: Seq<DfsFrame>) -> Seq<u16> { Seq::new(stack.len(), |i: int| stack[i].left) }
pub(in crate::optimal) open spec fn contains(stack: Seq<DfsFrame>, left: int) -> bool { nodes(stack).contains(left as u16) }

pub(in crate::optimal) open spec fn traversable(lefts: Seq<u16>, distance: Seq<u16>, depth: u16, left: int, right: int, layered: bool) -> bool {
    !layered || allowed(lefts, distance, depth, left, right)
}

#[verifier::opaque]
pub(in crate::optimal) open spec fn closed(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    seen: Seq<bool>, left: int, end: int, layered: bool) -> bool {
    forall|e: int| offsets[left] <= e < end && traversable(lefts, distance, depth, left, #[trigger] edges[e] as int, layered)
        ==> rights[edges[e] as int] != u16::MAX && seen[rights[edges[e] as int] as int]
}

pub(in crate::optimal) open spec fn exhausted(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, layered: bool) -> bool {
    forall|l: int| 0 <= l < seen.len() && seen[l] && !before[l] && !contains(stack, l)
        ==> closed(offsets, edges, lefts, rights, distance, depth, seen, l, offsets[l + 1] as int, layered)
}

#[verifier::opaque]
pub(in crate::optimal) open spec fn state(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, start: int, layered: bool) -> bool {
    domain(offsets, edges, lefts, rights, distance, depth) && before.len() == lefts.len() && increased(before, seen)
        && 0 <= start < lefts.len() && lefts[start] == u16::MAX && !before[start] && seen[start]
        && stack.len() <= lefts.len() && (stack.len() > 0 ==> stack[0].left == start)
        && nodes(stack).no_duplicates()
        && (forall|i: int| 0 <= i < stack.len() ==> (#[trigger] stack[i]).left < lefts.len()
            && seen[stack[i].left as int] && !before[stack[i].left as int]
            && offsets[stack[i].left as int] <= stack[i].next_edge <= offsets[stack[i].left as int + 1]
            && (layered ==> rank(lefts, distance, depth, stack[i].left as int) == depth as int + 1 - i)
            && closed(offsets, edges, lefts, rights, distance, depth, seen, stack[i].left as int, stack[i].next_edge as int, layered))
        && (forall|i: int| 0 <= i < stack.len() - 1 ==> offsets[(#[trigger] stack[i]).left as int] < stack[i].next_edge
            && rights[edges[stack[i].next_edge as int - 1] as int] == stack[i + 1].left)
        && exhausted(offsets, edges, lefts, rights, distance, depth, before, seen, stack, layered)
}

pub(in crate::optimal) open spec fn unseen(seen: Seq<bool>, end: int) -> nat
    recommends 0 <= end <= seen.len(), decreases end,
{
    if end <= 0 { 0 } else { unseen(seen, end - 1) + if seen[end - 1] { 0nat } else { 1nat } }
}
pub(in crate::optimal) open spec fn work(seen: Seq<bool>, stack: Seq<DfsFrame>) -> int { 2 * unseen(seen, seen.len() as int) as int + stack.len() as int }

pub(in crate::optimal) proof fn bounds(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16)
    requires domain(offsets, edges, lefts, rights, distance, depth),
    ensures lefts.len() <= 1953, rights.len() <= 1953, offsets.len() == lefts.len() + 1, distance.len() == rights.len(),
{ reveal(domain); }

pub(in crate::optimal) proof fn after_commit(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    after_lefts: Seq<u16>, after_rights: Seq<u16>)
    requires domain(offsets, edges, lefts, rights, distance, depth),
        proof::matching_valid(offsets, edges, after_lefts, after_rights), proof::matched_preserved(rights, after_rights),
    ensures domain(offsets, edges, after_lefts, after_rights, distance, depth),
{ reveal(domain); }

pub(in crate::optimal) proof fn unmodified(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16)
    requires domain(offsets, edges, lefts, rights, distance, depth),
    ensures proof::matching_valid(offsets, edges, lefts, rights),
{ reveal(domain); }

pub(in crate::optimal) proof fn last_assigned(initial_left: Seq<u16>, initial_right: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>,
    edges: Seq<u16>, stack: Seq<DfsFrame>, terminal: u16)
    requires stack.len() > 0, proof::path_state(initial_left, initial_right, lefts, rights, edges, stack, terminal, 0),
    ensures lefts[stack.last().left as int] == terminal,
{
    assert(lefts[stack[stack.len() as int - 1].left as int] == proof::path_right(edges, stack, terminal, stack.len() as int - 1));
}

pub(in crate::optimal) proof fn observed(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, start: int, layered: bool)
    requires state(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, layered),
    ensures increased(before, seen), seen[start], seen.len() == lefts.len(),
{ reveal(state); }

proof fn newly_seen(seen: Seq<bool>, left: int, end: int)
    requires 0 <= left < seen.len(), !seen[left], 0 <= end <= seen.len(),
    ensures unseen(seen, end) == unseen(seen.update(left, true), end) + if left < end { 1nat } else { 0nat },
    decreases end,
{
    if end > 0 { newly_seen(seen, left, end - 1); }
}

pub(in crate::optimal) proof fn start(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, left: int, layered: bool)
    requires domain(offsets, edges, lefts, rights, distance, depth), before.len() == lefts.len(),
        0 <= left < lefts.len(), lefts[left] == u16::MAX, !before[left],
    ensures state(offsets, edges, lefts, rights, distance, depth, before, before.update(left, true),
        seq![DfsFrame { left: left as u16, next_edge: offsets[left] }], left, layered),
{
    reveal(state); reveal(domain); reveal(closed);
    let stack = seq![DfsFrame { left: left as u16, next_edge: offsets[left] }];
    assert(nodes(stack) =~= seq![left as u16]);
}

pub(in crate::optimal) proof fn at(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, start: int, layered: bool)
    requires state(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, layered), stack.len() > 0,
    ensures stack.last().left < lefts.len(),
        edges.len() <= 3969,
        domain(offsets, edges, lefts, rights, distance, depth), seen.len() == lefts.len(), stack.len() <= lefts.len(),
        lefts.len() <= 1953, rights.len() <= 1953, offsets.len() == lefts.len() + 1,
        offsets[stack.last().left as int] <= stack.last().next_edge <= offsets[stack.last().left as int + 1] <= edges.len(),
        layered ==> rank(lefts, distance, depth, stack.last().left as int) == depth as int + 2 - stack.len(),
        closed(offsets, edges, lefts, rights, distance, depth, seen, stack.last().left as int, stack.last().next_edge as int, layered),
{
    reveal(state); reveal(domain);
    proof::row_bounds(offsets, edges.len(), stack.last().left as int);
}

proof fn closed_empty(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    seen: Seq<bool>, left: int, layered: bool)
    ensures closed(offsets, edges, lefts, rights, distance, depth, seen, left, offsets[left] as int, layered),
{ reveal(closed); }

pub(in crate::optimal) proof fn slot(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    left: int, edge: int)
    requires domain(offsets, edges, lefts, rights, distance, depth), 0 <= left < lefts.len(), offsets[left] <= edge < offsets[left + 1],
    ensures 0 <= edge < edges.len(), edges[edge] < rights.len(), distance.len() == rights.len(),
        rights[edges[edge] as int] == u16::MAX || rights[edges[edge] as int] < lefts.len(),
{
    reveal(domain);
    proof::row_bounds(offsets, edges.len(), left);
}

pub(in crate::optimal) proof fn neighbor(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    left: int, right: int)
    requires domain(offsets, edges, lefts, rights, distance, depth), proof::edge_in(offsets, edges, left, right),
    ensures 0 <= left < lefts.len(), 0 <= right < rights.len(),
{
    reveal(domain);
    let row = edges.subrange(offsets[left] as int, offsets[left + 1] as int);
    proof::row_bounds(offsets, edges.len(), left);
    let i = choose|i: int| 0 <= i < row.len() && row[i] == right as u16;
    assert(edges[i + offsets[left]] == right);
}

pub(in crate::optimal) proof fn scan(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    seen: Seq<bool>, left: int, edge: int, layered: bool)
    requires domain(offsets, edges, lefts, rights, distance, depth), seen.len() == lefts.len(), 0 <= left < lefts.len(),
        offsets[left] <= edge < offsets[left + 1], closed(offsets, edges, lefts, rights, distance, depth, seen, left, edge, layered),
        !traversable(lefts, distance, depth, left, edges[edge] as int, layered)
            || rights[edges[edge] as int] != u16::MAX && seen[rights[edges[edge] as int] as int],
    ensures closed(offsets, edges, lefts, rights, distance, depth, seen, left, edge + 1, layered),
{ reveal(domain); reveal(closed); }

pub(in crate::optimal) proof fn found(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, start: int, edge: int, layered: bool)
    requires state(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, layered), stack.len() > 0,
        offsets[stack.last().left as int] <= edge < offsets[stack.last().left as int + 1],
        rights[edges[edge] as int] == u16::MAX,
        layered ==> distance[edges[edge] as int] as int + stack.len() == depth as int + 1,
    ensures proof::path_links(offsets, edges, lefts, rights, stack, edges[edge]), layered ==> stack.len() == depth as int + 1,
{
    reveal(state); reveal(domain);
    at(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, layered);
    proof::edge_at(offsets, edges, stack.last().left as int, edge);
    assert forall|i: int, j: int| #![auto] 0 <= i < j < stack.len() implies stack[i].left != stack[j].left by {
        assert(nodes(stack)[i] == stack[i].left);
        assert(nodes(stack)[j] == stack[j].left);
    }
}

proof fn closed_grows(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, after: Seq<bool>, left: int, end: int, layered: bool)
    requires domain(offsets, edges, lefts, rights, distance, depth), before.len() == lefts.len(), increased(before, after),
        0 <= left < lefts.len(), offsets[left] <= end <= offsets[left + 1],
        closed(offsets, edges, lefts, rights, distance, depth, before, left, end, layered),
    ensures closed(offsets, edges, lefts, rights, distance, depth, after, left, end, layered),
{
    reveal(domain); reveal(closed);
    assert forall|e: int| offsets[left] <= e < end && traversable(lefts, distance, depth, left, #[trigger] edges[e] as int, layered)
        implies rights[edges[e] as int] != u16::MAX && after[rights[edges[e] as int] as int] by {
        let r = edges[e] as int;
        assert(before[rights[r] as int]);
        assert(rights[r] < lefts.len());
    }
}

pub(in crate::optimal) open spec fn pushed(stack: Seq<DfsFrame>, edge: u16, next: u16, next_edge: u16) -> Seq<DfsFrame> {
    stack.update(stack.len() as int - 1, DfsFrame { left: stack.last().left, next_edge: edge })
        .push(DfsFrame { left: next, next_edge })
}

pub(in crate::optimal) proof fn descend(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, start: int, edge: int, layered: bool)
    requires state(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, layered), stack.len() > 0,
        offsets[stack.last().left as int] <= edge < offsets[stack.last().left as int + 1],
        closed(offsets, edges, lefts, rights, distance, depth, seen, stack.last().left as int, edge, layered),
        layered ==> distance[edges[edge] as int] as int + stack.len() == depth as int + 1,
        rights[edges[edge] as int] != u16::MAX, !seen[rights[edges[edge] as int] as int],
    ensures stack.len() < lefts.len(),
        state(offsets, edges, lefts, rights, distance, depth, before, seen.update(rights[edges[edge] as int] as int, true),
            pushed(stack, (edge + 1) as u16, rights[edges[edge] as int], offsets[rights[edges[edge] as int] as int]), start, layered),
        work(seen.update(rights[edges[edge] as int] as int, true),
            pushed(stack, (edge + 1) as u16, rights[edges[edge] as int], offsets[rights[edges[edge] as int] as int])) < work(seen, stack),
{
    hide(proof::edge_in);
    hide(proof::rows_valid);
    hide(closed);
    reveal(state); reveal(domain);
    at(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, layered);
    let right = edges[edge] as int;
    let next = rights[right] as int;
    let after = seen.update(next, true);
    let result = pushed(stack, (edge + 1) as u16, next as u16, offsets[next]);
    assert(lefts[next] == right);
    proof::row_bounds(offsets, edges.len(), next);
    assert(!nodes(stack).contains(next as u16)) by {
        if nodes(stack).contains(next as u16) {
            let i = choose|i: int| 0 <= i < stack.len() && nodes(stack)[i] == next as u16;
            assert(seen[stack[i].left as int]);
        }
    }
    assert(levels_proof::bounded_ids(nodes(stack), lefts.len()));
    levels_proof::ids_room(nodes(stack), lefts.len(), next as u16);
    assert(nodes(result) =~= nodes(stack).push(next as u16));
    assert(increased(seen, after));
    closed_grows(offsets, edges, lefts, rights, distance, depth, seen, after, stack.last().left as int, edge, layered);
    scan(offsets, edges, lefts, rights, distance, depth, after, stack.last().left as int, edge, layered);
    closed_empty(offsets, edges, lefts, rights, distance, depth, after, next, layered);
    assert forall|i: int| 0 <= i < result.len() implies (#[trigger] result[i]).left < lefts.len()
        && after[result[i].left as int] && !before[result[i].left as int]
        && offsets[result[i].left as int] <= result[i].next_edge <= offsets[result[i].left as int + 1]
        && (layered ==> rank(lefts, distance, depth, result[i].left as int) == depth as int + 1 - i)
        && closed(offsets, edges, lefts, rights, distance, depth, after, result[i].left as int, result[i].next_edge as int, layered) by {
        if i < stack.len() - 1 {
            assert(result[i] == stack[i]);
            closed_grows(offsets, edges, lefts, rights, distance, depth, seen, after, stack[i].left as int, stack[i].next_edge as int, layered);
        } else if i == stack.len() - 1 {
            assert(result[i].left == stack.last().left);
            assert(result[i].next_edge == edge + 1);
        } else {
            assert(result[i].left == next);
            assert(result[i].next_edge == offsets[next]);
        }
    }
    assert forall|i: int| 0 <= i < result.len() - 1 implies offsets[(#[trigger] result[i]).left as int] < result[i].next_edge
        && rights[edges[result[i].next_edge as int - 1] as int] == result[i + 1].left by {}
    assert forall|l: int| 0 <= l < after.len() && after[l] && !before[l] && !contains(result, l)
        implies closed(offsets, edges, lefts, rights, distance, depth, after, l, offsets[l + 1] as int, layered) by {
        assert(l != next);
        assert(!contains(stack, l));
        proof::row_bounds(offsets, edges.len(), l);
        closed_grows(offsets, edges, lefts, rights, distance, depth, seen, after, l, offsets[l + 1] as int, layered);
    }
    newly_seen(seen, next, seen.len() as int);
}

pub(in crate::optimal) proof fn ascend(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    before: Seq<bool>, seen: Seq<bool>, stack: Seq<DfsFrame>, start: int, layered: bool)
    requires state(offsets, edges, lefts, rights, distance, depth, before, seen, stack, start, layered), stack.len() > 0,
        closed(offsets, edges, lefts, rights, distance, depth, seen, stack.last().left as int, offsets[stack.last().left as int + 1] as int, layered),
    ensures state(offsets, edges, lefts, rights, distance, depth, before, seen, stack.drop_last(), start, layered),
        work(seen, stack.drop_last()) < work(seen, stack),
{
    reveal(state); reveal(domain);
    let result = stack.drop_last();
    assert(nodes(result) =~= nodes(stack).drop_last());
    assert forall|l: int| 0 <= l < seen.len() && seen[l] && !before[l] && !contains(result, l)
        implies closed(offsets, edges, lefts, rights, distance, depth, seen, l, offsets[l + 1] as int, layered) by {
        if contains(stack, l) {
            let i = choose|i: int| 0 <= i < stack.len() && nodes(stack)[i] == l as u16;
            assert(i == stack.len() - 1) by { if i < result.len() { assert(nodes(result)[i] == l as u16); } }
        }
    }
}

pub(in crate::optimal) proof fn closed_neighbor(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16,
    seen: Seq<bool>, left: int, right: int)
    requires domain(offsets, edges, lefts, rights, distance, depth), seen.len() == lefts.len(),
        proof::edge_in(offsets, edges, left, right), allowed(lefts, distance, depth, left, right),
        closed(offsets, edges, lefts, rights, distance, depth, seen, left, offsets[left + 1] as int, true),
    ensures 0 <= right < rights.len(), rights[right] != u16::MAX, seen[rights[right] as int],
{
    reveal(domain); reveal(closed);
    proof::row_bounds(offsets, edges.len(), left);
    let row = edges.subrange(offsets[left] as int, offsets[left + 1] as int);
    let i = choose|i: int| 0 <= i < row.len() && row[i] == right as u16;
    let slot = i + offsets[left];
    assert(edges[slot] == right);
}

pub(in crate::optimal) proof fn tight_prefix(offsets: Seq<u16>, edges: Seq<u16>, lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>,
    roots: Seq<u16>, depth: u16, path: Seq<int>, end: int)
    requires levels_proof::global_closed(offsets, edges, lefts, distance, roots, depth), distance.len() == rights.len(),
        levels_proof::reverse_walk(offsets, edges, lefts, rights, path, path.last(), path.len() as int - 1),
        0 <= end < path.len(), end <= depth, depth < u16::MAX, distance[path[end]] == end,
        forall|r: int| 0 <= r < rights.len() && rights[r] == u16::MAX ==> distance[r] == 0,
    ensures forall|i: int| 0 <= i <= end ==> distance[#[trigger] path[i]] == i,
    decreases end,
{
    if end > 0 {
        levels_proof::walk_prefix_bound(offsets, edges, lefts, rights, distance, roots, depth, path, end - 1);
        levels_proof::relaxed_arc(offsets, edges, lefts, distance, roots, depth, path[end - 1], path[end]);
        tight_prefix(offsets, edges, lefts, rights, distance, roots, depth, path, end - 1);
    }
}

proof fn previous_arc(offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16, from: int, target: int)
    requires domain(offsets, edges, lefts, rights, distance, depth),
        proof::transposed(offsets, edges, transpose_offsets, transpose_edges, rights.len()),
        levels_proof::reverse_arc(transpose_offsets, transpose_edges, lefts, from, target),
    ensures 0 <= from < rights.len(), 0 <= target < rights.len(), rights[target] != u16::MAX,
        rights[target] < lefts.len(), lefts[rights[target] as int] == target, target != u16::MAX,
        proof::edge_in(offsets, edges, rights[target] as int, from),
{
    hide(proof::edge_in); hide(proof::rows_valid); hide(proof::owner); hide(proof::transpose_prefix);
    reveal(domain);
    let slot = choose|slot: int| transpose_offsets[from] <= slot < transpose_offsets[from + 1]
        && (#[trigger] transpose_edges[slot]) < lefts.len() && lefts[transpose_edges[slot] as int] == target;
    proof::transpose_at(offsets, edges, transpose_offsets, transpose_edges, rights.len(), from, slot);
    let left = transpose_edges[slot] as int;
    assert(rights[target] == left);
}

proof fn exhausted_walk(offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16, seen: Seq<bool>, path: Seq<int>, end: int, left: int)
    requires domain(offsets, edges, lefts, rights, distance, depth), seen.len() == lefts.len(),
        proof::transposed(offsets, edges, transpose_offsets, transpose_edges, rights.len()),
        levels_proof::reverse_walk(transpose_offsets, transpose_edges, lefts, rights, path, path.last(), path.len() as int - 1),
        forall|i: int| 0 <= i < path.len() ==> distance[#[trigger] path[i]] == i,
        forall|l: int| 0 <= l < seen.len() && seen[l] ==> closed(offsets, edges, lefts, rights, distance, depth, seen, l, offsets[l + 1] as int, true),
        0 <= end < path.len(), 0 <= left < lefts.len(), seen[left],
        rank(lefts, distance, depth, left) == end + 1, proof::edge_in(offsets, edges, left, path[end]),
    ensures false,
    decreases end,
{
    hide(proof::edge_in); hide(proof::rows_valid); hide(proof::owner); hide(proof::transpose_prefix);
    hide(proof::transposed); hide(levels_proof::reverse_arc);
    closed_neighbor(offsets, edges, lefts, rights, distance, depth, seen, left, path[end]);
    if end > 0 {
        assert(levels_proof::reverse_arc(transpose_offsets, transpose_edges, lefts, path[end - 1], path[end]));
        previous_arc(offsets, edges, transpose_offsets, transpose_edges, lefts, rights, distance, depth, path[end - 1], path[end]);
        let previous_left = rights[path[end]] as int;
        exhausted_walk(offsets, edges, transpose_offsets, transpose_edges, lefts, rights, distance, depth, seen, path, end - 1, previous_left);
    }
}

/// 空访问集上的首个最短根不能被真实 DFS 耗尽。
pub(in crate::optimal) proof fn first_root_succeeds(offsets: Seq<u16>, edges: Seq<u16>, transpose_offsets: Seq<u16>, transpose_edges: Seq<u16>,
    lefts: Seq<u16>, rights: Seq<u16>, distance: Seq<u16>, depth: u16, roots: Seq<u16>, seen: Seq<bool>, start: int)
    requires domain(offsets, edges, lefts, rights, distance, depth),
        proof::transposed(offsets, edges, transpose_offsets, transpose_edges, rights.len()),
        levels_proof::distance_witness(transpose_offsets, transpose_edges, lefts, rights, distance),
        levels_proof::global_closed(transpose_offsets, transpose_edges, lefts, distance, roots, depth),
        levels_proof::roots_sound(transpose_offsets, transpose_edges, lefts, distance, roots, depth), roots.contains(start as u16),
        state(offsets, edges, lefts, rights, distance, depth, Seq::new(lefts.len(), |_: int| false), seen, Seq::empty(), start, true),
    ensures false,
{
    hide(proof::edge_in); hide(proof::rows_valid); hide(proof::owner); hide(proof::transpose_prefix);
    reveal(domain); reveal(state);
    let index = choose|i: int| 0 <= i < roots.len() && roots[i] == start as u16;
    levels_proof::root_witness(transpose_offsets, transpose_edges, lefts, distance, roots, depth, index);
    let right = choose|r: int| 0 <= r < distance.len() && (#[trigger] distance[r]) == depth
        && levels_proof::free_incident(transpose_offsets, transpose_edges, lefts, r, roots[index]);
    levels_proof::distance_has_walk(transpose_offsets, transpose_edges, lefts, rights, distance, right);
    let path = choose|path: Seq<int>| levels_proof::reverse_walk(transpose_offsets, transpose_edges, lefts, rights, path, right, depth as int);
    tight_prefix(transpose_offsets, transpose_edges, lefts, rights, distance, roots, depth, path, depth as int);
    levels_proof::incident_edge(transpose_offsets, transpose_edges, lefts, right, roots[index]);
    let slot = choose|slot: int| transpose_offsets[right] <= slot < transpose_offsets[right + 1] && (#[trigger] transpose_edges[slot]) == roots[index];
    proof::transpose_at(offsets, edges, transpose_offsets, transpose_edges, rights.len(), right, slot);
    assert forall|l: int| 0 <= l < seen.len() && seen[l] implies closed(offsets, edges, lefts, rights, distance, depth, seen, l, offsets[l + 1] as int, true) by {
        assert(!contains(Seq::empty(), l));
    }
    exhausted_walk(offsets, edges, transpose_offsets, transpose_edges, lefts, rights, distance, depth, seen, path, depth as int, start);
}

fn real_five_edge_search() {
    let offsets = [0u16, 1, 3, 5];
    let edges = [0u16, 0, 1, 1, 2];
    let adjacency = SparseAdjacencyRef { offsets: &offsets, edges: &edges };
    let mut lefts = [u16::MAX; 3];
    let mut rights = [u16::MAX; 3];
    crate::optimal::matching::match_free(&adjacency, 1, 0, &mut lefts, &mut rights);
    crate::optimal::matching::match_free(&adjacency, 2, 1, &mut lefts, &mut rights);
    let distance = [2u16, 1, 0];
    let mut seen = [false; 3];
    let mut stack = ChordBuffer::<DfsFrame>::new(DfsFrame { left: 0, next_edge: 0 });
    reveal(domain);
    let ghost before_seen = seen@;
    let result = find_path(0, &adjacency, &lefts, &rights, &distance, 2, &mut seen, &mut stack, true);
    proof { observed(offsets@, edges@, lefts@, rights@, distance@, 2, before_seen, seen@, stack@, 0, true); }
    if result.is_none() {
        reveal(state); reveal(closed);
        assert(stack@ =~= Seq::<DfsFrame>::empty());
        assert(closed(offsets@, edges@, lefts@, rights@, distance@, 2, seen@, 0, 1, true));
        assert(seen[1]);
        assert(closed(offsets@, edges@, lefts@, rights@, distance@, 2, seen@, 1, 3, true));
        assert(seen[2]);
        assert(closed(offsets@, edges@, lefts@, rights@, distance@, 2, seen@, 2, 5, true));
        assert(false);
    }
    assert(result == Some(2));
    assert(stack@.len() == 3);
    assert(seen[0]);
    #[cfg(reject_dfs_path_depth)]
    assert(stack@.len() == 2);
    #[cfg(reject_dfs_root_seen)]
    assert(!seen[0]);
}

/// 追加搜索必须允许超过旧最短层的真实五边增广路。
fn real_unrestricted_search() {
    let offsets = [0u16, 1, 3, 5];
    let edges = [0u16, 0, 1, 1, 2];
    let adjacency = SparseAdjacencyRef { offsets: &offsets, edges: &edges };
    let mut lefts = [u16::MAX; 3];
    let mut rights = [u16::MAX; 3];
    crate::optimal::matching::match_free(&adjacency, 1, 0, &mut lefts, &mut rights);
    crate::optimal::matching::match_free(&adjacency, 2, 1, &mut lefts, &mut rights);
    let distance = [2u16, 1, 0];
    let mut seen = [false; 3];
    let mut stack = ChordBuffer::<DfsFrame>::new(DfsFrame { left: 0, next_edge: 0 });
    reveal(domain);
    let result = find_path(0, &adjacency, &lefts, &rights, &distance, 0, &mut seen, &mut stack, false);
    if result.is_none() {
        reveal(state); reveal(closed);
        assert(stack@ =~= Seq::<DfsFrame>::empty());
        assert(closed(offsets@, edges@, lefts@, rights@, distance@, 0, seen@, 0, 1, false));
        assert(seen[1]);
        assert(closed(offsets@, edges@, lefts@, rights@, distance@, 0, seen@, 1, 3, false));
        assert(seen[2]);
        assert(closed(offsets@, edges@, lefts@, rights@, distance@, 0, seen@, 2, 5, false));
        assert(false);
    }
    assert(result == Some(2));
    #[cfg(reject_dw_lost_path)]
    assert(result.is_none());
    reveal_with_fuel(proof::matched_count, 4);
    crate::optimal::matching::commit_path(&adjacency, stack.as_slice(), 2, &mut lefts, &mut rights);
    assert(proof::matched_count(lefts@, 3) == 3);
    #[cfg(reject_dw_lost_matching)]
    assert(proof::matched_count(lefts@, 3) == 2);
}
}
