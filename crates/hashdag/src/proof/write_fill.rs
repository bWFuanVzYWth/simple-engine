//! 均匀区域的奇数 LOD 展开；每条源区域的八个部分连续产生。
use super::super::ordered::proof as ordered;
use super::super::ordered::proof::{EditModel, Payload, StreamModel};
use super::super::validate::proof as validate;
use super::*;
use crate::{morton::proof as morton, state};
use vstd::arithmetic::{div_mod::*, power2::*};
use vstd::prelude::*;

verus! {
impl crate::HashDag {
    pub open(crate) spec fn fill_uniform_post(&self,before:&Self,root:crate::RootId,fills:&[UniformFill],result:Result<(),Error>)->bool {
        crate::write::proof::input_written(before,self,root,super::super::validate::proof::fills_prefix(fills@,fills@.len()),
            |initial:spec_fn(int)->u32,path:int|effect(fills@,initial,path,fills@.len()),result)
    }
}
pub(crate) open spec fn effect(fills:Seq<UniformFill>,initial:spec_fn(int)->u32,path:int,count:nat)->u32
    decreases count,
{
    if count==0 {initial(path)} else {
        let fill=fills[(count-1) as int];
        if super::super::validate::proof::base(fill) as int<=path<super::super::validate::proof::fill_end(fill) {fill.data}
        else {effect(fills,initial,path,(count-1) as nat)}
    }
}
impl Fills<'_> {
    pub(crate) open(in super::super) spec fn inputs(&self)->Seq<UniformFill> {self.0@}
}
impl crate::write::proof::BatchModel for Fills<'_> {
    open(crate) spec fn accepted(&self)->bool {super::super::validate::proof::fills_prefix(self.inputs(),self.inputs().len())}
    open(crate) spec fn effect(&self,initial:spec_fn(int)->u32,path:int)->u32 {effect(self.inputs(),initial,path,self.inputs().len())}
}
pub(crate) open spec fn parts(lod:u8) -> nat { if lod % 2 == 0 { 1 } else { 8 } }
pub(crate) open spec fn path(code:u64, lod:u8, part:u64) -> u64 {
    let base = code & !(((1u64 << (lod as u32 * 3)) - 1) as u64);
    (base + (part << ((lod / 2) as u32 * 6))) as u64
}
pub(crate) proof fn path_safe(code:u64, lod:u8, part:u64)
    requires code < 0x1000_0000_0000_0000u64, lod <= 20, part < parts(lod),
    ensures path(code, lod, part) < 0x1000_0000_0000_0000u64,
        ((code & !(((1u64 << (lod as u32 * 3)) - 1) as u64)) as int)
            + ((part << ((lod / 2) as u32 * 6)) as int) < 0x1000_0000_0000_0000u64,
{
    let bits = (lod as u32 * 3) as u32;
    let shift = ((lod / 2) as u32 * 6) as u32;
    super::super::validate::proof::span_safe(lod);
    let offset = part << shift;
    let base = code & !(((1u64 << bits) - 1) as u64);
    assert(offset <= 0x0fff_ffff_ffff_ffffu64) by(bit_vector)
        requires lod <= 20u8, shift == ((lod / 2u8) as u32 * 6u32) as u32, offset == part << shift,
            part < (if lod % 2u8 == 0u8 { 1u64 } else { 8u64 });
    assert(base <= 0x0fff_ffff_ffff_ffffu64 - offset) by(bit_vector)
        requires code < 0x1000_0000_0000_0000u64, lod <= 20u8,
            bits == (lod as u32 * 3u32) as u32, shift == ((lod / 2u8) as u32 * 6u32) as u32,
            offset == part << shift, base == code & !(((1u64 << bits) - 1u64) as u64),
            part < (if lod % 2u8 == 0u8 { 1u64 } else { 8u64 });
}

