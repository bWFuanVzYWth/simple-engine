//! 局部树叶片的直接覆盖；Ptr 导入另由分层映射连接。
use super::super::dense_chunks::proof as dense;
use super::super::ordered::proof::{BuiltSource, EditModel, PatchModel, Payload, StreamModel};
use super::super::proof::BatchModel;
use super::*;
use vstd::arithmetic::power2::*;
use vstd::prelude::*;
#[path = "write_built_stream.rs"]
pub(crate) mod stream_proof;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;
pub open(super) spec fn input_valid(lod:u8,len:usize)->bool {
    lod<=20 && super::super::validate::proof::dense_count(lod)==Some(len)
}
pub(super) proof fn empty_graph(pools:&[Pool;10])
    requires crate::state::well_formed(pools),
        forall|l:int|0<=l<10 ==> #[trigger] pools@[l].state().nodes.len()==0,
    ensures crate::state::all_occupied_live(crate::state::pools_view(pools)),
        crate::state::balanced(crate::state::pools_view(pools),Seq::empty()),
        forall|l:int|0<=l<10 ==> #[trigger] crate::state::pools_view(pools)[l].free.len()==0,
{
    let states=crate::state::pools_view(pools);
    assert forall|l:int|0<=l<10 implies #[trigger] states[l].nodes.len()==0 && states[l].free.len()==0 by {
        assert(pools@[l].state().nodes.len()==0);pools@[l].valid_model();
        if states[l].free.len()>0 {assert(0<=crate::node::proof::idx(states[l].free[0])<states[l].nodes.len());}
    }
    assert forall|l:int,i:int|0<=l<10 implies !crate::state::occupied(states[l],i) by {assert(states[l].nodes.len()==0);}
}
pub(super) proof fn no_free(before:crate::state::Pools,after:crate::state::Pools)
    requires crate::state::build_step(before,after),
        forall|l:int|0<=l<10 ==> #[trigger] before[l].free.len()==0,
    ensures forall|l:int|0<=l<10 ==> #[trigger] after[l].free.len()==0,
{
    assert forall|l:int|0<=l<10 implies #[trigger] after[l].free.len()==0 by {
        assert(after[l].free.is_prefix_of(before[l].free));
    }
}
pub(super) proof fn parts_phase(before:crate::state::Pools,after:crate::state::Pools,roots:Seq<Child>,values:Seq<u32>,height:nat,done:nat,count:nat)
    requires crate::state::valid_pools(before),crate::state::valid_pools(after),crate::state::frozen(before,after),height<=10,
        built_parts(before,roots,values,height,done,count),
    ensures built_parts(after,roots,values,height,done,count),
{
    assert forall|part:int|0<=part<done implies #[trigger] crate::state::child_valid(after,roots[part],height)
        && dense::content(after,roots[part],height)==values.subrange((part*count) as int,((part+1)*count) as int) by {
        assert(crate::state::child_valid(before,roots[part],height));
        assert(dense::content(before,roots[part],height)==values.subrange((part*count) as int,((part+1)*count) as int));
        crate::canonical::frozen_value(before,after,roots[part],height,0);
        assert forall|path:int|0<=path<crate::state::span(height) implies
            #[trigger] dense::content(before,roots[part],height)[path]==dense::content(after,roots[part],height)[path] by {
            crate::canonical::frozen_value(before,after,roots[part],height,path);
        }
        assert(dense::content(before,roots[part],height) =~= dense::content(after,roots[part],height));
    }
}
impl LocalBuilder {
    pub open(super) spec fn empty_builder(&self)->bool {self.frames@.len()==0}
}
impl LocalTree {
    pub open(in super::super) spec fn built_lod(&self)->Option<u8>{self.lod}
    pub open(super) spec fn built_from(&self,lod:u8,values:Seq<u32>)->bool {
        self.lod==Some(lod) && lod<=20 && crate::state::well_formed(&self.pools)
            && crate::state::all_occupied_live(crate::state::pools_view(&self.pools))
            && crate::state::balanced(crate::state::pools_view(&self.pools),Seq::empty())
            && (forall|l:int|0<=l<10 ==> #[trigger] crate::state::pools_view(&self.pools)[l].free.len()==0
                && !crate::state::pools_view(&self.pools)[l].constructing)
            && built_parts(crate::state::pools_view(&self.pools),self.roots@,values,(lod/2) as nat,root_count(lod),crate::state::span((lod/2) as nat))
            && values.len()==root_count(lod)*crate::state::span((lod/2) as nat)
    }
}
pub(super) open spec fn region_end(region:BuiltRegion)->int {
    crate::morton::proof::value(region.path) as int + super::super::validate::proof::size(lod(region.tree)) as int
}
pub(super) open spec fn batch_at(regions:Seq<BuiltRegion>,index:nat)->bool {
    ready(regions[index as int]) && region_end(regions[index as int])<=u64::MAX
        && (index==0 ||crate::morton::proof::value(regions[index as int].path) as int>=region_end(regions[(index-1) as int]))
}
pub open(super) spec fn batch_prefix(regions:Seq<BuiltRegion>,limit:nat)->bool {
    limit<=regions.len() && forall|i:nat|i<limit ==> #[trigger] batch_at(regions,i)
}
pub(crate) open(super) spec fn tree_value(tree:&LocalTree,path:int)->u32 {
    let h=lod(tree)/2;
    let width=crate::state::span(h as nat) as int;
    if lod(tree)<=20 && width>0 && 0<=path && 0<=path/width<root_count(lod(tree)) {
        crate::state::eval(crate::state::pools_view(&tree.pools),tree.roots@[path/width],h as nat,path%width)
    }else{0}
}
pub open(super) spec fn batch_effect(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,path:int,limit:nat)->u32
    decreases limit,
{
    if limit==0{initial(path)}else{
        let region=regions[(limit-1) as int];
        let base=crate::morton::proof::value(region.path) as int;
        if base<=path<region_end(region){tree_value(region.tree,path-base)}
        else{batch_effect(regions,initial,path,(limit-1) as nat)}
    }
}
impl BatchModel for BuiltBatch<'_,'_> {
    open(super) spec fn accepted(&self)->bool {batch_prefix(self.0@,self.0@.len())}
    open(super) spec fn effect(&self,initial:spec_fn(int)->u32,path:int)->u32 {batch_effect(self.0@,initial,path,self.0@.len())}
}
pub open(in crate::write) spec fn healthy(regions:Seq<BuiltRegion>)->bool {
    forall|i:int|0<=i<regions.len() ==> #[trigger] regions[i].tree.valid_local()
}
impl HashDag {
    pub open(in crate::write) spec fn set_built_post(&self,before:&HashDag,root:RootId,regions:&[BuiltRegion],result:Result<(),Error>)->bool {
        super::super::proof::input_written(before,self,root,batch_prefix(regions@,regions@.len()),
            |initial:spec_fn(int)->u32,path:int|batch_effect(regions@,initial,path,regions@.len()),result)
    }
}
pub(super) proof fn batch_failure(regions:Seq<BuiltRegion>,index:nat)
    requires index<regions.len(),!batch_at(regions,index),
    ensures !batch_prefix(regions,regions.len()),
{}
pub(super) proof fn batch_validated(regions:Seq<BuiltRegion>,index:nat)
    requires index<regions.len(),batch_prefix(regions,index),batch_at(regions,index),
    ensures batch_prefix(regions,index+1),
{}
impl<'a,'v> BuiltBatch<'a,'v> {
    #[verifier::type_invariant]
    pub(super) open spec fn healthy_sources(self)->bool {healthy(self.0@)}
}
pub(super) open spec fn root_count(lod:u8)->nat {if lod%2==0{1}else{8}}
pub(super) proof fn input_parts(lod:u8,len:usize)
    requires lod<=20,super::super::validate::proof::dense_count(lod)==Some(len),
    ensures 0<root_count(lod)<=8,
        len==root_count(lod)*crate::state::span((lod/2) as nat),
        (len as nat)/root_count(lod)==crate::state::span((lod/2) as nat),
{
    let bits=(lod as u32*3) as usize;
    let height=(lod/2) as u8;
    vstd::bits::lemma_usize_pow2_no_overflow(bits as nat);
    vstd::bits::lemma_usize_shl_is_mul(1,bits);
    super::super::frontier::proof::span_bits(height);
    vstd::bits::lemma_u64_pow2_no_overflow((height as nat*6) as nat);
    vstd::bits::lemma_u64_shl_is_mul(1,(height as nat*6) as u64);
    lemma2_to64();
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(lod as int,2);
    if lod%2==0 {lemma_pow2_adds((height as nat*6) as nat,0);}
    else {lemma_pow2_adds((height as nat*6) as nat,3);}
    assert(len==root_count(lod)*crate::state::span((lod/2) as nat));
    vstd::arithmetic::div_mod::lemma_div_by_multiple(crate::state::span((lod/2) as nat) as int,root_count(lod) as int);
}
pub(super) open spec fn built_parts(pools:crate::state::Pools,roots:Seq<Child>,values:Seq<u32>,height:nat,done:nat,count:nat)->bool {
    roots.len()==8 && done<=8 && count==crate::state::span(height)
        && forall|part:int|0<=part<done ==> #[trigger] crate::state::child_valid(pools,roots[part],height)
            && dense::content(pools,roots[part],height)==values.subrange((part*count) as int,((part+1)*count) as int)
}
pub(super) proof fn parts_frozen(before:crate::state::Pools,after:crate::state::Pools,roots:Seq<Child>,values:Seq<u32>,height:nat,done:nat,count:nat)
    requires crate::state::build_step(before,after),height<=10,built_parts(before,roots,values,height,done,count),
    ensures built_parts(after,roots,values,height,done,count),
{
    assert forall|part:int|0<=part<done implies #[trigger] crate::state::child_valid(after,roots[part],height)
        && dense::content(after,roots[part],height)==values.subrange((part*count) as int,((part+1)*count) as int) by {
        assert(crate::state::child_valid(before,roots[part],height));
        assert(dense::content(before,roots[part],height)==values.subrange((part*count) as int,((part+1)*count) as int));
        dense::content_frozen(before,after,roots[part],height);
    }
}
pub(super) proof fn part_added(pools:crate::state::Pools,before:Seq<Child>,after:Seq<Child>,values:Seq<u32>,height:nat,done:nat,count:nat,child:Child)
    requires built_parts(pools,before,values,height,done,count),done<8,after==before.update(done as int,child),
        crate::state::child_valid(pools,child,height),dense::content(pools,child,height)==values.subrange((done*count) as int,((done+1)*count) as int),
    ensures built_parts(pools,after,values,height,done+1,count),
{
    assert forall|part:int|0<=part<done+1 implies #[trigger] crate::state::child_valid(pools,after[part],height)
        && dense::content(pools,after[part],height)==values.subrange((part*count) as int,((part+1)*count) as int) by {
        if part!=done {assert(after[part]==before[part]);assert(crate::state::child_valid(pools,before[part],height));}
    }
}
pub(super) proof fn part_bounds(len:nat,parts:nat,count:nat,part:nat)
    requires len==parts*count,part<parts,
    ensures part*count+count<=len,part*count+count==(part+1)*count,
{
    assert(part*count+count<=parts*count) by(nonlinear_arith) requires part<parts;
    assert(part*count+count==(part+1)*count) by(nonlinear_arith);
}
impl LocalTree {
    pub open(super) spec fn empty_local(&self)->bool {
        self.lod is None && crate::state::well_formed(&self.pools)
            && (forall|i:int|0<=i<10 ==> #[trigger] self.pools@[i].state().nodes.len()==0)
            && (forall|i:int|0<=i<8 ==> #[trigger] self.roots@[i]==Child::Leaf(0))
    }
}
pub(super) proof fn local_cleared(tree:&LocalTree)
    requires tree.lod is None,
        forall|i:int|0<=i<8 ==> #[trigger] tree.roots@[i]==Child::Leaf(0),
        forall|i:int|0<=i<10 ==> #[trigger] tree.pools@[i].valid() && tree.pools@[i].state().nodes.len()==0,
    ensures tree.empty_local(),
{
    let pools=crate::state::pools_view(&tree.pools);
    assert(pools.len()==10);
    assert forall|level:int|0<=level<10 implies crate::state::valid_pool(#[trigger] crate::state::pools_view(&tree.pools)[level]) by {
        tree.pools@[level].valid_model();
    }
    assert forall|level:int|0<=level<10 implies #[trigger] pools[level].nodes.len()==0 by {
        assert(tree.pools@[level].valid());
        assert(tree.pools@[level].state().nodes.len()==0);
    }
    assert forall|level:int,index:int|0<=level<10 implies !crate::state::occupied(pools[level],index) by {
        assert(pools[level].nodes.len()==0);
    }
    assert(crate::state::valid_pools(pools));
    assert(crate::state::well_formed(&tree.pools));
    assert forall|i:int|0<=i<10 implies #[trigger] tree.pools@[i].state().nodes.len()==0 by {
        assert(tree.pools@[i].valid());
    }
}
impl ImportScratch {
    pub(crate) open(in super::super) spec fn empty(&self)->bool {
        (forall|i:int|0<=i<10 ==> #[trigger] self.maps@[i]@.len()==0)
            && (forall|i:int|0<=i<8 ==> #[trigger] self.roots@[i]==Child::Leaf(0))
    }
}
pub(crate) open(in super::super) spec fn source(tree:&LocalTree)->BuiltSource {
    BuiltSource{pools:crate::state::pools_view(&tree.pools),roots:tree.roots@,lod:lod(tree)}
}
pub(crate) open(in super::super) spec fn part_view(tree: &LocalTree, part: int) -> Payload {
    Payload::Built(source(tree),part)
}
pub(super) open spec fn small(tree:&LocalTree)->bool {
    tree.lod == Some(0) || tree.lod == Some(1)
}
pub(crate) open(in super::super) spec fn lod(tree:&LocalTree)->u8 { match tree.lod { Some(lod)=>lod, None=>255 } }
pub(super) open spec fn ready(region:BuiltRegion)->bool {
    crate::morton::proof::value(region.path)<0x1000_0000_0000_0000u64
        && region.tree.lod is Some && lod(region.tree)<=20
        && crate::morton::proof::value(region.path) & ((super::super::validate::proof::size(lod(region.tree))-1) as u64)==0
}
pub(super) open spec fn group(regions:Seq<BuiltRegion>,limit:nat)->bool {
    0<limit<=regions.len() && if small(regions[0].tree) {
        (forall|i:int| 0<=i<limit ==> small(#[trigger] regions[i].tree)
            && crate::morton::proof::value(regions[i].path)>>6 == crate::morton::proof::value(regions[0].path)>>6)
            && (limit==regions.len() || !small(regions[limit as int].tree)
                || crate::morton::proof::value(regions[limit as int].path)>>6 != crate::morton::proof::value(regions[0].path)>>6)
    } else {limit==1}
}
pub(super) open spec fn group_end(regions:Seq<BuiltRegion>)->nat {choose|limit:nat|group(regions,limit)}
pub(super) proof fn group_found(regions:Seq<BuiltRegion>,limit:nat)
    requires group(regions,limit),
    ensures group_end(regions)==limit,
{
    assert(exists|candidate:nat|group(regions,candidate));
    let selected=group_end(regions);
    assert(group(regions,selected));
    if small(regions[0].tree) {
        if selected<limit {assert(small(regions[selected as int].tree));
            assert(crate::morton::proof::value(regions[selected as int].path)>>6 == crate::morton::proof::value(regions[0].path)>>6);}
        else if limit<selected {assert(small(regions[limit as int].tree));
            assert(crate::morton::proof::value(regions[limit as int].path)>>6 == crate::morton::proof::value(regions[0].path)>>6);}
    }
}
pub(super) open spec fn stream(regions:Seq<BuiltRegion>,part:nat)->Seq<EditModel>
    decreases (regions.len()*8-part) as nat,
{
    if regions.len()==0 {Seq::empty()} else {
        let region=regions[0]; let limit=group_end(regions); let count=if small(region.tree){1nat}else{super::super::dense_chunks::proof::parts(lod(region.tree))};
        if 0<limit<=regions.len() && lod(region.tree)<=20 && part<count {
            let group=regions.subrange(0,limit as int);
            seq![EditModel {path:super::super::dense_chunks::proof::path(crate::morton::proof::value(region.path),lod(region.tree),part as u64),
                lod:if small(region.tree){1}else{lod(region.tree)/2},
                payload:if small(region.tree){Payload::Patch(|before:Seq<u32>|apply(before,group,group.len()))}
                    else{part_view(region.tree,part as int)}}]
                +if part+1==count{stream(regions.subrange(limit as int,regions.len() as int),0)}else{stream(regions,part+1)}
        }else{Seq::empty()}
    }
}
impl BuiltEdits<'_,'_> {
    pub(super) open spec fn valid(&self)->bool {
        (forall|i:int| 0<=i<self.pending@.len() ==> #[trigger] ready(self.pending@[i])
            && super::super::import::proof::source_valid(self.pending@[i].tree))
            && if self.pending@.len()==0 || small(self.pending@[0].tree) {self.part==0}
                else{self.part<super::super::dense_chunks::proof::parts(lod(self.pending@[0].tree))}
    }
    pub(super) open spec fn stream(&self)->Seq<EditModel>{stream(self.pending@,self.part as nat)}
}
pub(super) proof fn leaf_group_ready(regions:Seq<BuiltRegion>,limit:nat)
    requires group(regions,limit),small(regions[0].tree),
        forall|i:int|0<=i<regions.len() ==> #[trigger] ready(regions[i])
            && super::super::import::proof::source_valid(regions[i].tree),
    ensures leaf_regions(regions.take(limit as int)),
{
    assert forall|i:int|0<=i<limit implies #[trigger] leaf_region(regions.take(limit as int)[i]) by {
        let region=regions[i];
        let code=crate::morton::proof::value(region.path);
        assert(ready(regions[i]));
        assert(super::super::import::proof::source_valid(region.tree));
        assert(small(region.tree));
        if region.tree.lod==Some(1) {
            assert(lod(region.tree)==1);
            assert((1u64<<3u32)==8u64) by(bit_vector);
            assert(super::super::validate::proof::size(1)==8);
            assert(code & 7u64 == 0);
            assert(code%64u64+8u64<=64u64) by(bit_vector) requires code&7u64==0;
        }
        assert forall|part:int|0<=part<count(region.tree) implies #[trigger] region.tree.roots@[part] is Leaf by {
            assert(part<super::super::import::proof::parts(region.tree.lod->Some_0));
            assert((region.tree.lod->Some_0/2) as nat==0);
            assert(crate::state::child_valid(crate::state::pools_view(&region.tree.pools),region.tree.roots@[part],0));
        }
    }
}
pub(super) proof fn ready_suffix(regions:Seq<BuiltRegion>,limit:nat)
    requires limit<=regions.len(),
        forall|i:int|0<=i<regions.len() ==> #[trigger] ready(regions[i]) && super::super::import::proof::source_valid(regions[i].tree),
    ensures forall|i:int|0<=i<regions.subrange(limit as int,regions.len() as int).len() ==>
        #[trigger] ready(regions.subrange(limit as int,regions.len() as int)[i]),
        forall|i:int|0<=i<regions.subrange(limit as int,regions.len() as int).len() ==>
            #[trigger] super::super::import::proof::source_valid(regions.subrange(limit as int,regions.len() as int)[i].tree),
{
    let rest=regions.subrange(limit as int,regions.len() as int);
    assert forall|i:int|0<=i<rest.len() implies #[trigger] ready(rest[i]) by {
        assert(ready(regions[limit+i]));
    }
    assert forall|i:int|0<=i<rest.len() implies #[trigger] super::super::import::proof::source_valid(rest[i].tree) by {
        assert(ready(regions[limit+i]));
    }
}
impl StreamModel for BuiltEdits<'_,'_> {
    open(super) spec fn stream_valid(&self)->bool {self.valid()}
    open(super) spec fn remaining(&self)->Seq<EditModel> {self.stream()}
}
pub(crate) open(super) spec fn count(tree: &LocalTree) -> nat {
    if tree.lod == Some(0) { 1 } else { 8 }
}
pub(crate) open(super) spec fn start(region: BuiltRegion) -> int {
    (crate::morton::proof::value(region.path) % 64) as int
}
pub(crate) open(super) spec fn leaf_region(region: BuiltRegion) -> bool {
    (region.tree.lod == Some(0) || region.tree.lod == Some(1))
        && start(region) + count(region.tree) <= 64
        && forall|i:int| 0 <= i < count(region.tree) ==> #[trigger] region.tree.roots@[i] is Leaf
}
pub(crate) open(super) spec fn leaf_regions(regions: Seq<BuiltRegion>) -> bool {
    forall|i:int| 0 <= i < regions.len() ==> #[trigger] leaf_region(regions[i])
}
pub(crate) open(super) spec fn region_apply(before: Seq<u32>, region: BuiltRegion, limit: nat) -> Seq<u32> {
    Seq::new(64, |i:int| if start(region) <= i < start(region) + limit {
        match region.tree.roots@[i - start(region)] { Child::Leaf(value) => value, Child::Ptr(_) => 0 }
    } else { before[i] })
}
pub(crate) open(super) spec fn apply(before: Seq<u32>, regions: Seq<BuiltRegion>, limit: nat) -> Seq<u32>
    decreases limit,
{
    if limit == 0 { before } else {
        let region = regions[(limit - 1) as int];
        region_apply(apply(before, regions, (limit - 1) as nat), region, count(region.tree))
    }
}
impl PatchModel for &[BuiltRegion<'_>] {
    open(super) spec fn valid(self) -> bool { leaf_regions(self@) }
    open(super) spec fn effect(self, before: Seq<u32>) -> Seq<u32> { apply(before, self@, self@.len()) }
}

