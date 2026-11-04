//! 资源真实元数据、身份适配和 producer 帧条件的同源入口。
#![allow(dead_code)]
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]
#[path = "../src/arena.rs"]
mod arena;
mod owner {
    pub(crate) use crate::arena::{Arena, Key};
}
#[path = "../src/graph.rs"]
mod graph;
#[path = "../src/handles.rs"]
mod handles;
#[path = "../src/resource.rs"]
mod resource;

use resource::*;
use vstd::prelude::*;

struct Payload;
impl Payloads for Payload {
    type Queries = ();
    type Buffer = ();
    type Image = ();
    type Sampler = ();
    type Set = ();
    type Layout = ();
    type RayProgram = ();
    type ComputeProgram = ();
    type Structure = ();
    type Micromap = ();
    type SharedBuffer = ();
    type SharedTexture = ();
    type SampledTexture = ();
}

verus! {
#[cfg(reject_resource_missing_edge)]
fn reject_resource_missing_edge<P: Payloads>(data: &Data<P>, index: usize)
    requires index < data@.len(), data@[index as int] is Some,
{
    let edge = data.dependency_at(index);
    assert(edge is None);
}

#[cfg(reject_resource_rank_reversal)]
fn reject_resource_rank_reversal<P: Payloads>(data: &Data<P>, index: usize) {
    let parent = data.rank();
    proof {
        data.expected_kind(index as int);
        if let Some(kind) = data.edge_kind_value(index as int) {
            assert(kind.rank() <= parent);
        }
    }
}

#[cfg(reject_producer_changes_dependency)]
fn reject_producer_changes_dependency<P: Payloads>(data: &mut Data<P>, producer: Option<handles::RecordId>, index: usize)
    requires index < old(data)@.len(),
{
    let before = data.dependency_at(index);
    data.set_producer(producer);
    let after = data.dependency_at(index);
    assert(before != after);
}

#[cfg(reject_raw_changes_resource_link)]
fn reject_raw_changes_resource_link<P: Payloads>(data: &mut BindingData<P>, raw: P::Set) {
    let before = data.layout();
    *data.raw_mut() = raw;
    let after = data.layout();
    assert(before != after);
}

#[cfg(reject_producer_changes_payload)]
fn reject_producer_changes_payload<P: Payloads>(data: &mut Data<P>) {
    let ghost before = *data;
    let producer = data.producer();
    data.set_producer(producer);
    proof { before.producer_unchanged(); }
    assert(*data != before);
}
}

fn main() {}
