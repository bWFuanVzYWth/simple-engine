//! 不依赖 Vulkan 的真实所有者协议回归。

use super::{
    Error, Life, Record,
    resource::{self, Data},
};
use crate::{handles::RecordId, submission::RecordState};

struct Payload;
impl resource::Payloads for Payload {
    type Queries = usize;
    type Buffer = usize;
    type Image = usize;
    type Sampler = usize;
    type Set = usize;
    type Layout = usize;
    type RayProgram = usize;
    type ComputeProgram = usize;
    type Structure = usize;
    type Micromap = usize;
    type SharedBuffer = usize;
    type SharedTexture = usize;
    type SampledTexture = usize;
}

type Owner = Life<Payload, usize>;

fn record(life: &mut Owner, payload: usize) -> RecordId {
    life.insert_record(Record::new(Vec::new(), Vec::new(), Vec::new(), payload))
        .unwrap_or_else(|_| panic!("记录插入失败"))
}

#[test]
fn duplicate_batch_restores_pending_records() {
    let mut life = Owner::new(7);
    let id = record(&mut life, 42);
    assert!(matches!(
        life.prepare_batch(&[id, id]),
        Err(Error::InvalidState)
    ));
    assert_eq!(life.record(id).unwrap().state(), RecordState::Pending);
    assert_eq!(life.submitted_serial(), 0);
}

#[test]
fn foreign_batch_restores_accepted_prefix() {
    let mut life = Owner::new(7);
    let id = record(&mut life, 42);
    let foreign = record(&mut Owner::new(8), 99);
    assert!(matches!(
        life.prepare_batch(&[id, foreign]),
        Err(Error::ForeignOwner)
    ));
    assert_eq!(life.record(id).unwrap().state(), RecordState::Pending);
    assert_eq!(life.submitted_serial(), 0);
}

#[test]
fn accepted_record_releases_payload_once_after_completion() {
    let mut life = Owner::new(7);
    let id = record(&mut life, 42);
    let ids = [id];
    let serial = life.prepare_batch(&ids).unwrap().accept();
    assert!(matches!(life.take_completed(id), Err(Error::InvalidState)));
    assert!(life.observe_completion(serial));
    assert_eq!(life.take_completed(id).unwrap().into_buffers(), 42);
    assert!(matches!(life.take_completed(id), Err(Error::InvalidKey)));
}

#[test]
fn quarantined_batch_ignores_ordinary_completion() {
    let mut life = Owner::new(7);
    let id = record(&mut life, 42);
    let ids = [id];
    life.prepare_batch(&ids).unwrap().quarantine();
    assert_eq!(life.record(id).unwrap().state(), RecordState::Uncertain);
    assert!(!life.observe_completion(1));
    assert!(matches!(life.take_completed(id), Err(Error::InvalidState)));
}

#[test]
fn cancelled_build_stays_invalid_after_record_slot_reuse() {
    let mut life = Owner::new(7);
    let root = life
        .insert_resource(Data::Blas(resource::Blas::new(None, 99)))
        .unwrap_or_else(|_| panic!("资源插入失败"));
    let id = life
        .insert_record(Record::new(Vec::new(), Vec::new(), vec![root], 42))
        .unwrap_or_else(|_| panic!("构建记录插入失败"));
    life.take_pending(id).unwrap();
    let replacement = record(&mut life, 88);
    assert_eq!(id.key().slot, replacement.key().slot);
    assert_ne!(id.key().generation, replacement.key().generation);
    assert!(matches!(life.active(root), Err(Error::CancelledProducer)));
}

#[test]
fn completing_old_producer_preserves_new_producer() {
    let mut life = Owner::new(7);
    let root = life
        .insert_resource(Data::Blas(resource::Blas::new(None, 99)))
        .unwrap_or_else(|_| panic!("资源插入失败"));
    let build = || Record::new(Vec::new(), Vec::new(), vec![root], 42);
    let old = life.insert_record(build()).unwrap_or_else(|_| panic!());
    life.mark_external(old).unwrap();
    let current = life.insert_record(build()).unwrap_or_else(|_| panic!());
    life.take_external(old).unwrap();
    assert!(
        life.resource(root)
            .unwrap()
            .producer()
            .unwrap()
            .same(current)
    );
}

#[test]
fn collection_keeps_indirect_record_dependencies() {
    let mut life = Owner::new(7);
    let map = life
        .insert_resource(Data::Micromap(17))
        .unwrap_or_else(|_| panic!());
    let blas = life
        .insert_resource(Data::Blas(resource::Blas::new(
            Some(crate::handles::OpacityMicromap { key: map, count: 1 }),
            99,
        )))
        .unwrap_or_else(|_| panic!());
    life.insert_record(Record::new(
        vec![super::Op::Keepalive(crate::handles::Resource(blas))],
        Vec::new(),
        Vec::new(),
        42,
    ))
    .unwrap_or_else(|_| panic!());
    let plan = life.collect_plan(&[]).unwrap();
    assert!(plan.keep[map.slot as usize]);
}

#[test]
fn failed_collection_preserves_existing_retirement() {
    let mut life = Owner::new(7);
    let key = life
        .insert_resource(Data::Buffer(17))
        .unwrap_or_else(|_| panic!());
    let id = life
        .insert_record(Record::new(
            vec![super::Op::Keepalive(crate::handles::Resource(key))],
            Vec::new(),
            Vec::new(),
            42,
        ))
        .unwrap_or_else(|_| panic!());
    life.collect_plan(&[]).unwrap();
    let foreign = crate::handles::Resource(crate::owner::Key {
        owner: 8,
        slot: 0,
        generation: 0,
    });
    assert!(matches!(
        life.collect_plan(&[foreign]),
        Err(crate::graph::PlanError::ForeignOwner)
    ));
    assert_eq!(life.retirements.len(), 1);
    assert_eq!(life.retirements[0].resources, [key]);
    assert_eq!(life.retirements[0].waits, [id]);
    assert_eq!(life.record(id).unwrap().state(), RecordState::Pending);
    assert!(life.retiring[key.slot as usize]);
}
