//! dense 借用流的叶组与奇数 LOD 分段。
use super::super::ordered::proof as ordered;
use super::super::ordered::proof::{EditModel, Payload, StreamModel};
use super::super::set_cells::proof as cells_proof;
use super::super::{patch::proof as patch, validate::proof as validate};
use super::*;
use crate::write::commit::proof as commit;
use crate::{node::proof as node, state};
use vstd::prelude::*;
#[path = "write_dense_parts.rs"]
pub(crate) mod parts_proof;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;
pub(crate) proof fn low_region(region:DenseRegion)
    requires region.lod<2,region.values@.len()<=usize::MAX,validate::dense_count(region.lod)==Some(region.values@.len() as usize),
        crate::morton::proof::value(region.path)&((validate::size(region.lod)-1) as u64)==0,
    ensures patch::leaf_region(region),
{
    let code=crate::morton::proof::value(region.path);
    let bits=(region.lod as u32*3) as u32;
    assert(bits<usize::BITS);
    if region.lod==0 {assert(1usize<<bits==1usize) by(bit_vector) requires bits==0u32;assert(validate::dense_count(region.lod)==Some(1usize));}
    else {
        assert(region.lod==1);assert(1usize<<bits==8usize) by(bit_vector) requires bits==3u32;
        assert(1u64<<bits==8u64) by(bit_vector) requires bits==3u32;
        assert(validate::dense_count(region.lod)==Some(8usize));
        assert(code&7==0);
        assert(code%64<=56) by(bit_vector) requires code&7==0;
    }
    cells_proof::code_parts(code);
}
pub(super) proof fn leaf_group_ready(regions:Seq<DenseRegion>,limit:nat)
    requires validate::dense_prefix(regions,regions.len()),group(regions,limit),regions[0].lod<2,
    ensures patch::leaf_regions(regions.take(limit as int)),
        forall|i:int|0<=i<limit ==> crate::morton::proof::value(#[trigger] regions[i].path) as int/64==crate::morton::proof::value(regions[0].path) as int/64,
{
    assert forall|i:int|0<=i<limit implies #[trigger] patch::leaf_region(regions.take(limit as int)[i]) by {
        assert(validate::dense_at(regions,i as nat));low_region(regions[i]);
    }
    assert forall|i:int|0<=i<limit implies crate::morton::proof::value(#[trigger] regions[i].path) as int/64==crate::morton::proof::value(regions[0].path) as int/64 by {
        assert(regions[i].lod<2);
        assert(crate::morton::proof::value(regions[i].path)>>6==crate::morton::proof::value(regions[0].path)>>6);
        cells_proof::code_parts(crate::morton::proof::value(regions[i].path));cells_proof::code_parts(crate::morton::proof::value(regions[0].path));
    }
}
pub(crate) proof fn effect_same_initial(regions:Seq<DenseRegion>,a:spec_fn(int)->u32,b:spec_fn(int)->u32,path:int,count:nat)
    requires count<=regions.len(),a(path)==b(path),
    ensures effect(regions,a,path,count)==effect(regions,b,path,count),
    decreases count,
{if count>0{effect_same_initial(regions,a,b,path,(count-1) as nat);}}
pub(crate) proof fn effect_prefix(a:Seq<DenseRegion>,b:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int,count:nat)
    requires count<=a.len(),count<=b.len(),forall|i:int|0<=i<count ==> #[trigger] a[i]==b[i],
    ensures effect(a,initial,path,count)==effect(b,initial,path,count),
    decreases count,
{if count>0{effect_prefix(a,b,initial,path,(count-1) as nat);}}
pub(crate) proof fn effect_concat(a:Seq<DenseRegion>,b:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int)
    ensures effect(a+b,initial,path,a.len()+b.len())==effect(b,|p:int|effect(a,initial,p,a.len()),path,b.len()),
    decreases b.len(),
{
    if b.len()>0 {
        effect_concat(a,b.drop_last(),initial,path);effect_prefix(a+b,a+b.drop_last(),initial,path,(a.len()+b.len()-1) as nat);
        effect_prefix(b,b.drop_last(),|p:int|effect(a,initial,p,a.len()),path,(b.len()-1) as nat);
    } else {assert(a+b =~= a);}
}
pub(crate) proof fn effect_absent(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int,count:nat)
    requires count<=regions.len(),forall|i:int|0<=i<count ==>
        !(crate::morton::proof::value(#[trigger] regions[i].path) as int<=path<crate::morton::proof::value(regions[i].path) as int+regions[i].values@.len()),
    ensures effect(regions,initial,path,count)==initial(path),
    decreases count,
{if count>0{effect_absent(regions,initial,path,(count-1) as nat);}}
proof fn group_effect(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,base:int,target:int,count:nat)
    requires count<=regions.len(),base>=0,base%64==0,0<=target<64,
        forall|i:int|0<=i<count ==> crate::morton::proof::value(#[trigger] regions[i].path) as int/64==base/64,
    ensures effect(regions,initial,base+target,count)==patch::dense_last_value(regions,count,target,initial(base+target)),
    decreases count,
{
    if count>0 {group_effect(regions,initial,base,target,(count-1) as nat);
        cells_proof::group_address(crate::morton::proof::value(regions[(count-1) as int].path),base);}
}
pub(crate) proof fn leaf_group_value(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,base:int,target:int)
    requires base>=0,base%64==0,0<=target<64,
        forall|i:int|0<=i<regions.len() ==> crate::morton::proof::value(#[trigger] regions[i].path) as int/64==base/64,
    ensures patch::dense_apply(Seq::new(64,|i:int|initial(base+i)),regions,regions.len())[target]==effect(regions,initial,base+target,regions.len()),
{
    patch::dense_point_value(Seq::new(64,|i:int|initial(base+i)),regions,regions.len(),target);group_effect(regions,initial,base,target,regions.len());
}
pub(crate) proof fn leaf_apply_length(values:Seq<u32>,regions:Seq<DenseRegion>,count:nat)
    requires values.len()==64,count<=regions.len(),
    ensures patch::dense_apply(values,regions,count).len()==64,
    decreases count,
{if count>0{leaf_apply_length(values,regions,(count-1) as nat);}}
pub(crate) proof fn source_region(regions:Seq<DenseRegion>,i:nat)
    requires validate::dense_prefix(regions,regions.len()),i<regions.len(),
    ensures validate::dense_at(seq![regions[i as int]],0),
{assert(validate::dense_at(regions,i));}
pub(crate) proof fn high_aligned(region:DenseRegion)
    requires validate::dense_at(seq![region],0),region.lod>=2,
    ensures crate::morton::proof::value(region.path)%64==0,
{
    let code=crate::morton::proof::value(region.path);let lod=region.lod;let bits=(lod as u32*3) as u32;
    assert(code&63==0) by(bit_vector)
        requires lod>=2u8,lod<=20u8,bits==(lod as u32*3u32) as u32,code&(((1u64<<bits)-1u64) as u64)==0;
    cells_proof::code_parts(code);patch::slot_bits(code);
}
pub(crate) proof fn source_size(region:DenseRegion)
    requires validate::dense_at(seq![region],0),
    ensures region.values@.len()==validate::size(region.lod) as nat,region.values@.len()>0,
{
    validate::span_safe(region.lod);
    if region.lod<2 {
        low_region(region);let bits=(region.lod as u32*3) as u32;
        if region.lod==0 {assert(1u64<<bits==1u64) by(bit_vector) requires bits==0u32;}
        else {assert(1u64<<bits==8u64) by(bit_vector) requires bits==3u32;}
    } else {parts_proof::geometry(region,0);}
}
pub(crate) proof fn later_region(regions:Seq<DenseRegion>,left:nat,right:nat)
    requires validate::dense_prefix(regions,regions.len()),left<right<regions.len(),
    ensures crate::morton::proof::value(regions[right as int].path) as int>=validate::dense_end(regions[left as int]),
    decreases right-left,
{
    assert(validate::dense_at(regions,right));
    if right>left+1 {later_region(regions,left,(right-1) as nat);source_region(regions,(right-1) as nat);source_size(regions[(right-1) as int]);}
}
proof fn group_scan(regions:Seq<DenseRegion>,limit:nat)
    requires 0<limit<=regions.len(),regions[0].lod<2,
        forall|i:int|0<=i<limit ==> #[trigger] regions[i].lod<2
            && crate::morton::proof::value(regions[i].path)>>6==crate::morton::proof::value(regions[0].path)>>6,
    ensures group(regions,end(regions)),
    decreases regions.len()-limit,
{
    if limit==regions.len() || regions[limit as int].lod>=2
        || crate::morton::proof::value(regions[limit as int].path)>>6!=crate::morton::proof::value(regions[0].path)>>6 {
        group_found(regions,limit);
    } else {group_scan(regions,limit+1);}
}
pub(crate) open(super) spec fn low_head(regions:Seq<DenseRegion>)->EditModel {
    EditModel{path:crate::morton::proof::value(regions[0].path),lod:1,
        payload:Payload::Patch(|values:Seq<u32>|patch::dense_apply(values,regions.take(end(regions) as int),end(regions)))}
}
proof fn low_parts(regions:Seq<DenseRegion>)
    requires validate::dense_prefix(regions,regions.len()),regions.len()>0,regions[0].lod<2,
    ensures group(regions,end(regions)),patch::leaf_regions(regions.take(end(regions) as int)),ordered::edit_shape(low_head(regions)),
        stream(regions,0)==seq![low_head(regions)]+stream(regions.subrange(end(regions) as int,regions.len() as int),0),
        ordered::base(low_head(regions))==crate::morton::proof::value(regions[0].path) as int/64*64,
        ordered::end(low_head(regions))==ordered::base(low_head(regions))+64,
        forall|i:int|0<=i<end(regions) ==> crate::morton::proof::value(#[trigger] regions[i].path) as int/64==crate::morton::proof::value(regions[0].path) as int/64,
        forall|i:int|end(regions)<=i<regions.len() ==> crate::morton::proof::value(#[trigger] regions[i].path) as int>=ordered::end(low_head(regions)),
{
    group_scan(regions,1);let limit=end(regions);leaf_group_ready(regions,limit);
    reveal_with_fuel(state::span,2);source_region(regions,0);source_size(regions[0]);
    let first=crate::morton::proof::value(regions[0].path);
    cells_proof::code_parts(first);
    assert(regions.take(limit as int) =~= regions.subrange(0,limit as int));
    if limit<regions.len() {
        source_region(regions,limit);later_region(regions,0,limit);
        let next=crate::morton::proof::value(regions[limit as int].path);cells_proof::code_parts(next);
        if regions[limit as int].lod>=2 {high_aligned(regions[limit as int]);}
        else {assert(next>>6!=first>>6);}
        assert(next as int/64>first as int/64);
        assert forall|i:int|limit<=i<regions.len() implies crate::morton::proof::value(#[trigger] regions[i].path) as int>=ordered::end(low_head(regions)) by {
            if i>limit {later_region(regions,limit,i as nat);source_size(regions[limit as int]);}
        }
    }
}
pub(crate) open spec fn remaining_base(regions:Seq<DenseRegion>,part:nat)->int {
    if regions[0].lod<2 {crate::morton::proof::value(regions[0].path) as int/64*64}
    else {crate::morton::proof::value(regions[0].path) as int+part*state::span((regions[0].lod/2) as nat)}
}
pub(crate) proof fn aligned_end(edit:EditModel)
    requires 1<=edit.lod<=10,
    ensures ordered::end(edit)%64==0,ordered::end(edit)>0,
{
    crate::canonical::span_positive((edit.lod-1) as nat);let unit=state::span((edit.lod-1) as nat) as int;
    let width=state::span(edit.lod as nat) as int;let quotient=edit.path as int/width+1;
    assert(quotient*width==(edit.path as int/width)*width+width) by(nonlinear_arith)
        requires quotient==edit.path as int/width+1;
    assert(ordered::end(edit)==64*(quotient*unit)) by(nonlinear_arith)
        requires width==64*unit,ordered::end(edit)==quotient*width;
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod_converse(ordered::end(edit),64,quotient*unit,0);
}
pub(crate) proof fn floor_bound(value:int,bound:int)
    requires value>=bound>=0,bound%64==0,
    ensures value/64*64>=bound,
{
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(value,64);vstd::arithmetic::div_mod::lemma_mod_bound(value,64);
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(bound,64);
}
proof fn following_base(regions:Seq<DenseRegion>,bound:int)
    requires regions.len()>0,bound>=0,bound%64==0,crate::morton::proof::value(regions[0].path) as int>=bound,
    ensures remaining_base(regions,0)>=bound,
{if regions[0].lod<2 {floor_bound(crate::morton::proof::value(regions[0].path) as int,bound);}}
proof fn high_step(regions:Seq<DenseRegion>,part:nat)
    requires validate::dense_prefix(regions,regions.len()),regions.len()>0,regions[0].lod>=2,part<parts(regions[0].lod),
    ensures stream(regions,part)==seq![parts_proof::edit(regions[0],part)]+
        if part+1==parts(regions[0].lod){stream(regions.drop_first(),0)}else{stream(regions,part+1)},
{
    source_region(regions,0);assert(group(regions,1));group_found(regions,1);
    assert(regions.subrange(1,regions.len() as int) =~= regions.drop_first());
}
pub(crate) open spec fn stream_state(regions:Seq<DenseRegion>,part:nat)->bool {
    ordered::stream_ordered(stream(regions,part))
        && (forall|i:int|0<=i<stream(regions,part).len() ==> !(#[trigger] stream(regions,part)[i].payload is Built))
        && (regions.len()>0 ==> stream(regions,part).len()>0 && ordered::base(stream(regions,part)[0])==remaining_base(regions,part))
}
proof fn valid_cons(head:EditModel,tail:Seq<EditModel>)
    requires ordered::edit_shape(head),!(head.payload is Built),ordered::stream_ordered(tail),
        forall|i:int|0<=i<tail.len() ==> !(#[trigger] tail[i].payload is Built),
        tail.len()>0 ==> ordered::base(tail[0])>=ordered::end(head),
    ensures ordered::stream_ordered(seq![head]+tail),
        forall|i:int|0<=i<(seq![head]+tail).len() ==> !((#[trigger] (seq![head]+tail)[i]).payload is Built),
{
    assert forall|i:int|0<=i<tail.len() implies !(#[trigger] tail[i].payload is Built) && ordered::base(tail[i])>=ordered::end(head) by {cells_proof::later_edit(tail,i);}
    cells_proof::stream_cons_ordered(head,tail);
    assert forall|i:int|0<=i<(seq![head]+tail).len() implies !((#[trigger] (seq![head]+tail)[i]).payload is Built) by {if i>0{assert((seq![head]+tail)[i]==tail[i-1]);}}
}
proof fn valid_low_join(regions:Seq<DenseRegion>)
    requires validate::dense_prefix(regions,regions.len()),regions.len()>0,regions[0].lod<2,
        stream_state(regions.subrange(end(regions) as int,regions.len() as int),0),
    ensures stream_state(regions,0),
{
    hide(stream);low_parts(regions);let rest=regions.subrange(end(regions) as int,regions.len() as int);
    let head=low_head(regions);aligned_end(head);if rest.len()>0 {following_base(rest,ordered::end(head));}
    else {reveal(stream);}
    valid_cons(head,stream(rest,0));
}
proof fn valid_high_join(regions:Seq<DenseRegion>,part:nat)
    requires validate::dense_prefix(regions,regions.len()),regions.len()>0,regions[0].lod>=2,part<parts(regions[0].lod),
        if part+1==parts(regions[0].lod){stream_state(regions.drop_first(),0)}else{stream_state(regions,part+1)},
    ensures stream_state(regions,part),
{
    hide(stream);high_step(regions,part);source_region(regions,0);let region=regions[0];let head=parts_proof::edit(region,part);
    parts_proof::geometry(region,part);aligned_end(head);
    if part+1==parts(region.lod) {
        let rest=regions.drop_first();if rest.len()>0 {assert(validate::dense_at(regions,1));following_base(rest,ordered::end(head));}else{reveal(stream);}
        valid_cons(head,stream(rest,0));
    }else{parts_proof::geometry(region,part+1);valid_cons(head,stream(regions,part+1));}
}
pub(crate) proof fn stream_valid(regions:Seq<DenseRegion>,part:nat)
    requires validate::dense_prefix(regions,regions.len()),if regions.len()==0{part==0}else{part<parts(regions[0].lod)},
    ensures stream_state(regions,part),
    decreases regions.len()*8-part,
{
    hide(stream);
    if regions.len()>0 {
        source_region(regions,0);
        if regions[0].lod<2 {
            assert(part==0);low_parts(regions);let limit=end(regions);validate::dense_suffix(regions,limit as int);
            stream_valid(regions.subrange(limit as int,regions.len() as int),0);valid_low_join(regions);
        }else{
            if part+1==parts(regions[0].lod){validate::dense_suffix(regions,1);assert(regions.drop_first() =~= regions.subrange(1,regions.len() as int));stream_valid(regions.drop_first(),0);}
            else{stream_valid(regions,part+1);}
            valid_high_join(regions,part);
        }
    }else{reveal(stream);}
}
proof fn effect_tail(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int,count:nat)
    requires 0<count<=regions.len(),!(crate::morton::proof::value(regions[0].path) as int<=path<crate::morton::proof::value(regions[0].path) as int+regions[0].values@.len()),
    ensures effect(regions,initial,path,count)==effect(regions.drop_first(),initial,path,(count-1) as nat),
    decreases count,
{
    reveal_with_fuel(effect,2);
    if count>1 {effect_tail(regions,initial,path,(count-1) as nat);assert(regions[(count-1) as int]==regions.drop_first()[(count-2) as int]);}
}
proof fn effect_head_hit(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int,count:nat)
    requires validate::dense_prefix(regions,regions.len()),0<count<=regions.len(),
        crate::morton::proof::value(regions[0].path) as int<=path<validate::dense_end(regions[0]),
    ensures effect(regions,initial,path,count)==regions[0].values@[path-crate::morton::proof::value(regions[0].path) as int],
    decreases count,
{
    source_region(regions,0);source_size(regions[0]);
    if count>1 {later_region(regions,0,(count-1) as nat);effect_head_hit(regions,initial,path,(count-1) as nat);}
}
proof fn effect_head(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int)
    requires validate::dense_prefix(regions,regions.len()),regions.len()>0,
    ensures effect(regions,initial,path,regions.len())==if crate::morton::proof::value(regions[0].path) as int<=path<validate::dense_end(regions[0])
        {regions[0].values@[path-crate::morton::proof::value(regions[0].path) as int]}
        else{effect(regions.drop_first(),initial,path,regions.drop_first().len())},
{
    source_region(regions,0);source_size(regions[0]);
    if crate::morton::proof::value(regions[0].path) as int<=path<validate::dense_end(regions[0]) {effect_head_hit(regions,initial,path,regions.len());}
    else {effect_tail(regions,initial,path,regions.len());}
}
pub(crate) open spec fn expected(regions:Seq<DenseRegion>,part:nat,initial:spec_fn(int)->u32,path:int)->u32 {
    if regions.len()==0 || part==0 {effect(regions,initial,path,regions.len())}
    else if remaining_base(regions,part)<=path<validate::dense_end(regions[0]) {regions[0].values@[path-crate::morton::proof::value(regions[0].path) as int]}
    else {effect(regions.drop_first(),initial,path,regions.drop_first().len())}
}
proof fn low_input_hit(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int)
    requires validate::dense_prefix(regions,regions.len()),regions.len()>0,regions[0].lod<2,ordered::covers(low_head(regions),path),
    ensures effect(regions,initial,path,regions.len())==effect(regions.take(end(regions) as int),initial,path,end(regions)),
{
    hide(effect);low_parts(regions);let limit=end(regions);let group=regions.take(limit as int);let rest=regions.subrange(limit as int,regions.len() as int);
    assert(regions =~= group+rest);effect_concat(group,rest,initial,path);
    effect_absent(rest,|p:int|effect(group,initial,p,group.len()),path,rest.len());
}
proof fn low_input_miss(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int)
    requires validate::dense_prefix(regions,regions.len()),regions.len()>0,regions[0].lod<2,!ordered::covers(low_head(regions),path),
    ensures effect(regions,initial,path,regions.len())==effect(regions.subrange(end(regions) as int,regions.len() as int),initial,path,(regions.len()-end(regions)) as nat),
{
    hide(effect);low_parts(regions);let limit=end(regions);let group=regions.take(limit as int);let rest=regions.subrange(limit as int,regions.len() as int);let base=ordered::base(low_head(regions));
    assert(regions =~= group+rest);effect_concat(group,rest,initial,path);
    assert forall|i:int|0<=i<group.len() implies !(crate::morton::proof::value(#[trigger] group[i].path) as int<=path<crate::morton::proof::value(group[i].path) as int+group[i].values@.len()) by {
        cells_proof::group_address(crate::morton::proof::value(group[i].path),base);assert(patch::leaf_region(group[i]));
    }
    effect_absent(group,initial,path,group.len());effect_same_initial(rest,|p:int|effect(group,initial,p,group.len()),initial,path,rest.len());
}
proof fn low_payload(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int)
    requires validate::dense_prefix(regions,regions.len()),regions.len()>0,regions[0].lod<2,ordered::covers(low_head(regions),path),
    ensures ordered::payload_value(low_head(regions),initial,path)==effect(regions.take(end(regions) as int),initial,path,end(regions)),
{
    hide(effect);low_parts(regions);let group=regions.take(end(regions) as int);let base=ordered::base(low_head(regions));
    leaf_group_value(group,initial,base,path-base);leaf_apply_length(Seq::new(64,|i:int|initial(base+i)),group,group.len());
}
proof fn value_low_join(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int)
    requires validate::dense_prefix(regions,regions.len()),regions.len()>0,regions[0].lod<2,
        stream_state(regions.subrange(end(regions) as int,regions.len() as int),0),
        ordered::stream_effect(stream(regions.subrange(end(regions) as int,regions.len() as int),0),initial,path,stream(regions.subrange(end(regions) as int,regions.len() as int),0).len())
            ==effect(regions.subrange(end(regions) as int,regions.len() as int),initial,path,(regions.len()-end(regions)) as nat),
    ensures ordered::stream_effect(stream(regions,0),initial,path,stream(regions,0).len())==effect(regions,initial,path,regions.len()),
{
    hide(stream);hide(effect);
            low_parts(regions);let limit=end(regions);let group=regions.take(limit as int);let rest=regions.subrange(limit as int,regions.len() as int);
            let head=low_head(regions);let tail=stream(rest,0);
            aligned_end(head);if rest.len()>0{following_base(rest,ordered::end(head));}else{reveal(stream);}
            if ordered::covers(head,path) {
                low_payload(regions,initial,path);low_input_hit(regions,initial,path);cells_proof::stream_head_hit(head,tail,initial,path);
            } else {
                low_input_miss(regions,initial,path);cells_proof::stream_skip_head(head,tail,initial,path);
            }
}
proof fn value_high_join(regions:Seq<DenseRegion>,part:nat,initial:spec_fn(int)->u32,path:int)
    requires validate::dense_prefix(regions,regions.len()),regions.len()>0,regions[0].lod>=2,part<parts(regions[0].lod),
        if part+1==parts(regions[0].lod){stream_state(regions.drop_first(),0)
            && ordered::stream_effect(stream(regions.drop_first(),0),initial,path,stream(regions.drop_first(),0).len())==effect(regions.drop_first(),initial,path,regions.drop_first().len())}
        else {stream_state(regions,part+1) && ordered::stream_effect(stream(regions,part+1),initial,path,stream(regions,part+1).len())==expected(regions,part+1,initial,path)},
    ensures ordered::stream_effect(stream(regions,part),initial,path,stream(regions,part).len())==expected(regions,part,initial,path),
{
            hide(stream);hide(effect);source_region(regions,0);let region=regions[0];let head=parts_proof::edit(region,part);let tail:Seq<EditModel>;
            high_step(regions,part);source_size(region);parts_proof::geometry(region,part);effect_head(regions,initial,path);
            if part+1==parts(region.lod) {
                let rest=regions.drop_first();tail=stream(rest,0);
                if rest.len()>0 {assert(validate::dense_at(regions,1));aligned_end(head);following_base(rest,ordered::end(head));}
                else {reveal(stream);}
            } else {tail=stream(regions,part+1);parts_proof::geometry(region,part+1);}
            assert(stream(regions,part)==seq![head]+tail);
            if ordered::covers(head,path) {
                let width=state::span((region.lod/2) as nat);let count=parts(region.lod);
                assert(0<=part*width && (part+1)*width<=count*width) by(nonlinear_arith) requires part<count,width>0;
                assert(crate::morton::proof::value(region.path) as int<=path<validate::dense_end(region));
                parts_proof::value(region,part,initial,path);
                cells_proof::stream_head_hit(head,tail,initial,path);
                assert(expected(regions,part,initial,path)==ordered::payload_value(head,initial,path));
            }else{
                cells_proof::stream_skip_head(head,tail,initial,path);
                if part+1==parts(region.lod) {assert(expected(regions,part,initial,path)==effect(regions.drop_first(),initial,path,regions.drop_first().len()));}
                else {assert(expected(regions,part,initial,path)==expected(regions,part+1,initial,path));}
            }
}
pub(crate) proof fn stream_value(regions:Seq<DenseRegion>,part:nat,initial:spec_fn(int)->u32,path:int)
    requires validate::dense_prefix(regions,regions.len()),if regions.len()==0{part==0}else{part<parts(regions[0].lod)},
    ensures ordered::stream_effect(stream(regions,part),initial,path,stream(regions,part).len())==expected(regions,part,initial,path),
    decreases regions.len()*8-part,
{
    hide(stream);hide(effect);
    if regions.len()>0 {
        source_region(regions,0);
        if regions[0].lod<2 {
            assert(part==0);low_parts(regions);let limit=end(regions);let rest=regions.subrange(limit as int,regions.len() as int);
            validate::dense_suffix(regions,limit as int);stream_valid(rest,0);stream_value(rest,0,initial,path);value_low_join(regions,initial,path);
        }else{
            if part+1==parts(regions[0].lod){validate::dense_suffix(regions,1);assert(regions.drop_first() =~= regions.subrange(1,regions.len() as int));stream_valid(regions.drop_first(),0);stream_value(regions.drop_first(),0,initial,path);}
            else{stream_valid(regions,part+1);stream_value(regions,part+1,initial,path);}
            value_high_join(regions,part,initial,path);
        }
    }else{reveal(stream);reveal(effect);}
}
pub(crate) proof fn input_stream(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int)
    requires validate::dense_prefix(regions,regions.len()),
    ensures ordered::stream_ordered(stream(regions,0)),ordered::stream_effect(stream(regions,0),initial,path,stream(regions,0).len())==effect(regions,initial,path,regions.len()),
{stream_valid(regions,0);stream_value(regions,0,initial,path);}
impl crate::HashDag {
    pub open(crate) spec fn set_dense_regions_post(&self,before:&Self,root:crate::RootId,regions:&[DenseRegion],result:Result<(),Error>)->bool {
        crate::write::proof::input_written(before,self,root,validate::dense_prefix(regions@,regions@.len()),
            |initial:spec_fn(int)->u32,path:int|effect(regions@,initial,path,regions@.len()),result)
    }
}
pub(crate) open spec fn effect(regions:Seq<DenseRegion>,initial:spec_fn(int)->u32,path:int,count:nat)->u32
    decreases count,
{
    if count==0 {initial(path)} else {
        let region=regions[(count-1) as int];let base=crate::morton::proof::value(region.path) as int;
        if base<=path<base+region.values@.len() {region.values@[path-base]}
        else {effect(regions,initial,path,(count-1) as nat)}
    }
}
impl DenseBatch<'_,'_> {
    pub(crate) open(super) spec fn inputs(&self)->Seq<DenseRegion> {self.0@}
}
impl crate::write::proof::BatchModel for DenseBatch<'_,'_> {
    open(crate) spec fn accepted(&self)->bool {validate::dense_prefix(self.inputs(),self.inputs().len())}
    open(crate) spec fn effect(&self,initial:spec_fn(int)->u32,path:int)->u32 {effect(self.inputs(),initial,path,self.inputs().len())}
}
pub(crate) open spec fn content(pools:state::Pools,child:Child,height:nat)->Seq<u32> {
    Seq::new(state::span(height),|path:int|state::eval(pools,child,height,path))
}
pub(super) open spec fn row_values(pools:state::Pools,row:Seq<Child>,height:nat)->Seq<u32>
    decreases row.len(),
{
    if row.len()==0 {Seq::empty()} else {
        row_values(pools,row.drop_last(),height)+content(pools,row.last(),height)
    }
}
pub(super) proof fn row_push(pools:state::Pools,row:Seq<Child>,height:nat,child:Child)
    ensures row_values(pools,row.push(child),height)==row_values(pools,row,height)+content(pools,child,height),
{
    assert(row.push(child).drop_last() =~= row);
}
pub(super) proof fn row_concat(pools:state::Pools,a:Seq<Child>,b:Seq<Child>,height:nat)
    ensures row_values(pools,a+b,height)==row_values(pools,a,height)+row_values(pools,b,height),
    decreases b.len(),
{
    if b.len()==0 {assert(a+b =~= a);} else {
        row_concat(pools,a,b.drop_last(),height);
        assert((a+b).drop_last() =~= a+b.drop_last());
        assert((row_values(pools,a,height)+row_values(pools,b.drop_last(),height))+content(pools,b.last(),height)
            =~= row_values(pools,a,height)+(row_values(pools,b.drop_last(),height)+content(pools,b.last(),height)));
    }
}
pub(super) open spec fn repeat(values:Seq<u32>,count:nat)->Seq<u32>
    decreases count,
{if count==0 {Seq::empty()} else {repeat(values,(count-1) as nat)+values}}
pub(super) proof fn repeat_length(values:Seq<u32>,count:nat)
    ensures repeat(values,count).len()==values.len()*count,
    decreases count,
{
    reveal_with_fuel(repeat,2);
    if count>0 {
        let n=(count-1) as nat;repeat_length(values,n);
        assert(repeat(values,count).len()==repeat(values,n).len()+values.len());
        assert(values.len()*n+values.len()==values.len()*count) by(nonlinear_arith) requires n+1==count;
    } else {assert(repeat(values,count).len()==0);}
}
pub(super) proof fn repeat_split(values:Seq<u32>,a:nat,b:nat)
    ensures repeat(values,a+b)==repeat(values,a)+repeat(values,b),
    decreases b,
{
    if b>0 {repeat_split(values,a,(b-1) as nat);
        assert((repeat(values,a)+repeat(values,(b-1) as nat))+values =~= repeat(values,a)+(repeat(values,(b-1) as nat)+values));}
}
pub(super) proof fn repeated_row(pools:state::Pools,child:Child,height:nat,count:nat)
    ensures row_values(pools,Seq::new(count,|i:int|child),height)==repeat(content(pools,child,height),count),
    decreases count,
{
    if count>0 {
        repeated_row(pools,child,height,(count-1) as nat);
        assert(Seq::new(count,|i:int|child).drop_last() =~= Seq::new((count-1) as nat,|i:int|child));
    }
}
pub(super) proof fn row_length(pools:state::Pools,row:Seq<Child>,height:nat)
    ensures row_values(pools,row,height).len()==row.len()*state::span(height),
    decreases row.len(),
{
    if row.len()>0 {
        row_length(pools,row.drop_last(),height);
        assert((row.len()-1)*state::span(height)+state::span(height)==row.len()*state::span(height)) by(nonlinear_arith);
    }
}
pub(super) proof fn row_at(pools:state::Pools,row:Seq<Child>,height:nat,slot:int,path:int)
    requires 0<=slot<row.len(),0<=path<state::span(height),
    ensures row_values(pools,row,height)[slot*(state::span(height) as int)+path]==state::eval(pools,row[slot],height,path),
    decreases row.len(),
{
    crate::canonical::span_positive(height);
    row_length(pools,row.drop_last(),height);
    let width=state::span(height) as int;
    if slot<row.len()-1 {
        row_at(pools,row.drop_last(),height,slot,path);
        assert(0<=slot*width+path<(row.len()-1)*width) by(nonlinear_arith)
            requires 0<=slot<row.len()-1,0<=path<width;
    } else {
        assert(slot==row.len()-1);
    }
}
pub(super) proof fn represented_content(pools:state::Pools,level:u8,child:Child,row:Seq<Child>)
    requires state::valid_pools(pools),level<10,represented(pools,level,child,row),row.len()==64,
    ensures state::child_valid(pools,child,(level+1) as nat),
        content(pools,child,(level+1) as nat)==row_values(pools,row,level as nat),
{
    let height=level as nat;
    crate::canonical::span_positive(height);
    row_length(pools,row,height);
    assert forall|path:int|0<=path<state::span(height+1) implies
        #[trigger] content(pools,child,height+1)[path]==row_values(pools,row,height)[path] by {
        let width=state::span(height) as int;
        let slot=path/width;
        let local=path%width;
        vstd::arithmetic::div_mod::lemma_fundamental_div_mod(path,width);
        vstd::arithmetic::div_mod::lemma_mod_bound(path,width);
        assert(0<=slot<64) by(nonlinear_arith)
            requires 0<=path<64*width,width>0,path==slot*width+local,0<=local<width;
        crate::canonical::descend(pools,child,height+1,slot,local);
        assert(crate::canonical::child(pools,child,height+1,slot)==row[slot]);
        row_at(pools,row,height,slot,local);
    }
    assert(content(pools,child,height+1) =~= row_values(pools,row,height));
}
pub(crate) proof fn content_frozen(before:state::Pools,after:state::Pools,child:Child,height:nat)
    requires state::build_step(before,after),state::child_valid(before,child,height),height<=10,
    ensures state::child_valid(after,child,height),content(before,child,height)==content(after,child,height),
{
    crate::canonical::frozen_value(before,after,child,height,0);
    assert forall|path:int|0<=path<state::span(height) implies
        #[trigger] content(before,child,height)[path]==content(after,child,height)[path] by {
        crate::canonical::frozen_value(before,after,child,height,path);
    }
    assert(content(before,child,height) =~= content(after,child,height));
}
pub(super) open spec fn row_valid(pools:state::Pools,row:Seq<Child>,height:nat)->bool {
    forall|i:int|0<=i<row.len() ==> #[trigger] state::child_valid(pools,row[i],height)
}
pub(super) proof fn row_frozen(before:state::Pools,after:state::Pools,row:Seq<Child>,height:nat)
    requires state::build_step(before,after),row_valid(before,row,height),height<=10,
    ensures row_valid(after,row,height),row_values(before,row,height)==row_values(after,row,height),
    decreases row.len(),
{
    if row.len()>0 {
        row_frozen(before,after,row.drop_last(),height);
        content_frozen(before,after,row.last(),height);
        assert forall|i:int|0<=i<row.len() implies #[trigger] state::child_valid(after,row[i],height) by {
            if i<row.len()-1 {assert(row[i]==row.drop_last()[i]);}
            else {assert(row[i]==row.last());}
        }
    }
}
impl DenseFrame {
    pub(super) open spec fn prefix(&self)->Seq<Child> {decoded_slots(self.slots).take(self.len as int)}
    pub(super) open spec fn valid(&self,pools:state::Pools,level:nat)->bool {
        self.len<64 && row_valid(pools,self.prefix(),level) && self.cache.valid(pools,level as u8)
    }
}
pub(super) open spec fn frames_valid(pools:state::Pools,frames:Seq<DenseFrame>)->bool {
    frames.len()<10 && forall|i:int|0<=i<frames.len() ==> #[trigger] frames[i].valid(pools,(i+1) as nat)
}
pub(super) open spec fn empty_below(frames:Seq<DenseFrame>,count:nat)->bool {
    count<=frames.len() && forall|i:int|0<=i<count ==> #[trigger] frames[i].len==0
}
pub(super) open spec fn pending(pools:state::Pools,frames:Seq<DenseFrame>,count:nat)->Seq<u32>
    decreases count,
{
    if count==0 {Seq::empty()} else {
        row_values(pools,frames[count-1].prefix(),count)+pending(pools,frames,(count-1) as nat)
    }
}
pub(super) proof fn pending_empty(pools:state::Pools,frames:Seq<DenseFrame>,count:nat)
    requires empty_below(frames,count),
    ensures pending(pools,frames,count).len()==0,
    decreases count,
{
    if count>0 {pending_empty(pools,frames,(count-1) as nat);}
}
pub(super) proof fn caches_frozen(before:state::Pools,after:state::Pools,cache:DenseCommitCache,level:nat)
    requires state::build_step(before,after),cache.valid(before,level as u8),level<10,
    ensures cache.valid(after,level as u8),
{
    if cache.child is Some {
        match cache.child->Some_0 {
            Child::Leaf(_)=>{},
            Child::Ptr(index)=>{assert(before[level as int].nodes[node::idx(index)]==after[level as int].nodes[node::idx(index)]);},
        }
    }
}
pub(super) proof fn frames_frozen(before:state::Pools,after:state::Pools,frames:Seq<DenseFrame>,count:nat)
    requires state::build_step(before,after),frames_valid(before,frames),count<=frames.len(),
    ensures frames_valid(after,frames),pending(before,frames,count)==pending(after,frames,count),
    decreases count,
{
    assert forall|i:int|0<=i<frames.len() implies #[trigger] frames[i].valid(after,(i+1) as nat) by {
        assert(frames[i].valid(before,(i+1) as nat));
        row_frozen(before,after,frames[i].prefix(),(i+1) as nat);
        caches_frozen(before,after,frames[i].cache,(i+1) as nat);
    }
    if count>0 {
        frames_frozen(before,after,frames,(count-1) as nat);
        assert(frames[(count-1) as int].valid(before,(((count-1) as int)+1) as nat));
        row_frozen(before,after,frames[count-1].prefix(),count);
    }
}
pub(super) proof fn pending_adjust(pools:state::Pools,before:Seq<DenseFrame>,after:Seq<DenseFrame>,count:nat,level:nat,appended:Seq<u32>,carry:Seq<u32>)
    requires before.len()==after.len(),1<=level<=count<=before.len(),
        empty_below(before,(level-1) as nat),empty_below(after,(level-1) as nat),
        forall|i:int|0<=i<before.len() && i!=level-1 ==> #[trigger] after[i].prefix()==before[i].prefix(),
        row_values(pools,after[level-1].prefix(),level)+carry==row_values(pools,before[level-1].prefix(),level)+appended,
    ensures pending(pools,after,count)+carry==pending(pools,before,count)+appended,
    decreases count,
{
    if count==level {
        pending_empty(pools,before,(level-1) as nat);pending_empty(pools,after,(level-1) as nat);
    } else {
        pending_adjust(pools,before,after,(count-1) as nat,level,appended,carry);
        assert(after[count-1].prefix()==before[count-1].prefix());
        let upper=row_values(pools,before[count-1].prefix(),count);
        let left=pending(pools,after,(count-1) as nat);
        let right=pending(pools,before,(count-1) as nat);
        assert((upper+left)+carry =~= upper+(left+carry));
        assert((upper+right)+appended =~= upper+(right+appended));
    }
}
pub(super) proof fn append_pending(pools:state::Pools,before:Seq<DenseFrame>,after:Seq<DenseFrame>,level:nat,appended:Seq<u32>)
    requires frames_valid(pools,before),before.len()==after.len(),1<=level<=before.len(),empty_below(before,(level-1) as nat),
        forall|i:int|0<=i<before.len() && i!=level-1 ==> #[trigger] after[i]==before[i],
        after[level-1].cache==before[level-1].cache,after[level-1].len==before[level-1].len+1,after[level-1].len<64,
        exists|child:Child|state::child_valid(pools,child,level) && after[level-1].prefix()==before[level-1].prefix().push(child) && appended==content(pools,child,level),
    ensures frames_valid(pools,after),pending(pools,after,after.len())==pending(pools,before,before.len())+appended,
{
    let child=choose|child:Child|state::child_valid(pools,child,level) && after[level-1].prefix()==before[level-1].prefix().push(child) && appended==content(pools,child,level);
    assert(before[(level-1) as int].valid(pools,(((level-1) as int)+1) as nat));
    assert forall|i:int|0<=i<after.len() implies #[trigger] after[i].valid(pools,(i+1) as nat) by {
        if i!=level-1 {assert(after[i]==before[i]);assert(before[i].valid(pools,(i+1) as nat));}
        else {
            assert forall|slot:int|0<=slot<after[i].len implies #[trigger] state::child_valid(pools,after[i].prefix()[slot],level) by {
                if slot<before[i].len {assert(after[i].prefix()[slot]==before[i].prefix()[slot]);}
                else {assert(after[i].prefix()[slot]==child);}
            }
        }
    }
    row_push(pools,before[level-1].prefix(),level,child);
    pending_adjust(pools,before,after,after.len(),level,appended,Seq::empty());
}
pub(super) proof fn carry_pending(before_pools:state::Pools,after_pools:state::Pools,before:Seq<DenseFrame>,after:Seq<DenseFrame>,level:u8,appended:Seq<u32>,child:Child,row:Seq<Child>)
    requires state::build_step(before_pools,after_pools),frames_valid(before_pools,before),before.len()==after.len(),1<=level<=before.len(),empty_below(before,(level-1) as nat),
        forall|i:int|0<=i<before.len() && i!=level-1 ==> #[trigger] after[i]==before[i],
        after[(level-1) as int].len==0,after[(level-1) as int].cache.valid(after_pools,level),
        row.len()==64,represented(after_pools,level,child,row),
        exists|added:Child|state::child_valid(before_pools,added,level as nat) && row==before[(level-1) as int].prefix().push(added) && appended==content(before_pools,added,level as nat),
    ensures frames_valid(after_pools,after),empty_below(after,level as nat),
        pending(after_pools,after,after.len())+content(after_pools,child,(level+1) as nat)==pending(before_pools,before,before.len())+appended,
{
    let added=choose|added:Child|state::child_valid(before_pools,added,level as nat) && row==before[(level-1) as int].prefix().push(added) && appended==content(before_pools,added,level as nat);
    frames_frozen(before_pools,after_pools,before,before.len());content_frozen(before_pools,after_pools,added,level as nat);
    assert forall|i:int|0<=i<after.len() implies #[trigger] after[i].valid(after_pools,(i+1) as nat) by {
        if i!=level-1 {assert(after[i]==before[i]);assert(before[i].valid(after_pools,(i+1) as nat));}
    }
    represented_content(after_pools,level,child,row);
    row_push(after_pools,before[(level-1) as int].prefix(),level as nat,added);
    pending_adjust(after_pools,before,after,after.len(),level as nat,content(after_pools,added,level as nat),content(after_pools,child,(level+1) as nat));
}
pub(super) proof fn reset_valid(before_pools:state::Pools,after_pools:state::Pools,before:Seq<DenseFrame>,after:Seq<DenseFrame>,level:u8)
    requires state::build_step(before_pools,after_pools),frames_valid(before_pools,before),before.len()==after.len(),1<=level<=before.len(),
        forall|i:int|0<=i<before.len() && i!=level-1 ==> #[trigger] after[i]==before[i],
        after[(level-1) as int].len==0,after[(level-1) as int].cache.valid(after_pools,level),
    ensures frames_valid(after_pools,after),
{
    frames_frozen(before_pools,after_pools,before,before.len());
    assert forall|i:int|0<=i<after.len() implies #[trigger] after[i].valid(after_pools,(i+1) as nat) by {
        if i!=level-1 {assert(after[i]==before[i]);assert(before[i].valid(after_pools,(i+1) as nat));}
    }
}
pub(super) proof fn fill_pending(pools:state::Pools,before:Seq<DenseFrame>,after:Seq<DenseFrame>,child:Child,count:nat)
    requires frames_valid(pools,before),before.len()==after.len()>0,state::child_valid(pools,child,1),
        forall|i:int|1<=i<before.len() ==> #[trigger] after[i]==before[i],
        after[0].cache==before[0].cache,after[0].len==before[0].len+count,after[0].len<64,
        after[0].prefix()==before[0].prefix()+Seq::new(count,|i:int|child),
    ensures frames_valid(pools,after),pending(pools,after,after.len())==pending(pools,before,before.len())+repeat(content(pools,child,1),count),
{
    assert(before[0].valid(pools,(0int+1) as nat));
    assert forall|i:int|0<=i<after.len() implies #[trigger] after[i].valid(pools,(i+1) as nat) by {
        if i>0 {assert(after[i]==before[i]);assert(before[i].valid(pools,(i+1) as nat));}
        else {
            assert forall|slot:int|0<=slot<after[0].len implies #[trigger] state::child_valid(pools,after[0].prefix()[slot],1) by {
                if slot<before[0].len {assert(after[0].prefix()[slot]==before[0].prefix()[slot]);}
                else {assert(after[0].prefix()[slot]==child);}
            }
        }
    }
    row_concat(pools,before[0].prefix(),Seq::new(count,|i:int|child),1);
    repeated_row(pools,child,1,count);
    pending_adjust(pools,before,after,after.len(),1,repeat(content(pools,child,1),count),Seq::empty());
}
pub(super) proof fn fill_carry(before_pools:state::Pools,after_pools:state::Pools,before:Seq<DenseFrame>,after:Seq<DenseFrame>,child:Child,count:nat,parent:Child,row:Seq<Child>)
    requires state::build_step(before_pools,after_pools),frames_valid(before_pools,before),before.len()==after.len()>0,
        state::child_valid(before_pools,child,1),row==before[0].prefix()+Seq::new(count,|i:int|child),row.len()==64,
        forall|i:int|1<=i<before.len() ==> #[trigger] after[i]==before[i],
        after[0].len==0,after[0].cache.valid(after_pools,1),represented(after_pools,1,parent,row),
    ensures frames_valid(after_pools,after),empty_below(after,1),
        pending(after_pools,after,after.len())+content(after_pools,parent,2)==pending(before_pools,before,before.len())+repeat(content(before_pools,child,1),count),
{
    reset_valid(before_pools,after_pools,before,after,1);
    frames_frozen(before_pools,after_pools,before,before.len());content_frozen(before_pools,after_pools,child,1);
    represented_content(after_pools,1,parent,row);
    row_concat(after_pools,before[0].prefix(),Seq::new(count,|i:int|child),1);
    repeated_row(after_pools,child,1,count);
    pending_adjust(after_pools,before,after,after.len(),1,repeat(content(after_pools,child,1),count),content(after_pools,parent,2));
}
pub(super) proof fn append_repeat(prefix:Seq<u32>,values:Seq<u32>,count:nat,consumed:nat)
    requires consumed<=count,
    ensures (prefix+repeat(values,consumed))+repeat(values,(count-consumed) as nat)==prefix+repeat(values,count),
{
    repeat_split(values,consumed,(count-consumed) as nat);
    assert((prefix+repeat(values,consumed))+repeat(values,(count-consumed) as nat) =~= prefix+(repeat(values,consumed)+repeat(values,(count-consumed) as nat)));
}
pub(super) proof fn leaf_content(pools:state::Pools,child:Child,values:Seq<u32>)
    requires state::valid_pools(pools),values.len()==64,represented(pools,0,child,leaf_row(values)),
    ensures state::child_valid(pools,child,1),content(pools,child,1)==values,
{
    reveal_with_fuel(state::span,2);
    represented_content(pools,0,child,leaf_row(values));
    row_length(pools,leaf_row(values),0);
    assert forall|i:int|0<=i<64 implies #[trigger] row_values(pools,leaf_row(values),0)[i]==values[i] by {
        row_at(pools,leaf_row(values),0,i,0);
    }
    assert(row_values(pools,leaf_row(values),0) =~= values);
}
pub(super) proof fn uniform_content(pools:state::Pools,values:Seq<u32>,height:nat,value:u32)
    requires values.len()==state::span(height),crate::wide::proof::uniform(values)==Some(value),
    ensures content(pools,Child::Leaf(value),height)==values,
{
    assert(content(pools,Child::Leaf(value),height) =~= values);
}
pub(super) proof fn pending_bound(pools:state::Pools,frames:Seq<DenseFrame>,count:nat)
    requires frames_valid(pools,frames),count<=frames.len(),
    ensures pending(pools,frames,count).len()<state::span(count+1),
    decreases count,
{
    crate::canonical::span_positive(count);
    if count>0 {
        pending_bound(pools,frames,(count-1) as nat);
        assert(frames[(count-1) as int].valid(pools,(((count-1) as int)+1) as nat));
        row_length(pools,frames[count-1].prefix(),count);
        assert(frames[count-1].prefix().len()==frames[count-1].len);
        let width=state::span(count);
        assert(frames[count-1].len*width+pending(pools,frames,(count-1) as nat).len()<64*width) by(nonlinear_arith)
            requires frames[count-1].len<64,pending(pools,frames,(count-1) as nat).len()<width,width>0;
    } else {reveal_with_fuel(state::span,2);}
}
pub(super) open spec fn groups_match(values:Seq<u32>,groups:Seq<[u32;64]>)->bool {
    groups.len()*64==values.len() && forall|i:int,j:int|0<=i<groups.len() && 0<=j<64 ==> #[trigger] groups[i]@[j]==values[i*64+j]
}
pub(super) proof fn groups_shape(values:Seq<u32>,groups:Seq<[u32;64]>,height:nat)
    requires 0<height<=10,values.len()==state::span(height),groups.len()==values.len()/64,
        forall|i:int,j:int|0<=i<groups.len() && 0<=j<64 ==> #[trigger] groups[i]@[j]==values[i*64+j],
    ensures groups_match(values,groups),groups.len()>0,height==1 ==> groups.len()==1,
{
    crate::canonical::span_positive((height-1) as nat);
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod_converse(values.len() as int,64,state::span((height-1) as nat) as int,0);
    if height==1 {reveal_with_fuel(state::span,2);}
}
pub(super) proof fn input_group(values:Seq<u32>,groups:Seq<[u32;64]>,position:nat)
    requires groups_match(values,groups),position<groups.len(),
    ensures values.take(((position+1)*64) as int)==values.take((position*64) as int)+groups[position as int]@,
{
    assert(values.take(((position+1)*64) as int) =~= values.take((position*64) as int)+groups[position as int]@);
}
pub(super) proof fn input_run(values:Seq<u32>,groups:Seq<[u32;64]>,start:nat,count:nat,group:Seq<u32>)
    requires groups_match(values,groups),start+count<=groups.len(),
        forall|i:int|start<=i<start+count ==> #[trigger] groups[i]@==group,
    ensures values.take(((start+count)*64) as int)==values.take((start*64) as int)+repeat(group,count),
    decreases count,
{
    if count>0 {
        input_run(values,groups,start,(count-1) as nat,group);
        input_group(values,groups,(start+count-1) as nat);
        assert(groups[(start+count-1) as int]@==group);
        assert((values.take((start*64) as int)+repeat(group,(count-1) as nat))+group =~= values.take((start*64) as int)+(repeat(group,(count-1) as nat)+group));
    }
}
pub(crate) open spec fn leaf_row(values: Seq<u32>) -> Seq<Child> {
    Seq::new(64, |i:int| Child::Leaf(values[i]))
}
pub(crate) open spec fn decoded_slots(slots: [EncodedChild; 64]) -> Seq<Child> {
    Seq::new(64, |i:int| node::decode(slots@[i]))
}
pub(crate) open spec fn represented(pools: state::Pools, level: u8, child: Child, row: Seq<Child>) -> bool {
    level < pools.len() && commit::present(pools[level as int], child)
        && commit::image(pools[level as int], child) == row
}
pub(crate) open spec fn row_domain(pools: state::Pools, level:u8, row:Seq<Child>) -> bool {
    row.len() == 64 && forall|i:int| 0 <= i < 64 ==> #[trigger] state::child_valid(pools,row[i],level as nat)
}
impl DenseCommitCache {
    pub(super) open spec fn valid(&self, pools:state::Pools, level:u8) -> bool {
        match self.child { None => true, Some(child) => represented(pools,level,child,decoded_slots(self.slots)) }
    }
}
pub(super) proof fn leaf_entry(entry: PoolEntry)
    requires node::mask(entry.mask) == 0,
    ensures node::children(entry) == leaf_row(node::values(entry.values)),
{
    assert forall|i:int| 0 <= i < 64 implies #[trigger] node::children(entry)[i] == Child::Leaf(node::values(entry.values)[i]) by {
        let shift = i as u64;
        assert(0u64 & (1u64 << shift) == 0) by(bit_vector);
    }
    assert(node::children(entry) =~= leaf_row(node::values(entry.values)));
}
pub(super) proof fn uniform_leaf(values: Seq<u32>, value: u32)
    requires values.len() == 64, crate::wide::proof::uniform(values) == Some(value),
    ensures leaf_row(values) == Seq::new(64, |i:int| Child::Leaf(value)),
{
    assert(leaf_row(values) =~= Seq::new(64, |i:int| Child::Leaf(value)));
}
pub(crate) open spec fn parts(lod:u8) -> nat { if lod < 2 || lod % 2 == 0 { 1 } else { 8 } }
pub(super) open spec fn group(regions: Seq<DenseRegion>, limit:nat) -> bool {
    0 < limit <= regions.len() && if regions[0].lod < 2 {
        (forall|i:int| 0 <= i < limit ==> #[trigger] regions[i].lod < 2
            && crate::morton::proof::value(regions[i].path) >> 6 == crate::morton::proof::value(regions[0].path) >> 6)
            && (limit == regions.len() || regions[limit as int].lod >= 2
                || crate::morton::proof::value(regions[limit as int].path) >> 6 != crate::morton::proof::value(regions[0].path) >> 6)
    } else { limit == 1 }
}
pub(super) open spec fn end(regions:Seq<DenseRegion>) -> nat { choose|limit:nat| group(regions,limit) }
pub(super) proof fn group_found(regions: Seq<DenseRegion>, limit:nat)
    requires group(regions,limit),
    ensures end(regions)==limit,
{
    assert(exists|candidate:nat| group(regions,candidate));
    let selected = end(regions);
    assert(group(regions,selected));
    if regions[0].lod < 2 {
        if selected < limit { assert(regions[selected as int].lod < 2);
            assert(crate::morton::proof::value(regions[selected as int].path) >> 6 == crate::morton::proof::value(regions[0].path) >> 6); }
        else if limit < selected { assert(regions[limit as int].lod < 2);
            assert(crate::morton::proof::value(regions[limit as int].path) >> 6 == crate::morton::proof::value(regions[0].path) >> 6); }
    }
}
pub(crate) open spec fn path(code:u64,lod:u8,part:u64)->u64 {
    (code + (part << ((lod / 2) as u32 * 6))) as u64
}
pub(crate) proof fn path_safe(code:u64,lod:u8,part:u64)
    requires code < 0x1000_0000_0000_0000u64, lod <= 20, part < parts(lod),
        code & ((validate::size(lod) - 1) as u64) == 0,
    ensures code as int + ((part << ((lod / 2) as u32 * 6)) as int) < 0x1000_0000_0000_0000u64,
        path(code,lod,part) < 0x1000_0000_0000_0000u64,
{
    super::super::fill_uniform::proof::path_safe(code,lod,part);
    assert(code & !((validate::size(lod) - 1) as u64) == code) by(bit_vector)
        requires code & ((validate::size(lod) - 1) as u64) == 0;
}
pub(super) proof fn segment(len:nat, parts:nat, part:nat)
    requires parts > 0, part < parts,
    ensures part * (len / parts) + len / parts <= len,
        part * (len / parts) + len / parts == (part + 1) * (len / parts),
{
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(len as int,parts as int);
    assert(parts * (len / parts) <= len);
    assert((part + 1) * (len / parts) <= parts * (len / parts)) by(nonlinear_arith)
        requires 0 <= part < parts, 0 <= len / parts;
    let width = len / parts;
    assert(part * width + width == (part + 1) * width) by(nonlinear_arith);
}
pub(super) proof fn segment_values(values:Seq<u32>, parts:nat, part:nat)
    requires parts > 0, part < parts,
    ensures values.subrange((part*(values.len()/parts)) as int, values.len() as int)
        .subrange(0,(values.len()/parts) as int)
        == values.subrange((part*(values.len()/parts)) as int, ((part+1)*(values.len()/parts)) as int),
{
    segment(values.len(),parts,part);
    assert(values.subrange((part*(values.len()/parts)) as int, values.len() as int)
        .subrange(0,(values.len()/parts) as int)
        =~= values.subrange((part*(values.len()/parts)) as int, ((part+1)*(values.len()/parts)) as int));
}
pub(crate) open(super) spec fn stream(regions:Seq<DenseRegion>,part:nat)->Seq<EditModel>
    decreases (regions.len()*8-part) as nat,
{
    if regions.len()==0 { Seq::empty() } else {
        let region=regions[0]; let limit=end(regions); let count=parts(region.lod);
        if 0 < limit <= regions.len() && part < count && region.lod <= 20 {
            let group=regions.subrange(0,limit as int);
            let size=region.values@.len()/count;
            seq![EditModel {path:if region.lod<2 {crate::morton::proof::value(region.path)} else {path(crate::morton::proof::value(region.path),region.lod,part as u64)},
                lod:if region.lod<2 {1} else {region.lod/2},
                payload:if region.lod<2 {Payload::Patch(|before:Seq<u32>|patch::dense_apply(before,group,group.len()))}
                    else {Payload::Dense(region.values@.subrange((part*size) as int,((part+1)*size) as int))}}]
                + if part+1==count {stream(regions.subrange(limit as int,regions.len() as int),0)} else {stream(regions,part+1)}
        } else {Seq::empty()}
    }
}
impl DenseEdits<'_,'_> {
    pub(super) open spec fn valid(&self)->bool {
        validate::dense_prefix(self.pending@,self.pending@.len())
            && if self.pending@.len()==0 {self.part==0} else {self.part<parts(self.pending@[0].lod)}
    }
    pub(super) open spec fn stream(&self)->Seq<EditModel> {stream(self.pending@,self.part as nat)}
}
impl StreamModel for DenseEdits<'_,'_> {
    open(super) spec fn stream_valid(&self)->bool {self.valid()}
    open(super) spec fn remaining(&self)->Seq<EditModel> {self.stream()}
}
}
