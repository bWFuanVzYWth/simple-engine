//! 预留容量 Patricia 索引；分支位沿路径严格递增，普通操作不分配或重建全表。

use crate::{Chunk, Handle};

#[derive(Clone, Copy)]
enum Node {
    Branch { bit: u16, zero: usize, one: usize },
    Leaf { chunk: Chunk, handle: Handle },
}

/// 预分配叶和压缩分支的二进制基数索引。
///
/// key 使用四个 `u64`：层级和翻转符号位的 xyz。路径最多 256 位；
/// 查询、删除走一遍，新增最多走两遍。删除只释放叶及其父分支。
pub(crate) struct Index {
    nodes: Vec<Option<Node>>,
    free: Vec<usize>,
    root: Option<usize>,
    len: usize,
    capacity: usize,
}

impl Index {
    /// 帧外预留 `2 * capacity - 1` 个节点；零容量支持空查询。
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let count = if capacity == 0 {
            0
        } else {
            capacity
                .checked_mul(2)
                .and_then(|count| count.checked_sub(1))
                .expect("索引容量超出范围")
        };
        let mut nodes = Vec::with_capacity(count);
        nodes.resize_with(count, || None);
        let mut free = Vec::with_capacity(count);
        free.extend((0..count).rev());
        Self {
            nodes,
            free,
            root: None,
            len: 0,
            capacity,
        }
    }

    /// 已保存的不同区块键数量。
    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    /// 是否没有区块键。
    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn reserved_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Option<Node>>()
            + self.free.capacity() * std::mem::size_of::<usize>()
    }

    pub(crate) fn reserve_capacity(
        &mut self,
        capacity: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        let count = capacity * 2 - 1;
        self.nodes.try_reserve_exact(count - self.nodes.len())?;
        self.free.try_reserve_exact(count - self.free.len())
    }

    pub(crate) fn grow_capacity(&mut self, capacity: usize) {
        let old = self.nodes.len();
        let count = capacity * 2 - 1;
        self.nodes.resize(count, None);
        self.free.extend((old..count).rev());
        self.capacity = capacity;
    }

    /// 借用对应句柄；不存在时不修改索引。
    pub(crate) fn get(&self, chunk: &Chunk) -> Option<&Handle> {
        let words = words(*chunk);
        let mut current = self.root?;
        loop {
            match self.node(current) {
                Node::Branch { bit, zero, one } => {
                    current = if bit_set(words, *bit) { *one } else { *zero };
                }
                Node::Leaf { chunk: key, handle } => return (key == chunk).then_some(handle),
            }
        }
    }

    /// 新增键或替换现有句柄；宿主必须保证新增时未达到容量。
    pub(crate) fn insert(&mut self, chunk: Chunk, handle: Handle) -> Option<Handle> {
        let Some(mut current) = self.root else {
            assert!(self.len < self.capacity, "索引容量已满");
            self.root = Some(self.allocate(Node::Leaf { chunk, handle }));
            self.len = 1;
            return None;
        };
        let key = words(chunk);
        let old_chunk = loop {
            match *self.node(current) {
                Node::Branch { bit, zero, one } => {
                    current = if bit_set(key, bit) { one } else { zero };
                }
                Node::Leaf {
                    chunk: old_chunk,
                    handle: old_handle,
                } => {
                    if old_chunk == chunk {
                        self.nodes[current] = Some(Node::Leaf { chunk, handle });
                        return Some(old_handle);
                    }
                    break old_chunk;
                }
            }
        };
        assert!(self.len < self.capacity, "索引容量已满");
        let bit = first_difference(key, words(old_chunk));
        current = self.root.expect("非空索引拥有根");
        let mut parent = None;
        loop {
            match *self.node(current) {
                Node::Branch {
                    bit: branch_bit,
                    zero,
                    one,
                } if branch_bit < bit => {
                    let direction = bit_set(key, branch_bit);
                    parent = Some((current, direction));
                    current = if direction { one } else { zero };
                }
                _ => break,
            }
        }
        let leaf = self.allocate(Node::Leaf { chunk, handle });
        let (zero, one) = if bit_set(key, bit) {
            (current, leaf)
        } else {
            (leaf, current)
        };
        let branch = self.allocate(Node::Branch { bit, zero, one });
        if let Some((parent, direction)) = parent {
            self.set_child(parent, direction, branch);
        } else {
            self.root = Some(branch);
        }
        self.len += 1;
        None
    }

    /// 删除键，最多回收叶及父分支，不遍历或销毁兄弟子树。
    pub(crate) fn remove(&mut self, chunk: &Chunk) -> Option<Handle> {
        let key = words(*chunk);
        let mut current = self.root?;
        let mut parent = None;
        let mut grandparent = None;
        let handle = loop {
            match *self.node(current) {
                Node::Branch { bit, zero, one } => {
                    let direction = bit_set(key, bit);
                    grandparent = parent;
                    parent = Some((current, direction));
                    current = if direction { one } else { zero };
                }
                Node::Leaf { chunk: old, handle } => {
                    if old != *chunk {
                        return None;
                    }
                    break handle;
                }
            }
        };
        if let Some((parent, direction)) = parent {
            let Node::Branch { zero, one, .. } = *self.node(parent) else {
                unreachable!("叶的父节点必须是分支")
            };
            let sibling = if direction { zero } else { one };
            if let Some((grandparent, direction)) = grandparent {
                self.set_child(grandparent, direction, sibling);
            } else {
                self.root = Some(sibling);
            }
            self.release(parent);
        } else {
            self.root = None;
        }
        self.release(current);
        self.len -= 1;
        Some(handle)
    }

    fn node(&self, index: usize) -> &Node {
        self.nodes[index].as_ref().expect("索引链接指向有效节点")
    }

    fn allocate(&mut self, node: Node) -> usize {
        let index = self.free.pop().expect("叶容量保证分支槽充足");
        debug_assert!(self.nodes[index].is_none());
        self.nodes[index] = Some(node);
        index
    }

    fn release(&mut self, index: usize) {
        self.nodes[index] = None;
        self.free.push(index);
    }

    fn set_child(&mut self, index: usize, direction: bool, child: usize) {
        let Some(Node::Branch { zero, one, .. }) = self.nodes[index].as_mut() else {
            unreachable!("只有分支可以替换子节点")
        };
        if direction {
            *one = child;
        } else {
            *zero = child;
        }
    }
}

