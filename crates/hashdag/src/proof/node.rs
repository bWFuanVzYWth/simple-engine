//! 节点的真实字段视图；槽语义不借用 payload 数值判定指针。
use super::*;
use vstd::prelude::*;

verus! {
pub open(crate) spec fn index(value: NodeIdx) -> u32 { value.0 }
pub open(crate) spec fn idx(value: NodeIdx) -> int { value.0 as int }
pub open(crate) spec fn at_index(value: u32) -> NodeIdx { NodeIdx(value) }
pub open(crate) spec fn make_index(value: u32) -> NodeIdx { NodeIdx(value) }
pub open(crate) spec fn encoded(value: EncodedChild) -> u64 { value.0 }
pub open(crate) spec fn values(value: NodeValues) -> Seq<u32> { value.words@ }
pub open(crate) spec fn mask_words(value: NodeMask) -> Seq<u32> { value.words@ }
pub open(crate) spec fn pointer_values(value: Pointers) -> Seq<u32> { value.values@ }
pub open(crate) spec fn pointer_bits(value: Pointers) -> u64 { value.bits }
/// 每个置位槽各产出一次；同一个 NodeIdx 的重复边保留顺序与重数。
pub open(crate) spec fn pointer_sequence(words: Seq<u32>, bits: u64) -> Seq<NodeIdx>
    recommends words.len() == 64,
    decreases bits,
{
    if bits == 0 { Seq::empty() } else {
        proof { assert(bits != 0 ==> bits & ((bits - 1) as u64) < bits) by(bit_vector); }
        seq![NodeIdx(words[bits.trailing_zeros() as int])]
            + pointer_sequence(words, bits & ((bits - 1) as u64))
    }
}
pub open(crate) spec fn remaining(value: Pointers) -> Seq<NodeIdx> {
    pointer_sequence(value.values@, value.bits)
}
pub proof fn pointer_step(words: Seq<u32>, bits: u64)
    requires words.len() == 64,
    ensures pointer_sequence(words, bits).len() == 0 <==> bits == 0,
        bits != 0 ==> pointer_sequence(words, bits)[0] == at_index(words[bits.trailing_zeros() as int]),
        bits != 0 ==> pointer_sequence(words, bits).drop_first() == pointer_sequence(words, bits & ((bits - 1) as u64)),
{
    reveal_with_fuel(pointer_sequence, 2);
}
/// 从 64 个物理槽直接按序过滤 Ptr；与最低置位算法分别定义。
pub open(crate) spec fn pointer_slots(words: Seq<u32>, bits: u64, start: nat) -> Seq<NodeIdx>
    recommends words.len() == 64,
    decreases 64 - start,
{
    if start >= 64 { Seq::empty() } else {
        let rest = pointer_slots(words, bits, start + 1);
        if bits & (1u64 << start) == 0 { rest } else { seq![NodeIdx(words[start as int])] + rest }
    }
}
pub proof fn cleared_least(bits: u64)
    requires bits != 0,
    ensures bits & ((bits - 1) as u64) == bits & !(1u64 << bits.trailing_zeros()),
        forall|j: u32| j < bits.trailing_zeros() ==> #[trigger] (bits & (1u64 << j)) == 0,
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(bits);
    let slot = bits.trailing_zeros();
    assert(slot < 64 && ((bits >> slot) & 1u64) == 1 && bits << ((64 - slot) as u64) == 0 ==>
        bits & ((bits - 1) as u64) == bits & !(1u64 << slot)) by(bit_vector);
    assert forall|j: u32| j < slot implies #[trigger] (bits & (1u64 << j)) == 0 by {
        assert((bits >> (j as u64)) & 1u64 == 0);
        assert(j < 64 && ((bits >> (j as u64)) & 1u64) == 0 ==> (bits & (1u64 << j)) == 0) by(bit_vector);
    }
}
pub proof fn pointer_remove(words: Seq<u32>, bits: u64, slot: u32, start: nat)
    requires words.len() == 64, slot < 64, bits & (1u64 << slot) != 0,
        forall|j: u32| j < slot ==> #[trigger] (bits & (1u64 << j)) == 0,
    ensures pointer_slots(words, bits, start) == if start <= slot {
        seq![at_index(words[slot as int])] + pointer_slots(words, bits & !(1u64 << slot), start)
    } else { pointer_slots(words, bits & !(1u64 << slot), start) },
    decreases 64 - start,
{
    if start < 64 {
        pointer_remove(words, bits, slot, start + 1);
        let other = start as u32;
        assert(slot < 64 && other < 64 ==>
            (((bits & !(1u64 << slot)) & (1u64 << other) == 0)
            <==> (slot == other || bits & (1u64 << other) == 0))) by(bit_vector);
        if start < slot {
            assert(bits & (1u64 << (start as u32)) == 0);
        } else if start == slot {
            assert(bits & (1u64 << start) != 0);
            assert((bits & !(1u64 << slot)) & (1u64 << start) == 0);
        }
    }
}
pub proof fn pointer_sequence_matches(words: Seq<u32>, bits: u64)
    requires words.len() == 64,
    ensures pointer_sequence(words, bits) == pointer_slots(words, bits, 0),
    decreases bits,
{
    if bits != 0 {
        assert(bits != 0 ==> bits & ((bits - 1) as u64) < bits) by(bit_vector);
        nonzero_bit(bits);
        cleared_least(bits);
        pointer_sequence_matches(words, bits & ((bits - 1) as u64));
        pointer_remove(words, bits, bits.trailing_zeros(), 0);
    } else {
        pointer_slots_bound(words, bits, 0);
        pointer_slots_zero(words, 0);
    }
}
pub proof fn pointer_slots_zero(words: Seq<u32>, start: nat)
    requires words.len() == 64,
    ensures pointer_slots(words, 0, start) == Seq::<NodeIdx>::empty(),
    decreases 64 - start,
{
    if start < 64 {
        pointer_slots_zero(words, start + 1);
        let shift = start as u64;
        assert(0u64 & (1u64 << shift) == 0) by(bit_vector);
    }
}
pub proof fn pointer_slots_bound(words: Seq<u32>, bits: u64, start: nat)
    requires words.len() == 64, start <= 64,
    ensures pointer_slots(words, bits, start).len() <= 64 - start,
    decreases 64 - start,
{
    if start < 64 { pointer_slots_bound(words, bits, start + 1); }
}
pub proof fn pointer_sequence_bound(words: Seq<u32>, bits: u64)
    requires words.len() == 64,
    ensures pointer_sequence(words, bits).len() <= 64,
        pointer_sequence(words, bits) == pointer_slots(words, bits, 0),
{
    pointer_sequence_matches(words, bits);
    pointer_slots_bound(words, bits, 0);
}
pub open(crate) spec fn mask(value: NodeMask) -> u64 {
    value.words@[0] as u64 | ((value.words@[1] as u64) << 32)
}
pub open(crate) spec fn payload(value: Child) -> u32 {
    match value { Child::Leaf(v) => v, Child::Ptr(i) => i.0 }
}
pub open(crate) spec fn encode(value: Child) -> EncodedChild {
    match value { Child::Leaf(v) => EncodedChild(v as u64),
        Child::Ptr(i) => EncodedChild(PTR_TAG | i.0 as u64) }
}
pub open(crate) spec fn decode(value: EncodedChild) -> Child {
    if value.0 & PTR_TAG == 0 { Child::Leaf(value.0 as u32) }
    else { Child::Ptr(NodeIdx(value.0 as u32)) }
}
pub open(crate) spec fn child_at(words: Seq<u32>, bits: u64, slot: int) -> Child {
    if 0 <= slot < words.len() {
        if bits & (1u64 << slot) == 0 { Child::Leaf(words[slot]) }
        else { Child::Ptr(NodeIdx(words[slot])) }
    } else { Child::Leaf(0) }
}
pub open(crate) spec fn children(entry: PoolEntry) -> Seq<Child> {
    Seq::new(64, |i: int| child_at(entry.values.words@, mask(entry.mask), i))
}
pub open(crate) spec fn ref_children(node: NodeRef) -> Seq<Child> {
    Seq::new(64, |i: int| child_at(node.values.words@, mask(node.mask), i))
}
pub open(crate) spec fn uniform(slots: Seq<Child>) -> bool {
    slots.len() == 64 && slots[0] is Leaf && forall|i: int| 0 <= i < 64 ==>
        #[trigger] slots[i] == slots[0]
}
pub proof fn split_mask(bits: u64)
    ensures (bits as u32 as u64) | (((bits >> 32) as u32 as u64) << 32) == bits,
{
    assert((bits as u32 as u64) | (((bits >> 32) as u32 as u64) << 32) == bits) by(bit_vector);
}
pub proof fn encoding_round_trip(value: Child)
    ensures decode(encode(value)) == value,
{
    match value {
        Child::Leaf(v) => { assert((v as u64) & (1u64 << 63) == 0) by(bit_vector); }
        Child::Ptr(i) => {
            let n = i.0;
            assert(((1u64 << 63) | n as u64) & (1u64 << 63) != 0) by(bit_vector);
            assert(((1u64 << 63) | n as u64) as u32 == n) by(bit_vector);
        }
    }
}
pub proof fn selected_slot(path: u64, level: u8)
    requires level < 10,
    ensures (path >> (6 * level)) & 63 < 64,
        (((path >> (6 * level)) & 63) as u8) as int == ((path >> (6 * level)) & 63) as int,
{
    assert((path >> (6 * level)) & 63 < 64) by(bit_vector);
}
pub proof fn updated_bit(bits: u64, slot: u8, other: u8, pointer: bool)
    requires slot < 64, other < 64,
    ensures (((bits & !(1u64 << slot)) | ((pointer as u64) << slot)) & (1u64 << other) != 0)
        <==> if slot == other { pointer } else { bits & (1u64 << other) != 0 },
{
    assert(slot < 64 && other < 64 ==> ((((bits & !(1u64 << slot)) | ((pointer as u64) << slot)) & (1u64 << other) != 0)
        <==> if slot == other { pointer } else { bits & (1u64 << other) != 0 })) by(bit_vector);
}
pub proof fn changed_child(before: PoolEntry, after: PoolEntry, slot: u8, child: Child)
    requires slot < 64,
        values(after.values) == values(before.values).update(slot as int, payload(child)),
        mask(after.mask) == (mask(before.mask) & !(1u64 << slot)) | (((child is Ptr) as u64) << slot),
    ensures children(after) == children(before).update(slot as int, child),
{
    assert forall|i: int| 0 <= i < 64 implies #[trigger] children(after)[i] == children(before).update(slot as int, child)[i] by {
        updated_bit(mask(before.mask), slot, i as u8, child is Ptr);
        if i == slot { match child { Child::Leaf(_) => {}, Child::Ptr(_) => {} } }
    }
    assert(children(after) =~= children(before).update(slot as int, child));
}
pub proof fn nonzero_bit(bits: u64)
    requires bits != 0,
    ensures bits.trailing_zeros() < 64,
        bits & (1u64 << bits.trailing_zeros()) != 0,
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(bits);
    let shift = bits.trailing_zeros();
    assert(shift < 64 && ((bits >> (shift as u64)) & 1u64) == 1u64
        ==> bits & (1u64 << shift) != 0) by(bit_vector);
}
pub proof fn uniform_entry(entry: PoolEntry)
    ensures uniform(children(entry)) <==> mask(entry.mask) == 0 && crate::wide::proof::uniform(values(entry.values)) is Some,
{
    let bits = mask(entry.mask);
    let words = values(entry.values);
    if bits != 0 {
        nonzero_bit(bits);
        let slot = bits.trailing_zeros() as int;
        assert(children(entry)[slot] is Ptr);
        assert(!uniform(children(entry)));
    } else {
        assert forall|i: int| 0 <= i < 64 implies #[trigger] children(entry)[i] == Child::Leaf(words[i]) by {
            let shift = i as u64;
            assert(0u64 & (1u64 << shift) == 0) by(bit_vector);
        }
        if uniform(children(entry)) {
            assert forall|i: int| 0 <= i < 64 implies #[trigger] words[i] == words[0] by {
                assert(children(entry)[i] == children(entry)[0]);
            }
        }
        if crate::wide::proof::uniform(words) is Some {
            assert(uniform(children(entry)));
        }
    }
}
pub proof fn fold_result(entry: PoolEntry, result: Option<u32>)
    requires mask(entry.mask) == 0, result == crate::wide::proof::uniform(values(entry.values)),
    ensures result is Some <==> uniform(children(entry)),
        result is Some ==> forall|i: int| 0 <= i < 64 ==>
            #[trigger] children(entry)[i] == Child::Leaf(result->Some_0),
{
    uniform_entry(entry);
    if result is Some {
        assert forall|i: int| 0 <= i < 64 implies #[trigger] children(entry)[i] == Child::Leaf(result->Some_0) by {
            let shift = i as u64;
            assert(0u64 & (1u64 << shift) == 0) by(bit_vector);
        }
    }
}

