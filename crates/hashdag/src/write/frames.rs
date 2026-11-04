#[cfg(not(verus_only))]
use crate::vec_ops;
use crate::write::dense_chunks::DenseFrame;
#[cfg(verus_only)]
use hash_dag_std::vec_ops;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "../proof/write_frames.rs"]
mod proof;

pub use super::frontier::BuildFrame;

/// 实例独占祖先帧、dense 深度累积器及局部导入编号翻译表。
/// 翻译表按最大单棵局部树保留容量；编号只借用节点，构造引用仍由池管理。
/// 每次退出清空长度而不缩容，不持有跨事务的节点引用。
#[derive(Default)]
#[cfg_attr(verus_only, verus_verify)]
pub struct WriteScratch {
    pub(super) frames: Vec<BuildFrame>,
    pub(super) dense: Vec<DenseFrame>,
    pub(super) imported: super::built::ImportScratch,
    #[cfg(test)]
    pub(super) work: WriteWork,
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct WriteWork {
    pub opened_prefixes: usize,
    pub max_depth: usize,
}

#[cfg_attr(verus_only, verus_verify)]
impl WriteScratch {
    /// 创建空缓冲，不申请堆容量。
    #[cfg_attr(verus_only, verus_spec(result => ensures result.empty(), no_unwind))]
    pub(crate) const fn new() -> Self {
        Self {
            frames: vec_ops::new(),
            dense: vec_ops::new(),
            imported: super::built::ImportScratch::new(),
            #[cfg(test)]
            work: WriteWork {
                opened_prefixes: 0,
                max_depth: 0,
            },
        }
    }
}

impl WriteScratch {
    #[cfg(test)]
    pub(super) fn record_frame(&mut self) {
        self.work.opened_prefixes += 1;
        self.work.max_depth = self.work.max_depth.max(self.frames.len());
    }

    /// 堆后备的保留字节，不包含 `Self` 头部或 allocator 元数据。
    pub fn retained_bytes(&self) -> usize {
        self.frames.capacity() * size_of::<BuildFrame>()
            + self.dense.capacity() * size_of::<DenseFrame>()
            + self.imported.retained_bytes()
    }

    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(ensures final(self).empty()))]
    pub(crate) fn clear(&mut self) {
        self.frames.clear();
        self.dense.clear();
        self.imported.clear();
    }
}
