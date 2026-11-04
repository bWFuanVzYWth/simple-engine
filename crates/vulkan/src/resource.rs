//! 生命周期所有者实际保存的资源；链接与 raw payload 各存一份。

use crate::graph::{Adjacency, EdgeModel, Graph, Model};
use crate::handles::{self, RecordId, ResourceAccess};
use crate::owner::{Arena, Key};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(not(verus_only))]
use vstd::view::View;

#[path = "proof/resource.rs"]
mod proof;

/// 平台 payload 只参与所有权移动；链接算法不读取其内容。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) trait Payloads {
    type Queries;
    type Buffer;
    type Image;
    type Sampler;
    type Set;
    type Layout;
    type RayProgram;
    type ComputeProgram;
    type Structure;
    type Micromap;
    type SharedBuffer;
    type SharedTexture;
    type SampledTexture;
}

/// 真实资源类别；同 rank 不表示类型可互换。
#[cfg(verus_only)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum ResourceKind {
    Timestamps,
    Buffer,
    Image,
    Sampler,
    Bindings,
    Layout,
    RayProgram,
    ComputeProgram,
    Ray,
    Compute,
    Blas,
    Acceleration,
    Micromap,
}

/// 唯一保存的 descriptor 输入；平台共享对象不制造 native 依赖。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum Binding<P: Payloads> {
    StorageBuffer(handles::Buffer),
    UniformBuffer(handles::Buffer),
    SharedBuffer(P::SharedBuffer),
    StorageImage(handles::Image),
    SharedTexture(P::SharedTexture),
    SampledImage(handles::Image),
    SampledTexture(P::SampledTexture),
    Sampler(handles::Sampler),
    Acceleration(handles::AccelerationStructure),
    External(handles::ExternalBinding),
}

#[cfg_attr(verus_only, verus_verify)]
impl<P: Payloads> Binding<P> {
    #[inline]
    #[cfg_attr(verus_only, verus_spec(key => ensures key == self.dependency_view()))]
    pub(crate) const fn dependency(&self) -> Option<Key> {
        match self {
            Self::StorageBuffer(value) | Self::UniformBuffer(value) => Some(value.key),
            Self::StorageImage(value) | Self::SampledImage(value) => Some(value.key),
            Self::Sampler(value) => Some(value.key),
            Self::Acceleration(value) => Some(value.key),
            _ => None,
        }
    }

    #[inline]
    #[cfg_attr(verus_only, verus_spec(access => ensures access == self.access_view()))]
    pub(crate) const fn access(&self) -> Option<ResourceAccess> {
        match self {
            Self::StorageBuffer(value) | Self::UniformBuffer(value) => {
                Some(ResourceAccess::Buffer(*value))
            }
            Self::StorageImage(value) | Self::SampledImage(value) => {
                Some(ResourceAccess::Image(*value))
            }
            _ => None,
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct BindingData<P: Payloads> {
    links: BindingLinks<P>,
    raw: P::Set,
}

#[cfg_attr(verus_only, verus_verify)]
struct BindingLinks<P: Payloads> {
    layout: Key,
    resources: Vec<Binding<P>>,
}

#[cfg_attr(verus_only, verus_verify)]
impl<P: Payloads> BindingData<P> {
    #[cfg_attr(verus_only, verus_spec(value =>
        requires resources@.len() < usize::MAX,
        ensures value.layout_value() == layout, value.entries() == resources@,
    ))]
    pub(crate) const fn new(layout: Key, resources: Vec<Binding<P>>, raw: P::Set) -> Self {
        Self {
            links: BindingLinks { layout, resources },
            raw,
        }
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures value == self.layout_value()))]
    pub(crate) const fn layout(&self) -> Key {
        self.links.layout
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures value@ == self.entries()))]
    pub(crate) fn resources(&self) -> &[Binding<P>] {
        &self.links.resources
    }
    #[cfg(not(verus_only))]
    pub(crate) fn resource_capacity(&self) -> usize {
        self.links.resources.capacity()
    }
    #[inline]
    pub(crate) const fn raw(&self) -> &P::Set {
        &self.raw
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures
        final(self).layout_value() == old(self).layout_value(), final(self).entries() == old(self).entries(),
    ))]
    pub(crate) fn raw_mut(&mut self) -> &mut P::Set {
        &mut self.raw
    }
}

#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Program<R> {
    set_layout: Key,
    raw: R,
}

