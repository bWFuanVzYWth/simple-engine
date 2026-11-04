//! 资源链接的数学视图及同源适配证明。

use super::*;

#[cfg(not(verus_only))]
impl<P: Payloads> View for Resources<'_, P> {
    type V = Model;
}
#[cfg(not(verus_only))]
impl<P: Payloads> View for Data<P> {
    type V = EdgeModel;
}

#[cfg(verus_only)]
verus! {
use crate::graph::NodeModel;

impl<P: Payloads> Binding<P> {
    pub(crate) open(super) spec fn dependency_kind_view(&self) -> Option<ResourceKind> {
        match self {
            Self::StorageBuffer(_) | Self::UniformBuffer(_) => Some(ResourceKind::Buffer),
            Self::StorageImage(_) | Self::SampledImage(_) => Some(ResourceKind::Image),
            Self::Sampler(_) => Some(ResourceKind::Sampler),
            Self::Acceleration(_) => Some(ResourceKind::Acceleration), _ => None,
        }
    }
    pub(crate) open(super) spec fn dependency_view(&self) -> Option<Key> {
        match self {
            Self::StorageBuffer(value) | Self::UniformBuffer(value) => Some(value.key),
            Self::StorageImage(value) | Self::SampledImage(value) => Some(value.key),
            Self::Sampler(value) => Some(value.key),
            Self::Acceleration(value) => Some(value.key),
            _ => None,
        }
    }
    pub(crate) open(super) spec fn access_view(&self) -> Option<ResourceAccess> {
        match self {
            Self::StorageBuffer(value) | Self::UniformBuffer(value) => Some(ResourceAccess::Buffer(*value)),
            Self::StorageImage(value) | Self::SampledImage(value) => Some(ResourceAccess::Image(*value)),
            _ => None,
        }
    }
}

impl<P: Payloads> BindingLinks<P> {
    #[verifier::type_invariant]
    pub(super) open(super) spec fn bounded(&self) -> bool { self.resources@.len() < usize::MAX }
}
impl<P: Payloads> BindingData<P> {
    pub(crate) open(super) spec fn layout_value(&self) -> Key { self.links.layout }
    pub(crate) open(super) spec fn entries(&self) -> Seq<Binding<P>> { self.links.resources@ }
}
impl<R> Program<R> {
    pub(crate) open(super) spec fn layout_value(&self) -> Key { self.set_layout }
}
impl Version {
    pub(crate) open(super) spec fn binding_value(&self) -> handles::Bindings { self.bindings }
    pub(crate) open(super) spec fn program_value(&self) -> Key { self.program }
}
impl<R> Blas<R> {
    pub(crate) open(super) spec fn opacity_value(&self) -> Option<handles::OpacityMicromap> { self.opacity }
    pub(crate) open(super) spec fn producer_value(&self) -> Option<RecordId> { self.producer }
}
impl<R> Acceleration<R> {
    pub(crate) open(super) spec fn bottom_values(&self) -> Seq<handles::TriangleBlas> { self.bottoms@ }
    pub(crate) open(super) spec fn producer_value(&self) -> Option<RecordId> { self.producer }
}

pub(crate) open spec fn rank(kind: ResourceKind) -> u8 {
    match kind {
        ResourceKind::Ray | ResourceKind::Compute => 0,
        ResourceKind::Bindings => 1,
        ResourceKind::RayProgram | ResourceKind::ComputeProgram => 2,
        ResourceKind::Layout => 3, ResourceKind::Acceleration => 4,
        ResourceKind::Blas => 5, ResourceKind::Micromap => 6, _ => 7,
    }
}
impl ResourceKind {
    pub(crate) open spec fn rank(self) -> u8 { rank(self) }
}
pub(crate) open spec fn is_build(kind: ResourceKind) -> bool {
    kind == ResourceKind::Blas || kind == ResourceKind::Acceleration
}

impl<P: Payloads> Data<P> {
    pub(crate) open(super) spec fn kind_value(&self) -> ResourceKind {
        match self {
            Self::Timestamps(_) => ResourceKind::Timestamps, Self::Buffer(_) => ResourceKind::Buffer,
            Self::Image(_) => ResourceKind::Image, Self::Sampler(_) => ResourceKind::Sampler,
            Self::Bindings(_) => ResourceKind::Bindings, Self::Layout(_) => ResourceKind::Layout,
            Self::RayProgram(_) => ResourceKind::RayProgram, Self::ComputeProgram(_) => ResourceKind::ComputeProgram,
            Self::Ray(_) => ResourceKind::Ray, Self::Compute(_) => ResourceKind::Compute,
            Self::Blas(_) => ResourceKind::Blas, Self::Acceleration(_) => ResourceKind::Acceleration,
            Self::Micromap(_) => ResourceKind::Micromap,
        }
    }
    pub(crate) open(super) spec fn producer_value(&self) -> Option<RecordId> {
        match self { Self::Blas(data) => data.producer, Self::Acceleration(data) => data.producer, _ => None }
    }
    pub(crate) open(super) spec fn with_producer(&self, producer: Option<RecordId>) -> Self {
        match self {
            Self::Blas(data) => Self::Blas(Blas { opacity: data.opacity, producer, raw: data.raw }),
            Self::Acceleration(data) => Self::Acceleration(Acceleration { bottoms: data.bottoms, producer, raw: data.raw }),
            _ => *self,
        }
    }
    pub(crate) proof fn producer_unchanged(&self)
        ensures self.with_producer(self.producer_value()) == *self,
    {}
    pub(crate) proof fn producer_overwrite(&self, first: Option<RecordId>, second: Option<RecordId>)
        ensures self.with_producer(first).with_producer(second) == self.with_producer(second),
    {}
    pub(crate) proof fn producer_updated(&self, producer: Option<RecordId>)
        ensures
            self.with_producer(producer).kind_value() == self.kind_value(),
            self.with_producer(producer)@ == self@,
            self.with_producer(producer).producer_value() ==
                if is_build(self.kind_value()) { producer } else { self.producer_value() },
    {}
    pub(crate) proof fn timestamps_edges(queries: P::Queries)
        ensures Self::Timestamps(queries)@ == Seq::<Option<Key>>::empty(),
    {}
    pub(crate) proof fn producer_is_build_output(&self)
        requires self.producer_value() is Some,
        ensures self.kind_value() == ResourceKind::Blas || self.kind_value() == ResourceKind::Acceleration,
    {}
    pub(crate) proof fn expected_kind(&self, index: int)
        requires index >= 0,
        ensures
            self.edge_kind_value(index) is Some <==> self.edge_value(index) is Some,
            self.edge_kind_value(index) is Some ==> self.edge_kind_value(index)->Some_0.rank() > rank(self.kind_value()),
    { expected_kind(self, index); }
    pub(crate) open(super) spec fn edge_value(&self, index: int) -> Option<Key> {
        if 0 <= index < self@.len() { self@[index] } else { None }
    }
    pub(crate) open(super) spec fn edge_kind_value(&self, index: int) -> Option<ResourceKind> {
        match self {
            Self::Bindings(data) => if index == 0 { Some(ResourceKind::Layout) }
                else if 0 < index <= data.links.resources@.len() { data.links.resources@[index - 1].dependency_kind_view() } else { None },
            Self::Ray(_) => if index == 0 { Some(ResourceKind::RayProgram) } else if index == 1 { Some(ResourceKind::Bindings) } else { None },
            Self::Compute(_) => if index == 0 { Some(ResourceKind::ComputeProgram) } else if index == 1 { Some(ResourceKind::Bindings) } else { None },
            Self::RayProgram(_) | Self::ComputeProgram(_) => if index == 0 { Some(ResourceKind::Layout) } else { None },
            Self::Blas(data) => if index == 0 && data.opacity is Some { Some(ResourceKind::Micromap) } else { None },
            Self::Acceleration(data) => if 0 <= index < data.bottoms@.len() { Some(ResourceKind::Blas) } else { None },
            _ => None,
        }
    }
}

impl<P: Payloads> View for Data<P> {
    type V = EdgeModel;
    open(super) spec fn view(&self) -> EdgeModel {
        match self {
            Self::Bindings(data) => Seq::new(1, |i: int| Some(data.links.layout)) +
                data.links.resources@.map(|i: int, binding: Binding<P>| binding.dependency_view()),
            Self::Ray(data) | Self::Compute(data) => seq![Some(data.program), Some(data.bindings.key)],
            Self::RayProgram(data) => seq![Some(data.set_layout)],
            Self::ComputeProgram(data) => seq![Some(data.set_layout)],
            Self::Blas(data) => match data.opacity { Some(map) => seq![Some(map.key)], None => Seq::empty() },
            Self::Acceleration(data) => data.bottoms@.map(|i: int, blas: handles::TriangleBlas| Some(blas.key)),
            _ => Seq::empty(),
        }
    }
}

pub(super) proof fn edge_shape<P: Payloads>(data: &Data<P>)
    ensures data@.len() == match data {
        Data::Bindings(value) => 1 + value.links.resources@.len(),
        Data::Ray(_) | Data::Compute(_) => 2,
        Data::RayProgram(_) | Data::ComputeProgram(_) => 1,
        Data::Blas(value) => if value.opacity is Some { 1nat } else { 0nat },
        Data::Acceleration(value) => value.bottoms@.len(), _ => 0,
    },
{
    broadcast use vstd::seq_lib::group_seq_properties;
}

pub(super) proof fn edge_at<P: Payloads>(data: &Data<P>, index: int)
    requires index >= 0,
    ensures data.edge_value(index) == match data {
        Data::Bindings(value) => if index == 0 { Some(value.links.layout) }
            else if index - 1 < value.links.resources@.len() {
                value.links.resources@[index - 1].dependency_view()
            } else { None },
        Data::Ray(value) | Data::Compute(value) => if index == 0 { Some(value.program) }
            else if index == 1 { Some(value.bindings.key) } else { None },
        Data::RayProgram(value) => if index == 0 { Some(value.set_layout) } else { None },
        Data::ComputeProgram(value) => if index == 0 { Some(value.set_layout) } else { None },
        Data::Blas(value) => if index == 0 { match value.opacity { Some(map) => Some(map.key), None => None } } else { None },
        Data::Acceleration(value) => if index < value.bottoms@.len() { Some(value.bottoms@[index].key) } else { None },
        _ => None,
    },
{
    broadcast use vstd::seq_lib::group_seq_properties;
}

pub(super) proof fn expected_kind<P: Payloads>(data: &Data<P>, index: int)
    requires index >= 0,
    ensures
        data.edge_kind_value(index) is Some <==> data.edge_value(index) is Some,
        data.edge_kind_value(index) is Some ==> rank(data.edge_kind_value(index)->Some_0) > rank(data.kind_value()),
{
    edge_at(data, index);
}

impl<'a, P: Payloads> Resources<'a, P> {
    #[verifier::type_invariant]
    pub(super) open(super) spec fn well_formed(&self) -> bool { self.resources.valid() }
    pub(crate) open spec fn model(owner: u64, resources: &Arena<Data<P>>) -> Model {
        Model { owner, nodes: Seq::new(resources.slot_count(), |slot: int| {
            if resources.occupied(slot as usize) {
                let generation = resources.generation_at(slot as usize);
                let key = Key { owner, slot: slot as u32, generation };
                Some(NodeModel { generation, edges: resources.lookup(key)->Some_0@ })
            } else { None }
        }) }
    }
    pub(crate) proof fn same_model(owner: u64, before: &Arena<Data<P>>, after: &Arena<Data<P>>)
        requires before.valid(), after.valid(), before.same_shape(after),
            forall|key: Key| #![auto] before.lookup(key) is Some && after.lookup(key) is Some
                ==> before.lookup(key)->Some_0@ == after.lookup(key)->Some_0@,
        ensures Self::model(owner, before) == Self::model(owner, after),
    {
        before.shape_slots(after);
        before.slot_bound();
        let left = Self::model(owner, before);
        let right = Self::model(owner, after);
        assert(left.nodes =~= right.nodes) by {
            assert forall|slot: int| 0 <= slot < left.nodes.len() implies left.nodes[slot] == right.nodes[slot] by {
                let index = slot as usize;
                assert(index == slot);
                if before.occupied(index) {
                    before.occupied_lookup(index, owner);
                    let key = Key { owner, slot: index as u32, generation: before.generation_at(index) };
                    before.shape_preserves_presence(after, key);
                    assert(before.lookup(key)->Some_0@ == after.lookup(key)->Some_0@);
                }
            }
        }
    }
    pub(crate) proof fn contains(owner: u64, resources: &Arena<Data<P>>, key: Key)
        requires resources.valid(),
        ensures crate::graph::model::has_key(Self::model(owner, resources), key)
            <==> key.owner == owner && resources.lookup(key) is Some,
    {
        let graph = Self::model(owner, resources);
        if key.owner == owner && resources.lookup(key) is Some {
            resources.lookup_bounds(key);
        }
        if crate::graph::model::has_key(graph, key) {
            resources.occupied_lookup(key.slot as usize, owner);
            assert(resources.generation_at(key.slot as usize) == key.generation);
        }
    }
}

impl<'a, P: Payloads> View for Resources<'a, P> {
    type V = Model;
    open(super) spec fn view(&self) -> Model {
        Self::model(self.owner, self.resources)
    }
}
}