pub(crate) open spec fn part_edit(fill: UniformFill, part: nat) -> EditModel {
    EditModel { path: path(morton::value(fill.path), fill.lod, part as u64), lod: fill.lod / 2, payload: Payload::Uniform(fill.data) }
}

pub(crate) proof fn part_geometry(fill: UniformFill, part: nat)
    requires fill.lod <= 20, part < parts(fill.lod), morton::value(fill.path) < 0x1000_0000_0000_0000u64,
    ensures ordered::edit_shape(part_edit(fill, part)),
        state::span((fill.lod / 2) as nat) > 0,
        ordered::base(part_edit(fill, part)) == validate::base(fill) as int + part * state::span((fill.lod / 2) as nat),
        ordered::end(part_edit(fill, part)) == validate::base(fill) as int + (part + 1) * state::span((fill.lod / 2) as nat),
        validate::size(fill.lod) == parts(fill.lod) * state::span((fill.lod / 2) as nat),
{
    let lod = fill.lod;
    let code = morton::value(fill.path);
    let height = (lod / 2) as u8;
    let bits = (lod as u32 * 3) as u32;
    let shift = (height as u32 * 6) as u32;
    let width = (1u64 << shift) as u64;
    let full = (1u64 << bits) as u64;
    let count = if lod % 2 == 0 { 1u64 } else { 8u64 };
    let p = part as u64;
    crate::canonical::span_positive(height as nat);
    assert(full == width * count) by(bit_vector)
        requires lod <= 20u8, bits == (lod as u32 * 3u32) as u32,
            shift == ((lod / 2u8) as u32 * 6u32) as u32,
            width == 1u64 << shift, full == 1u64 << bits,
            count == if lod % 2u8 == 0u8 { 1u64 } else { 8u64 };
    assert(width <= 0x1000_0000_0000_0000u64) by(bit_vector) requires shift <= 60u32, width == 1u64 << shift;
    assert(part * width <= u64::MAX) by(nonlinear_arith) requires part < 8, width <= 0x1000_0000_0000_0000u64;
    super::super::frontier::proof::span_bits(height);
    vstd::bits::lemma_u64_pow2_no_overflow(shift as nat);
    vstd::bits::lemma_u64_shl_is_mul(1, shift as u64);
    vstd::bits::lemma_u64_shl_is_mul(part as u64, shift as u64);
    path_safe(code, lod, part as u64);
    let actual = path(code, lod, part as u64);
    assert(actual & ((width - 1) as u64) == 0) by(bit_vector)
        requires code < 0x1000_0000_0000_0000u64, lod <= 20u8,
            bits == (lod as u32 * 3u32) as u32, shift == ((lod / 2u8) as u32 * 6u32) as u32,
            p < if lod % 2u8 == 0u8 { 1u64 } else { 8u64 },
            width == 1u64 << shift,
            actual == ((code & !(((1u64 << bits) - 1) as u64)) + (p << shift)) as u64;
    super::super::frontier::proof::path_digits(actual, height);
    lemma_fundamental_div_mod(actual as int, width as int);
    assert((full as int) == count as int * width as int) by(nonlinear_arith)
        requires full == width * count, width * count <= u64::MAX;
    assert((part + 1) * width == part * width + width) by(nonlinear_arith);
}

pub(crate) open spec fn stream(fills: Seq<UniformFill>, part:nat) -> Seq<EditModel>
    decreases (fills.len() * 8 - part) as nat,
{
    if fills.len() == 0 { Seq::empty() } else {
        let fill = fills[0];
        let count = parts(fill.lod);
        if fill.lod <= 20 && part < count {
            seq![EditModel { path: path(crate::morton::proof::value(fill.path), fill.lod, part as u64),
                lod: fill.lod / 2, payload: Payload::Uniform(fill.data) }]
                + if part + 1 == count { stream(fills.drop_first(), 0) } else { stream(fills, part + 1) }
        } else { Seq::empty() }
    }
}