#[cfg_attr(verus_only, verus_verify)]
impl<R> Program<R> {
    #[cfg_attr(verus_only, verus_spec(value => ensures value.layout_value() == set_layout))]
    pub(crate) const fn new(set_layout: Key, raw: R) -> Self {
        Self { set_layout, raw }
    }
    #[inline]
    pub(crate) const fn raw(&self) -> &R {
        &self.raw
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures final(self).layout_value() == old(self).layout_value()))]
    pub(crate) fn raw_mut(&mut self) -> &mut R {
        &mut self.raw
    }
}

#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Version {
    bindings: handles::Bindings,
    program: Key,
}

#[cfg_attr(verus_only, verus_verify)]
impl Version {
    #[cfg_attr(verus_only, verus_spec(value => ensures value.binding_value() == bindings, value.program_value() == program))]
    pub(crate) const fn new(bindings: handles::Bindings, program: Key) -> Self {
        Self { bindings, program }
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures value == self.binding_value()))]
    pub(crate) const fn bindings(&self) -> handles::Bindings {
        self.bindings
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures value == self.program_value()))]
    pub(crate) const fn program(&self) -> Key {
        self.program
    }
}

#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Blas<R> {
    opacity: Option<handles::OpacityMicromap>,
    producer: Option<RecordId>,
    raw: R,
}

#[cfg_attr(verus_only, verus_verify)]
impl<R> Blas<R> {
    #[cfg_attr(verus_only, verus_spec(value => ensures value.opacity_value() == opacity, value.producer_value() is None))]
    pub(crate) const fn new(opacity: Option<handles::OpacityMicromap>, raw: R) -> Self {
        Self {
            opacity,
            producer: None,
            raw,
        }
    }
    #[inline]
    pub(crate) const fn raw(&self) -> &R {
        &self.raw
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures
        final(self).opacity_value() == old(self).opacity_value(), final(self).producer_value() == old(self).producer_value(),
    ))]
    pub(crate) fn raw_mut(&mut self) -> &mut R {
        &mut self.raw
    }
    pub(crate) fn into_raw(self) -> R {
        self.raw
    }
}

#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Acceleration<R> {
    bottoms: Vec<handles::TriangleBlas>,
    producer: Option<RecordId>,
    raw: R,
}

#[cfg_attr(verus_only, verus_verify)]
impl<R> Acceleration<R> {
    #[cfg_attr(verus_only, verus_spec(value => ensures value.bottom_values() == bottoms@, value.producer_value() is None))]
    pub(crate) const fn new(bottoms: Vec<handles::TriangleBlas>, raw: R) -> Self {
        Self {
            bottoms,
            producer: None,
            raw,
        }
    }
    #[cfg(not(verus_only))]
    pub(crate) fn bottom_capacity(&self) -> usize {
        self.bottoms.capacity()
    }
    #[inline]
    pub(crate) const fn raw(&self) -> &R {
        &self.raw
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures
        final(self).bottom_values() == old(self).bottom_values(), final(self).producer_value() == old(self).producer_value(),
    ))]
    pub(crate) fn raw_mut(&mut self) -> &mut R {
        &mut self.raw
    }
    pub(crate) fn into_raw(self) -> R {
        self.raw
    }
}

#[cfg_attr(verus_only, verus_verify)]
pub(crate) enum Data<P: Payloads> {
    Timestamps(P::Queries),
    Buffer(P::Buffer),
    Image(P::Image),
    Sampler(P::Sampler),
    Bindings(BindingData<P>),
    Layout(P::Layout),
    RayProgram(Program<P::RayProgram>),
    ComputeProgram(Program<P::ComputeProgram>),
    Ray(Version),
    Compute(Version),
    Blas(Blas<P::Structure>),
    Acceleration(Acceleration<P::Structure>),
    Micromap(P::Micromap),
}

