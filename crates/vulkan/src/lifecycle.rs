//! 资源、记录与退役关系的唯一所有者；原始设备操作在 native 边界执行。
//!
//! 依赖和记录均保存真实执行数据。容器不保存第二套图或证明用运行历史；
//! GPU 完成与 raw 对象有效性由宿主边界提供，局部证明不替代这些事实。

use crate::{
    owner::{Arena, Key},
    retirement::Retirement,
    submission::Timeline,
};
use std::collections::TryReserveError;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(not(verus_only))]
use vstd::prelude::{proof, proof_decl};

#[cfg(verus_only)]
#[path = "proof/lifecycle.rs"]
pub(crate) mod proof;

#[path = "record.rs"]
mod record;
#[path = "resource.rs"]
pub(crate) mod resource;
#[path = "native_graph.rs"]
pub(crate) mod roots;

#[cfg(verus_only)]
pub(crate) use crate::record_ids::proof::keys as record_keys;
pub(crate) use crate::record_ids::record_ids;
pub use record::{Flight, Recorded};
pub(crate) use record::{Op, Record};
use resource::{Data, Payloads, Resources};

/// 可恢复的身份、阶段或容器失败；分配错误保留标准库来源。
#[derive(Debug)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum Error {
    ForeignOwner,
    InvalidKey,
    Retired,
    CancelledProducer,
    InvalidState,
    SerialExhausted,
    Full,
    Allocation(TryReserveError),
}

/// 所有生命周期容器只在此保存一份；字段不向 native 暴露可变访问。
/// 遵守各接口约定时，从 new 出发保持退役资源的完整 Key 全局唯一；这不替代等待覆盖或销毁顺序。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Life<P: Payloads, B> {
    owner: u64,
    resources: Arena<Data<P>>,
    records: Arena<Record<B>>,
    retiring: Vec<bool>,
    retirements: Vec<Retirement>,
    timeline: Timeline,
}

#[cfg_attr(verus_only, verus_verify)]
impl<P: Payloads, B> Life<P, B> {
    #[cfg_attr(verus_only, verus_spec(life => ensures record::proof::created(&life, owner),
        life.records_valid(), life.resources_valid(), life.submitted_value() == 0, life.completed_value() == 0,
        life.groups_unique(),
    ))]
    pub(crate) fn new(owner: u64) -> Self {
        Self {
            owner,
            resources: Arena::new(),
            records: Arena::new(),
            retiring: Vec::new(),
            retirements: Vec::new(),
            timeline: Timeline::new(),
        }
    }
}

impl<P: Payloads, B> Life<P, B> {
    pub(crate) const fn owner(&self) -> u64 {
        self.owner
    }