pub(crate) proof fn suffix(fills: Seq<UniformFill>)
    requires fills.len() > 0, validate::fills_prefix(fills, fills.len()),
    ensures validate::fills_prefix(fills.drop_first(), fills.drop_first().len()),
{
    let tail = fills.drop_first();
    assert forall|i: nat| i < tail.len() implies #[trigger] validate::fill_at(tail, i) by {
        assert(validate::fill_at(fills, i + 1));
    }
}

proof fn ordered_cons(head: EditModel, tail: Seq<EditModel>)
    requires ordered::edit_shape(head), ordered::stream_ordered(tail), head.payload is Uniform,
        forall|i: int| 0 <= i < tail.len() ==> #[trigger] tail[i].payload is Uniform,
        tail.len() > 0 ==> ordered::end(head) <= ordered::base(tail[0]),
    ensures ordered::stream_ordered(seq![head] + tail),
        forall|i: int| 0 <= i < (seq![head] + tail).len() ==> #[trigger] (seq![head] + tail)[i].payload is Uniform,
{
    let joined = seq![head] + tail;
    assert forall|i: int| 0 <= i < joined.len() implies #[trigger] ordered::edit_shape(joined[i]) by {
        if i > 0 { assert(joined[i] == tail[i - 1]); }
    }
    assert forall|i: int| 0 <= i && i + 1 < joined.len() implies ordered::end(#[trigger] joined[i]) <= ordered::base(joined[i + 1]) by {
        if i > 0 { assert(joined[i] == tail[i - 1]); assert(joined[i + 1] == tail[i]); }
    }
    assert forall|i: int| 0 <= i < joined.len() implies #[trigger] joined[i].payload is Uniform by {
        if i > 0 { assert(joined[i] == tail[i - 1]); }
    }
}

