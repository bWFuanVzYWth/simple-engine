//! 单项覆盖事务的借用增量；资源仍由 [`State`] 唯一持有。

use crate::{Chunk, Handle, State};

/// 一个完整覆盖事务对宿主视图的修改；先准备全部项，发布后再整体启用。
pub enum VisibleChange<'a, R> {
    /// 从候选视图删除旧叶，资源继续由状态按最后使用值退役。
    Remove {
        /// 旧叶的稳定身份。
        handle: Handle,
        /// 旧叶的区块键。
        chunk: Chunk,
    },
    /// 插入新叶或替换同句柄的资源；借用不会转移所有权。
    Upsert {
        /// 新叶的稳定身份。
        handle: Handle,
        /// 新叶的区块键。
        chunk: Chunk,
        /// 本次事务准备使用的完整资源。
        resource: &'a R,
    },
}

/// 经校验的单项覆盖事务增量；最多九项，不分配、不扫描其他节点。
///
/// 删除项先于插入项。迭代结束不代表已发布；宿主准备失败须撤销候选并拒绝原 token。
pub struct PatchResources<'a, R> {
    pub(crate) state: &'a State<R>,
    pub(crate) entries: [(Handle, bool); 9],
    pub(crate) next: usize,
    pub(crate) len: usize,
}

impl<'a, R> Iterator for PatchResources<'a, R> {
    type Item = VisibleChange<'a, R>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.len {
            return None;
        }
        let (handle, upsert) = self.entries[self.next];
        self.next += 1;
        let chunk = self.state.chunk(handle).expect("借用期间事务身份有效");
        Some(if upsert {
            VisibleChange::Upsert {
                handle,
                chunk,
                resource: self
                    .state
                    .prepared_resource(handle)
                    .expect("已校验完整候选资源"),
            }
        } else {
            VisibleChange::Remove { handle, chunk }
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.next;
        (remaining, Some(remaining))
    }
}

impl<R> ExactSizeIterator for PatchResources<'_, R> {}
impl<R> std::iter::FusedIterator for PatchResources<'_, R> {}
