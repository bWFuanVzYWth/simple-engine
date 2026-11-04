use crate::ZOrder20x3;

use crate::hashdag::{DAG_DEPTH, Error, ROOT_LOD};
use crate::node::{Child, NodeIdx, NodeRef, slot_at};
use crate::pool::Pool;
use crate::read::SubtreeState;

/// 沿 path 从 root child 下钻到 `target_lod`。
/// 提前命中 Leaf 则直接返回。
#[inline]
pub(super) fn to_lod(
    pools: &[Pool; DAG_DEPTH as usize],
    mut child: Child,
    path: ZOrder20x3,
    target_lod: u8,
) -> Result<Child, Error> {
    for pool_lod in (target_lod..=ROOT_LOD).rev() {
        child = match child {
            Child::Leaf(_) => return Ok(child),
            Child::Ptr(idx) => {
                let pool = pools
                    .get(usize::from(pool_lod))
                    .ok_or(Error::InvalidLod(pool_lod))?;
                pool.get_live(idx)
                    .ok_or(Error::NodeMissing(idx))?
                    .child_at(path, pool_lod)
            }
        };
    }
    Ok(child)
}

/// 同一目标层的祖先只按空间前缀复用；相同 NodeIdx 不等于相同请求位置。
pub(super) struct Ordered<'a> {
    pools: &'a [Pool; DAG_DEPTH as usize],
    ancestors: [Child; DAG_DEPTH as usize + 1],
    previous: Option<u64>,
    reached: u8,
    target: u8,
    #[cfg(test)]
    pub(super) node_reads: usize,
}

impl<'a> Ordered<'a> {
    /// `target` 为原生目标层，而非空间 LOD。
    #[cfg(test)]
    pub(super) fn new(pools: &'a [Pool; DAG_DEPTH as usize], root: Child, target: u8) -> Self {
        let mut ancestors = [Child::Leaf(0); DAG_DEPTH as usize + 1];
        ancestors[usize::from(DAG_DEPTH)] = root;
        Self {
            pools,
            ancestors,
            previous: None,
            reached: DAG_DEPTH,
            target,
            #[cfg(test)]
            node_reads: 0,
        }
    }

    fn read_node(&mut self, level: u8, index: NodeIdx) -> Result<NodeRef<'a>, Error> {
        #[cfg(test)]
        {
            self.node_reads += 1;
        }
        node(self.pools, level, index)
    }

    #[inline]
    pub(super) fn seek(&mut self, path: ZOrder20x3) -> Result<Child, Error> {
        let mut level = self.previous.map_or(DAG_DEPTH, |previous| {
            let changed_bits = u64::BITS - (previous ^ path.value()).leading_zeros();
            let changed_level = changed_bits.div_ceil(6) as u8;
            changed_level.max(self.target).max(self.reached)
        });
        let mut child = self.ancestors[usize::from(level)];
        while level > self.target {
            let Child::Ptr(index) = child else { break };
            level -= 1;
            child = self.read_node(level, index)?.child_at(path, level);
            self.ancestors[usize::from(level)] = child;
        }
        self.previous = Some(path.value());
        self.reached = level;
        Ok(child)
    }

    pub(super) fn value(&mut self, path: ZOrder20x3) -> Result<u32, Error> {
        match self.seek(path)? {
            Child::Leaf(value) => Ok(value),
            Child::Ptr(index) => Err(Error::NodeMissing(index)),
        }
    }

    pub(super) fn state(&mut self, path: ZOrder20x3, lod: u8) -> Result<SubtreeState, Error> {
        let index = match self.seek(path)? {
            Child::Leaf(value) => return Ok(SubtreeState::Uniform { value }),
            Child::Ptr(index) => index,
        };
        let level = self
            .target
            .checked_sub(1)
            .ok_or(Error::NodeMissing(index))?;
        let node = self.read_node(level, index)?;
        Ok(state_for_node(node, path, level, lod))
    }
}

/// 分类已定位到 `lod.div_ceil(2)` 的 child；Ptr 仍须验证目标节点。
#[inline]
pub(super) fn state_for_child(
    pools: &[Pool; DAG_DEPTH as usize],
    child: Child,
    path: ZOrder20x3,
    lod: u8,
) -> Result<SubtreeState, Error> {
    let index = match child {
        Child::Leaf(value) => return Ok(SubtreeState::Uniform { value }),
        Child::Ptr(index) => index,
    };
    let level = lod
        .div_ceil(2)
        .checked_sub(1)
        .ok_or(Error::NodeMissing(index))?;
    let node = pools
        .get(usize::from(level))
        .ok_or(Error::InvalidLod(level))?
        .get_live(index)
        .ok_or(Error::NodeMissing(index))?;
    Ok(state_for_node(node, path, level, lod))
}

#[inline]
fn state_for_node(node: NodeRef<'_>, path: ZOrder20x3, level: u8, lod: u8) -> SubtreeState {
    if lod.is_multiple_of(2) {
        return SubtreeState::Mixed;
    }
    let first = slot_at(path, level) & 0b11_1000;
    let Child::Leaf(value) = node.child(first) else {
        return SubtreeState::Mixed;
    };
    if (first + 1..first + 8).all(|slot| node.child(slot) == Child::Leaf(value)) {
        SubtreeState::Uniform { value }
    } else {
        SubtreeState::Mixed
    }
}

fn node(
    pools: &[Pool; DAG_DEPTH as usize],
    level: u8,
    index: NodeIdx,
) -> Result<NodeRef<'_>, Error> {
    // 有效 root 与父边保活节点；读取借用禁止回收，只需检查物理槽范围。
    pools
        .get(usize::from(level))
        .ok_or(Error::InvalidLod(level))?
        .get_live(index)
        .ok_or(Error::NodeMissing(index))
}

/// 输入形状与顺序已由公共边界检查；同一对齐区域只分类一次并广播结果。
#[inline]
pub(super) fn batch<T: Copy>(
    pools: &[Pool; DAG_DEPTH as usize],
    root: Child,
    paths: &[ZOrder20x3],
    lod: u8,
    output: &mut [T],
    mut query: impl FnMut(&mut Ordered<'_>, ZOrder20x3) -> Result<T, Error>,
) -> Result<(), Error> {
    let mut ancestors = [Child::Leaf(0); DAG_DEPTH as usize + 1];
    ancestors[usize::from(DAG_DEPTH)] = root;
    let mut ordered = Ordered {
        pools,
        ancestors,
        previous: None,
        reached: DAG_DEPTH,
        target: lod.div_ceil(2),
        #[cfg(test)]
        node_reads: 0,
    };
    let mut start = 0;
    while start < paths.len() {
        let path = paths[start];
        let region = path.value() >> (3 * lod);
        let mut end = start + 1;
        while end < paths.len() && paths[end].value() >> (3 * lod) == region {
            end += 1;
        }
        output[start..end].fill(query(&mut ordered, path)?);
        start = end;
    }
    Ok(())
}
