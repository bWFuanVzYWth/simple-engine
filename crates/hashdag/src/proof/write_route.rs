//! 单项路由以活动祖先链解释候选值；不持有图的执行副本。
use super::*;
use crate::state;
use crate::write::{
    frontier::proof as frontier, ordered::proof as ordered, resolve::proof as resolve,
};
use vstd::arithmetic::div_mod::*;
use vstd::prelude::*;
#[path = "write_route_kept.rs"]
pub(in crate::write) mod kept;
verus! {
broadcast use vstd::seq_lib::group_seq_properties;
pub(in crate::write) open(in crate::write) spec fn ready<P:LeafPatch>(pools:state::Pools,root:Child,edit:Edit<P>,replacement:Replacement<P>,scratch:&WriteScratch)->bool {
    state::valid_pools(pools) && state::all_occupied_live(pools)
        && (forall|level:int|0<=level<10 ==> #[trigger] pools[level].constructing)
        && state::child_valid(pools,root,10) && frontier::chain_ready(pools,scratch.frames@)
        && ordered::edit_ready(edit) && ordered::edit_shape(ordered::edit_view(edit))
        && ordered::replacement_valid(replacement) && resolve::selected(edit,replacement,pools)
        && (edit.lod==10 ==> scratch.frames@.len()==0)
}
pub(in crate::write) open(in crate::write) spec fn routed<P:LeafPatch>(before:state::Pools,after:state::Pools,old_root:Child,new_root:Child,edit:Edit<P>,old:&WriteScratch,new:&WriteScratch)->bool {
    effected(before,after,old_root,new_root,old.frames@,new.frames@,edit)
}
pub(in crate::write) open(in crate::write) spec fn effected<P:LeafPatch>(before:state::Pools,after:state::Pools,old_root:Child,new_root:Child,old:Seq<BuildFrame>,new:Seq<BuildFrame>,edit:Edit<P>)->bool {
    state::child_valid(after,new_root,10) && frontier::chain_ready(after,new)
        && forall|point:int|0<=point<state::span(10) ==>
            #[trigger] frontier::candidate_value(after,new_root,new,point)==if ordered::covers(ordered::edit_view(edit),point) {
                ordered::payload_value(ordered::edit_view(edit),|p:int|frontier::candidate_value(before,old_root,old,p),point)
            }else{frontier::candidate_value(before,old_root,old,point)}
}
pub(in crate::write) open(in crate::write) spec fn preserved(before:state::Pools,after:state::Pools,root:Child,old:Seq<BuildFrame>,new:Seq<BuildFrame>)->bool {
    state::child_valid(after,root,10) && frontier::chain_ready(after,new)
        && forall|point:int|0<=point<state::span(10) ==> #[trigger] frontier::candidate_value(after,root,new,point)==frontier::candidate_value(before,root,old,point)
}
pub(in crate::write) open(in crate::write) spec fn at_edit<P:LeafPatch>(frames:Seq<BuildFrame>,edit:Edit<P>)->bool {
    frames.len()>0 && frames.last().pool_lod>=edit.lod
        && frontier::covers(frames.last(),crate::morton::proof::value(edit.path) as int)
}
pub(in crate::write) open(in crate::write) spec fn needs_close(frames:Seq<BuildFrame>,path:ZOrder20x3,lod:u8)->bool {
    frames.len()>0 && (frames.last().pool_lod<lod || !frontier::contains(frames.last(),path))
}
pub(in crate::write) proof fn root_opened(pools:state::Pools,root:Child,frame:BuildFrame)
    requires state::valid_pools(pools),state::child_valid(pools,root,10),frame.pool_lod==9,
        crate::morton::proof::value(frame.path)==0,frame.original==root,!frame.any_modified,
        crate::node::proof::children(frame.entry)==crate::write::commit::proof::image(pools[9],root),
    ensures preserved(pools,pools,root,Seq::empty(),seq![frame]),
{
    frontier::span_bits(10);
    assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
    frontier::root_opened(pools,root,frame,0);
    assert forall|point:int|0<=point<state::span(10) implies #[trigger] frontier::candidate_value(pools,root,seq![frame],point)
        ==frontier::candidate_value(pools,root,Seq::empty(),point) by {
        frontier::root_opened(pools,root,frame,point as u64);
    }
}
pub(in crate::write) proof fn preserved_transitive(a:state::Pools,b:state::Pools,c:state::Pools,root:Child,af:Seq<BuildFrame>,bf:Seq<BuildFrame>,cf:Seq<BuildFrame>)
    requires preserved(a,b,root,af,bf),preserved(b,c,root,bf,cf),
    ensures preserved(a,c,root,af,cf),
{}
pub(in crate::write) proof fn can_close<P:LeafPatch>(pools:state::Pools,frames:Seq<BuildFrame>,edit:Edit<P>)
    requires frontier::chain_ready(pools,frames),frames.len()>0,ordered::edit_shape(ordered::edit_view(edit)),edit.lod<10,
        needs_close(frames,edit.path,edit.lod),
    ensures frames.len()>=2,
{
    if frames.len()==1 {
        assert(frontier::frame_ready(pools,frames[0]));
        assert(frames[0].pool_lod==9);
        frontier::path_digits(crate::morton::proof::value(edit.path),10);
        frontier::span_bits(10);
        assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
        lemma_fundamental_div_mod_converse(crate::morton::proof::value(edit.path) as int,state::span(10) as int,0,crate::morton::proof::value(edit.path) as int);
        assert(frontier::covers(frames[0],crate::morton::proof::value(edit.path) as int));
        frontier::contains_model(frames[0],edit.path);
    }
}
pub(in crate::write) proof fn close_preserved(before:state::Pools,after:state::Pools,root:Child,old:Seq<BuildFrame>,new:Seq<BuildFrame>,child:Child)
    requires state::build_step(before,after),state::child_valid(before,root,10),frontier::chain_ready(before,old),old.len()>1,
        frontier::chain_ready(after,new),new.len()+1==old.len(),
        forall|code:u64|code<0x1000_0000_0000_0000u64 ==> #[trigger] frontier::closed_value(before,after,old,new,child,code),
    ensures preserved(before,after,root,old,new),
{
    crate::canonical::frozen_value(before,after,root,10,0);
    frontier::span_bits(10);
    assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
    assert forall|point:int|0<=point<state::span(10) implies #[trigger] frontier::candidate_value(after,root,new,point)
        ==frontier::candidate_value(before,root,old,point) by {
        assert(frontier::closed_value(before,after,old,new,child,point as u64));
    }
}
pub(in crate::write) proof fn opening_preserved(pools:state::Pools,root:Child,frames:Seq<BuildFrame>,child:BuildFrame)
    requires state::valid_pools(pools),state::child_valid(pools,root,10),frontier::chain_ready(pools,frames),frames.len()>0,
        frontier::fresh_child(frames.last(),child,pools),
    ensures preserved(pools,pools,root,frames,frames.push(child)),
{
    frontier::opened_chain(pools,frames,child);
    frontier::span_bits(10);
    assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
    assert forall|point:int|0<=point<state::span(10) implies #[trigger] frontier::candidate_value(pools,root,frames.push(child),point)
        ==frontier::candidate_value(pools,root,frames,point) by {frontier::opening_value(pools,root,frames,child,point as u64);}
}
pub(in crate::write) proof fn aligned_child<P:LeafPatch>(pools:state::Pools,frames:Seq<BuildFrame>,edit:Edit<P>,child:BuildFrame)
    requires state::valid_pools(pools),frontier::chain_ready(pools,frames),at_edit(frames,edit),frames.last().pool_lod>edit.lod,
        child.pool_lod+1==frames.last().pool_lod,!child.any_modified,
        crate::morton::proof::value(child.path)==crate::morton::proof::value(edit.path)
            & !(((1u64<<(frames.last().pool_lod as u32*6))-1) as u64),
        child.original==crate::node::proof::children(frames.last().entry)[frontier::slot(edit.path,frames.last().pool_lod)],
        crate::node::proof::children(child.entry)==crate::write::commit::proof::image(pools[child.pool_lod as int],child.original),
    ensures frontier::fresh_child(frames.last(),child,pools),at_edit(frames.push(child),edit),
{
    let level=frames.last().pool_lod;
    assert(frontier::frame_ready(pools,frames.last()));
    let shift=(level as u32*6) as u32;
    let path=crate::morton::proof::value(edit.path);
    let aligned=crate::morton::proof::value(child.path);
    assert(aligned>>shift==path>>shift) by(bit_vector)
        requires shift<64,aligned==path & !(((1u64<<shift)-1) as u64);
    frontier::path_digits(path,level);frontier::path_digits(aligned,level);
    let width=state::span(level as nat) as int;
    crate::canonical::span_positive(level as nat);
    lemma_div_denominator(path as int,width,64);lemma_div_denominator(aligned as int,width,64);
}
pub(in crate::write) proof fn leaf_value(pools:state::Pools,child:Child,offset:int)
    requires state::valid_pools(pools),state::child_valid(pools,child,1),0<=offset<64,
    ensures ordered::leaf_values(pools,child)[offset]==state::eval(pools,child,1,offset),
{
    reveal_with_fuel(state::eval,2);
    assert(state::span(0)==1);
    assert(state::span(1)==64);
    ordered::leaf_image(pools,child);
    match child {
        Child::Leaf(_)=>{},
        Child::Ptr(index)=>{
            assert(state::occupied(pools[0],crate::node::proof::idx(index)));
            assert(pools[0].nodes[crate::node::proof::idx(index)]->Some_0[offset] is Leaf);
        },
    }
}
pub(in crate::write) proof fn resolved_value<P:LeafPatch>(before:state::Pools,after:state::Pools,edit:Edit<P>,replacement:Replacement<P>,original:Child,child:Child,initial:spec_fn(int)->u32)
    requires state::build_step(before,after),ordered::edit_ready(edit),ordered::edit_shape(ordered::edit_view(edit)),
        resolve::selected(edit,replacement,before),ordered::resolved(before,after,replacement,original,Ok(child)),
        state::child_valid(before,original,edit.lod as nat),
        forall|offset:int|0<=offset<state::span(edit.lod as nat) ==>
            #[trigger] initial(ordered::base(ordered::edit_view(edit))+offset)==state::eval(before,original,edit.lod as nat,offset),
    ensures state::child_valid(after,child,edit.lod as nat),
        forall|offset:int|0<=offset<state::span(edit.lod as nat) ==>
            #[trigger] state::eval(after,child,edit.lod as nat,offset)
                ==ordered::payload_value(ordered::edit_view(edit),initial,ordered::base(ordered::edit_view(edit))+offset),
{
    match replacement {
        Replacement::Value(value)=>{},
        Replacement::Patch(patch)=>{
            assert(edit.lod==1);
            reveal_with_fuel(state::span,2);
            assert(state::span(1)==64);
            assert forall|offset:int|0<=offset<64 implies #[trigger] ordered::leaf_values(before,original)[offset]
                ==initial(ordered::base(ordered::edit_view(edit))+offset) by {leaf_value(before,original,offset);}
            assert(ordered::leaf_values(before,original) =~= Seq::new(64,|i:int|initial(ordered::base(ordered::edit_view(edit))+i)));
            let changed=patch.effect(ordered::leaf_values(before,original));
            assert(changed.len()==64);
            assert(crate::write::commit::proof::image(after[0],child)==Seq::new(64,|i:int|Child::Leaf(changed[i])));
            ordered::leaf_image(after,child);
            assert forall|i:int|0<=i<64 implies #[trigger] ordered::leaf_values(after,child)[i]==changed[i] by {
                let row=crate::write::commit::proof::image(after[0],child);
                assert(row[i]==Child::Leaf(changed[i]));
                assert(row[i]==Child::Leaf(ordered::leaf_values(after,child)[i]));
                assert(Child::Leaf(ordered::leaf_values(after,child)[i])==Child::Leaf(changed[i]));
            }
            assert(ordered::leaf_values(after,child) =~= changed);
            assert forall|offset:int|0<=offset<64 implies #[trigger] state::eval(after,child,1,offset)
                ==ordered::payload_value(ordered::edit_view(edit),initial,ordered::base(ordered::edit_view(edit))+offset) by {
                leaf_value(after,child,offset);
            }
        },
    }
}

