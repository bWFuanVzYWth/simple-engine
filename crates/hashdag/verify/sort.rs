//! 标准排序的键与排列契约，不定义节点可达性。
use vstd::prelude::*;
verus! {
pub open spec fn ready<T, F: Fn(&T) -> (u8, u32)>(items: Seq<T>, key: F) -> bool {
    forall|value: T| #[trigger] items.contains(value) ==>
        key.requires((&value,))
        && forall|a: (u8, u32), b: (u8, u32)| key.ensures((&value,), a) && key.ensures((&value,), b) ==> a == b
}
pub open spec fn keys_le<T, F: Fn(&T) -> (u8, u32)>(left: T, right: T, key: F) -> bool {
    exists|a: (u8, u32), b: (u8, u32)| key.ensures((&left,), a) && key.ensures((&right,), b)
        && (a.0 < b.0 || a.0 == b.0 && a.1 <= b.1)
}
pub open spec fn ordered<T, F: Fn(&T) -> (u8, u32)>(items: Seq<T>, key: F) -> bool {
    forall|i: int, j: int| 0 <= i < j < items.len() ==> #[trigger] keys_le(items[i], items[j], key)
}
}
