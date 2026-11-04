//! 子树选择与缓存输出的擦除语义。
use super::*;
use crate::{
    state::{self, Pools},
    write::{import::proof as import, ordered::proof as ordered},
};
use vstd::prelude::*;
verus! {
pub(in crate::write) open(in crate::write) spec fn ready<P:LeafPatch>(edit:Edit<P>,pools:Pools,scratch:&WriteScratch)->bool {
    ordered::edit_ready(edit) && match edit.data {
        EditData::Built{tree,part}=>part==0 || import::imported(tree,pools,&scratch.imported),
        _=>true,
    }
}
pub(in crate::write) open(in crate::write) spec fn selected<P:LeafPatch>(edit:Edit<P>,replacement:Replacement<P>,pools:Pools)->bool {
    match replacement {
        Replacement::Value(child)=>state::child_valid(pools,child,edit.lod as nat)
            && !(edit.data is Patch)
            && forall|initial:spec_fn(int)->u32,offset:int|0<=offset<state::span(edit.lod as nat) ==>
                state::eval(pools,child,edit.lod as nat,offset)
                    == #[trigger] ordered::payload_value(ordered::edit_view(edit),initial,ordered::base(ordered::edit_view(edit))+offset),
        Replacement::Patch(patch)=>edit.data==EditData::Patch(patch),
    }
}
pub(in crate::write) open(in crate::write) spec fn cache_same(before:&WriteScratch,after:&WriteScratch)->bool {
    import::maps(&before.imported)==import::maps(&after.imported) && before.imported.roots@==after.imported.roots@
}
pub(in crate::write) open(in crate::write) spec fn cache_kept(before:Pools,after:Pools,old:&WriteScratch,new:&WriteScratch)->bool {
    cache_same(old,new) && forall|tree:&super::super::built::LocalTree| import::source_valid(tree)
        && import::imported(tree,before,&old.imported) ==> import::imported(tree,after,&new.imported)
}
pub(in crate::write) open(in crate::write) spec fn context<P:LeafPatch>(edit:Edit<P>,before:Pools,after:Pools,old:&WriteScratch,new:&WriteScratch)->bool {
    match edit.data {
        EditData::Built{tree,..}=>import::imported(tree,after,&new.imported),
        _=>cache_kept(before,after,old,new),
    }
}
pub(in crate::write) proof fn cache_frozen(before:Pools,after:Pools,old:&WriteScratch,new:&WriteScratch)
    requires state::build_step(before,after),cache_same(old,new),
    ensures cache_kept(before,after,old,new),
{
    assert forall|tree:&super::super::built::LocalTree| import::source_valid(tree)
        && import::imported(tree,before,&old.imported) implies import::imported(tree,after,&new.imported) by {
        import::imported_frozen(tree,before,after,&old.imported,&new.imported);
    }
}
pub(in crate::write) proof fn uniform_selected<P:LeafPatch>(edit:Edit<P>,pools:Pools,value:u32)
    requires ordered::edit_ready(edit),edit.data==EditData::Uniform(value),state::valid_pools(pools),
    ensures selected(edit,Replacement::Value(Child::Leaf(value)),pools),
{}
pub(in crate::write) proof fn dense_selected<P:LeafPatch>(edit:Edit<P>,pools:Pools,child:Child,values:Seq<u32>)
    requires ordered::edit_ready(edit),match edit.data {EditData::Dense(input)=>input@==values,_=>false},
        state::child_valid(pools,child,edit.lod as nat),
        super::super::dense_chunks::proof::content(pools,child,edit.lod as nat)==values,
    ensures selected(edit,Replacement::Value(child),pools),
{}
pub(in crate::write) proof fn built_selected<P:LeafPatch>(edit:Edit<P>,pools:Pools,scratch:&WriteScratch,tree:&super::super::built::LocalTree,part:usize)
    requires ordered::edit_ready(edit),edit.data==(EditData::Built{tree,part}),
        import::imported(tree,pools,&scratch.imported),
    ensures selected(edit,Replacement::Value(scratch.imported.roots@[part as int]),pools),
{
    let part=part as int;
    let original=tree.roots@[part];let child=scratch.imported.roots@[part];
    assert(import::corresponds(state::pools_view(&tree.pools),pools,original,child,edit.lod as nat));
    assert forall|initial:spec_fn(int)->u32,offset:int|0<=offset<state::span(edit.lod as nat) implies
        state::eval(pools,child,edit.lod as nat,offset)==#[trigger] ordered::payload_value(ordered::edit_view(edit),initial,ordered::base(ordered::edit_view(edit))+offset) by {
        assert(super::super::dense_chunks::proof::content(state::pools_view(&tree.pools),original,edit.lod as nat)[offset]
            ==super::super::dense_chunks::proof::content(pools,child,edit.lod as nat)[offset]);
    }
}
}
