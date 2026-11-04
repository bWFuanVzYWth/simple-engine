//! scratch 仅保存借用描述，空状态不持有任何节点引用。
use super::*;
use vstd::prelude::*;
verus! {
impl WriteScratch {
    pub(crate) open(in crate::write) spec fn empty(&self)->bool {
        self.frames@.len()==0 && self.dense@.len()==0 && self.imported.empty()
    }
    pub(crate) open(in crate::write) spec fn routing_empty(&self)->bool {self.frames@.len()==0}
}
}