pub(crate) proof fn stream_valid(fills: Seq<UniformFill>, part: nat)
    requires validate::fills_prefix(fills, fills.len()),
        forall|i: int| 0 <= i < fills.len() ==> morton::value(#[trigger] fills[i].path) < 0x1000_0000_0000_0000u64,
        if fills.len() == 0 { part == 0 } else { part < parts(fills[0].lod) },
    ensures ordered::stream_ordered(stream(fills, part)),
        forall|i: int| 0 <= i < stream(fills, part).len() ==> #[trigger] stream(fills, part)[i].payload is Uniform,
        fills.len() > 0 ==> stream(fills, part).len() > 0 && stream(fills, part)[0] == part_edit(fills[0], part),
    decreases fills.len() * 8 - part,
{
    if fills.len() > 0 {
        let fill = fills[0];
        assert(validate::fill_at(fills, 0));
        part_geometry(fill, part);
        let tail: Seq<EditModel>;
        if part + 1 == parts(fill.lod) {
            suffix(fills);
            stream_valid(fills.drop_first(), 0);
            tail = stream(fills.drop_first(), 0);
            if fills.len() > 1 {
                part_geometry(fills[1], 0);
                assert(validate::fill_at(fills, 1));
            }
        } else {
            stream_valid(fills, part + 1);
            tail = stream(fills, part + 1);
            part_geometry(fill, part + 1);
        }
        assert(stream(fills, part) == seq![part_edit(fill, part)] + tail);
        ordered_cons(part_edit(fill, part), tail);
    }
}

proof fn later_fill(fills: Seq<UniformFill>, i: int)
    requires validate::fills_prefix(fills, fills.len()), 0 < i < fills.len(),
    ensures validate::base(fills[i]) as int >= validate::fill_end(fills[0]),
    decreases i,
{
    assert(validate::fill_at(fills, i as nat));
    if i > 1 {
        later_fill(fills, i - 1);
        assert(validate::fill_at(fills, (i - 1) as nat));
        validate::span_safe(fills[i - 1].lod);
    }
}

proof fn effect_tail(fills: Seq<UniformFill>, initial: spec_fn(int) -> u32, point: int, count: nat)
    requires 0 < count <= fills.len(), !(validate::base(fills[0]) as int <= point < validate::fill_end(fills[0])),
    ensures effect(fills, initial, point, count) == effect(fills.drop_first(), initial, point, (count - 1) as nat),
    decreases count,
{
    reveal_with_fuel(effect, 2);
    if count > 1 { assert(fills[(count - 1) as int] == fills.drop_first()[(count - 2) as int]); }
    if count > 1 { effect_tail(fills, initial, point, (count - 1) as nat); }
}

proof fn effect_head_hit(fills: Seq<UniformFill>, initial: spec_fn(int) -> u32, point: int, count: nat)
    requires validate::fills_prefix(fills, fills.len()), 0 < count <= fills.len(),
        validate::base(fills[0]) as int <= point < validate::fill_end(fills[0]),
    ensures effect(fills, initial, point, count) == fills[0].data,
    decreases count,
{
    if count > 1 {
        later_fill(fills, count - 1);
        effect_head_hit(fills, initial, point, (count - 1) as nat);
    }
}

proof fn effect_head(fills: Seq<UniformFill>, initial: spec_fn(int) -> u32, point: int)
    requires validate::fills_prefix(fills, fills.len()), fills.len() > 0,
    ensures effect(fills, initial, point, fills.len()) ==
        if validate::base(fills[0]) as int <= point < validate::fill_end(fills[0]) { fills[0].data }
        else { effect(fills.drop_first(), initial, point, fills.drop_first().len()) },
{
    if validate::base(fills[0]) as int <= point < validate::fill_end(fills[0]) {
        effect_head_hit(fills, initial, point, fills.len());
    } else { effect_tail(fills, initial, point, fills.len()); }
}

proof fn later_edit(edits: Seq<EditModel>, i: int)
    requires ordered::stream_ordered(edits), 0 <= i < edits.len(),
    ensures ordered::base(edits[i]) >= ordered::base(edits[0]),
    decreases i,
{
    if i > 0 {
        later_edit(edits, i - 1);
        crate::canonical::span_positive(edits[i - 1].lod as nat);
    }
}

proof fn before_edits(edits: Seq<EditModel>, initial: spec_fn(int) -> u32, point: int)
    requires ordered::stream_ordered(edits), edits.len() == 0 || point < ordered::base(edits[0]),
    ensures !super::super::set_cells::proof::stream_hits(edits, point),
        ordered::stream_effect(edits, initial, point, edits.len()) == initial(point),
{
    assert forall|i: int| 0 <= i < edits.len() implies !#[trigger] ordered::covers(edits[i], point) by {
        later_edit(edits, i);
    }
    super::super::set_cells::proof::stream_no_cover(edits, initial, point);
}