pub open(crate) spec fn raw_equal(node: NodeRef, entry: PoolEntry) -> bool {
    values(*node.values) == values(entry.values) && mask_words(node.mask) == mask_words(entry.mask)
}
pub proof fn same_children(node: NodeRef, entry: PoolEntry)
    ensures (ref_children(node) == children(entry)) <==> raw_equal(node, entry),
{
    if raw_equal(node, entry) {
        assert(ref_children(node) =~= children(entry));
    }
    if ref_children(node) == children(entry) {
        assert forall|i: int| 0 <= i < 64 implies #[trigger] values(*node.values)[i] == values(entry.values)[i] by {
            assert(ref_children(node)[i] == children(entry)[i]);
        }
        assert(values(*node.values) =~= values(entry.values));
        let first = mask(node.mask);
        let second = mask(entry.mask);
        if first != second {
            let difference = first ^ second;
            assert(first != second ==> first ^ second != 0) by(bit_vector);
            nonzero_bit(difference);
            let bit = difference.trailing_zeros();
            assert(bit < 64 && difference == first ^ second && difference & (1u64 << bit) != 0 ==>
                ((first & (1u64 << bit) == 0) != (second & (1u64 << bit) == 0))) by(bit_vector);
            assert(ref_children(node)[bit as int] == children(entry)[bit as int]);
            assert(false);
        }
        let a = node.mask.words@[0];
        let b = node.mask.words@[1];
        let c = entry.mask.words@[0];
        let d = entry.mask.words@[1];
        assert((a as u64 | ((b as u64) << 32)) == (c as u64 | ((d as u64) << 32)) ==> a == c && b == d) by(bit_vector);
        assert(node.mask.words@ =~= entry.mask.words@);
    }
}
pub proof fn entries_same(a: PoolEntry, b: PoolEntry)
    ensures (children(a) == children(b)) <==> (values(a.values) == values(b.values) && mask_words(a.mask) == mask_words(b.mask)),
{
    let node = NodeRef { values: &a.values, mask: a.mask };
    same_children(node, b);
}
impl vstd::std_specs::cmp::PartialEqSpecImpl<PoolEntry> for NodeRef<'_> {
    open spec fn obeys_eq_spec() -> bool { true }
    open spec fn eq_spec(&self, other: &PoolEntry) -> bool { raw_equal(*self, *other) }
}
impl vstd::std_specs::cmp::PartialEqSpecImpl for EncodedChild {
    open spec fn obeys_eq_spec() -> bool { true }
    open spec fn eq_spec(&self, other: &Self) -> bool { encoded(*self) == encoded(*other) }
}
}
