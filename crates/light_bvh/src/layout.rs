//! sibling packet 的地址协议；与空间分区及灯几何无关。

/// 一次选择使用的整数样本数；selector 的合法范围为 `0..SELECTORS`。
pub const SELECTORS: u32 = 1 << 24;
/// child 的叶标记；其余 bits 为原输入 quad 索引。
pub const LEAF: u32 = 1 << 31;

const PACKET_WORDS: usize = 10;
const LEAF_WORDS: usize = 8;
const LEAF_ALIGNMENT_MASK: usize = 7;

/// 指定灯容量的完整编码word数；零容量返回0，usize溢出返回None。
///
/// 包含按协议对齐的内部packet和叶目录，不分配或校验灯的采样数量上限。
/// 实际构树仍须遵循[`SELECTORS`]；容量预留与[`super::Tree::words`]使用同一布局。
#[inline]
pub const fn storage_words(lights: usize) -> Option<usize> {
    let Some(packets) = lights.saturating_sub(1).checked_mul(PACKET_WORDS) else {
        return None;
    };
    let Some(aligned) = packets.checked_add(LEAF_ALIGNMENT_MASK) else {
        return None;
    };
    // packet的乘10已成功，所以相同lights乘8不会溢出。
    (aligned & !LEAF_ALIGNMENT_MASK).checked_add(lights * LEAF_WORDS)
}

pub(super) const fn leaf_offset(lights: usize) -> usize {
    ((lights - 1) * PACKET_WORDS + LEAF_ALIGNMENT_MASK) & !LEAF_ALIGNMENT_MASK
}

pub(super) const fn packet_offset(packet: usize) -> usize {
    (packet >> 2) * 40 + (packet & 3) * 8
}

// 非 root 逻辑节点按相邻左右编号，直接定位其父 packet 中的字段。
pub(super) const fn metric_offset(node: usize) -> usize {
    packet_offset((node - 1) >> 1) + ((node - 1) & 1) * 4
}

pub(super) const fn child_offset(node: usize, packets: usize) -> usize {
    let packet = (node - 1) >> 1;
    let block = packet >> 2;
    let remaining = packets - block * 4;
    let count = if remaining < 4 { remaining } else { 4 };
    block * 40 + count * 8 + (packet & 3) * 2 + ((node - 1) & 1)
}
