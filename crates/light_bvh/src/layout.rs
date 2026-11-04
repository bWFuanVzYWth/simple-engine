//! sibling packet 的地址协议；与空间分区及灯几何无关。

/// 一次选择使用的整数样本数；selector 的合法范围为 `0..SELECTORS`。
pub const SELECTORS: u32 = 1 << 24;
/// child 的叶标记；其余 bits 为原输入 quad 索引。
pub const LEAF: u32 = 1 << 31;

pub(super) const fn leaf_offset(lights: usize) -> usize {
    ((lights - 1) * 10 + 7) & !7
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