    /// 读取完整身份对应的真实资源；退役状态不影响只读访问。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result => ensures
        result is Ok <==> self.resource_lookup(key) is Some,
        result is Ok ==> *result->Ok_0 == self.resource_lookup(key)->Some_0,
    ))]
    pub(crate) fn resource(&self, key: Key) -> Result<&Data<P>, Error> {
        if key.owner != self.owner {
            return Err(Error::ForeignOwner);
        }
        self.resources.find(key).ok_or(Error::InvalidKey)
    }

    /// 检查完整身份、首次退役及尚存 producer；失败保留具体原因。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result => requires self.resources_valid(),
        ensures result is Ok <==> self.active_key(key),
    ))]
    pub(crate) fn active(&self, key: Key) -> Result<(), Error> {
        let resource = self.resource(key)?;
        match resource.producer() {
            Some(id) if !self.contains_record(id) => return Err(Error::CancelledProducer),
            _ => {}
        }
        proof! { self.resources.lookup_bounds(key); }
        if self.retiring[key.slot as usize] {
            return Err(Error::Retired);
        }
        Ok(())
    }

    /// 移入真实资源并分配新身份；失败归还原值，依赖有效性由构造入口校验。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).resources_valid(),
        ensures proof::inserted(old(self), final(self), resource, result),
    ))]
    pub(crate) fn insert_resource(&mut self, resource: Data<P>) -> Result<Key, (Error, Data<P>)> {
        match self.resources.try_insert(self.owner, resource) {
            Ok(key) => {
                self.retiring.resize(self.resources.len(), false);
                self.retiring[key.slot as usize] = false;
                Ok(key)
            }
            Err((error, resource)) => Err((
                match error {
                    crate::owner::InsertError::Full => Error::Full,
                    crate::owner::InsertError::Allocation(error) => Error::Allocation(error),
                },
                resource,
            )),
        }
    }

    /// 消耗完整身份并移出原值；调用者另行维护 GPU 与依赖的销毁条件。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).resources_valid(),
        ensures final(self).resources_valid(),
            result == old(self).resource_lookup(key), final(self).resource_lookup(key) is None,
            result is Some ==> final(self).resource_revoked(key),
            proof::removed(old(self), final(self), key, result),
    ))]
    pub(crate) fn take_resource(&mut self, key: Key) -> Option<Data<P>> {
        if key.owner != self.owner {
            return None;
        }
        self.resources.remove(key)
    }

    /// 只借用查询 payload，不能改变资源类别或依赖。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result => requires old(self).resources_valid(),
        ensures final(self).resources_valid(), result is Some <==> old(self).query_lookup(key) is Some,
            result is Some ==> *result->Some_0 == old(self).query_lookup(key)->Some_0,
            result is Some ==> final(self).query_lookup(key) == Some(*final(result->Some_0)),
            proof::queried(old(self), final(self), key),
    ))]
    pub(crate) fn queries_mut(&mut self, key: Key) -> Option<&mut P::Queries> {
        proof! { self.resources.shape_reflexive(); }
        if key.owner != self.owner {
            return None;
        }
        match self.resources.find_mut(key)? {
            Data::Timestamps(raw) => Some(raw),
            _ => None,
        }
    }

    /// 从真实槽位、代际和资源链接借用图，不创建执行期图副本。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(graph => requires self.resources_valid(),
        ensures graph@ == self.resource_graph(),
    ))]
    pub(crate) fn graph(&self) -> Resources<'_, P> {
        Resources::new(self.owner, &self.resources)
    }

    /// 从真实发布根和全部记录根制定并提交计划；失败不改变生命周期状态。
    /// 不执行实际释放或等待 GPU；销毁由 native 在计划成功后处理。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).resources_valid(), old(self).records_valid(),
        ensures proof::collected(old(self), final(self), roots@, &result),
            result is Err ==> *final(self) == *old(self),
            final(self).resources_valid(), final(self).records_valid(),
            old(self).groups_unique() ==> final(self).groups_unique(),
    ))]
    pub(crate) fn collect_plan(
        &mut self,
        roots: &[crate::handles::Resource],
    ) -> Result<crate::graph::Plan, crate::graph::PlanError> {
        let graph = self.graph();
        let mut published = roots::ResourceRoots::new(roots);
        let mut records = self.record_roots();
        let plan = crate::graph::plan(&graph, &mut published, &mut records)?;
        proof! { proof::plan_matches_owner(self, &plan, &graph); }
        self.commit_plan(&plan);
        Ok(plan)
    }

    /// 提交同一真实图上的计划；原位更新退役标记并保留存续组的首次 waits。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(
        requires old(self).resources_valid(), old(self).records_valid(), old(self).plan_valid(plan),
        ensures proof::committed(old(self), final(self), plan),
            old(self).groups_unique() ==> final(self).groups_unique(),
    ))]
    pub(crate) fn commit_plan(&mut self, plan: &crate::graph::Plan) {
        proof_decl! { let ghost before = *self; }
        let graph = Resources::new(self.owner, &self.resources);
        proof! { proof::plan_ready(self, plan, &graph); }
        crate::retirement::reconcile(
            &graph,
            plan,
            &mut self.retirements,
            self.owner,
            &self.records,
        );
        publish_mask(self.retiring.as_mut_slice(), plan.published.as_slice());
        proof! { let length = self.retiring.len(); proof::commit_finished(&before, self, plan, &graph, length); }
    }

    /// 从游标起移出首个已无存活等待身份的组；记录移除协议维护完成事实。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result => ensures
        proof::ready_taken(old(self), final(self), *old(cursor), *final(cursor), result),
        old(self).groups_unique() ==> final(self).groups_unique(),
    ))]
    pub(crate) fn take_ready(&mut self, cursor: &mut usize) -> Option<Retirement> {
        crate::retirement::take_ready(&mut self.retirements, cursor, self.owner, &self.records)
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<P: Payloads, B> Life<P, B> {
    #[cfg_attr(verus_only, verus_spec(serial => ensures serial ==
        if self.submitted_value() < u64::MAX { Some((self.submitted_value() + 1) as u64) } else { None },
    ))]
    pub(crate) const fn next_serial(&self) -> Option<u64> {
        self.timeline.next()
    }

    #[cfg_attr(verus_only, verifier::allow_in_spec)]
    #[cfg_attr(verus_only, verus_spec(serial => returns self.submitted_value()))]
    pub(crate) const fn submitted_serial(&self) -> u64 {
        self.timeline.submitted()
    }

    #[cfg_attr(verus_only, verifier::allow_in_spec)]
    #[cfg_attr(verus_only, verus_spec(serial => returns self.completed_value()))]
    pub(crate) const fn completed_serial(&self) -> u64 {
        self.timeline.completed()
    }

    #[cfg_attr(verus_only, verus_spec(accepted => requires old(self).records_valid(),
        ensures final(self).records_valid(),
            proof::groups_unchanged(old(self), final(self)),
            final(self).submitted_value() == old(self).submitted_value(),
            final(self).completed_value() >= old(self).completed_value(),
            accepted == (serial <= old(self).submitted_value()),
    ))]
    pub(crate) const fn observe_completion(&mut self, serial: u64) -> bool {
        self.timeline.observe(serial)
    }
}

