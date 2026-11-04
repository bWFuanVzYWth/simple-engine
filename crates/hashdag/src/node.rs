//! CPU 与 GPU 共用的 64 叉节点数据。payload 与类型 mask 分别连续存储；维护侧表不属于此 ABI。

use crate::ZOrder20x3;

use crate::hashdag::Error;
#[cfg(not(verus_only))]
use crate::simd as simd_ops;
#[cfg(verus_only)]
use hash_dag_std::simd_ops;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
#[path = "proof/node.rs"]
pub(crate) mod proof;

#[cfg_attr(verus_only, verus_verify)]
pub(crate) const PTR_TAG: u64 = 1 << 63;

/// 层内节点编号；字节偏移须在扩宽后计算。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(verus_only, verus_verify)]
pub struct NodeIdx(pub(crate) u32);

#[cfg_attr(verus_only, verus_verify)]
impl NodeIdx {
    /// 层内零号位置，不表示它当前存活。
    #[cfg_attr(verus_only, verus_spec(ensures proof::index(Self::ZERO) == 0))]
    pub const ZERO: Self = Self(0);
    /// 层内可表示的最大位置。
    #[cfg_attr(verus_only, verus_spec(ensures proof::index(Self::MAX) == u32::MAX))]
    pub const MAX: Self = Self(u32::MAX);

    /// 从位置数值构造，超出 u32 范围返回 `None`；不验证槽的所属实例或存活状态。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == if value <= u32::MAX as u64 { Some(proof::at_index(value as u32)) } else { None },
        no_unwind
    ))]
    pub const fn new(value: u64) -> Option<Self> {
        if value <= u32::MAX as u64 {
            Some(Self(value as u32))
        } else {
            None
        }
    }

    /// 返回层内位置，字节偏移计算应先扩宽。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures result == proof::index(self) as u64, no_unwind))]
    pub const fn get(self) -> u64 {
        self.0 as u64
    }

    #[inline]
    #[cfg_attr(verus_only, verus_spec(result => ensures result == Ok(proof::index(self) as usize), no_unwind))]
    pub(crate) fn as_usize(self) -> Result<usize, Error> {
        Ok(self.0 as usize)
    }
}

/// 完整 32 位叶数据或下一层的节点编号。节点层级由遍历位置确定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(verus_only, verus_verify)]
pub enum Child {
    /// 下一层节点的位置；依赖原 DAG、原生层级和有效 root 的生命周期。
    Ptr(NodeIdx),
    /// 整个子区域的同一值，不赋予空气或材料含义。
    Leaf(u32),
}

/// 构建 scratch 的比较键；不进入常驻节点数组或上传 ABI。
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
pub struct EncodedChild(pub(crate) u64);