pub(crate) proof fn stream_value(fills: Seq<UniformFill>, part: nat, initial: spec_fn(int) -> u32, point: int)
    requires validate::fills_prefix(fills, fills.len()),
        forall|i: int| 0 <= i < fills.len() ==> morton::value(#[trigger] fills[i].path) < 0x1000_0000_0000_0000u64,
        if fills.len() == 0 { part == 0 } else { part < parts(fills[0].lod) },
    ensures ordered::stream_effect(stream(fills, part), initial, point, stream(fills, part).len()) ==
        if fills.len() > 0 && validate::base(fills[0]) as int + part * state::span((fills[0].lod / 2) as nat) <= point
            < validate::fill_end(fills[0]) { fills[0].data }
        else if fills.len() > 0 { effect(fills.drop_first(), initial, point, fills.drop_first().len()) }
        else { initial(point) },
    decreases fills.len() * 8 - part,
{
    if fills.len() > 0 {
        let fill = fills[0];
        assert(validate::fill_at(fills, 0));
        part_geometry(fill, part);
        let width = state::span((fill.lod / 2) as nat);
        let count = parts(fill.lod);
        assert((part + 1) * width <= count * width) by(nonlinear_arith) requires part < count;
        let tail: Seq<EditModel>;
        if part + 1 == parts(fill.lod) {
            suffix(fills);
            stream_value(fills.drop_first(), 0, initial, point);
            stream_valid(fills.drop_first(), 0);
            tail = stream(fills.drop_first(), 0);
            if fills.len() > 1 { part_geometry(fills[1], 0); assert(validate::fill_at(fills, 1)); }
            if ordered::covers(part_edit(fill, part), point) { before_edits(tail, initial, point); }
            if fills.len() > 1 { effect_head(fills.drop_first(), initial, point); }
        } else {
            stream_value(fills, part + 1, initial, point);
            stream_valid(fills, part + 1);
            tail = stream(fills, part + 1);
            part_geometry(fill, part + 1);
            if ordered::covers(part_edit(fill, part), point) { before_edits(tail, initial, point); }
        }
        assert(stream(fills, part) == seq![part_edit(fill, part)] + tail);
        super::super::set_cells::proof::stream_prepend(part_edit(fill, part), tail, initial, point);
        if !super::super::set_cells::proof::stream_hits(tail, point) {
            assert forall|i: int| 0 <= i < tail.len() implies !#[trigger] ordered::covers(tail[i], point) by {}
            super::super::set_cells::proof::stream_no_cover(tail, initial, point);
        }
        if ordered::covers(part_edit(fill, part), point) {
            assert(!super::super::set_cells::proof::stream_hits(tail, point));
            assert(ordered::stream_effect(stream(fills, part), initial, point, stream(fills, part).len()) == fill.data);
            assert(validate::base(fill) as int + part * state::span((fill.lod / 2) as nat) <= point < validate::fill_end(fill));
        } else {
            assert(ordered::stream_effect(stream(fills, part), initial, point, stream(fills, part).len())
                == ordered::stream_effect(tail, initial, point, tail.len()));
            if part + 1 == parts(fill.lod) {
                assert(!(validate::base(fill) as int + part * state::span((fill.lod / 2) as nat) <= point < validate::fill_end(fill)));
            }
        }
    }
}

pub(crate) proof fn input_stream(fills: Seq<UniformFill>, initial: spec_fn(int) -> u32, point: int)
    requires validate::fills_prefix(fills, fills.len()),
    ensures ordered::stream_ordered(stream(fills, 0)),
        ordered::stream_effect(stream(fills, 0), initial, point, stream(fills, 0).len()) == effect(fills, initial, point, fills.len()),
{
    assert forall|i: int| 0 <= i < fills.len() implies morton::value(#[trigger] fills[i].path) < 0x1000_0000_0000_0000u64 by {
        assert(validate::fill_at(fills, i as nat));
    }
    stream_valid(fills, 0);
    stream_value(fills, 0, initial, point);
    if fills.len() > 0 { effect_head(fills, initial, point); }
}
impl FillEdits<'_> {
    pub(super) open spec fn prepared(&self,fill:UniformFill)->bool {
        self.base==validate::base(fill) && self.lod==fill.lod/2
            && self.shift==self.lod*6 && self.parts==parts(fill.lod)
    }
    pub(super) open spec fn valid(&self) -> bool {
        (forall|i:int| 0 <= i < self.pending@.len() ==> #[trigger] self.pending@[i].lod <= 20)
            && if self.pending@.len() == 0 { self.part == 0 } else {
                self.part < parts(self.pending@[0].lod) && (self.part>0 ==> self.prepared(self.pending@[0]))
            }
    }
    pub(super) open spec fn stream(&self) -> Seq<EditModel> { stream(self.pending@, self.part as nat) }
}
impl StreamModel for FillEdits<'_> {
    open(super) spec fn stream_valid(&self)->bool {self.valid()}
    open(super) spec fn remaining(&self)->Seq<EditModel> {self.stream()}
}
}
