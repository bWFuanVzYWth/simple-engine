//! 原生叶修改的擦除语义；每个生产批内核分别连接其逐槽覆盖规范。
use super::*;
use crate::write::patch::proof as patch;
use crate::{node::proof as node, state};
use vstd::prelude::*;

verus! {
pub(crate) struct BuiltSource {
    pub pools:state::Pools,
    pub roots:Seq<Child>,
    pub lod:u8,
}
pub(crate) enum Payload {
    Uniform(u32),
    Dense(Seq<u32>),
    Built(BuiltSource, int),
    Patch(spec_fn(Seq<u32>) -> Seq<u32>),
}
pub(crate) struct EditModel {
    pub path: u64,
    pub lod: u8,
    pub payload: Payload,
}

pub(crate) open(in super::super) spec fn edit_view<P: LeafPatch>(edit: Edit<P>) -> EditModel {
    EditModel {
        path: crate::morton::proof::value(edit.path), lod: edit.lod,
        payload: match edit.data {
            EditData::Uniform(value) => Payload::Uniform(value),
            EditData::Dense(values) => Payload::Dense(values@),
            EditData::Built { tree, part } => super::super::built::proof::part_view(tree, part as int),
            EditData::Patch(patch) => Payload::Patch(|before: Seq<u32>| patch.effect(before)),
        },
    }
}
pub(crate) open(in super::super) spec fn edit_ready<P:LeafPatch>(edit:Edit<P>)->bool {
    edit.lod<=10 && match edit.data {
        EditData::Uniform(_)=>true,
        EditData::Dense(values)=>values@.len()==state::span(edit.lod as nat),
        EditData::Built{tree,part}=>super::super::import::proof::source_valid(tree)
            && edit.lod==tree.lod->Some_0/2 && part<super::super::import::proof::parts(tree.lod->Some_0),
        EditData::Patch(patch)=>edit.lod==1 && patch.valid(),
    }
}

pub(crate) open spec fn same_edit(actual: EditModel, expected: EditModel) -> bool {
    actual.path == expected.path && actual.lod == expected.lod
        && match (actual.payload, expected.payload) {
            (Payload::Uniform(a), Payload::Uniform(b)) => a == b,
            (Payload::Dense(a), Payload::Dense(b)) => a == b,
            (Payload::Built(a, part_a), Payload::Built(b, part_b)) => a == b && part_a == part_b,
            (Payload::Patch(a), Payload::Patch(b)) => forall|before: Seq<u32>| before.len() == 64 ==> a(before) == b(before),
            _ => false,
        }
}

pub(crate) open spec fn scope(edit:EditModel)->int {state::span(edit.lod as nat) as int}
pub(crate) open spec fn base(edit:EditModel)->int {(edit.path as int/scope(edit))*scope(edit)}
pub(crate) open spec fn end(edit:EditModel)->int {base(edit)+scope(edit)}
pub(crate) open spec fn covers(edit:EditModel,path:int)->bool {base(edit)<=path<end(edit)}
pub(crate) open spec fn edit_shape(edit:EditModel)->bool {
    edit.lod<=10 && edit.path<0x1000_0000_0000_0000u64
        && match edit.payload {
            Payload::Uniform(_)=>true,
            Payload::Dense(values)=>values.len()==state::span(edit.lod as nat),
            Payload::Built(source,part)=>2<=source.lod<=20 && source.lod/2==edit.lod && source.roots.len()==8
                && 0<=part<if source.lod%2==0{1int}else{8int},
            Payload::Patch(_)=>edit.lod==1,
        }
}
pub(crate) open spec fn built_continuation(previous:EditModel,current:EditModel)->bool {
    match current.payload {
        Payload::Built(source,part)=>part==0 || match previous.payload {
            Payload::Built(old_source,old_part)=>old_source==source && old_part+1==part && previous.lod==current.lod,
            _=>false,
        },
        _=>true,
    }
}
pub(crate) open spec fn stream_ordered(edits:Seq<EditModel>)->bool {
    (forall|i:int|0<=i<edits.len() ==> #[trigger] edit_shape(edits[i]))
        && (forall|i:int|0<=i && i+1<edits.len() ==> end(#[trigger] edits[i])<=base(edits[i+1]))
        && (edits.len()>0 ==> match edits[0].payload{Payload::Built(_,part)=>part==0,_=>true})
        && (forall|i:int|0<i<edits.len() ==> #[trigger] built_continuation(edits[i-1],edits[i]))
}
pub(crate) open spec fn payload_value(edit:EditModel,initial:spec_fn(int)->u32,path:int)->u32 {
    let offset=path-base(edit);
    match edit.payload {
        Payload::Uniform(value)=>value,
        Payload::Dense(values)=>if 0<=offset<values.len(){values[offset]}else{0},
        Payload::Built(source,part)=>if 0<=part<source.roots.len(){state::eval(source.pools,source.roots[part],edit.lod as nat,offset)}else{0},
        Payload::Patch(patch)=>{
            let values=patch(Seq::new(64,|i:int|initial(base(edit)+i)));
            if 0<=offset<values.len(){values[offset]}else{0}
        },
    }
}
pub(crate) open spec fn stream_effect(edits:Seq<EditModel>,initial:spec_fn(int)->u32,path:int,count:nat)->u32
    recommends count<=edits.len(),
    decreases count,
{
    if count==0{initial(path)}else{
        let edit=edits[(count-1) as int];
        if covers(edit,path){payload_value(edit,initial,path)}else{stream_effect(edits,initial,path,(count-1) as nat)}
    }
}

pub(crate) trait PatchModel {
    spec fn valid(self) -> bool;
    spec fn effect(self, before: Seq<u32>) -> Seq<u32>;
}
pub(crate) trait StreamModel {
    spec fn stream_valid(&self) -> bool;
    spec fn remaining(&self) -> Seq<EditModel>;
}
pub(in crate::write) open(in crate::write) spec fn replacement_valid<P:LeafPatch>(replacement:Replacement<P>)->bool {
    match replacement {Replacement::Value(_)=>true,Replacement::Patch(patch)=>patch.valid()}
}
pub(in crate::write) open(in crate::write) spec fn replacement_kept<P:LeafPatch>(replacement:Replacement<P>,child:Child)->bool {
    match child {Child::Ptr(_)=>false,Child::Leaf(value)=>match replacement {
        Replacement::Value(other)=>other==child,
        Replacement::Patch(patch)=>patch.effect(Seq::new(64,|i:int|value))==Seq::new(64,|i:int|value),
    }}
}
pub(in crate::write) open(in crate::write) spec fn resolved<P:LeafPatch>(before:state::Pools,after:state::Pools,replacement:Replacement<P>,original:Child,result:Result<Child,Error>)->bool {
    match replacement {
        Replacement::Value(child)=>result==Ok(child) && before==after,
        Replacement::Patch(patch)=>result is Ok ==> patch.effect(leaf_values(before,original)).len()==64
            && state::child_valid(after,result->Ok_0,1)
            && super::super::commit::proof::image(after[0],result->Ok_0)
                ==Seq::new(64,|i:int|Child::Leaf(patch.effect(leaf_values(before,original))[i])),
    }
}

pub(crate) open spec fn leaf_values(pools:state::Pools,child:Child)->Seq<u32> {
    match child {
        Child::Leaf(value)=>Seq::new(64,|i:int|value),
        Child::Ptr(index)=>{
            let k=node::idx(index);
            if pools.len()>0 && state::occupied(pools[0],k) {
                Seq::new(64,|i:int|match pools[0].nodes[k]->Some_0[i]{Child::Leaf(value)=>value,Child::Ptr(_)=>0})
            }else{Seq::new(64,|i:int|0)}
        },
    }
}
pub(crate) proof fn leaf_image(pools:state::Pools,child:Child)
    requires state::valid_pools(pools),state::child_valid(pools,child,1),
    ensures super::super::commit::proof::image(pools[0],child)==Seq::new(64,|i:int|Child::Leaf(leaf_values(pools,child)[i])),
{
    match child {
        Child::Leaf(_)=>{},
        Child::Ptr(index)=>{
            let row=pools[0].nodes[node::idx(index)]->Some_0;
            assert(state::occupied(pools[0],node::idx(index)));
            assert forall|i:int|0<=i<64 implies #[trigger] row[i] is Leaf by {
                assert(state::child_valid(pools,row[i],0));
            }
            assert(row =~= Seq::new(64,|i:int|Child::Leaf(leaf_values(pools,child)[i])));
        },
    }
}
pub(crate) proof fn decoded_leaf(pools:state::Pools,view:crate::node::NodeRef,index:crate::NodeIdx)
    requires state::valid_pools(pools),state::live(pools[0],node::idx(index)),
        node::ref_children(view)==pools[0].nodes[node::idx(index)]->Some_0,
    ensures node::values(*view.values)==leaf_values(pools,Child::Ptr(index)),
{
    let row=node::ref_children(view);
    assert(state::occupied(pools[0],node::idx(index)));
    assert forall|i:int|0<=i<64 implies #[trigger] node::values(*view.values)[i]==leaf_values(pools,Child::Ptr(index))[i] by {
        assert(state::child_valid(pools,row[i],0));
        assert(row[i] is Leaf);
    }
    assert(node::values(*view.values) =~= leaf_values(pools,Child::Ptr(index)));
}

impl PatchModel for &[SetCell] {
    open(crate) spec fn valid(self) -> bool { true }
    open(crate) spec fn effect(self, before: Seq<u32>) -> Seq<u32> {
        patch::apply(before, self@, self@.len())
    }
}

impl PatchModel for &[DenseRegion<'_>] {
    open(crate) spec fn valid(self) -> bool { patch::leaf_regions(self@) }
    open(crate) spec fn effect(self, before: Seq<u32>) -> Seq<u32> {
        patch::dense_apply(before, self@, self@.len())
    }
}

impl PatchModel for Infallible {
    open(crate) spec fn valid(self) -> bool { false }
    open(crate) spec fn effect(self, before: Seq<u32>) -> Seq<u32> { before }
}
}