impl<P: Payloads, B> Life<P, B> {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.resources.retained_bytes()
            + self.records.retained_bytes()
            + self.retiring.capacity() * size_of::<bool>()
            + self.retirements.capacity() * size_of::<Retirement>()
            + self
                .retirements
                .iter()
                .map(|group| {
                    group.resources.capacity() * size_of::<Key>()
                        + group.waits.capacity() * size_of::<crate::handles::RecordId>()
                })
                .sum::<usize>()
    }

    pub(crate) fn resources(&self) -> impl Iterator<Item = (Key, &Data<P>)> + Clone {
        self.resources.iter(self.owner)
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = (crate::handles::RecordId, &Record<B>)> {
        self.records
            .iter(self.owner)
            .map(|(key, record)| (crate::handles::RecordId::new(key), record))
    }
}

/// 借用固定切片边界，原位写入发布掩码的补集；不分配或复制存储。
#[inline]
#[cfg_attr(verus_only, verus_spec(
    requires old(mask)@.len() == published@.len(),
    ensures final(mask)@.len() == old(mask)@.len(),
        forall|i: int| #![auto] 0 <= i < published@.len() ==> final(mask)@[i] == !published@[i],
))]
fn publish_mask(mask: &mut [bool], published: &[bool]) {
    let len = mask.len().min(published.len());
    let mut slot = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant len == mask@.len(), mask@.len() == published@.len(), slot <= len,
            forall|i: int| #![auto] 0 <= i < slot ==> mask@[i] == !published@[i],
        decreases len - slot,
    ))]
    while slot < len {
        mask[slot] = !published[slot];
        slot += 1;
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
