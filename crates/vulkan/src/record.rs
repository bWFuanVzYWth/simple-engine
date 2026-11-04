//! 真实录制数据与记录身份；设备操作在 native 中消费这些操作。
//!
//! roots 与访问分类直接读取同一 Op，记录中的缓冲按值转移，不维护镜像或引用计数。
//! GPU 完成事实和 native 构造 AS roots 的关系仍由设备边界提供。

use super::resource::{Data, Payloads};
use super::{Error, Life};
use crate::{
    handles::{
        Buffer, ComputePipeline, RayTracingPipeline, RecordId, Resource, ResourceAccess,
        TimestampQueries,
    },
    owner::{Arena, InsertError, Key},
    submission::RecordState,
};
#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "proof/record.rs"]
pub(crate) mod proof;

#[path = "record_batch.rs"]
pub(super) mod batch;

#[derive(Clone, Copy)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum Op {
    Trace(RayTracingPipeline, [u32; 3]),
    Dispatch(ComputePipeline, [u32; 3]),
    Barrier,
    Clear(Buffer),
    Keepalive(Resource),
    Access(ResourceAccess),
    ResetTimestamps(TimestampQueries),
    Timestamp(TimestampQueries, u32),
}

#[cfg_attr(verus_only, verus_verify)]
impl Op {
    #[cfg_attr(verus_only, verus_spec(key => ensures key == self.root_view()))]
    pub(crate) const fn root(&self) -> Option<Key> {
        match self {
            Self::Trace(p, _) => Some(p.key),
            Self::Dispatch(p, _) => Some(p.key),
            Self::Clear(buffer) => Some(buffer.key),
            Self::Keepalive(resource) => Some(resource.0),
            Self::Access(access) => Some(access.key()),
            Self::ResetTimestamps(queries) | Self::Timestamp(queries, _) => Some(queries.key),
            Self::Barrier => None,
        }
    }

    /// 操作直接登记的 host 互斥；Trace/Dispatch 的绑定访问由资源表提供。
    #[cfg_attr(verus_only, verus_spec(key => ensures key == self.access_view()))]
    pub(crate) const fn direct_access(&self) -> Option<Key> {
        match self {
            Self::Clear(buffer) => Some(buffer.key),
            Self::Access(access) => Some(access.key()),
            Self::ResetTimestamps(queries) => Some(queries.key),
            _ => None,
        }
    }

    #[cfg_attr(verus_only, verus_spec(key => ensures key == self.reset_view()))]
    pub(crate) const fn resets(&self) -> Option<Key> {
        match self {
            Self::ResetTimestamps(queries) => Some(queries.key),
            _ => None,
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Record<B> {
    pub(super) operations: Vec<Op>,
    pub(super) uses: Vec<Key>,
    pub(super) state: RecordState,
    pub(super) build_roots: Vec<Key>,
    pub(super) buffers: B,
}

#[cfg_attr(verus_only, verus_verify)]
impl<B> Record<B> {
    #[cfg_attr(verus_only, verus_spec(record => ensures
        record.operations_value() == operations, record.uses_value() == uses, record.roots_value() == build_roots,
        record.buffers_value() == buffers, record.state_value() is Pending,
    ))]
    pub(crate) fn new(
        operations: Vec<Op>,
        uses: Vec<Key>,
        build_roots: Vec<Key>,
        buffers: B,
    ) -> Self {
        Self {
            operations,
            uses,
            state: RecordState::Pending,
            build_roots,
            buffers,
        }
    }

    #[cfg_attr(verus_only, verus_spec(operations => ensures operations@ == self.operations_value()@))]
    pub(crate) fn operations(&self) -> &[Op] {
        &self.operations
    }

    #[cfg_attr(verus_only, verus_spec(uses => ensures uses@ == self.uses_value()@))]
    pub(crate) fn uses(&self) -> &[Key] {
        &self.uses
    }

    #[cfg_attr(verus_only, verus_spec(roots => ensures roots@ == self.roots_value()@))]
    pub(crate) fn build_roots(&self) -> &[Key] {
        &self.build_roots
    }

    #[cfg_attr(verus_only, verus_spec(state => ensures state == self.state_value()))]
    pub(crate) const fn state(&self) -> RecordState {
        self.state
    }

    #[cfg_attr(verus_only, verus_spec(buffers => ensures *buffers == self.buffers_value()))]
    pub(crate) const fn buffers(&self) -> &B {
        &self.buffers
    }