const fn words(chunk: Chunk) -> [u64; 4] {
    [
        chunk.lod as u64,
        chunk.coord[0] as u64 ^ (1 << 63),
        chunk.coord[1] as u64 ^ (1 << 63),
        chunk.coord[2] as u64 ^ (1 << 63),
    ]
}

const fn bit_set(words: [u64; 4], bit: u16) -> bool {
    words[(bit / 64) as usize] & (1_u64 << (63 - bit % 64)) != 0
}

fn first_difference(left: [u64; 4], right: [u64; 4]) -> u16 {
    for word in 0..4 {
        let difference = left[word] ^ right[word];
        if difference != 0 {
            return word as u16 * 64 + difference.leading_zeros() as u16;
        }
    }
    unreachable!("新增叶的键必须不同")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn handle(index: usize, generation: u64) -> Handle {
        Handle { index, generation }
    }

    #[test]
    fn preserves_negative_and_extreme_coordinate_keys() {
        let mut index = Index::with_capacity(6);
        let chunks = [
            Chunk::new([i64::MIN, 0, 0], 0),
            Chunk::new([i64::MAX, 0, 0], 0),
            Chunk::new([-1, i64::MIN, 3], 255),
            Chunk::new([-1, i64::MAX, 3], 255),
            Chunk::new([0, 0, i64::MIN], 31),
            Chunk::new([0, 0, i64::MAX], 31),
        ];
        for (slot, chunk) in chunks.into_iter().enumerate() {
            assert_eq!(index.insert(chunk, handle(slot, 1)), None);
        }
        for (slot, chunk) in chunks.into_iter().enumerate() {
            assert_eq!(index.get(&chunk), Some(&handle(slot, 1)));
        }
    }

    #[test]
    fn replacement_at_capacity_updates_slot_generation() {
        let mut index = Index::with_capacity(1);
        let chunk = Chunk::new([-4, 7, -9], 2);
        assert_eq!(index.insert(chunk, handle(0, 1)), None);
        assert_eq!(index.insert(chunk, handle(0, 3)), Some(handle(0, 1)));
        assert_eq!(index.len(), 1);
        assert_eq!(index.get(&chunk), Some(&handle(0, 3)));
    }

    #[test]
    fn deleting_a_leaf_preserves_its_sibling_subtree() {
        let mut index = Index::with_capacity(4);
        let chunks = [
            Chunk::new([-1, 0, 0], 0),
            Chunk::new([0, 0, 0], 0),
            Chunk::new([0, 0, 1], 0),
            Chunk::new([0, 0, 2], 0),
        ];
        for (slot, chunk) in chunks.into_iter().enumerate() {
            index.insert(chunk, handle(slot, 1));
        }
        assert_eq!(index.remove(&chunks[0]), Some(handle(0, 1)));
        for (slot, chunk) in chunks.into_iter().enumerate().skip(1) {
            assert_eq!(index.get(&chunk), Some(&handle(slot, 1)));
        }
        assert_eq!(index.remove(&chunks[0]), None);
    }

    #[test]
    fn churn_matches_ordered_map_without_growing_arenas() {
        let mut index = Index::with_capacity(64);
        let arena_len = index.nodes.len();
        let free_capacity = index.free.capacity();
        let mut expected = BTreeMap::new();
        let mut random = 0x9e37_79b9_7f4a_7c15_u64;
        for generation in 1..=10_000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let id = (random % 96) as i64;
            let chunk = Chunk::new([id - 48, id * 17 % 37 - 18, id * id - 4000], (id % 7) as u8);
            if random & 3 == 0 {
                assert_eq!(index.remove(&chunk), expected.remove(&chunk));
            } else if expected.len() < 64 || expected.contains_key(&chunk) {
                let value = handle(id as usize, generation);
                assert_eq!(index.insert(chunk, value), expected.insert(chunk, value));
            }
            assert_eq!(index.get(&chunk), expected.get(&chunk));
            assert_eq!(index.len(), expected.len());
        }
        for (chunk, value) in expected {
            assert_eq!(index.remove(&chunk), Some(value));
        }
        assert!(index.is_empty());
        assert_eq!(index.root, None);
        assert_eq!(index.nodes.len(), arena_len);
        assert_eq!(index.free.len(), arena_len);
        assert_eq!(index.free.capacity(), free_capacity);
    }
}
