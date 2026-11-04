//! 深层同值叶剪枝；目标小区域的全部候选值都已等于叶值。
use super::*;
use crate::write::ordered::EditData;
use crate::{morton::proof as morton, node::proof as node};
use vstd::arithmetic::div_mod::*;
use vstd::prelude::*;

verus! {
pub(in crate::write) proof fn inside_slot<P:LeafPatch>(pools:state::Pools,frames:Seq<BuildFrame>,edit:Edit<P>,point:int)
    requires state::valid_pools(pools),frontier::chain_ready(pools,frames),at_edit(frames,edit),
        ordered::edit_shape(ordered::edit_view(edit)),ordered::covers(ordered::edit_view(edit),point),
    ensures frontier::covers(frames.last(),point),point>=0,
        point / state::span(frames.last().pool_lod as nat) as int
            == morton::value(edit.path) as int / state::span(frames.last().pool_lod as nat) as int,
{
    let frame=frames.last();let model=ordered::edit_view(edit);
    let small=state::span(edit.lod as nat) as int;
    let ratio=state::span((frame.pool_lod-edit.lod) as nat) as int;
    let large=state::span(frame.pool_lod as nat) as int;
    scope_bounds(model);crate::canonical::span_positive(edit.lod as nat);
    crate::canonical::span_positive((frame.pool_lod-edit.lod) as nat);
    span_sum(edit.lod as nat,(frame.pool_lod-edit.lod) as nat);
    lemma_fundamental_div_mod_converse(point,small,model.path as int/small,point-ordered::base(model));
    lemma_div_denominator(point,small,ratio);
    lemma_div_denominator(model.path as int,small,ratio);
    lemma_div_denominator(point,large,64);
    lemma_div_denominator(model.path as int,large,64);
}

pub(in crate::write) proof fn candidate_leaf<P:LeafPatch>(pools:state::Pools,root:Child,frames:Seq<BuildFrame>,edit:Edit<P>,value:u32,point:int)
    requires state::valid_pools(pools),frontier::chain_ready(pools,frames),at_edit(frames,edit),
        ordered::edit_shape(ordered::edit_view(edit)),ordered::covers(ordered::edit_view(edit),point),
        node::children(frames.last().entry)[frontier::slot(edit.path,frames.last().pool_lod)]==Child::Leaf(value),
    ensures frontier::candidate_value(pools,root,frames,point)==value,
{
    let frame=frames.last();
    assert(frontier::frame_ready(pools,frame));
    inside_slot(pools,frames,edit,point);
    frontier::frontier_last(pools,frames,0,point);
    frontier::path_digits(morton::value(edit.path),frame.pool_lod);
    let width=state::span(frame.pool_lod as nat) as int;
    crate::canonical::span_positive(frame.pool_lod as nat);
    lemma_mod_bound(point/width,64);
    lemma_mod_bound(point,width);
    lemma_mod_breakdown(point,width,64);
    lemma_fundamental_div_mod_converse(point%(width*64),width,point/width%64,point%width);
    lemma_small_mod((point/width%64) as nat,64);
    assert((point % (width*64))/width%64==point/width%64);
}

pub(in crate::write) proof fn kept_candidate<P:LeafPatch>(pools:state::Pools,root:Child,frames:Seq<BuildFrame>,edit:Edit<P>,replacement:Replacement<P>)
    requires state::valid_pools(pools),state::child_valid(pools,root,10),frontier::chain_ready(pools,frames),at_edit(frames,edit),
        ordered::edit_ready(edit),ordered::edit_shape(ordered::edit_view(edit)),resolve::selected(edit,replacement,pools),
        ordered::replacement_kept(replacement,node::children(frames.last().entry)[frontier::slot(edit.path,frames.last().pool_lod)]),
    ensures effected(pools,pools,root,root,frames,frames,edit),
{
    let old=node::children(frames.last().entry)[frontier::slot(edit.path,frames.last().pool_lod)];
    if let Child::Leaf(value)=old {
        let initial=|p:int|frontier::candidate_value(pools,root,frames,p);
        let constant=|p:int|value;
        assert forall|p:int|#[trigger] ordered::covers(ordered::edit_view(edit),p) implies initial(p)==constant(p) by {
            candidate_leaf(pools,root,frames,edit,value,p);
        }
        assert forall|point:int|0<=point<state::span(10) implies #[trigger] frontier::candidate_value(pools,root,frames,point)
            ==if ordered::covers(ordered::edit_view(edit),point){ordered::payload_value(ordered::edit_view(edit),initial,point)}else{initial(point)} by {
            if ordered::covers(ordered::edit_view(edit),point) {
                candidate_leaf(pools,root,frames,edit,value,point);
                payload_extensional(ordered::edit_view(edit),initial,constant,point);
                match replacement {
                    Replacement::Value(child)=>{
                        assert(child==old);
                        let offset=point-ordered::base(ordered::edit_view(edit));
                        assert(state::eval(pools,child,edit.lod as nat,offset)==value);
                        assert(ordered::payload_value(ordered::edit_view(edit),constant,ordered::base(ordered::edit_view(edit))+offset)==value);
                        assert(ordered::payload_value(ordered::edit_view(edit),constant,point)==value);
                    },
                    Replacement::Patch(patch)=>{
                        assert(edit.data==EditData::Patch(patch));assert(edit.lod==1);
                        reveal_with_fuel(state::span,2);
                        assert(patch.effect(Seq::new(64,|i:int|value))==Seq::new(64,|i:int|value));
                        assert(Seq::new(64,|i:int|constant(ordered::base(ordered::edit_view(edit))+i)) =~= Seq::new(64,|i:int|value));
                        assert(0<=point-ordered::base(ordered::edit_view(edit))<64);
                        assert(ordered::payload_value(ordered::edit_view(edit),constant,point)==value);
                    },
                }
            }
        }
    }else{assert(false);}
}
}