    #[cfg_attr(verus_only, verus_spec(buffers => ensures buffers == self.buffers_value()))]
    pub(crate) fn into_buffers(self) -> B {
        self.buffers
    }
}

#[cfg(not(verus_only))]
impl<B> Record<B> {
    pub(crate) fn metadata_bytes(&self) -> usize {
        self.operations.capacity() * size_of::<Op>()
            + (self.uses.capacity() + self.build_roots.capacity()) * size_of::<Key>()
    }
}

/// owner 内固定命令版本的唯一外部票据；须提交或显式取消。
#[must_use = "须提交或通过 Context::cancel 取消命令"]
#[cfg_attr(verus_only, verus_verify)]
pub struct Recorded {
    pub(crate) key: RecordId,
}

/// 外部已编码批次的完成票据；资源仍由唯一 Life 所有者保活。
#[must_use = "宿主确认 GPU 完成后须调用 Context::complete"]
#[cfg_attr(verus_only, verus_verify)]
pub struct Flight {
    pub(crate) key: RecordId,
}

#[cfg_attr(verus_only, verus_verify)]
impl<P: Payloads, B> Life<P, B> {
    #[cfg_attr(verus_only, verus_spec(record => ensures
        record is Ok <==> self.record_lookup(id) is Some,
        record is Ok ==> *record->Ok_0 == self.record_lookup(id)->Some_0,
    ))]
    pub(crate) fn record(&self, id: RecordId) -> Result<&Record<B>, Error> {
        let key = id.key();
        if key.owner != self.owner {
            return Err(Error::ForeignOwner);
        }
        self.records.find(key).ok_or(Error::InvalidKey)
    }

    #[cfg_attr(verus_only, verus_spec(present => ensures present == (self.record_lookup(id) is Some)))]
    pub(crate) fn contains_record(&self, id: RecordId) -> bool {
        id.key().owner == self.owner && self.records.find(id.key()).is_some()
    }

    /// 接管 Pending 记录；状态或分配失败归还原值。
    /// 构建根须由内部创建路径提供本 owner 的当前 AS 输出，此 native 调用关系尚未验证。
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).records_valid(), old(self).resources_valid(),
        proof::build_inputs(old(self), &record),
        ensures proof::record_inserted(old(self), final(self), record, result),
    ))]
    pub(crate) fn insert_record(
        &mut self,
        record: Record<B>,
    ) -> Result<RecordId, (Error, Record<B>)> {
        proof_decl! { let ghost before = *self; let ghost value = record; }
        proof! { self.records.shape_reflexive(); }
        if !matches!(record.state, RecordState::Pending) {
            return Err((Error::InvalidState, record));
        }
        let key = match self.records.try_insert(self.owner, record) {
            Ok(key) => key,
            Err((error, record)) => {
                let error = match error {
                    InsertError::Full => Error::Full,
                    InsertError::Allocation(error) => Error::Allocation(error),
                };
                return Err((error, record));
            }
        };
        let id = RecordId::new(key);
        let record = self.records.find(key).expect("记录刚插入");
        proof! { proof::producers_empty(&self.resources, self.owner, record.build_roots@, id, false); }
        let mut i = 0;
        #[cfg_attr(verus_only, verus_spec(invariant proof::inserting(&before, self, value, id, i as int), *record == value,
            decreases record.build_roots@.len() - i,
        ))]
        while i < record.build_roots.len() {
            proof_decl! { let ghost previous = self.resources; }
            self.resources
                .find_mut(record.build_roots[i])
                .expect("构建输出已插入")
                .set_producer(Some(id));
            proof! { proof::producers_step(&before.resources, &previous, &self.resources, self.owner, record.build_roots@, i as int, id, false); }
            i += 1;
        }
        Ok(id)
    }

    /// Pending 进入外部编码阶段；失败不改变记录内容。
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).records_valid(),
        ensures proof::transitioned(old(self), final(self), id, RecordState::External, result),
    ))]
    pub(crate) fn mark_external(&mut self, id: RecordId) -> Result<(), Error> {
        proof_decl! { let ghost before = *self; }
        proof! { self.records.shape_reflexive(); }
        let key = id.key();
        if key.owner != self.owner {
            return Err(Error::ForeignOwner);
        }
        let record = match self.records.find_mut(key) {
            Some(record) => record,
            None => {
                proof! { self.records.entries_preserve_values(&before.records); }
                return Err(Error::InvalidKey);
            }
        };
        if !matches!(record.state, RecordState::Pending) {
            proof! { proof::transition_rejected(&before, self, id, RecordState::External, Error::InvalidState); }
            return Err(Error::InvalidState);
        }
        record.state = RecordState::External;
        proof! { proof::transition_done(&before, self, id, RecordState::External); }
        Ok(())
    }

    /// Pending 开始同步执行；host wait 的完成事实由 native 提供。
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).records_valid(),
        ensures proof::transitioned(old(self), final(self), id, RecordState::Synchronous, result),
    ))]
    pub(crate) fn begin_sync(&mut self, id: RecordId) -> Result<(), Error> {
        proof_decl! { let ghost before = *self; }
        proof! { self.records.shape_reflexive(); }
        let key = id.key();
        if key.owner != self.owner {
            return Err(Error::ForeignOwner);
        }
        let record = match self.records.find_mut(key) {
            Some(record) => record,
            None => {
                proof! { self.records.entries_preserve_values(&before.records); }
                return Err(Error::InvalidKey);
            }
        };
        let next = match record.state.begin_sync() {
            Some(next) => next,
            None => {
                proof! { proof::transition_rejected(&before, self, id, RecordState::Synchronous, Error::InvalidState); }
                return Err(Error::InvalidState);
            }
        };
        record.state = next;
        proof! { proof::transition_done(&before, self, id, RecordState::Synchronous); }
        Ok(())
    }

    /// 取消当前 Pending 并按值归还；保留 AS 的失效 producer 票据。
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).records_valid(), old(self).resources_valid(),
        ensures proof::taken(old(self), final(self), id, proof::Take::Pending, result),
    ))]
    pub(crate) fn take_pending(&mut self, id: RecordId) -> Result<Record<B>, Error> {
        proof! { self.records.shape_reflexive(); }
        if !matches!(self.record(id)?.state, RecordState::Pending) {
            return Err(Error::InvalidState);
        }
        Ok(self.records.remove(id.key()).expect("完整记录已校验"))
    }

    /// 宿主确认 GPU 完成后移出 External，只清除仍指向此记录的 producer。
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).records_valid(), old(self).resources_valid(),
        ensures proof::taken(old(self), final(self), id, proof::Take::External, result),
    ))]
    pub(crate) fn take_external(&mut self, id: RecordId) -> Result<Record<B>, Error> {
        proof! { self.records.shape_reflexive(); }
        if !matches!(self.record(id)?.state, RecordState::External) {
            return Err(Error::InvalidState);
        }
        let record = self.records.remove(id.key()).expect("完整记录已校验");
        self.finish_producers(id, &record.build_roots);
        Ok(record)
    }

    /// 只移出已接受且序号不超过完成水位的记录。
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).records_valid(), old(self).resources_valid(),
        ensures proof::taken(old(self), final(self), id, proof::Take::Completed, result),
    ))]
    pub(crate) fn take_completed(&mut self, id: RecordId) -> Result<Record<B>, Error> {
        proof! { self.records.shape_reflexive(); }
        if !self.record(id)?.state.completed(self.timeline.completed()) {
            return Err(Error::InvalidState);
        }
        let record = self.records.remove(id.key()).expect("完整记录已校验");
        self.finish_producers(id, &record.build_roots);
        Ok(record)
    }

    /// 同步执行的前置失败也可仍为 Pending；成功仅接受已开始的同步阶段。
    /// completed 由 native 的实际 host wait 结果提供。
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).records_valid(), old(self).resources_valid(),
        ensures proof::taken(old(self), final(self), id, proof::Take::Sync(completed), result),
    ))]
    pub(crate) fn take_sync(&mut self, id: RecordId, completed: bool) -> Result<Record<B>, Error> {
        proof! { self.records.shape_reflexive(); }
        let state = self.record(id)?.state;
        if !matches!(state, RecordState::Synchronous)
            && (completed || !matches!(state, RecordState::Pending))
        {
            return Err(Error::InvalidState);
        }
        let record = self.records.remove(id.key()).expect("完整记录已校验");
        if completed {
            self.finish_producers(id, &record.build_roots);
        }
        Ok(record)
    }

    #[cfg_attr(verus_only, verus_spec(requires old(self).resources_valid(),
        ensures proof::producers_finished(old(self), final(self), roots@, id),
    ))]
    fn finish_producers(&mut self, id: RecordId, roots: &[Key]) {
        proof_decl! { let ghost before = *self; }
        proof! { proof::producers_empty(&self.resources, self.owner, roots@, id, true); }
        let mut i = 0;
        #[cfg_attr(verus_only, verus_spec(invariant proof::finishing(&before, self, roots@, i as int, id),
            decreases roots@.len() - i,
        ))]
        while i < roots.len() {
            proof_decl! { let ghost previous = self.resources; }
            finish_root(&mut self.resources, self.owner, roots[i], id);
            proof! { proof::finish_step(&before.resources, &previous, &self.resources, self.owner, roots@, i as int, id); }
            i += 1;
        }
    }

    /// 仅供宿主已确认整个 owner 空闲且以后不再提交其命令时的析构清理。
    /// 此处只验证数据移出，不证明宿主提供的空闲事实。
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).records_valid(), old(self).resources_valid(),
        ensures proof::taken(old(self), final(self), id, proof::Take::Shutdown, result),
    ))]
    pub(crate) fn take_shutdown(&mut self, id: RecordId) -> Result<Record<B>, Error> {
        proof! { self.records.shape_reflexive(); }
        self.record(id)?;
        let record = self.records.remove(id.key()).expect("完整记录已校验");
        self.finish_producers(id, &record.build_roots);
        Ok(record)
    }

    /// 故障清理保守保留完整当前身份；其他 owner 或失效身份不影响记录。
    #[cfg_attr(verus_only, verus_spec(requires old(self).records_valid(),
        ensures proof::resolved(old(self), final(self), ids@, 0, false),
    ))]
    pub(crate) fn quarantine_records(&mut self, ids: &[RecordId]) {
        batch::quarantine(&mut self.records, self.owner, ids);
    }

    /// 逐项预检 Pending，失败回滚本次前缀；成功借用 owner 与原身份切片。
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).records_valid(),
        ensures result is Err ==> proof::prepare_failed(old(self), final(self)),
            result is Ok ==> proof::attempt_started(old(self), &result->Ok_0, ids@),
            result is Ok ==> *final(self) == *final(result->Ok_0.owner_ref()),
    ))]
    pub(crate) fn prepare_batch<'a>(
        &'a mut self,
        ids: &'a [RecordId],
    ) -> Result<Attempt<'a, P, B>, Error> {
        proof! { self.records.shape_reflexive(); }
        let serial = self.timeline.next().ok_or(Error::SerialExhausted)?;
        batch::prepare(&mut self.records, self.owner, ids, serial)?;
        Ok(Attempt {
            life: self,
            ids,
            serial,
        })
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(requires old(resources).valid(),
    ensures proof::root_finished(old(resources), final(resources), owner, root, id),
))]
fn finish_root<P: Payloads>(resources: &mut Arena<Data<P>>, owner: u64, root: Key, id: RecordId) {
    proof_decl! { let ghost before = *resources; }
    proof! { resources.shape_reflexive(); }
    if root.owner != owner {
        return;
    }
    let Some(data) = resources.find_mut(root) else {
        proof! { resources.entries_preserve_values(&before); }
        return;
    };
    data.finish_producer(id);
}