pub(in crate::write) proof fn selected_frozen<P:LeafPatch>(before:state::Pools,after:state::Pools,edit:Edit<P>,replacement:Replacement<P>)
    requires state::build_step(before,after),ordered::edit_ready(edit),resolve::selected(edit,replacement,before),
    ensures resolve::selected(edit,replacement,after),
{
    if let Replacement::Value(child)=replacement {
        crate::canonical::frozen_value(before,after,child,edit.lod as nat,0);
        assert forall|initial:spec_fn(int)->u32,offset:int|0<=offset<state::span(edit.lod as nat) implies
            #[trigger] ordered::payload_value(ordered::edit_view(edit),initial,ordered::base(ordered::edit_view(edit))+offset)
                ==state::eval(after,child,edit.lod as nat,offset) by {
            crate::canonical::frozen_value(before,after,child,edit.lod as nat,offset);
        }
    }
}

pub(in crate::write) proof fn edit_geometry<P:LeafPatch>(edit:Edit<P>,frame:BuildFrame,point:u64)
    requires ordered::edit_shape(ordered::edit_view(edit)),frame.pool_lod==edit.lod,edit.lod<10,
        frontier::covers(frame,crate::morton::proof::value(edit.path) as int),
    ensures ordered::covers(ordered::edit_view(edit),point as int) <==> frontier::covers(frame,point as int)
        && ((point>>(edit.lod as u32*6))%64) as int==frontier::slot(edit.path,edit.lod),
{
    let path=crate::morton::proof::value(edit.path);
    let width=state::span(edit.lod as nat) as int;
    crate::canonical::span_positive(edit.lod as nat);
    frontier::path_digits(point,edit.lod);
    frontier::path_digits(path,edit.lod);
    lemma_div_denominator(point as int,width,64);
    lemma_div_denominator(path as int,width,64);
    lemma_fundamental_div_mod(point as int,width);
    lemma_fundamental_div_mod(path as int,width);
    lemma_fundamental_div_mod(point as int/width,64);
    lemma_fundamental_div_mod(path as int/width,64);
    lemma_mod_bound(point as int,width);
    lemma_mod_bound(path as int,width);
    assert(((point as int)>=(path as int)/width*width && (point as int)<(path as int)/width*width+width)
        <==> point as int/width==path as int/width) by(nonlinear_arith)
        requires width>0,point as int==point as int/width*width+point as int%width,0<=point as int%width<width;
}

