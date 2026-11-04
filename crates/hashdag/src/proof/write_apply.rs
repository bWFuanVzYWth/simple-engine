//! 有序流的逐项归纳；活动帧解释候选内容，执行不复制整图或批次。
use super::*;
use crate::{
    state::{self, Pools},
    write::{
        frontier::proof as frontier, import::proof as import, resolve::proof as resolve,
        route::proof as route,
    },
};
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(super) open spec fn active(initial: Pools, original: Child, source: Seq<proof::EditModel>, done: nat,
    pools: Pools, root: Child, frames: Seq<BuildFrame>) -> bool {
    done <= source.len() && frontier::chain_ready(pools, frames)
        && forall|code: u64| code < 0x1000_0000_0000_0000u64 ==>
            #[trigger] frontier::candidate_value(pools, root, frames, code as int)
                == proof::stream_effect(source, |p: int| state::eval(initial, original, 10, p), code as int, done)
}

pub(super) open spec fn cache<P: LeafPatch>(last: Option<Edit<P>>, pools: Pools, scratch: &WriteScratch) -> bool {
    match last {
        Some(edit) => match edit.data { EditData::Built { tree, .. } => import::source_valid(tree) && import::imported(tree, pools, &scratch.imported), _ => true },
        None => true,
    }
}

proof fn ordered_before(source: Seq<proof::EditModel>, i: int, j: int)
    requires proof::stream_ordered(source), 0 <= i < j < source.len(),
    ensures proof::end(source[i]) <= proof::base(source[j]),
    decreases j - i,
{
    if i + 1 < j {
        ordered_before(source, i, j - 1);
        crate::canonical::span_positive(source[j - 1].lod as nat);
    }
}

proof fn no_cover_prefix(source: Seq<proof::EditModel>, initial: spec_fn(int) -> u32, point: int, done: nat)
    requires done <= source.len(), forall|i: int| 0 <= i < done ==> !#[trigger] proof::covers(source[i], point),
    ensures proof::stream_effect(source, initial, point, done) == initial(point),
    decreases done,
{
    if done > 0 { no_cover_prefix(source, initial, point, (done - 1) as nat); }
}

proof fn unwritten(source: Seq<proof::EditModel>, initial: spec_fn(int) -> u32, point: int, done: nat)
    requires proof::stream_ordered(source), done < source.len(), proof::covers(source[done as int], point),
    ensures proof::stream_effect(source, initial, point, done) == initial(point),
{
    assert forall|i: int| 0 <= i < done implies !#[trigger] proof::covers(source[i], point) by {
        ordered_before(source, i, done as int);
    }
    no_cover_prefix(source, initial, point, done);
}

proof fn same_payload(a: proof::EditModel, b: proof::EditModel, initial: spec_fn(int) -> u32, point: int)
    requires proof::same_edit(a, b),
    ensures proof::covers(a, point) == proof::covers(b, point),
        proof::payload_value(a, initial, point) == proof::payload_value(b, initial, point),
{
    if let (proof::Payload::Patch(left), proof::Payload::Patch(right)) = (a.payload, b.payload) {
        let values = Seq::new(64, |i: int| initial(proof::base(a) + i));
        assert(values.len() == 64);
        assert(left(values) == right(values));
    }
}

proof fn local_payload(edit: proof::EditModel, a: spec_fn(int) -> u32, b: spec_fn(int) -> u32, point: int)
    requires proof::edit_shape(edit), proof::covers(edit, point),
        forall|p: int| #[trigger] proof::covers(edit, p) ==> a(p) == b(p),
    ensures proof::payload_value(edit, a, point) == proof::payload_value(edit, b, point),
{
    if let proof::Payload::Patch(patch) = edit.payload {
        assert(edit.lod == 1);
        reveal_with_fuel(state::span, 2);
        assert forall|i: int| 0 <= i < 64 implies #[trigger] a(proof::base(edit) + i) == b(proof::base(edit) + i) by {
            assert(proof::covers(edit, proof::base(edit) + i));
        }
        assert(Seq::new(64, |i: int| a(proof::base(edit) + i)) =~= Seq::new(64, |i: int| b(proof::base(edit) + i)));
    }
}