/// 借用同一 owner 与原始批次身份；正常路径必须显式接受或隔离。
#[must_use = "提交后须接受或隔离整个批次"]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Attempt<'a, P: Payloads, B> {
    life: &'a mut Life<P, B>,
    ids: &'a [RecordId],
    serial: u64,
}

#[cfg_attr(verus_only, verus_verify)]
impl<P: Payloads, B> Attempt<'_, P, B> {
    #[cfg_attr(verus_only, verus_spec(serial => ensures serial == self.serial_value()))]
    pub(crate) const fn serial(&self) -> u64 {
        self.serial
    }

    #[cfg_attr(verus_only, verus_spec(serial => ensures serial == self.serial_value(),
        proof::resolved(old(self.owner_ref()), final(self.owner_ref()), self.ids_value(), self.serial_value(), true),
    ))]
    pub(crate) fn accept(self) -> u64 {
        proof! { use_type_invariant(&self); }
        let Self { life, ids, serial } = self;
        assert!(life.timeline.accept(serial));
        batch::accept(&mut life.records, life.owner, ids, serial);
        serial
    }

    #[cfg_attr(verus_only, verus_spec(ensures
        proof::resolved(old(self.owner_ref()), final(self.owner_ref()), self.ids_value(), self.serial_value(), false),
    ))]
    pub(crate) fn quarantine(self) {
        proof! { use_type_invariant(&self); }
        let Self { life, ids, .. } = self;
        batch::quarantine(&mut life.records, life.owner, ids);
    }
}