#[cfg_attr(verus_only, verus_verify)]
impl<P: Payloads> Data<P> {
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures value == proof::rank(self.kind_value())))]
    pub(crate) const fn rank(&self) -> u8 {
        match self {
            Self::Ray(_) | Self::Compute(_) => 0,
            Self::Bindings(_) => 1,
            Self::RayProgram(_) | Self::ComputeProgram(_) => 2,
            Self::Layout(_) => 3,
            Self::Acceleration(_) => 4,
            Self::Blas(_) => 5,
            Self::Micromap(_) => 6,
            _ => 7,
        }
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures value == self.producer_value()))]
    pub(crate) const fn producer(&self) -> Option<RecordId> {
        match self {
            Self::Blas(data) => data.producer,
            Self::Acceleration(data) => data.producer,
            _ => None,
        }
    }
    #[cfg_attr(verus_only, verus_spec(changed => ensures
        *final(self) == old(self).with_producer(producer),
        final(self)@ == old(self)@, final(self).kind_value() == old(self).kind_value(),
        changed == proof::is_build(old(self).kind_value()),
        final(self).producer_value() == if changed { producer } else { old(self).producer_value() },
    ))]
    pub(super) fn set_producer(&mut self, producer: Option<RecordId>) -> bool {
        match self {
            Self::Blas(data) => {
                data.producer = producer;
                true
            }
            Self::Acceleration(data) => {
                data.producer = producer;
                true
            }
            _ => false,
        }
    }
    #[cfg_attr(verus_only, verus_spec(ensures
        *final(self) == if old(self).producer_value() == Some(id) {
            old(self).with_producer(None)
        } else { *old(self) },
        final(self)@ == old(self)@, final(self).kind_value() == old(self).kind_value(),
        final(self).producer_value() == if old(self).producer_value() == Some(id) { None } else { old(self).producer_value() },
    ))]
    pub(super) fn finish_producer(&mut self, id: RecordId) {
        let matches = match self.producer() {
            Some(producer) => producer.same(id),
            None => false,
        };
        if matches {
            self.set_producer(None);
        }
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures value == self@.len()))]
    pub(crate) fn dependency_count(&self) -> usize {
        #[cfg(verus_only)]
        proof! { if let Self::Bindings(data) = self { use_type_invariant(&data.links); } proof::edge_shape(self); }
        match self {
            Self::Bindings(data) => 1 + data.links.resources.len(),
            Self::Ray(_) | Self::Compute(_) => 2,
            Self::RayProgram(_) | Self::ComputeProgram(_) => 1,
            Self::Blas(data) => {
                if data.opacity.is_some() {
                    1
                } else {
                    0
                }
            }
            Self::Acceleration(data) => data.bottoms.len(),
            _ => 0,
        }
    }
    #[inline]
    #[cfg_attr(verus_only, verus_spec(value => ensures value == self.edge_value(index as int),
        index < self@.len() ==> value == self@[index as int],
        index >= self@.len() ==> value is None,
    ))]
    pub(crate) fn dependency_at(&self, index: usize) -> Option<Key> {
        #[cfg(verus_only)]
        proof! { proof::edge_at(self, index as int); }
        match self {
            Self::Bindings(data) => match index {
                0 => Some(data.links.layout),
                _ => match data.links.resources.get(index - 1) {
                    Some(binding) => binding.dependency(),
                    None => None,
                },
            },
            Self::Ray(data) | Self::Compute(data) => match index {
                0 => Some(data.program),
                1 => Some(data.bindings.key),
                _ => None,
            },
            Self::RayProgram(data) => {
                if index == 0 {
                    Some(data.set_layout)
                } else {
                    None
                }
            }
            Self::ComputeProgram(data) => {
                if index == 0 {
                    Some(data.set_layout)
                } else {
                    None
                }
            }
            Self::Blas(data) => {
                if index == 0 {
                    Some(data.opacity?.key)
                } else {
                    None
                }
            }
            Self::Acceleration(data) => Some(data.bottoms.get(index)?.key),
            _ => None,
        }
    }
    pub(crate) fn dependencies(&self, out: &mut Vec<Key>) {
        for index in 0..self.dependency_count() {
            if let Some(key) = self.dependency_at(index) {
                out.push(key);
            }
        }
    }
}

/// Graph 直接借用实际槽表与链接；不创建图副本。
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Resources<'a, P: Payloads> {
    owner: u64,
    resources: &'a Arena<Data<P>>,
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a, P: Payloads> Resources<'a, P> {
    #[cfg_attr(verus_only, verus_spec(value =>
        requires resources.valid(),
        ensures value@ == Self::model(owner, resources),
    ))]
    pub(crate) const fn new(owner: u64, resources: &'a Arena<Data<P>>) -> Self {
        Self { owner, resources }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a, P: Payloads> Graph for Resources<'a, P> {
    type Node = &'a Data<P>;
    fn owner(&self) -> u64 {
        self.owner
    }
    fn len(&self) -> usize {
        #[cfg(verus_only)]
        proof! { use_type_invariant(self); self.resources.slot_bound(); }
        self.resources.len()
    }
    fn node(&self, slot: usize) -> Option<(Key, Self::Node)> {
        #[cfg(verus_only)]
        proof! { use_type_invariant(self); }
        self.resources.at(slot, self.owner)
    }
    fn present_key(&self, slot: usize) -> Key {
        #[cfg(verus_only)]
        proof! { use_type_invariant(self); }
        self.resources.present_key(slot, self.owner)
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<P: Payloads> Adjacency for &Data<P> {
    #[inline]
    fn len(&self) -> usize {
        self.dependency_count()
    }
    #[inline]
    fn edge(&self, index: usize) -> Option<Key> {
        self.dependency_at(index)
    }
}