pub(super) proof fn wide_first(source: Seq<proof::EditModel>, done: nat)
    requires proof::stream_ordered(source), done < source.len(), source[done as int].lod == 10,
    ensures done == 0,
{
    frontier::span_bits(10);
    assert(proof::edit_shape(source[done as int]));
    assert((1u64 << 60u32) == 0x1000_0000_0000_0000u64) by(bit_vector);
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod_converse(source[done as int].path as int, state::span(10) as int, 0, source[done as int].path as int);
    if done > 0 {
        let previous = source[(done - 1) as int];
        crate::canonical::span_positive(previous.lod as nat);
        vstd::arithmetic::div_mod::lemma_div_pos_is_pos(previous.path as int, proof::scope(previous));
        assert(proof::end(previous) <= proof::base(source[done as int]));
    }
}

pub(super) proof fn frozen_active(initial: Pools, original: Child, source: Seq<proof::EditModel>, done: nat,
    before: Pools, after: Pools, root: Child, frames: Seq<BuildFrame>)
    requires active(initial, original, source, done, before, root, frames), state::valid_pools(before),
        state::valid_pools(after), state::frozen(before, after), state::child_valid(before, root, 10),
    ensures active(initial, original, source, done, after, root, frames), state::child_valid(after, root, 10),
{
    crate::canonical::frozen_value(before, after, root, 10, 0);
    frontier::chain_frozen(before, after, frames);
    assert forall|code: u64| code < 0x1000_0000_0000_0000u64 implies
        #[trigger] frontier::candidate_value(after, root, frames, code as int)
            == proof::stream_effect(source, |p: int| state::eval(initial, original, 10, p), code as int, done) by {
        frontier::candidate_frozen(before, after, root, frames, code as int);
    }
}

pub(super) proof fn route_ready<P: LeafPatch>(pools: &[Pool; 10], root: Child, source: Seq<proof::EditModel>, done: nat,
    edit: Edit<P>, replacement: Replacement<P>, scratch: &WriteScratch)
    requires super::super::commit::proof::building(pools), state::child_valid(state::pools_view(pools), root, 10),
        frontier::chain_ready(state::pools_view(pools), scratch.frames@),
        proof::stream_ordered(source), done < source.len(), proof::same_edit(proof::edit_view(edit), source[done as int]),
        proof::edit_ready(edit), resolve::selected(edit, replacement, state::pools_view(pools)),
        done == 0 ==> scratch.frames@.len() == 0,
    ensures route::ready(state::pools_view(pools), root, edit, replacement, scratch),
{
    assert(proof::edit_shape(source[done as int]));
    assert(proof::edit_shape(proof::edit_view(edit)));
    if let Replacement::Patch(patch) = replacement {
        assert(edit.data == EditData::Patch(patch));
    }
    if edit.lod == 10 { wide_first(source, done); }
}

pub(super) proof fn advanced<P: LeafPatch>(initial: Pools, original: Child, source: Seq<proof::EditModel>, done: nat,
    before: Pools, after: Pools, old_root: Child, root: Child, edit: Edit<P>, old: &WriteScratch, new: &WriteScratch)
    requires active(initial, original, source, done, before, old_root, old.frames@),
        proof::stream_ordered(source), done < source.len(), proof::same_edit(proof::edit_view(edit), source[done as int]),
        route::routed(before, after, old_root, root, edit, old, new),
    ensures active(initial, original, source, done + 1, after, root, new.frames@),
{
    frontier::span_bits(10);
    assert((1u64 << 60u32) == 0x1000_0000_0000_0000u64) by(bit_vector);
    let expected = source[done as int];
    let input = |p: int| state::eval(initial, original, 10, p);
    let previous = |p: int| frontier::candidate_value(before, old_root, old.frames@, p);
    assert forall|p: int| proof::covers(expected, p) implies previous(p) == input(p) by {
        frontier::span_bits(10);
        route::scope_bounds(expected);
        unwritten(source, input, p, done);
        // edit_shape 将整个覆盖区限制在 60-bit 根空间。
        assert(0 <= p < state::span(10));
        let code = p as u64;
        assert(code < 0x1000_0000_0000_0000u64 && code as int == p);
        assert(frontier::candidate_value(before, old_root, old.frames@, code as int)
            == proof::stream_effect(source, input, code as int, done));
    }
    assert forall|code: u64| code < 0x1000_0000_0000_0000u64 implies
        #[trigger] frontier::candidate_value(after, root, new.frames@, code as int)
            == proof::stream_effect(source, input, code as int, done + 1) by {
        same_payload(proof::edit_view(edit), expected, previous, code as int);
        if proof::covers(expected, code as int) { local_payload(expected, previous, input, code as int); }
    }
}