pub(super) proof fn region_step(before: Seq<u32>, region: BuiltRegion, position: nat, after: Seq<u32>)
    requires before.len() == 64, leaf_region(region), position < count(region.tree),
        after == region_apply(before, region, position).update(start(region) + position,
            match region.tree.roots@[position as int] { Child::Leaf(value) => value, Child::Ptr(_) => 0 }),
    ensures after == region_apply(before, region, position + 1),
{
    assert(after =~= region_apply(before, region, position + 1));
}

pub(super) open spec fn kept(regions: Seq<BuiltRegion>, limit: nat, value: u32) -> bool {
    limit <= regions.len() && forall|i:int, j:int| 0 <= i < limit && 0 <= j < count(regions[i].tree)
        ==> #[trigger] regions[i].tree.roots@[j] == Child::Leaf(value)
}

pub(super) proof fn kept_step(regions: Seq<BuiltRegion>, limit: nat, value: u32)
    requires limit < regions.len(), kept(regions, limit, value),
        forall|i:int| 0 <= i < count(regions[limit as int].tree) ==> #[trigger] regions[limit as int].tree.roots@[i] == Child::Leaf(value),
    ensures kept(regions, limit + 1, value),
{}

pub(super) proof fn keeps(regions: Seq<BuiltRegion>, limit: nat, value: u32)
    requires limit <= regions.len(), kept(regions, limit, value),
    ensures apply(Seq::new(64, |i:int| value), regions, limit) == Seq::new(64, |i:int| value),
    decreases limit,
{
    if limit > 0 {
        keeps(regions, (limit - 1) as nat, value);
        let region = regions[(limit - 1) as int];
        assert(region_apply(Seq::new(64, |i:int| value), region, count(region.tree))
            =~= Seq::new(64, |i:int| value));
    }
}
}
