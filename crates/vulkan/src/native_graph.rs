//! 借用真实发布根和记录根，按槽与操作顺序扫描；不分配或复制根列表。
//!
//! Seeds 的完整序列、重置、空槽和无根操作跳过均验证；资源邻接读取同一 Life 的实际存储。

use super::{Life, Op, Record, resource::Payloads};
use crate::{
    graph::{SeedModel, Seeds},
    handles::Resource,
    owner::{Key, Values},
};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(not(verus_only))]
use vstd::{
    prelude::{proof, proof_decl},
    view::View,
};

#[path = "proof/native_graph.rs"]
pub(crate) mod proof;

#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct ResourceRoots<'a> {
    roots: &'a [Resource],
    remaining: &'a [Resource],
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> ResourceRoots<'a> {
    #[cfg_attr(verus_only, verus_spec(source => ensures
        source@.keys == proof::resources(roots@), source@.position == 0,
    ))]
    pub(crate) const fn new(roots: &'a [Resource]) -> Self {
        proof! { proof::resources_start(roots); }
        Self {
            roots,
            remaining: roots,
        }
    }

    #[cfg_attr(verus_only, verus_spec(result => requires old(self).valid(),
        ensures final(self).valid(), proof::advanced(old(self)@, final(self)@, result)))]
    fn advance(&mut self) -> Option<Key> {
        proof_decl! { let ghost before = *self; }
        let (resource, remaining) = self.remaining.split_first()?;
        self.remaining = remaining;
        proof! { proof::resources_yielded(before, *self); }
        Some(resource.0)
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl Seeds for ResourceRoots<'_> {
    fn reset(&mut self) {
        proof! { proof::resources_start(self.roots); }
        self.remaining = self.roots;
    }
    fn next(&mut self) -> Option<Key> {
        self.advance()
    }
    fn hint(&self) -> usize {
        self.roots.len()
    }
}

#[cfg(not(verus_only))]
impl Iterator for ResourceRoots<'_> {
    type Item = Key;
    fn next(&mut self) -> Option<Key> {
        self.advance()
    }
}

/// 一个记录的剩余构建根和操作；保持原存储与扫描顺序。
#[cfg_attr(verus_only, verus_verify)]
struct Items<'a> {
    build_roots: &'a [Key],
    operations: &'a [Op],
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> Items<'a> {
    #[cfg_attr(verus_only, verus_spec(items => ensures items@.len() == 0))]
    const fn empty() -> Self {
        Self {
            build_roots: &[],
            operations: &[],
        }
    }

    #[cfg_attr(verus_only, verus_spec(items => ensures items@ == proof::record(record)))]
    fn new<B>(record: &'a Record<B>) -> Self {
        Self {
            build_roots: record.build_roots(),
            operations: record.operations(),
        }
    }

    #[cfg_attr(verus_only, verus_spec(result => ensures
        result is Some <==> old(self)@.len() > 0,
        result is Some ==> result->Some_0 == old(self)@[0] && final(self)@ == old(self)@.drop_first(),
        result is None ==> final(self)@.len() == 0,
    ))]
    fn advance(&mut self) -> Option<Key> {
        if let Some((key, remaining)) = self.build_roots.split_first() {
            self.build_roots = remaining;
            return Some(*key);
        }
        proof_decl! { let ghost original = self@; }
        #[cfg_attr(verus_only, verus_spec(invariant self@ == original, original == old(self)@,
            self.build_roots@.len() == 0,
            decreases self.operations@.len(),
        ))]
        while !self.operations.is_empty() {
            let (op, remaining) = self.operations.split_first().unwrap();
            self.operations = remaining;
            if let Some(key) = op.root() {
                return Some(key);
            }
        }
        None
    }
}

/// 两个槽位游标及当前记录的借用；不保存根快照。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct RecordRoots<'a, B> {
    initial: Values<'a, Record<B>>,
    remaining: Values<'a, Record<B>>,
    items: Items<'a>,
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a, B> RecordRoots<'a, B> {
    #[cfg_attr(verus_only, verus_spec(source => ensures
        source@.keys == proof::records(records@), source@.position == 0,
    ))]
    pub(crate) fn new(records: Values<'a, Record<B>>) -> Self {
        proof! { proof::records_start(records@); }
        Self {
            remaining: records.clone(),
            initial: records,
            items: Items::empty(),
        }
    }

    #[inline]
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).valid(),
        ensures final(self).valid(), proof::advanced(old(self)@, final(self)@, result)))]
    fn advance(&mut self) -> Option<Key> {
        proof_decl! { let ghost before = *self; }
        #[cfg_attr(verus_only, verus_spec(invariant proof::scanning(before, *self), before == *old(self),
            decreases self.remaining.slots_left(),
        ))]
        loop {
            proof_decl! { let ghost previous = *self; }
            if let Some(key) = self.items.advance() {
                proof! { proof::yielded(before, previous, *self, key); }
                return Some(key);
            }
            let record = match self.remaining.advance() {
                Some(record) => record,
                None => {
                    proof! { proof::exhausted(before, previous, *self); }
                    return None;
                }
            };
            self.items = Items::new(record);
            proof! { proof::loaded(before, previous, *self, record); }
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<B> Seeds for RecordRoots<'_, B> {
    fn reset(&mut self) {
        proof! { proof::records_start(self.initial@); }
        self.remaining = self.initial.clone();
        self.items = Items::empty();
    }
    #[inline]
    fn next(&mut self) -> Option<Key> {
        self.advance()
    }
    fn hint(&self) -> usize {
        0
    }
}

#[cfg(not(verus_only))]
impl<B> Iterator for RecordRoots<'_, B> {
    type Item = Key;
    fn next(&mut self) -> Option<Key> {
        self.advance()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<P: Payloads, B> Life<P, B> {
    #[cfg_attr(verus_only, verus_spec(source => ensures source@.keys == self.record_root_keys(),
        source@.position == 0,
    ))]
    pub(crate) fn record_roots(&self) -> RecordRoots<'_, B> {
        RecordRoots::new(self.records.values_cursor())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner::Arena;

    #[test]
    fn record_roots_reset_covers_live_records() {
        let owner = 7;
        let keys = [0, 1, 2].map(|slot| Key {
            owner,
            slot,
            generation: 0,
        });
        let record = |build_roots, operations| Record::new(operations, Vec::new(), build_roots, ());
        let mut records = Arena::default();
        let hole = records
            .try_insert(owner, record(Vec::new(), Vec::new()))
            .unwrap_or_else(|_| panic!());
        for value in [
            record(
                vec![keys[0]],
                vec![Op::Barrier, Op::Keepalive(Resource(keys[1]))],
            ),
            record(Vec::new(), vec![Op::Barrier]),
            record(Vec::new(), vec![Op::Keepalive(Resource(keys[2]))]),
        ] {
            records
                .try_insert(owner, value)
                .unwrap_or_else(|_| panic!());
        }
        records.remove(hole);
        let mut source = RecordRoots::new(records.values_cursor());
        assert_eq!(Iterator::next(&mut source), Some(keys[0]));
        source.reset();
        assert_eq!(source.collect::<Vec<_>>(), keys);
    }
}
