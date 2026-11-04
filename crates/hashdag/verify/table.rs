//! hashbrown 成员与候选查找契约；判等谓词及正确的重哈希由调用者证明。

use hashbrown::{DefaultHashBuilder, HashTable};
use vstd::prelude::*;

verus! {
#[verifier::external_type_specification]
#[verifier::external_body]
pub struct ExHashBuilder(DefaultHashBuilder);

pub uninterp spec fn entries<T, A: allocator_api2::alloc::Allocator>(table: HashTable<T, A>) -> Map<T, u64>;

pub uninterp spec fn hash(builder: DefaultHashBuilder, words: Seq<u32>, mask: Seq<u32>) -> u64;

pub open spec fn predicate_ready<T, F: Fn(&T) -> bool>(entries: Map<T, u64>, predicate: F) -> bool {
    forall|value: T| #[trigger] entries.contains_key(value) ==> predicate.requires((&value,))
}

pub open spec fn rehash_ready<T, F: Fn(&T) -> u64>(entries: Map<T, u64>, rehash: F) -> bool {
    forall|value: T| #[trigger] entries.contains_key(value) ==>
        rehash.requires((&value,)) && forall|result: u64| rehash.ensures((&value,), result) ==> result == entries[value]
}

/// 查找可测试额外的 hash 碰撞候选，但不会返回谓词不成立的成员；
/// 返回 None 则所有以该 hash 插入的成员都不满足谓词。
pub open spec fn found<T, F: Fn(&T) -> bool>(entries: Map<T, u64>, query: u64, predicate: F, result: Option<T>) -> bool {
    match result {
        Some(value) => entries.contains_key(value) && predicate.ensures((&value,), true),
        None => forall|value: T| #[trigger] entries.contains_key(value) && entries[value] == query ==>
            predicate.ensures((&value,), false),
    }
}
}