#[cfg_attr(verus_only, verus_verify)]
impl EncodedChild {
    #[cfg_attr(verus_only, verus_spec(result => ensures proof::encoded(result) == value as u64, no_unwind))]
    pub const fn leaf(value: u32) -> Self {
        Self(value as u64)
    }
    #[cfg_attr(verus_only, verus_spec(result => ensures proof::encoded(result) == (1u64 << 63) | proof::index(index) as u64, no_unwind))]
    pub const fn ptr(index: NodeIdx) -> Self {
        Self(PTR_TAG | index.get())
    }
    #[cfg_attr(verus_only, verus_spec(result => ensures result == proof::decode(self), no_unwind))]
    pub const fn decode(self) -> Child {
        let payload = self.0 as u32;
        if self.0 & PTR_TAG == 0 {
            Child::Leaf(payload)
        } else {
            Child::Ptr(NodeIdx(payload))
        }
    }
    #[cfg_attr(verus_only, verus_spec(result => ensures result == match proof::decode(self) {
        Child::Leaf(value) => Some(value), Child::Ptr(_) => None,
    }, no_unwind))]
    pub const fn leaf_value(self) -> Option<u32> {
        match self.decode() {
            Child::Leaf(value) => Some(value),
            Child::Ptr(_) => None,
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl Child {
    /// 比较类型与完整 payload；常量入口也供无 unwind 的内部路径复用。
    #[cfg_attr(verus_only, verus_spec(result => ensures result == (self == other), no_unwind))]
    pub(crate) const fn same(self, other: Self) -> bool {
        match (self, other) {
            (Self::Leaf(a), Self::Leaf(b)) => a == b,
            (Self::Ptr(a), Self::Ptr(b)) => a.0 == b.0,
            _ => false,
        }
    }

    #[cfg_attr(verus_only, verus_spec(result => ensures result == proof::encode(self), no_unwind))]
    pub(crate) const fn encode(self) -> EncodedChild {
        match self {
            Self::Leaf(value) => EncodedChild::leaf(value),
            Self::Ptr(index) => EncodedChild::ptr(index),
        }
    }
}

/// 64 个 Morton 顺序子项，步长 256 B，CPU 起址按 cache line 对齐。
/// 每个 u32 是完整叶数据或层内节点编号，解释由独立 `NodeMask` 决定。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C, align(64))]
#[cfg_attr(verus_only, verus_verify)]
pub struct NodeValues {
    pub(crate) words: [u32; 64],
}

#[cfg_attr(verus_only, verus_verify)]
impl NodeValues {
    /// 按局部 Morton 顺序借用 payload；指针槽由对应 [`NodeMask`] 标记。
    #[cfg_attr(verus_only, verus_spec(result => ensures result@ == proof::values(*self), no_unwind))]
    pub const fn words(&self) -> &[u32; 64] {
        &self.words
    }
}

/// 64 位指针 mask，以低、高两个 u32 表示，步长和对齐均为 8 B。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C, align(8))]
#[cfg_attr(verus_only, verus_verify)]
pub struct NodeMask {
    pub(crate) words: [u32; 2],
}

#[cfg_attr(verus_only, verus_verify)]
impl NodeMask {
    /// 指针位图的低、高各 32 bit；置位表示对应 payload 是下一层节点编号。
    #[cfg_attr(verus_only, verus_spec(result => ensures result@ == proof::mask_words(*self), no_unwind))]
    pub const fn words(&self) -> &[u32; 2] {
        &self.words
    }

    #[cfg_attr(verus_only, verus_spec(result => ensures proof::mask(result) == bits, no_unwind))]
    const fn from_bits(bits: u64) -> Self {
        #[cfg(verus_only)]
        proof! { proof::split_mask(bits); }
        Self {
            words: [bits as u32, (bits >> 32) as u32],
        }
    }
    #[cfg_attr(verus_only, verus_spec(result => ensures result == proof::mask(self), no_unwind))]
    pub(crate) const fn bits(self) -> u64 {
        self.words[0] as u64 | ((self.words[1] as u64) << 32)
    }
}

/// 构建期独占节点；含 CPU 对齐填充，不作为整体上传。
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
pub struct PoolEntry {
    pub values: NodeValues,
    pub mask: NodeMask,
}

/// 不复制 payload 的节点只读视图，借用期受节点池约束。
#[derive(Clone, Copy, Debug, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct NodeRef<'a> {
    pub values: &'a NodeValues,
    pub mask: NodeMask,
}

/// 按槽号递增枚举每条 Ptr 边；重复目标仍逐次返回，不复制 payload。
#[cfg_attr(verus_only, verus_verify)]
pub struct Pointers<'a> {
    pub(crate) values: &'a [u32; 64],
    pub(crate) bits: u64,
}

#[cfg_attr(verus_only, verus_verify)]
impl Pointers<'_> {
    /// 取出最低尚未枚举的指针槽；耗尽后持续返回 `None`。
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures proof::pointer_values(*final(self)) == proof::pointer_values(*old(self)),
            result is None <==> proof::remaining(*old(self)).len() == 0,
            result is Some ==> result->Some_0 == proof::remaining(*old(self))[0]
                && proof::remaining(*final(self)) == proof::remaining(*old(self)).drop_first(),
            proof::pointer_bits(*old(self)) == 0 ==> result is None && proof::pointer_bits(*final(self)) == 0,
            proof::pointer_bits(*old(self)) != 0 ==> result == Some(proof::at_index(proof::pointer_values(*old(self))[proof::pointer_bits(*old(self)).trailing_zeros() as int]))
                && proof::pointer_bits(*final(self)) == proof::pointer_bits(*old(self)) & ((proof::pointer_bits(*old(self)) - 1) as u64),
        no_unwind
    ))]
    pub fn next(&mut self) -> Option<NodeIdx> {
        #[cfg(verus_only)]
        proof! { proof::pointer_step(self.values@, self.bits); }
        if self.bits == 0 {
            return None;
        }
        let slot = simd_ops::trailing_zeros(self.bits) as usize;
        self.bits &= self.bits - 1;
        Some(NodeIdx(self.values[slot]))
    }
}

impl Iterator for Pointers<'_> {
    type Item = NodeIdx;
    fn next(&mut self) -> Option<Self::Item> {
        Pointers::next(self)
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => requires level < 10,
    ensures result == ((crate::morton::proof::value(path) >> (6 * level)) & 63) as u8, result < 64,
        result as u64 == (crate::morton::proof::value(path) >> (6 * level)) & 63,
    no_unwind
))]
pub const fn slot_at(path: ZOrder20x3, level: u8) -> u8 {
    #[cfg(verus_only)]
    proof! { proof::selected_slot(crate::morton::proof::value(path), level); }
    ((path.value() >> (6 * level)) & 63) as u8
}

