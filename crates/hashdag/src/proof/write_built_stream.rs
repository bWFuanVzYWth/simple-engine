//! 独立局部树覆盖与惰性原生流的对应；只有完整输入要求首段缓存协议。
use super::*;
use crate::write::{
    fill_uniform::{UniformFill, proof as fill},
    ordered::proof as ordered,
    set_cells::proof as cells,
};
use crate::{morton::proof as morton, state};
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;
pub(super) open spec fn part_edit(region:BuiltRegion,part:nat)->EditModel {
    EditModel{path:dense::path(morton::value(region.path),lod(region.tree),part as u64),lod:lod(region.tree)/2,payload:part_view(region.tree,part as int)}
}
pub(super) open spec fn block(region:BuiltRegion,part:nat)->Seq<EditModel> {
    Seq::new((root_count(lod(region.tree))-part) as nat,|i:int|part_edit(region,(part+i) as nat))
}
proof fn geometry(region:BuiltRegion,part:nat)
    requires ready(region),!small(region.tree),part<root_count(lod(region.tree)),
    ensures ordered::edit_shape(part_edit(region,part)),state::span((lod(region.tree)/2) as nat)>0,
        ordered::base(part_edit(region,part))==morton::value(region.path) as int+part*state::span((lod(region.tree)/2) as nat),
        ordered::end(part_edit(region,part))==morton::value(region.path) as int+(part+1)*state::span((lod(region.tree)/2) as nat),
        root_count(lod(region.tree))*state::span((lod(region.tree)/2) as nat)==crate::write::validate::proof::size(lod(region.tree)),
{
    let uniform=UniformFill{path:region.path,lod:lod(region.tree),data:0};
    let code=morton::value(region.path);let mask=(crate::write::validate::proof::size(lod(region.tree))-1) as u64;
    assert(code & !mask==code) by(bit_vector) requires code&mask==0;
    assert(crate::write::validate::proof::base(uniform)==code);
    fill::part_geometry(uniform,part);
    assert(2<=lod(region.tree)<=20);
    assert(part_edit(region,part).path==fill::part_edit(uniform,part).path);
}
proof fn part_value(region:BuiltRegion,part:nat,initial:spec_fn(int)->u32,point:int)
    requires ready(region),!small(region.tree),part<root_count(lod(region.tree)),ordered::covers(part_edit(region,part),point),
    ensures ordered::payload_value(part_edit(region,part),initial,point)==tree_value(region.tree,point-morton::value(region.path) as int),
{
    geometry(region,part);let width=state::span((lod(region.tree)/2) as nat) as int;
    let local=point-ordered::base(part_edit(region,part));
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod_converse(point-morton::value(region.path) as int,width,part as int,local);
}
proof fn block_value(region:BuiltRegion,part:nat,initial:spec_fn(int)->u32,point:int,count:nat)
    requires ready(region),!small(region.tree),part<root_count(lod(region.tree)),count<=block(region,part).len(),
    ensures ordered::stream_effect(block(region,part),initial,point,count)==
        if morton::value(region.path) as int+part*state::span((lod(region.tree)/2) as nat)<=point
            <morton::value(region.path) as int+(part+count)*state::span((lod(region.tree)/2) as nat)
        {tree_value(region.tree,point-morton::value(region.path) as int)}else{initial(point)},
    decreases count,
{
    if count>0 {
        block_value(region,part,initial,point,(count-1) as nat);let last=(part+count-1) as nat;
        geometry(region,part);geometry(region,last);
        assert(block(region,part)[(count-1) as int]==part_edit(region,last));
        if ordered::covers(part_edit(region,last),point){part_value(region,last,initial,point);}
        let width=state::span((lod(region.tree)/2) as nat);
        assert(part*width<=last*width) by(nonlinear_arith) requires part<=last;
    }
}
proof fn block_ordered(region:BuiltRegion)
    requires ready(region),!small(region.tree),
    ensures ordered::stream_ordered(block(region,0)),block(region,0).len()>0,
        ordered::base(block(region,0)[0])==morton::value(region.path),ordered::end(block(region,0).last())==region_end(region),
{
    let output=block(region,0);let n=root_count(lod(region.tree));
    assert forall|i:int|0<=i<output.len() implies #[trigger] ordered::edit_shape(output[i]) by {geometry(region,i as nat);}
    assert forall|i:int|0<=i && i+1<output.len() implies ordered::end(#[trigger] output[i])<=ordered::base(output[i+1]) by {geometry(region,i as nat);geometry(region,(i+1) as nat);}
    assert forall|i:int|0<=i<output.len() implies if i==0 {match output[i].payload{Payload::Built(_,p)=>p==0,_=>true}}
        else{#[trigger] ordered::built_continuation(output[i-1],output[i])} by {}
    geometry(region,0);geometry(region,(n-1) as nat);
}
proof fn split_high(regions:Seq<BuiltRegion>,part:nat)
    requires batch_prefix(regions,regions.len()),regions.len()>0,!small(regions[0].tree),part<root_count(lod(regions[0].tree)),
    ensures stream(regions,part)==block(regions[0],part)+stream(regions.drop_first(),0),
    decreases root_count(lod(regions[0].tree))-part,
{
    assert(batch_at(regions,0));assert(group(regions,1));group_found(regions,1);
    assert(regions.subrange(0,1) =~= seq![regions[0]]);assert(regions.subrange(1,regions.len() as int) =~= regions.drop_first());
    if part+1<root_count(lod(regions[0].tree)) {
        split_high(regions,part+1);
        assert(block(regions[0],part) =~= seq![part_edit(regions[0],part)]+block(regions[0],part+1));
        assert(seq![part_edit(regions[0],part)]+(block(regions[0],part+1)+stream(regions.drop_first(),0))
            =~= (seq![part_edit(regions[0],part)]+block(regions[0],part+1))+stream(regions.drop_first(),0));
    }else{assert(block(regions[0],part) =~= seq![part_edit(regions[0],part)]);}
}
proof fn suffix(regions:Seq<BuiltRegion>,start:nat)
    requires healthy(regions),batch_prefix(regions,regions.len()),start<=regions.len(),
    ensures healthy(regions.subrange(start as int,regions.len() as int)),batch_prefix(regions.subrange(start as int,regions.len() as int),(regions.len()-start) as nat),
{
    let rest=regions.subrange(start as int,regions.len() as int);
    assert forall|i:nat|i<rest.len() implies #[trigger] batch_at(rest,i) by {assert(batch_at(regions,start+i));}
}
spec fn source_domains(regions:Seq<BuiltRegion>)->bool {
    forall|i:int|0<=i<regions.len() ==> #[trigger] ready(regions[i]) && crate::write::import::proof::source_valid(regions[i].tree)
}
proof fn sources(regions:Seq<BuiltRegion>)
    requires healthy(regions),batch_prefix(regions,regions.len()),
    ensures source_domains(regions),
{
    assert forall|i:int|0<=i<regions.len() implies #[trigger] ready(regions[i]) && crate::write::import::proof::source_valid(regions[i].tree) by {
        assert(batch_at(regions,i as nat));assert(regions[i].tree.valid_local());
    }
}
proof fn later_region(regions:Seq<BuiltRegion>,left:nat,right:nat)
    requires batch_prefix(regions,regions.len()),left<right<regions.len(),
    ensures morton::value(regions[right as int].path) as int>=region_end(regions[left as int]),
    decreases right-left,
{
    assert(batch_at(regions,right));
    if right>left+1{later_region(regions,left,(right-1) as nat);assert(batch_at(regions,(right-1) as nat));crate::write::validate::proof::span_safe(lod(regions[(right-1) as int].tree));}
}
proof fn high_aligned(region:BuiltRegion)
    requires ready(region),!small(region.tree),
    ensures morton::value(region.path)%64==0,
{
    let code=morton::value(region.path);let l=lod(region.tree);let bits=(l as u32*3) as u32;
    assert(code&63==0) by(bit_vector) requires 2u8<=l<=20u8,bits==(l as u32*3u32) as u32,code&(((1u64<<bits)-1u64) as u64)==0;
    crate::write::patch::proof::slot_bits(code);
}
proof fn low_size(region:BuiltRegion)
    requires ready(region),small(region.tree),
    ensures region_end(region)==morton::value(region.path) as int+count(region.tree),count(region.tree)>0,
{
    let l=lod(region.tree);let bits=(l as u32*3) as u32;
    if l==0 {assert(1u64<<bits==1u64) by(bit_vector) requires bits==0u32;}
    else {assert(l==1);assert(1u64<<bits==8u64) by(bit_vector) requires bits==3u32;}
}
proof fn group_scan(regions:Seq<BuiltRegion>,limit:nat)
    requires 0<limit<=regions.len(),small(regions[0].tree),
        forall|i:int|0<=i<limit ==> #[trigger] small(regions[i].tree) && morton::value(regions[i].path)>>6==morton::value(regions[0].path)>>6,
    ensures group(regions,group_end(regions)),
    decreases regions.len()-limit,
{
    if limit==regions.len() || !small(regions[limit as int].tree) || morton::value(regions[limit as int].path)>>6!=morton::value(regions[0].path)>>6 {group_found(regions,limit);}
    else {group_scan(regions,limit+1);}
}
pub(super) open spec fn low_head(regions:Seq<BuiltRegion>)->EditModel {
    EditModel{path:morton::value(regions[0].path),lod:1,payload:Payload::Patch(|values:Seq<u32>|apply(values,regions.take(group_end(regions) as int),group_end(regions)))}
}
proof fn low_parts(regions:Seq<BuiltRegion>)
    requires healthy(regions),batch_prefix(regions,regions.len()),regions.len()>0,small(regions[0].tree),
    ensures group(regions,group_end(regions)),leaf_regions(regions.take(group_end(regions) as int)),ordered::edit_shape(low_head(regions)),
        stream(regions,0)==seq![low_head(regions)]+stream(regions.subrange(group_end(regions) as int,regions.len() as int),0),
        ordered::base(low_head(regions))==morton::value(regions[0].path) as int/64*64,ordered::end(low_head(regions))==ordered::base(low_head(regions))+64,
        forall|i:int|0<=i<group_end(regions) ==> morton::value(#[trigger] regions[i].path) as int/64==morton::value(regions[0].path) as int/64,
        forall|i:int|group_end(regions)<=i<regions.len() ==> morton::value(#[trigger] regions[i].path) as int>=ordered::end(low_head(regions)),
{
    sources(regions);group_scan(regions,1);let limit=group_end(regions);leaf_group_ready(regions,limit);assert(batch_at(regions,0));
    reveal_with_fuel(state::span,2);low_size(regions[0]);let first=morton::value(regions[0].path);cells::code_parts(first);
    let l=lod(regions[0].tree);assert(l<=1);
    assert(0u64<<((l/2u8) as u32*6u32)==0u64) by(bit_vector) requires l<=1u8;
    assert(dense::path(first,l,0)==first);
    assert(regions.take(limit as int) =~= regions.subrange(0,limit as int));
    assert forall|i:int|0<=i<limit implies morton::value(#[trigger] regions[i].path) as int/64==first as int/64 by {assert(small(regions[i].tree));cells::code_parts(morton::value(regions[i].path));}
    if limit<regions.len() {
        assert(ready(regions[limit as int]));later_region(regions,0,limit);let next=morton::value(regions[limit as int].path);cells::code_parts(next);
        if !small(regions[limit as int].tree){high_aligned(regions[limit as int]);}else{assert(next>>6!=first>>6);}
        assert(next as int/64>first as int/64);
        assert forall|i:int|limit<=i<regions.len() implies morton::value(#[trigger] regions[i].path) as int>=ordered::end(low_head(regions)) by {
            if i>limit{later_region(regions,limit,i as nat);crate::write::validate::proof::span_safe(lod(regions[limit as int].tree));}
        }
    }
}
proof fn low_tree_value(region:BuiltRegion,local:int)
    requires ready(region),leaf_region(region),0<=local<count(region.tree),
    ensures tree_value(region.tree,local)==match region.tree.roots@[local]{Child::Leaf(v)=>v,Child::Ptr(_)=>0},
{
    reveal_with_fuel(state::span,2);assert(region.tree.roots@[local] is Leaf);
    assert(region.tree.lod==Some(0)||region.tree.lod==Some(1));assert(lod(region.tree)/2==0);
    assert(root_count(lod(region.tree))==count(region.tree));
    assert(0<=local/1<root_count(lod(region.tree)));
    assert(tree_value(region.tree,local)==state::eval(state::pools_view(&region.tree.pools),region.tree.roots@[local],0,0));
    match region.tree.roots@[local] {Child::Leaf(value)=>{assert(tree_value(region.tree,local)==value);},Child::Ptr(_)=>{assert(false);}}
}
proof fn apply_length(values:Seq<u32>,regions:Seq<BuiltRegion>,limit:nat)
    requires values.len()==64,limit<=regions.len(),
    ensures apply(values,regions,limit).len()==64,
    decreases limit,
{if limit>0{apply_length(values,regions,(limit-1) as nat);}}
spec fn low_domains(regions:Seq<BuiltRegion>,base:int)->bool {
    forall|i:int|0<=i<regions.len() ==> #[trigger] ready(regions[i]) && morton::value(regions[i].path) as int/64==base/64
}
proof fn low_apply_value(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,base:int,target:int,limit:nat)
    requires limit<=regions.len(),leaf_regions(regions),base>=0,base%64==0,0<=target<64,
        low_domains(regions,base),
    ensures apply(Seq::new(64,|i:int|initial(base+i)),regions,limit)[target]==batch_effect(regions,initial,base+target,limit),
    decreases limit,
{
    if limit>0 {
        low_apply_value(regions,initial,base,target,(limit-1) as nat);let region=regions[(limit-1) as int];
        assert(ready(region));assert(leaf_region(region));low_size(region);cells::group_address(morton::value(region.path),base);
        if start(region)<=target<start(region)+count(region.tree){low_tree_value(region,target-start(region));}
    }
}
proof fn effect_prefix(a:Seq<BuiltRegion>,b:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int,n:nat)
    requires n<=a.len(),n<=b.len(),forall|i:int|0<=i<n ==> #[trigger] a[i]==b[i],
    ensures batch_effect(a,initial,point,n)==batch_effect(b,initial,point,n),decreases n,
{if n>0{effect_prefix(a,b,initial,point,(n-1) as nat);}}
proof fn effect_concat(a:Seq<BuiltRegion>,b:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int)
    ensures batch_effect(a+b,initial,point,a.len()+b.len())==batch_effect(b,|p:int|batch_effect(a,initial,p,a.len()),point,b.len()),decreases b.len(),
{
    if b.len()>0{effect_concat(a,b.drop_last(),initial,point);effect_prefix(a+b,a+b.drop_last(),initial,point,(a.len()+b.len()-1) as nat);
        effect_prefix(b,b.drop_last(),|p:int|batch_effect(a,initial,p,a.len()),point,(b.len()-1) as nat);}else{assert(a+b =~= a);}
}
proof fn effect_initial(regions:Seq<BuiltRegion>,a:spec_fn(int)->u32,b:spec_fn(int)->u32,point:int,n:nat)
    requires n<=regions.len(),a(point)==b(point),ensures batch_effect(regions,a,point,n)==batch_effect(regions,b,point,n),decreases n,
{if n>0{effect_initial(regions,a,b,point,(n-1) as nat);}}
proof fn effect_absent(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int,n:nat)
    requires n<=regions.len(),forall|i:int|0<=i<n ==> !(morton::value(#[trigger] regions[i].path) as int<=point<region_end(regions[i])),
    ensures batch_effect(regions,initial,point,n)==initial(point),decreases n,
{if n>0{effect_absent(regions,initial,point,(n-1) as nat);}}
spec fn ranges(edits:Seq<EditModel>)->bool {
    (forall|i:int|0<=i<edits.len() ==> #[trigger] ordered::edit_shape(edits[i]))
        && (forall|i:int|0<=i && i+1<edits.len() ==> ordered::end(#[trigger] edits[i])<=ordered::base(edits[i+1]))
}
spec fn protocol(edits:Seq<EditModel>)->bool {
    (edits.len()>0 ==> match edits[0].payload{Payload::Built(_,p)=>p==0,_=>true})
        && forall|i:int|0<i<edits.len() ==> #[trigger] ordered::built_continuation(edits[i-1],edits[i])
}
proof fn append_ranges(a:Seq<EditModel>,b:Seq<EditModel>)
    requires a.len()>0,ranges(a),ranges(b),b.len()>0 ==> ordered::end(a.last())<=ordered::base(b[0]),
    ensures ranges(a+b),
{
    let out=a+b;
    assert forall|i:int|0<=i<out.len() implies #[trigger] ordered::edit_shape(out[i]) by {if i<a.len(){assert(out[i]==a[i]);}else{assert(out[i]==b[i-a.len()]);}}
    assert forall|i:int|0<=i && i+1<out.len() implies ordered::end(#[trigger] out[i])<=ordered::base(out[i+1]) by {
        if i+1<a.len(){assert(out[i]==a[i]);assert(out[i+1]==a[i+1]);}
        else if i+1==a.len(){assert(out[i]==a.last());assert(out[i+1]==b[0]);}
        else{assert(out[i]==b[i-a.len()]);assert(out[i+1]==b[i+1-a.len()]);}
    }
}
proof fn append_protocol(a:Seq<EditModel>,b:Seq<EditModel>)
    requires a.len()>0,protocol(a),protocol(b),ensures protocol(a+b),
{
    let out=a+b;
    assert(out[0]==a[0]);
    assert forall|i:int|0<i<out.len() implies #[trigger] ordered::built_continuation(out[i-1],out[i]) by {
        if i<a.len(){assert(out[i]==a[i]);assert(out[i-1]==a[i-1]);assert(ordered::built_continuation(a[i-1],a[i]));}
        else if i==a.len(){assert(out[i]==b[0]);match b[0].payload{Payload::Built(_,p)=>{assert(p==0);},_=>{}}}
        else{let j=i-a.len();assert(0<j<b.len());assert(out[i]==b[j]);assert(out[i-1]==b[j-1]);assert(ordered::built_continuation(b[j-1],b[j]));}
    }
}
proof fn append_ordered(a:Seq<EditModel>,b:Seq<EditModel>)
    requires a.len()>0,ordered::stream_ordered(a),ordered::stream_ordered(b),b.len()>0 ==> ordered::end(a.last())<=ordered::base(b[0]),
    ensures ordered::stream_ordered(a+b),
{append_ranges(a,b);append_protocol(a,b);}
proof fn append_value(a:Seq<EditModel>,b:Seq<EditModel>,initial:spec_fn(int)->u32,point:int)
    ensures ordered::stream_effect(a+b,initial,point,a.len()+b.len())==if cells::stream_hits(b,point)
        {ordered::stream_effect(b,initial,point,b.len())}else{ordered::stream_effect(a,initial,point,a.len())},
    decreases b.len(),
{
    if b.len()==0{assert(a+b =~= a);}else{
        let rest=b.drop_last();append_value(a,rest,initial,point);
        cells::stream_prefix(a+b,a+rest,initial,point,(a.len()+b.len()-1) as nat);cells::stream_prefix(b,rest,initial,point,rest.len());
        assert(cells::stream_hits(b,point)==(cells::stream_hits(rest,point)||ordered::covers(b.last(),point))) by {
            if cells::stream_hits(b,point){let i=choose|i:int|0<=i<b.len() && ordered::covers(b[i],point);if i<b.len()-1{assert(rest[i]==b[i]);}}
            if cells::stream_hits(rest,point){let i=choose|i:int|0<=i<rest.len() && ordered::covers(rest[i],point);assert(rest[i]==b[i]);}
        }
    }
}
pub(super) open spec fn initial_base(regions:Seq<BuiltRegion>)->int {
    if small(regions[0].tree){morton::value(regions[0].path) as int/64*64}else{morton::value(regions[0].path) as int}
}
pub(super) open spec fn stream_state(regions:Seq<BuiltRegion>)->bool {
    ordered::stream_ordered(stream(regions,0)) && (regions.len()>0 ==> stream(regions,0).len()>0 && ordered::base(stream(regions,0)[0])==initial_base(regions))
}
proof fn next_base(regions:Seq<BuiltRegion>,bound:int)
    requires regions.len()>0,morton::value(regions[0].path) as int>=bound,bound>=0,bound%64==0,
    ensures initial_base(regions)>=bound,
{if small(regions[0].tree){dense::floor_bound(morton::value(regions[0].path) as int,bound);}}
proof fn low_valid_join(regions:Seq<BuiltRegion>)
    requires healthy(regions),batch_prefix(regions,regions.len()),regions.len()>0,small(regions[0].tree),
        stream_state(regions.subrange(group_end(regions) as int,regions.len() as int)),
    ensures stream_state(regions),
{
    hide(stream);low_parts(regions);let rest=regions.subrange(group_end(regions) as int,regions.len() as int);let head=low_head(regions);
    dense::aligned_end(head);if rest.len()>0{next_base(rest,ordered::end(head));}else{reveal(stream);}
    assert(ordered::stream_ordered(seq![head]));append_ordered(seq![head],stream(rest,0));
}
proof fn high_valid_join(regions:Seq<BuiltRegion>)
    requires healthy(regions),batch_prefix(regions,regions.len()),regions.len()>0,!small(regions[0].tree),stream_state(regions.drop_first()),
    ensures stream_state(regions),
{
    hide(stream);assert(batch_at(regions,0));let region=regions[0];let rest=regions.drop_first();block_ordered(region);split_high(regions,0);
    geometry(region,(root_count(lod(region.tree))-1) as nat);dense::aligned_end(block(region,0).last());
    if rest.len()>0{assert(batch_at(regions,1));next_base(rest,region_end(region));}else{reveal(stream);}
    append_ordered(block(region,0),stream(rest,0));
}
proof fn stream_valid(regions:Seq<BuiltRegion>)
    requires healthy(regions),batch_prefix(regions,regions.len()),ensures stream_state(regions),decreases regions.len(),
{
    hide(stream);
    if regions.len()>0{
        assert(batch_at(regions,0));
        if small(regions[0].tree){low_parts(regions);let n=group_end(regions);suffix(regions,n);stream_valid(regions.subrange(n as int,regions.len() as int));low_valid_join(regions);}
        else{suffix(regions,1);assert(regions.drop_first() =~= regions.subrange(1,regions.len() as int));stream_valid(regions.drop_first());high_valid_join(regions);}
    }else{reveal(stream);}
}
proof fn effect_tail(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int,n:nat)
    requires 0<n<=regions.len(),!(morton::value(regions[0].path) as int<=point<region_end(regions[0])),
    ensures batch_effect(regions,initial,point,n)==batch_effect(regions.drop_first(),initial,point,(n-1) as nat),decreases n,
{
    reveal_with_fuel(batch_effect,2);
    if n>1{effect_tail(regions,initial,point,(n-1) as nat);assert(regions[(n-1) as int]==regions.drop_first()[(n-2) as int]);}
}
proof fn effect_head_hit(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int,n:nat)
    requires batch_prefix(regions,regions.len()),0<n<=regions.len(),morton::value(regions[0].path) as int<=point<region_end(regions[0]),
    ensures batch_effect(regions,initial,point,n)==tree_value(regions[0].tree,point-morton::value(regions[0].path) as int),decreases n,
{
    if n>1{later_region(regions,0,(n-1) as nat);effect_head_hit(regions,initial,point,(n-1) as nat);}
}
proof fn input_low_hit(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int)
    requires healthy(regions),batch_prefix(regions,regions.len()),regions.len()>0,small(regions[0].tree),ordered::covers(low_head(regions),point),
    ensures batch_effect(regions,initial,point,regions.len())==batch_effect(regions.take(group_end(regions) as int),initial,point,group_end(regions)),
{
    hide(batch_effect);low_parts(regions);let group=regions.take(group_end(regions) as int);let rest=regions.subrange(group_end(regions) as int,regions.len() as int);
    assert(regions =~= group+rest);effect_concat(group,rest,initial,point);effect_absent(rest,|p:int|batch_effect(group,initial,p,group.len()),point,rest.len());
}
proof fn input_low_miss(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int)
    requires healthy(regions),batch_prefix(regions,regions.len()),regions.len()>0,small(regions[0].tree),!ordered::covers(low_head(regions),point),
    ensures batch_effect(regions,initial,point,regions.len())==batch_effect(regions.subrange(group_end(regions) as int,regions.len() as int),initial,point,(regions.len()-group_end(regions)) as nat),
{
    hide(batch_effect);low_parts(regions);sources(regions);let group=regions.take(group_end(regions) as int);let rest=regions.subrange(group_end(regions) as int,regions.len() as int);let base=ordered::base(low_head(regions));
    assert(regions =~= group+rest);effect_concat(group,rest,initial,point);
    assert forall|i:int|0<=i<group.len() implies !(morton::value(#[trigger] group[i].path) as int<=point<region_end(group[i])) by {
        assert(ready(group[i]));assert(leaf_region(group[i]));low_size(group[i]);cells::group_address(morton::value(group[i].path),base);
    }
    effect_absent(group,initial,point,group.len());effect_initial(rest,|p:int|batch_effect(group,initial,p,group.len()),initial,point,rest.len());
}
proof fn low_payload(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int)
    requires healthy(regions),batch_prefix(regions,regions.len()),regions.len()>0,small(regions[0].tree),ordered::covers(low_head(regions),point),
    ensures ordered::payload_value(low_head(regions),initial,point)==batch_effect(regions.take(group_end(regions) as int),initial,point,group_end(regions)),
{
    hide(batch_effect);low_parts(regions);sources(regions);let group=regions.take(group_end(regions) as int);let base=ordered::base(low_head(regions));
    assert forall|i:int|0<=i<group.len() implies #[trigger] ready(group[i]) && morton::value(group[i].path) as int/64==base/64 by {assert(ready(regions[i]));}
    low_apply_value(group,initial,base,point-base,group.len());apply_length(Seq::new(64,|i:int|initial(base+i)),group,group.len());
}
proof fn low_value_join(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int)
    requires healthy(regions),batch_prefix(regions,regions.len()),regions.len()>0,small(regions[0].tree),group(regions,group_end(regions)),
        stream_state(regions.subrange(group_end(regions) as int,regions.len() as int)),
        ordered::stream_effect(stream(regions.subrange(group_end(regions) as int,regions.len() as int),0),initial,point,stream(regions.subrange(group_end(regions) as int,regions.len() as int),0).len())
            ==batch_effect(regions.subrange(group_end(regions) as int,regions.len() as int),initial,point,(regions.len()-group_end(regions)) as nat),
    ensures ordered::stream_effect(stream(regions,0),initial,point,stream(regions,0).len())==batch_effect(regions,initial,point,regions.len()),
{
    hide(stream);hide(batch_effect);low_parts(regions);let rest=regions.subrange(group_end(regions) as int,regions.len() as int);let head=low_head(regions);let tail=stream(rest,0);
    dense::aligned_end(head);if rest.len()>0{next_base(rest,ordered::end(head));}else{reveal(stream);}
    if ordered::covers(head,point){input_low_hit(regions,initial,point);low_payload(regions,initial,point);cells::stream_head_hit(head,tail,initial,point);}
    else{input_low_miss(regions,initial,point);cells::stream_skip_head(head,tail,initial,point);}
}
proof fn high_value_join(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int)
    requires healthy(regions),batch_prefix(regions,regions.len()),regions.len()>0,!small(regions[0].tree),stream_state(regions.drop_first()),
        ordered::stream_effect(stream(regions.drop_first(),0),initial,point,stream(regions.drop_first(),0).len())==batch_effect(regions.drop_first(),initial,point,regions.drop_first().len()),
    ensures ordered::stream_effect(stream(regions,0),initial,point,stream(regions,0).len())==batch_effect(regions,initial,point,regions.len()),
{
    hide(stream);hide(batch_effect);assert(batch_at(regions,0));let region=regions[0];let rest=regions.drop_first();let a=block(region,0);let b=stream(rest,0);
    block_ordered(region);split_high(regions,0);geometry(region,(root_count(lod(region.tree))-1) as nat);dense::aligned_end(a.last());
    if rest.len()>0{assert(batch_at(regions,1));next_base(rest,region_end(region));}else{reveal(stream);}
    block_value(region,0,initial,point,a.len());append_value(a,b,initial,point);
    if morton::value(region.path) as int<=point<region_end(region){
        assert forall|i:int|0<=i<b.len() implies !#[trigger] ordered::covers(b[i],point) by {cells::later_edit(b,i);}
        assert(!cells::stream_hits(b,point));effect_head_hit(regions,initial,point,regions.len());
    }else{
        effect_tail(regions,initial,point,regions.len());
        if !cells::stream_hits(b,point){cells::stream_no_cover(b,initial,point);}
    }
}
proof fn stream_value(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int)
    requires healthy(regions),batch_prefix(regions,regions.len()),
    ensures ordered::stream_effect(stream(regions,0),initial,point,stream(regions,0).len())==batch_effect(regions,initial,point,regions.len()),decreases regions.len(),
{
    hide(stream);hide(batch_effect);
    if regions.len()>0{
        assert(batch_at(regions,0));
        if small(regions[0].tree){low_parts(regions);let n=group_end(regions);let rest=regions.subrange(n as int,regions.len() as int);suffix(regions,n);stream_valid(rest);stream_value(rest,initial,point);low_value_join(regions,initial,point);}
        else{let rest=regions.drop_first();suffix(regions,1);assert(rest =~= regions.subrange(1,regions.len() as int));stream_valid(rest);stream_value(rest,initial,point);high_value_join(regions,initial,point);}
    }else{reveal(stream);reveal(batch_effect);}
}

pub(in super::super) proof fn input_stream(regions:Seq<BuiltRegion>,initial:spec_fn(int)->u32,point:int)
    requires healthy(regions),batch_prefix(regions,regions.len()),
    ensures ordered::stream_ordered(stream(regions,0)),
        ordered::stream_effect(stream(regions,0),initial,point,stream(regions,0).len())==batch_effect(regions,initial,point,regions.len()),
{
    stream_valid(regions);stream_value(regions,initial,point);
}
}