pub(super) proof fn next_ready<P: LeafPatch>(source: Seq<proof::EditModel>, done: nat, last: Option<Edit<P>>, edit: Edit<P>, pools: Pools, scratch: &WriteScratch)
    requires proof::stream_ordered(source), done < source.len(), cache(last, pools, scratch),
        proof::same_edit(proof::edit_view(edit), source[done as int]), proof::edit_ready(edit),
        done == 0 ==> last is None,
        done > 0 ==> last is Some && proof::same_edit(proof::edit_view(last->Some_0), source[(done - 1) as int]),
    ensures resolve::ready(edit, pools, scratch),
{
    if let EditData::Built { tree, part } = edit.data {
        if part > 0 {
            assert(done > 0);
            let previous = last->Some_0;
            assert(proof::built_continuation(source[(done - 1) as int], source[done as int]));
            if let EditData::Built { tree: old_tree, .. } = previous.data {
                assert(import::imported(old_tree, pools, &scratch.imported));
                assert(state::pools_view(&tree.pools) == state::pools_view(&old_tree.pools));
                assert(tree.roots@ == old_tree.roots@ && tree.lod == old_tree.lod);
                assert(import::imported(tree, pools, &scratch.imported));
            } else { assert(false); }
        }
    }
}

pub(super) proof fn cache_frozen<P: LeafPatch>(last: Option<Edit<P>>, before: Pools, after: Pools, old: &WriteScratch, new: &WriteScratch)
    requires cache(last, before, old), state::build_step(before, after), resolve::cache_same(old, new),
    ensures cache(last, after, new),
{
    if let Some(edit) = last {
        if let EditData::Built { tree, .. } = edit.data {
            import::imported_frozen(tree, before, after, &old.imported, &new.imported);
        }
    }
}

pub(super) proof fn closed(initial: Pools, original: Child, source: Seq<proof::EditModel>, done: nat,
    before: Pools, after: Pools, old_root: Child, root: Child, frames: Seq<BuildFrame>, next: Seq<BuildFrame>)
    requires active(initial, original, source, done, before, old_root, frames), frames.len() > 0,
        frontier::chain_ready(after, next),
        if frames.len() == 1 { next.len() == 0 } else { next.len() > 0 },
        forall|code: u64| code < 0x1000_0000_0000_0000u64 ==> #[trigger] frontier::closed_value(before, after, frames, next, root, code),
    ensures active(initial, original, source, done, after, root, next),
{
    assert forall|code: u64| code < 0x1000_0000_0000_0000u64 implies
        #[trigger] frontier::candidate_value(after, root, next, code as int)
            == proof::stream_effect(source, |p: int| state::eval(initial, original, 10, p), code as int, done) by {
        assert(frontier::closed_value(before, after, frames, next, root, code));
        assert(frontier::candidate_value(before, old_root, frames, code as int)
            == proof::stream_effect(source, |p: int| state::eval(initial, original, 10, p), code as int, done));
    }
}

pub(super) proof fn completed(initial: Pools, original: Child, source: Seq<proof::EditModel>, pools: Pools, root: Child, frames: Seq<BuildFrame>)
    requires active(initial, original, source, source.len(), pools, root, frames), frames.len() == 0,
    ensures forall|point: int| 0 <= point < state::span(10) ==> #[trigger] state::eval(pools, root, 10, point)
        == proof::stream_effect(source, |p: int| state::eval(initial, original, 10, p), point, source.len()),
{
    frontier::span_bits(10);
    assert((1u64 << 60u32) == 0x1000_0000_0000_0000u64) by(bit_vector);
    assert forall|point: int| 0 <= point < state::span(10) implies #[trigger] state::eval(pools, root, 10, point)
        == proof::stream_effect(source, |p: int| state::eval(initial, original, 10, p), point, source.len()) by {
        let code = point as u64;
        assert(code < 0x1000_0000_0000_0000u64 && code as int == point);
        assert(frontier::candidate_value(pools, root, frames, code as int)
            == proof::stream_effect(source, |p: int| state::eval(initial, original, 10, p), code as int, source.len()));
    }
}
}