#[cfg_attr(verus_only, verus_verify)]
impl PoolEntry {
    /// 仅枚举指针槽，不解码和复制叶项；调用方借用节点到维护结束。
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures proof::pointer_values(result) == proof::values(self.values), proof::pointer_bits(result) == proof::mask(self.mask),
            proof::remaining(result).len() <= 64, no_unwind
    ))]
    pub fn pointers(&self) -> Pointers<'_> {
        #[cfg(verus_only)]
        proof! { proof::pointer_sequence_bound(self.values.words@, proof::mask(self.mask)); }
        Pointers {
            values: &self.values.words,
            bits: self.mask.bits(),
        }
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is Some <==> proof::uniform(proof::children(*self)),
            result is Some ==> forall|i: int| 0 <= i < 64 ==>
                #[trigger] proof::children(*self)[i] == Child::Leaf(result->Some_0),
        no_unwind
    ))]
    pub fn uniform_value(&self) -> Option<u32> {
        #[cfg(verus_only)]
        proof! { proof::uniform_entry(*self); }
        if self.mask.bits() != 0 {
            return None;
        }
        let result = crate::wide::uniform_values(self.values.words.as_slice());
        #[cfg(verus_only)]
        proof! { proof::fold_result(*self, result); }
        result
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        ensures proof::values(result.values) == slots@, proof::mask(result.mask) == mixed, no_unwind
    ))]
    pub const fn new(slots: &[u32; 64], mixed: u64) -> Self {
        Self {
            values: NodeValues { words: *slots },
            mask: NodeMask::from_bits(mixed),
        }
    }
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures forall|i: int| 0 <= i < 64 ==>
            #[trigger] proof::children(result)[i] == proof::decode(slots@[i]), no_unwind
    ))]
    pub fn from_encoded_slots(slots: &[EncodedChild; 64]) -> Self {
        let mut entry = Self::new(&simd_ops::array_fill(0), 0);
        let mut slot = 0usize;
        #[cfg_attr(verus_only, verus_spec(
            invariant slot <= 64, forall|i: int| 0 <= i < slot ==>
                #[trigger] proof::children(entry)[i] == proof::decode(slots@[i]),
            decreases 64 - slot,
        ))]
        while slot < 64 {
            entry.set_child(slot as u8, slots[slot].decode());
            slot += 1;
        }
        entry
    }
    #[cfg(test)]
    pub fn from_children(children: &[Child; 64]) -> Self {
        Self::from_encoded_slots(&children.map(Child::encode))
    }
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures proof::values(*result.values) == proof::values(self.values), result.mask == self.mask, no_unwind
    ))]
    pub const fn as_ref(&self) -> NodeRef<'_> {
        NodeRef {
            values: &self.values,
            mask: self.mask,
        }
    }
    #[cfg_attr(verus_only, verus_spec(result => requires level < 10,
        ensures result == proof::children(*self)[((crate::morton::proof::value(path) >> (6 * level)) & 63) as int], no_unwind
    ))]
    pub fn child_at(&self, path: ZOrder20x3, level: u8) -> Child {
        self.child(slot_at(path, level))
    }
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == proof::child_at(proof::values(self.values), proof::mask(self.mask), slot as int), no_unwind
    ))]
    pub fn child(&self, slot: u8) -> Child {
        self.as_ref().child(slot)
    }
    #[cfg_attr(verus_only, verus_spec(requires level < 10,
        ensures proof::children(*final(self)) == proof::children(*old(self)).update(((crate::morton::proof::value(path) >> (6 * level)) & 63) as int, child),
        no_unwind
    ))]
    pub fn set_child_at(&mut self, path: ZOrder20x3, level: u8, child: Child) {
        self.set_child(slot_at(path, level), child);
    }
    #[cfg_attr(verus_only, verus_spec(
        ensures proof::children(*final(self)) == if slot < 64 {
            proof::children(*old(self)).update(slot as int, child)
        } else { proof::children(*old(self)) }, no_unwind
    ))]
    pub fn set_child(&mut self, slot: u8, child: Child) {
        if slot >= 64 {
            return;
        }
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        let (value, pointer) = match child {
            Child::Leaf(value) => (value, false),
            Child::Ptr(index) => (index.0, true),
        };
        self.values.words[slot as usize] = value;
        let bits = self.mask.bits() & !(1_u64 << slot);
        self.mask = NodeMask::from_bits(bits | ((pointer as u64) << slot));
        #[cfg(verus_only)]
        proof! { proof::changed_child(before, *self, slot, child); }
    }
    #[cfg(test)]
    pub fn children(&self) -> [Child; 64] {
        self.as_ref().children()
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl NodeRef<'_> {
    /// 完整比较 payload 与 mask，哈希值不参与相等判定。
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == proof::raw_equal(self, *other),
            result <==> proof::ref_children(self) == proof::children(*other), no_unwind
    ))]
    pub fn same(self, other: &PoolEntry) -> bool {
        #[cfg(verus_only)]
        proof! { proof::same_children(self, *other); }
        simd_ops::equal_words(self.values.words(), other.values.words())
            && simd_ops::equal_words(self.mask.words(), other.mask.words())
    }
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result.values == *self.values, result.mask == self.mask, no_unwind
    ))]
    pub const fn to_entry(self) -> PoolEntry {
        PoolEntry {
            values: *self.values,
            mask: self.mask,
        }
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == proof::child_at(proof::values(*self.values), proof::mask(self.mask), slot as int), no_unwind
    ))]
    pub fn child(self, slot: u8) -> Child {
        if slot >= 64 {
            return Child::Leaf(0);
        }
        let value = self.values.words[slot as usize];
        if self.mask.bits() & (1_u64 << slot) == 0 {
            Child::Leaf(value)
        } else {
            Child::Ptr(NodeIdx(value))
        }
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result => requires level < 10,
        ensures result == proof::ref_children(self)[((crate::morton::proof::value(path) >> (6 * level)) & 63) as int], no_unwind
    ))]
    pub fn child_at(self, path: ZOrder20x3, level: u8) -> Child {
        self.child(slot_at(path, level))
    }
    #[cfg_attr(verus_only, verus_spec(result => ensures result@ == proof::ref_children(self), no_unwind))]
    pub fn children(self) -> [Child; 64] {
        let mut out = simd_ops::array_fill(Child::Leaf(0));
        let mut slot = 0usize;
        #[cfg_attr(verus_only, verus_spec(
            invariant slot <= 64, forall|i: int| 0 <= i < slot ==>
                #[trigger] out@[i] == proof::ref_children(self)[i],
            decreases 64 - slot,
        ))]
        while slot < 64 {
            out[slot] = self.child(slot as u8);
            slot += 1;
        }
        out
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl PartialEq<PoolEntry> for NodeRef<'_> {
    #[cfg_attr(verus_only, verus_spec(result => ensures result == proof::raw_equal(*self, *other)))]
    fn eq(&self, other: &PoolEntry) -> bool {
        self.same(other)
    }
}

