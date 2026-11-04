//! 高 LOD dense 的连续原生分段；数据仍借用原输入。
use super::*;
use crate::morton::proof as morton;
use crate::write::fill_uniform::{UniformFill, proof as fill};
use crate::write::ordered::proof as ordered;
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) open spec fn domain(region: DenseRegion) -> bool {
    region.lod >= 2 && validate::dense_at(seq![region], 0)
}
pub(crate) open spec fn edit(region: DenseRegion, part: nat) -> EditModel {
    let width = region.values@.len() / parts(region.lod);
    EditModel { path: path(morton::value(region.path), region.lod, part as u64), lod: region.lod / 2,
        payload: Payload::Dense(region.values@.subrange((part * width) as int, ((part + 1) * width) as int)) }
}
pub(crate) open spec fn block(region: DenseRegion, part: nat) -> Seq<EditModel> {
    Seq::new((parts(region.lod) - part) as nat, |i: int| edit(region, (part + i) as nat))
}

pub(crate) proof fn geometry(region: DenseRegion, part: nat)
    requires domain(region), part < parts(region.lod),
    ensures ordered::edit_shape(edit(region, part)),
        state::span((region.lod / 2) as nat) > 0,
        region.values@.len() == parts(region.lod) * state::span((region.lod / 2) as nat),
        region.values@.len() == validate::size(region.lod) as nat,
        ordered::base(edit(region, part)) == morton::value(region.path) as int + part * state::span((region.lod / 2) as nat),
        ordered::end(edit(region, part)) == morton::value(region.path) as int + (part + 1) * state::span((region.lod / 2) as nat),
{
    let uniform = UniformFill { path: region.path, lod: region.lod, data: 0 };
    let code = morton::value(region.path);
    let size = validate::size(region.lod);
    let aligned = validate::base(uniform);
    assert(aligned == code) by(bit_vector)
        requires code & ((size - 1) as u64) == 0, aligned == code & !((size - 1) as u64);
    fill::part_geometry(uniform, part);
    let bits = (region.lod as u32 * 3) as u32;
    let len = region.values@.len() as usize;
    assert(validate::dense_count(region.lod) == Some(len));
    assert((1usize << bits) as u64 == 1u64 << bits) by(bit_vector)
        requires bits < usize::BITS, bits <= 60u32;
    let width = state::span((region.lod / 2) as nat);
    let count = parts(region.lod);
    assert(region.values@.len() == count * width);
    vstd::arithmetic::div_mod::lemma_div_by_multiple(width as int, count as int);
    assert((part + 1) * width <= count * width) by(nonlinear_arith) requires part < count;
    assert(path(morton::value(region.path), region.lod, part as u64) == fill::part_edit(uniform, part).path);
}

pub(crate) proof fn value(region: DenseRegion, part: nat, initial: spec_fn(int) -> u32, point: int)
    requires domain(region), part < parts(region.lod), ordered::covers(edit(region, part), point),
    ensures ordered::payload_value(edit(region, part), initial, point) == region.values@[point - morton::value(region.path) as int],
{
    geometry(region, part);
    let width = state::span((region.lod / 2) as nat);
    let count = parts(region.lod);
    vstd::arithmetic::div_mod::lemma_div_by_multiple(width as int, count as int);
    assert(region.values@.len() / count == width);
    assert((part + 1) * width <= count * width) by(nonlinear_arith) requires part < count;
    assert(0 <= point - ordered::base(edit(region, part)) < width);
    let offset = point - ordered::base(edit(region, part));
    let values = region.values@.subrange((part * width) as int, ((part + 1) * width) as int);
    assert(values.len() == width);
    assert(edit(region, part).payload == Payload::Dense(values));
    assert(0 <= point - (morton::value(region.path) as int) < region.values@.len());
    assert(values[offset] == region.values@[point - morton::value(region.path) as int]);
}

proof fn prefix_value(region: DenseRegion, part: nat, initial: spec_fn(int) -> u32, point: int, count: nat)
    requires domain(region), part < parts(region.lod), count <= block(region, part).len(),
    ensures ordered::stream_effect(block(region, part), initial, point, count) ==
        if morton::value(region.path) as int + part * state::span((region.lod / 2) as nat) <= point
            < morton::value(region.path) as int + (part + count) * state::span((region.lod / 2) as nat) {
            region.values@[point - morton::value(region.path) as int]
        } else { initial(point) },
    decreases count,
{
    if count > 0 {
        prefix_value(region, part, initial, point, (count - 1) as nat);
        let last = (part + count - 1) as nat;
        geometry(region, last);
        geometry(region, part);
        assert(block(region, part)[(count - 1) as int] == edit(region, last));
        if ordered::covers(edit(region, last), point) { value(region, last, initial, point); }
        let width = state::span((region.lod / 2) as nat);
        assert(part * width <= last * width) by(nonlinear_arith) requires part <= last;
    }
}

pub(crate) proof fn block_contract(region: DenseRegion, part: nat, initial: spec_fn(int) -> u32, point: int)
    requires domain(region), part < parts(region.lod),
    ensures ordered::stream_ordered(block(region, part)), block(region, part).len() > 0,
        ordered::base(block(region, part)[0]) == morton::value(region.path) as int + part * state::span((region.lod / 2) as nat),
        ordered::end(block(region, part).last()) == validate::dense_end(region),
        ordered::stream_effect(block(region, part), initial, point, block(region, part).len()) ==
            if morton::value(region.path) as int + part * state::span((region.lod / 2) as nat) <= point < validate::dense_end(region) {
                region.values@[point - morton::value(region.path) as int]
            } else { initial(point) },
{
    let output = block(region, part);
    geometry(region, part);
    geometry(region, (parts(region.lod) - 1) as nat);
    assert(output.last() == edit(region, (parts(region.lod) - 1) as nat));
    assert forall|i: int| 0 <= i < output.len() implies #[trigger] ordered::edit_shape(output[i]) by {
        geometry(region, (part + i) as nat);
    }
    assert forall|i: int| 0 <= i && i + 1 < output.len() implies ordered::end(#[trigger] output[i]) <= ordered::base(output[i + 1]) by {
        geometry(region, (part + i) as nat);
        geometry(region, (part + i + 1) as nat);
    }
    assert forall|i: int| 0 <= i < output.len() implies #[trigger] output[i].payload is Dense by {}
    prefix_value(region, part, initial, point, output.len());
}

pub(super) proof fn split(regions: Seq<DenseRegion>, part: nat)
    requires regions.len() > 0, validate::dense_prefix(regions, regions.len()),
        domain(regions[0]), part < parts(regions[0].lod),
    ensures stream(regions, part) == block(regions[0], part) + stream(regions.drop_first(), 0),
    decreases parts(regions[0].lod) - part,
{
    group_found(regions, 1);
    let region = regions[0];
    if part + 1 < parts(region.lod) { split(regions, part + 1); }
    let rest = regions.subrange(1, regions.len() as int);
    assert(rest =~= regions.drop_first());
    if part + 1 == parts(region.lod) {
        assert(block(region, part) =~= seq![edit(region, part)]);
    } else {
        assert(block(region, part) =~= seq![edit(region, part)] + block(region, part + 1));
    }
}
}
