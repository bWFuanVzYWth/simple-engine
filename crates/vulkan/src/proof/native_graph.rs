//! 真实根序列、借用后缀与 Seeds 的对应；普通编译只保留擦除关联类型。

use super::*;

#[cfg(not(verus_only))]
impl View for ResourceRoots<'_> {
    type V = SeedModel;
}
#[cfg(not(verus_only))]
impl<B> View for RecordRoots<'_, B> {
    type V = SeedModel;
}
#[cfg(not(verus_only))]
impl View for Items<'_> {
    type V = ();
}

#[cfg(verus_only)]
verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) open spec fn resources(input: Seq<Resource>) -> Seq<Key> {
    input.map(|i: int, resource: Resource| resource.0)
}

pub(crate) open spec fn operations(input: Seq<Op>) -> Seq<Key>
    decreases input.len(),
{
    if input.len() == 0 { Seq::empty() }
    else {
        let rest = operations(input.drop_first());
        match input[0].root_view() { Some(key) => seq![key] + rest, None => rest }
    }
}

pub(crate) open spec fn record<B>(value: &Record<B>) -> Seq<Key> {
    value.roots_value()@ + operations(value.operations_value()@)
}

pub(crate) open spec fn records<B>(input: Seq<Record<B>>) -> Seq<Key>
    decreases input.len(),
{
    if input.len() == 0 { Seq::empty() }
    else { record(&input[0]) + records(input.drop_first()) }
}

/// 包含每个现存记录的构建根和 Op 根；不依赖根的去重或排序。
pub(crate) proof fn record_member<B>(input: Seq<Record<B>>, value: Record<B>, key: Key)
    requires input.contains(value), record(&value).contains(key),
    ensures records(input).contains(key),
    decreases input.len(),
{
    let index = choose|index: int| 0 <= index < input.len() && input[index] == value;
    if index != 0 {
        assert(input.drop_first().contains(value));
        record_member(input.drop_first(), value, key);
    }
}

pub(super) open spec fn advanced(before: SeedModel, after: SeedModel, result: Option<Key>) -> bool {
    after.keys == before.keys && before.position <= before.keys.len()
        && match result {
            Some(key) => before.position < before.keys.len() && key == before.keys[before.position as int]
                && after.position == before.position + 1,
            None => before.position == before.keys.len() && after.position == before.position,
        }
}

impl View for ResourceRoots<'_> {
    type V = SeedModel;
    open(super) spec fn view(&self) -> SeedModel {
        SeedModel { keys: resources(self.roots@), position: if self.valid() {
            (self.roots@.len() - self.remaining@.len()) as nat
        } else { resources(self.roots@).len() + 1 } }
    }
}
impl<'a> ResourceRoots<'a> {
    pub(super) open spec fn valid(&self) -> bool {
        self.remaining@.len() <= self.roots@.len()
            && self.remaining@ == self.roots@.skip(self.roots@.len() - self.remaining@.len())
    }
}

impl View for Items<'_> {
    type V = Seq<Key>;
    open(super) spec fn view(&self) -> Seq<Key> { self.build_roots@ + operations(self.operations@) }
}

impl<B> View for RecordRoots<'_, B> {
    type V = SeedModel;
    open(super) spec fn view(&self) -> SeedModel {
        SeedModel { keys: records(self.initial@), position: if self.valid() {
            (records(self.initial@).len() - self.rest().len()) as nat
        } else { records(self.initial@).len() + 1 } }
    }
}
impl<'a, B> RecordRoots<'a, B> {
    pub(super) open spec fn rest(&self) -> Seq<Key> { self.items@ + records(self.remaining@) }
    pub(super) open spec fn valid(&self) -> bool {
        self.rest().len() <= records(self.initial@).len()
            && self.rest() == records(self.initial@).skip(records(self.initial@).len() - self.rest().len())
    }
}

pub(super) open spec fn scanning<B>(before: RecordRoots<B>, after: RecordRoots<B>) -> bool {
    before.valid() && after.valid() && before.initial == after.initial && after.rest() == before.rest()
}

pub(super) proof fn resources_start(input: &[Resource])
    ensures input@.skip(0) == input@,
{}

pub(super) proof fn resources_yielded(before: ResourceRoots, after: ResourceRoots)
    requires before.valid(), before.remaining@.len() > 0, after.roots == before.roots,
        after.remaining@ == before.remaining@.drop_first(),
    ensures after.valid(), advanced(before@, after@, Some(before.remaining@[0].0)),
{
    let offset = before.roots@.len() - before.remaining@.len();
    assert(before.remaining@ == before.roots@.skip(offset));
    assert(after.remaining@ =~= before.roots@.skip(offset + 1));
    assert(before.roots@[offset] == before.remaining@[0]);
}

pub(super) proof fn records_start<B>(input: Seq<Record<B>>)
    ensures records(input).skip(0) == records(input), operations(Seq::empty()).len() == 0,
{}

pub(super) proof fn loaded<B>(before: RecordRoots<B>, previous: RecordRoots<B>, after: RecordRoots<B>, value: &Record<B>)
    requires scanning(before, previous), after.initial == previous.initial, previous.items@.len() == 0,
        previous.remaining@.len() > 0, *value == previous.remaining@[0],
        after.remaining@ == previous.remaining@.drop_first(), after.items@ == record(value),
    ensures scanning(before, after),
{}

pub(super) proof fn yielded<B>(before: RecordRoots<B>, previous: RecordRoots<B>, after: RecordRoots<B>, key: Key)
    requires scanning(before, previous), after.initial == previous.initial, after.remaining == previous.remaining,
        previous.items@.len() > 0, previous.items@[0] == key, after.items@ == previous.items@.drop_first(),
    ensures after.valid(), advanced(before@, after@, Some(key)),
{
    assert(after.rest() =~= before.rest().drop_first());
    assert(after.rest() =~= records(after.initial@).skip(records(after.initial@).len() - after.rest().len()));
}

pub(super) proof fn exhausted<B>(before: RecordRoots<B>, previous: RecordRoots<B>, after: RecordRoots<B>)
    requires scanning(before, previous), after.initial == previous.initial, previous.items@.len() == 0,
        previous.remaining@.len() == 0, after.remaining@.len() == 0, after.items@.len() == 0,
    ensures after.valid(), advanced(before@, after@, None),
{}
}
