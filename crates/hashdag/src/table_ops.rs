//! hashbrown 单次容器操作的适配；不解释节点、引用或事务。

use hashbrown::{DefaultHashBuilder, HashTable, TryReserveError};
use std::hash::{BuildHasher, Hash, Hasher};
#[cfg(verus_only)]
use vstd::prelude::*;

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
pub fn builder() -> DefaultHashBuilder {
    DefaultHashBuilder::default()
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
pub fn retained<T>(table: &HashTable<T>) -> usize {
    table.allocation_size()
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result => ensures crate::table_model::entries(result) == Map::<T, u64>::empty()))]
pub fn new<T>() -> HashTable<T> {
    HashTable::new()
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires crate::table_model::predicate_ready(crate::table_model::entries(*table), equal),
    ensures crate::table_model::found(crate::table_model::entries(*table), hash, equal, result),
))]
pub fn find<T: Copy, F: Fn(&T) -> bool>(table: &HashTable<T>, hash: u64, equal: F) -> Option<T> {
    table.find(hash, equal).copied()
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires crate::table_model::rehash_ready(crate::table_model::entries(*old(table)), rehash),
    ensures crate::table_model::entries(*final(table)) == crate::table_model::entries(*old(table)),
))]
pub fn reserve<T, F: Fn(&T) -> u64>(
    table: &mut HashTable<T>,
    additional: usize,
    rehash: F,
) -> Result<(), TryReserveError> {
    table.try_reserve(additional, rehash)
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(
    requires !crate::table_model::entries(*old(table)).contains_key(value), crate::table_model::rehash_ready(crate::table_model::entries(*old(table)), rehash),
    ensures crate::table_model::entries(*final(table)) == crate::table_model::entries(*old(table)).insert(value, hash),
))]
pub fn insert<T, F: Fn(&T) -> u64>(table: &mut HashTable<T>, hash: u64, value: T, rehash: F) {
    table.insert_unique(hash, value, rehash);
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires crate::table_model::predicate_ready(crate::table_model::entries(*old(table)), equal),
    ensures crate::table_model::found(crate::table_model::entries(*old(table)), hash, equal, result),
        crate::table_model::entries(*final(table)) == if result is Some { crate::table_model::entries(*old(table)).remove(result->Some_0) } else { crate::table_model::entries(*old(table)) },
))]
pub fn remove<T: Copy, F: Fn(&T) -> bool>(
    table: &mut HashTable<T>,
    hash: u64,
    equal: F,
) -> Option<T> {
    table
        .find_entry(hash, equal)
        .ok()
        .map(|entry| entry.remove().0)
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(ensures crate::table_model::entries(*final(table)) == Map::<T, u64>::empty()))]
pub fn clear<T>(table: &mut HashTable<T>) {
    table.clear();
}

/// 依次哈希完整 payload 与 mask，与两字段元组的输入顺序一致。
#[inline(always)]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verifier::external_body)]
#[cfg_attr(verus_only, verus_spec(result => ensures result == crate::table_model::hash(*builder, words@, mask@)))]
pub fn hash(builder: &DefaultHashBuilder, words: &[u32; 64], mask: &[u32; 2]) -> u64 {
    let mut state = builder.build_hasher();
    words.hash(&mut state);
    mask.hash(&mut state);
    state.finish()
}