const _: () = assert!(size_of::<NodeIdx>() == 4);
const _: () = assert!(size_of::<NodeValues>() == 256 && align_of::<NodeValues>() == 64);
const _: () = assert!(size_of::<NodeMask>() == 8 && align_of::<NodeMask>() == 8);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_arrays_preserve_full_payload_and_all_mask_bits() {
        for slot in 0_u8..64 {
            let mut entry = PoolEntry::new(&[u32::MAX; 64], 0);
            entry.set_child(slot, Child::Ptr(NodeIdx::MAX));
            let payload: &[u32] = bytemuck::cast_slice(std::slice::from_ref(&entry.values));
            let mask: &[u32] = bytemuck::cast_slice(std::slice::from_ref(&entry.mask));
            assert_eq!(payload, &[u32::MAX; 64]);
            let expected = if slot < 32 {
                [1 << slot, 0]
            } else {
                [0, 1 << (slot - 32)]
            };
            assert_eq!(mask, &expected);
            assert_eq!(entry.pointers().collect::<Vec<_>>(), [NodeIdx::MAX]);
            assert_eq!(entry.uniform_value(), None);
            for (index, actual) in (0_u8..64).zip(entry.children()) {
                assert_eq!(
                    actual,
                    if index == slot {
                        Child::Ptr(NodeIdx::MAX)
                    } else {
                        Child::Leaf(u32::MAX)
                    }
                );
            }
            entry.set_child(slot, Child::Leaf(u32::MAX));
            assert_eq!(entry.mask.bits(), 0);
            assert_eq!(entry.pointers().next(), None);
            assert_eq!(entry.uniform_value(), Some(u32::MAX));
        }
        assert!(NodeIdx::new(u64::from(u32::MAX) + 1).is_none());
    }

    #[test]
    fn pointer_iteration_matches_decoded_children() {
        let mut values = [0_u32; 64];
        for (value, index) in values.iter_mut().zip(0_u32..64) {
            *value = index;
        }
        for bits in [0, u64::MAX, 0xaaaa_5555_8000_0001, 0x0000_0001_8000_0000] {
            let entry = PoolEntry::new(&values, bits);
            let expected: Vec<_> = entry
                .children()
                .into_iter()
                .filter_map(|child| match child {
                    Child::Ptr(index) => Some(index),
                    Child::Leaf(_) => None,
                })
                .collect();
            assert_eq!(entry.pointers().collect::<Vec<_>>(), expected);
            assert_eq!(entry.uniform_value(), None);
        }
    }
}