pub(in crate::write) proof fn chosen_value<P:LeafPatch>(pools:state::Pools,root:Child,frames:Seq<BuildFrame>,edit:Edit<P>,offset:int)
    requires state::valid_pools(pools),frontier::chain_ready(pools,frames),frames.len()>0,
        ordered::edit_shape(ordered::edit_view(edit)),frames.last().pool_lod==edit.lod,edit.lod<10,
        frontier::covers(frames.last(),crate::morton::proof::value(edit.path) as int),0<=offset<state::span(edit.lod as nat),
    ensures frontier::candidate_value(pools,root,frames,ordered::base(ordered::edit_view(edit))+offset)
        ==state::eval(pools,crate::node::proof::children(frames.last().entry)[frontier::slot(edit.path,edit.lod)],edit.lod as nat,offset),
{
    let width=state::span(edit.lod as nat) as int;
    let path=crate::morton::proof::value(edit.path);
    let point=ordered::base(ordered::edit_view(edit))+offset;
    crate::canonical::span_positive(edit.lod as nat);
    frontier::path_digits(path,edit.lod);
    lemma_fundamental_div_mod_converse(point,width,path as int/width,offset);
    lemma_div_denominator(point,width,64);
    lemma_div_denominator(path as int,width,64);
    assert(frontier::covers(frames.last(),point));
    frontier::frontier_last(pools,frames,0,point);
    let local=point%(width*64);
    let digit=path as int/width%64;
    lemma_mod_breakdown(point,width,64);
    lemma_mod_bound(path as int/width,64);
    assert(local==digit*width+offset) by(nonlinear_arith) requires local==width*digit+offset;
    lemma_fundamental_div_mod_converse(local,width,digit,offset);
    lemma_small_mod(digit as nat,64);
}
proof fn span_sum(a:nat,b:nat)
    ensures state::span(a+b)==state::span(a)*state::span(b),
    decreases b,
{
    if b>0 {
        span_sum(a,(b-1) as nat);
        assert(state::span(a+b)==state::span(a)*state::span(b)) by(nonlinear_arith)
            requires state::span(a+b)==64*state::span((a+b-1) as nat),state::span(b)==64*state::span((b-1) as nat),
                state::span((a+b-1) as nat)==state::span(a)*state::span((b-1) as nat);
    }else{
        assert(state::span(0)==1);
        assert(state::span(a+b)==state::span(a));
    }
}
pub(in crate::write) proof fn scope_bounds(edit:ordered::EditModel)
    requires ordered::edit_shape(edit),
    ensures 0<=ordered::base(edit)<ordered::end(edit)<=state::span(10),
{
    let width=state::span(edit.lod as nat) as int;
    let count=state::span((10-edit.lod) as nat) as int;
    crate::canonical::span_positive(edit.lod as nat);
    crate::canonical::span_positive((10-edit.lod) as nat);
    span_sum(edit.lod as nat,(10-edit.lod) as nat);
    frontier::span_bits(10);
    assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
    let path=edit.path as int;
    let q=path/width;
    lemma_div_pos_is_pos(path,width);
    lemma_fundamental_div_mod(path,width);
    lemma_mod_bound(path,width);
    assert(q<count) by(nonlinear_arith) requires path<count*width,path==q*width+path%width,0<=path%width,width>0;
    assert(ordered::end(edit)<=state::span(10)) by(nonlinear_arith)
        requires q<count,width>0,state::span(10)==count*width,ordered::end(edit)==q*width+width;
}
pub(in crate::write) proof fn payload_extensional(edit:ordered::EditModel,left:spec_fn(int)->u32,right:spec_fn(int)->u32,point:int)
    requires ordered::edit_shape(edit),ordered::covers(edit,point),
        forall|p:int|#[trigger] ordered::covers(edit,p) ==> left(p)==right(p),
    ensures ordered::payload_value(edit,left,point)==ordered::payload_value(edit,right,point),
{
    if let ordered::Payload::Patch(patch)=edit.payload {
        reveal_with_fuel(state::span,2);
        assert(edit.lod==1);
        assert(state::span(1)==64);
        assert forall|i:int|0<=i<64 implies #[trigger] left(ordered::base(edit)+i)==right(ordered::base(edit)+i) by {
            assert(ordered::covers(edit,ordered::base(edit)+i));
        }
        assert(Seq::new(64,|i:int|left(ordered::base(edit)+i)) =~= Seq::new(64,|i:int|right(ordered::base(edit)+i)));
    }
}
pub(in crate::write) proof fn effected_transitive<P:LeafPatch>(before:state::Pools,mid:state::Pools,after:state::Pools,root:Child,new_root:Child,old:Seq<BuildFrame>,prepared:Seq<BuildFrame>,new:Seq<BuildFrame>,edit:Edit<P>)
    requires preserved(before,mid,root,old,prepared),effected(mid,after,root,new_root,prepared,new,edit),ordered::edit_shape(ordered::edit_view(edit)),
    ensures effected(before,after,root,new_root,old,new,edit),
{
    scope_bounds(ordered::edit_view(edit));
    let a=|p:int|frontier::candidate_value(before,root,old,p);
    let b=|p:int|frontier::candidate_value(mid,root,prepared,p);
    assert forall|p:int|#[trigger] ordered::covers(ordered::edit_view(edit),p) implies a(p)==b(p) by {}
    assert forall|point:int|0<=point<state::span(10) implies #[trigger] frontier::candidate_value(after,new_root,new,point)
        ==if ordered::covers(ordered::edit_view(edit),point){ordered::payload_value(ordered::edit_view(edit),a,point)}else{a(point)} by {
        if ordered::covers(ordered::edit_view(edit),point){payload_extensional(ordered::edit_view(edit),a,b,point);}
    }
}
pub(in crate::write) proof fn replaced_effect<P:LeafPatch>(before:state::Pools,after:state::Pools,root:Child,old:Seq<BuildFrame>,new:Seq<BuildFrame>,edit:Edit<P>,replacement:Replacement<P>,child:Child)
    requires state::build_step(before,after),state::child_valid(before,root,10),frontier::chain_ready(before,old),at_edit(old,edit),
        old.last().pool_lod==edit.lod,ordered::edit_ready(edit),ordered::edit_shape(ordered::edit_view(edit)),resolve::selected(edit,replacement,before),
        ordered::resolved(before,after,replacement,crate::node::proof::children(old.last().entry)[frontier::slot(edit.path,edit.lod)],Ok(child)),
        frontier::last_replaced(old,new,edit.path,child),
    ensures effected(before,after,root,root,old,new,edit),
{
    let initial=|p:int|frontier::candidate_value(before,root,old,p);
    let original=crate::node::proof::children(old.last().entry)[frontier::slot(edit.path,edit.lod)];
    assert(frontier::frame_ready(before,old.last()));
    assert(edit.lod<10);
    frontier::slot_matches(edit.path,edit.lod);
    assert(state::child_valid(before,original,edit.lod as nat));
    assert forall|offset:int|0<=offset<state::span(edit.lod as nat) implies #[trigger] initial(ordered::base(ordered::edit_view(edit))+offset)
        ==state::eval(before,original,edit.lod as nat,offset) by {chosen_value(before,root,old,edit,offset);}
    resolved_value(before,after,edit,replacement,original,child,initial);
    frontier::chain_frozen(before,after,old);
    crate::canonical::frozen_value(before,after,root,10,0);
    frontier::replaced_chain(after,old,new,edit.path,child);
    frontier::span_bits(10);
    assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
    assert forall|point:int|0<=point<state::span(10) implies #[trigger] frontier::candidate_value(after,root,new,point)
        ==if ordered::covers(ordered::edit_view(edit),point){ordered::payload_value(ordered::edit_view(edit),initial,point)}else{initial(point)} by {
        frontier::candidate_frozen(before,after,root,old,point);
        frontier::last_frame_replaced(after,root,old,new,edit.path,child,point as u64);
        edit_geometry(edit,old.last(),point as u64);
        if ordered::covers(ordered::edit_view(edit),point){
            let width=state::span(edit.lod as nat) as int;
            let offset=point-ordered::base(ordered::edit_view(edit));
            lemma_fundamental_div_mod_converse(point,width,crate::morton::proof::value(edit.path) as int/width,offset);
            assert(state::eval(after,child,edit.lod as nat,offset)==ordered::payload_value(ordered::edit_view(edit),initial,point));
        }
    }
}
pub(in crate::write) proof fn whole_effect<P:LeafPatch>(before:state::Pools,after:state::Pools,root:Child,edit:Edit<P>,replacement:Replacement<P>,child:Child)
    requires state::build_step(before,after),state::child_valid(before,root,10),edit.lod==10,
        ordered::edit_ready(edit),ordered::edit_shape(ordered::edit_view(edit)),resolve::selected(edit,replacement,before),
        ordered::resolved(before,after,replacement,root,Ok(child)),
    ensures effected(before,after,root,child,Seq::empty(),Seq::empty(),edit),
{
    scope_bounds(ordered::edit_view(edit));
    frontier::span_bits(10);
    assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
    lemma_fundamental_div_mod_converse(crate::morton::proof::value(edit.path) as int,state::span(10) as int,0,crate::morton::proof::value(edit.path) as int);
    resolved_value(before,after,edit,replacement,root,child,|p:int|state::eval(before,root,10,p));
}
pub(in crate::write) proof fn root_kept<P:LeafPatch>(pools:state::Pools,root:Child,edit:Edit<P>,replacement:Replacement<P>)
    requires state::valid_pools(pools),state::child_valid(pools,root,10),ordered::edit_ready(edit),ordered::edit_shape(ordered::edit_view(edit)),
        resolve::selected(edit,replacement,pools),ordered::replacement_kept(replacement,root),
    ensures effected(pools,pools,root,root,Seq::empty(),Seq::empty(),edit),
{
    assert(frontier::chain_ready(pools,Seq::empty()));
    if let Child::Leaf(value)=root {
        let initial=|p:int|state::eval(pools,root,10,p);
        assert(initial =~= (|p:int|frontier::candidate_value(pools,root,Seq::empty(),p)));
        match replacement {
            Replacement::Value(child)=>{
                assert(child==root);
                assert forall|point:int|0<=point<state::span(10) implies #[trigger] frontier::candidate_value(pools,root,Seq::empty(),point)
                    ==if ordered::covers(ordered::edit_view(edit),point){ordered::payload_value(ordered::edit_view(edit),initial,point)}else{initial(point)} by {
                    if ordered::covers(ordered::edit_view(edit),point){
                        let offset=point-ordered::base(ordered::edit_view(edit));
                        assert(state::eval(pools,child,edit.lod as nat,offset)==value);
                        assert(ordered::payload_value(ordered::edit_view(edit),initial,ordered::base(ordered::edit_view(edit))+offset)==value);
                        assert(ordered::payload_value(ordered::edit_view(edit),initial,point)==value);
                    }
                }
                assert(effected(pools,pools,root,root,Seq::empty(),Seq::empty(),edit));
            },
            Replacement::Patch(patch)=>{
                assert(edit.data==super::super::ordered::EditData::Patch(patch));
                assert(edit.lod==1);
                reveal_with_fuel(state::span,2);
                assert(Seq::new(64,|i:int|initial(ordered::base(ordered::edit_view(edit))+i)) =~= Seq::new(64,|i:int|value));
                assert(patch.effect(Seq::new(64,|i:int|value))==Seq::new(64,|i:int|value));
                assert forall|point:int|0<=point<state::span(10) implies #[trigger] frontier::candidate_value(pools,root,Seq::empty(),point)
                    ==if ordered::covers(ordered::edit_view(edit),point){ordered::payload_value(ordered::edit_view(edit),initial,point)}else{initial(point)} by {
                    if ordered::covers(ordered::edit_view(edit),point){
                        let offset=point-ordered::base(ordered::edit_view(edit));
                        assert(0<=offset<64);
                        assert(ordered::payload_value(ordered::edit_view(edit),initial,point)==value);
                    }
                }
                assert(effected(pools,pools,root,root,Seq::empty(),Seq::empty(),edit));
            },
        }
    }else{
        assert(false);
    }
}
}
