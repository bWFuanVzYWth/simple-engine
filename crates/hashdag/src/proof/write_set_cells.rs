//! 原生叶分组视图；仅在规范中递归展开，执行保留借用游标。
use super::super::ordered::proof as ordered;
use super::super::ordered::proof::{EditModel, Payload, StreamModel};
use super::super::patch::proof as patch;
use super::*;
use vstd::prelude::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;
pub(crate) open spec fn input_domain(cells:Seq<SetCell>)->bool {
    patch::input_domain(cells)
}
pub(crate) open spec fn stream_hits(edits:Seq<EditModel>,path:int)->bool {exists|i:int|0<=i<edits.len() && #[trigger] ordered::covers(edits[i],path)}
pub(crate) proof fn stream_prefix(a:Seq<EditModel>,b:Seq<EditModel>,initial:spec_fn(int)->u32,path:int,count:nat)
    requires count<=a.len(),count<=b.len(),forall|i:int|0<=i<count ==> #[trigger] a[i]==b[i],
    ensures ordered::stream_effect(a,initial,path,count)==ordered::stream_effect(b,initial,path,count),
    decreases count,
{
    if count>0 {assert(a[(count-1) as int]==b[(count-1) as int]);stream_prefix(a,b,initial,path,(count-1) as nat);}
}
pub(crate) proof fn stream_no_cover(edits:Seq<EditModel>,initial:spec_fn(int)->u32,path:int)
    requires forall|i:int|0<=i<edits.len() ==> !#[trigger] ordered::covers(edits[i],path),
    ensures ordered::stream_effect(edits,initial,path,edits.len())==initial(path),
    decreases edits.len(),
{
    if edits.len()>0 {stream_no_cover(edits.drop_last(),initial,path);
        assert(edits.take((edits.len()-1) as int) =~= edits.drop_last());
        stream_prefix(edits,edits.drop_last(),initial,path,(edits.len()-1) as nat);}
}
pub(crate) proof fn stream_prepend(head:EditModel,tail:Seq<EditModel>,initial:spec_fn(int)->u32,path:int)
    ensures ordered::stream_effect(seq![head]+tail,initial,path,tail.len()+1)
        ==if stream_hits(tail,path){ordered::stream_effect(tail,initial,path,tail.len())}
            else if ordered::covers(head,path){ordered::payload_value(head,initial,path)}else{initial(path)},
    decreases tail.len(),
{
    reveal_with_fuel(ordered::stream_effect,2);
    if tail.len()>0 {
        let rest=tail.drop_last();stream_prepend(head,rest,initial,path);
        assert((seq![head]+tail).drop_last() =~= seq![head]+rest);
        assert((seq![head]+tail).take(tail.len() as int) =~= seq![head]+rest);
        stream_prefix(seq![head]+tail,seq![head]+rest,initial,path,tail.len());
        assert(tail.take((tail.len()-1) as int) =~= rest);
        stream_prefix(tail,rest,initial,path,rest.len());
        assert(stream_hits(tail,path)==(stream_hits(rest,path)||ordered::covers(tail.last(),path))) by {
            if stream_hits(tail,path) {
                let i=choose|i:int|0<=i<tail.len() && ordered::covers(tail[i],path);
                if i<tail.len()-1 {assert(rest[i]==tail[i]);}
            }
            if stream_hits(rest,path) {let i=choose|i:int|0<=i<rest.len() && ordered::covers(rest[i],path);assert(tail[i]==rest[i]);}
        }
    } else {assert(seq![head]+tail =~= seq![head]);}
}
pub(crate) proof fn code_parts(code:u64)
    ensures code>>6==code/64,code%64<64,
{
    assert(code>>6==code/64 && code%64<64) by(bit_vector);
}
pub(crate) proof fn group_address(code:u64,base:int)
    requires base>=0,base%64==0,code as int/64==base/64,
    ensures code as int==base+(code%64) as int,
{
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(code as int,64);
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(base,64);
}
pub(crate) proof fn effect_same_initial(cells:Seq<SetCell>,a:spec_fn(int)->u32,b:spec_fn(int)->u32,path:int,count:nat)
    requires count<=cells.len(),a(path)==b(path),
    ensures effect(cells,a,path,count)==effect(cells,b,path,count),
    decreases count,
{if count>0{effect_same_initial(cells,a,b,path,(count-1) as nat);}}
pub(crate) proof fn effect_prefix(a:Seq<SetCell>,b:Seq<SetCell>,initial:spec_fn(int)->u32,path:int,count:nat)
    requires count<=a.len(),count<=b.len(),forall|i:int|0<=i<count ==> #[trigger] a[i]==b[i],
    ensures effect(a,initial,path,count)==effect(b,initial,path,count),
    decreases count,
{
    if count>0 {assert(a[(count-1) as int]==b[(count-1) as int]);effect_prefix(a,b,initial,path,(count-1) as nat);}
}
pub(crate) proof fn effect_concat(a:Seq<SetCell>,b:Seq<SetCell>,initial:spec_fn(int)->u32,path:int)
    ensures effect(a+b,initial,path,a.len()+b.len())
        ==effect(b,|p:int|effect(a,initial,p,a.len()),path,b.len()),
    decreases b.len(),
{
    if b.len()>0 {
        effect_concat(a,b.drop_last(),initial,path);
        assert((a+b).drop_last() =~= a+b.drop_last());
        assert((a+b).take((a.len()+b.len()-1) as int) =~= a+b.drop_last());
        effect_prefix(a+b,a+b.drop_last(),initial,path,(a.len()+b.len()-1) as nat);
        assert(b.take((b.len()-1) as int) =~= b.drop_last());
        effect_prefix(b,b.drop_last(),|p:int|effect(a,initial,p,a.len()),path,(b.len()-1) as nat);
    } else {assert(a+b =~= a);}
}
pub(crate) proof fn effect_absent(cells:Seq<SetCell>,initial:spec_fn(int)->u32,path:int,count:nat)
    requires count<=cells.len(),forall|i:int|0<=i<count ==> crate::morton::proof::value(#[trigger] cells[i].path) as int!=path,
    ensures effect(cells,initial,path,count)==initial(path),
    decreases count,
{if count>0{effect_absent(cells,initial,path,(count-1) as nat);}}
proof fn group_effect(cells:Seq<SetCell>,initial:spec_fn(int)->u32,base:int,target:int,count:nat)
    requires count<=cells.len(),base>=0,base%64==0,0<=target<64,
        forall|i:int|0<=i<count ==> crate::morton::proof::value(#[trigger] cells[i].path) as int/64==base/64,
    ensures effect(cells,initial,base+target,count)==patch::last_value(cells,count,target,initial(base+target)),
    decreases count,
{
    if count>0 {
        group_effect(cells,initial,base,target,(count-1) as nat);
        group_address(crate::morton::proof::value(cells[(count-1) as int].path),base);
    }
}
pub(crate) proof fn leaf_group(cells:Seq<SetCell>,initial:spec_fn(int)->u32,base:int,target:int)
    requires base>=0,base%64==0,0<=target<64,
        forall|i:int|0<=i<cells.len() ==> crate::morton::proof::value(#[trigger] cells[i].path) as int/64==base/64,
    ensures patch::apply(Seq::new(64,|i:int|initial(base+i)),cells,cells.len())[target]==effect(cells,initial,base+target,cells.len()),
{
    patch::point_value(Seq::new(64,|i:int|initial(base+i)),cells,cells.len(),target);
    group_effect(cells,initial,base,target,cells.len());
}
pub(crate) proof fn leaf_apply_length(values:Seq<u32>,cells:Seq<SetCell>,count:nat)
    requires values.len()==64,count<=cells.len(),
    ensures patch::apply(values,cells,count).len()==64,
    decreases count,
{if count>0{leaf_apply_length(values,cells,(count-1) as nat);}}
pub(crate) proof fn stream_skip_head(head:EditModel,tail:Seq<EditModel>,initial:spec_fn(int)->u32,path:int)
    requires !ordered::covers(head,path),
    ensures ordered::stream_effect(seq![head]+tail,initial,path,tail.len()+1)==ordered::stream_effect(tail,initial,path,tail.len()),
{
    stream_prepend(head,tail,initial,path);
    if !stream_hits(tail,path) {stream_no_cover(tail,initial,path);}
}
pub(crate) proof fn stream_cons_ordered(head:EditModel,tail:Seq<EditModel>)
    requires ordered::edit_shape(head),!(head.payload is Built),ordered::stream_ordered(tail),
        forall|i:int|0<=i<tail.len() ==> !(#[trigger] tail[i].payload is Built) && ordered::base(tail[i])>=ordered::end(head),
    ensures ordered::stream_ordered(seq![head]+tail),
{
    let all=seq![head]+tail;
    assert forall|i:int|0<=i<all.len() implies #[trigger] ordered::edit_shape(all[i]) by {
        if i>0 {assert(all[i]==tail[i-1]);}
    }
    assert forall|i:int|0<=i && i+1<all.len() implies ordered::end(#[trigger] all[i])<=ordered::base(all[i+1]) by {
        if i==0 {assert(all[1]==tail[0]);assert(!(tail[0].payload is Built));} else {assert(all[i]==tail[i-1]);assert(all[i+1]==tail[i]);assert(ordered::end(tail[i-1])<=ordered::base(tail[i]));}
    }
    assert forall|i:int|0<=i<all.len() implies !(#[trigger] all[i].payload is Built) by {if i>0{assert(all[i]==tail[i-1]);}}
}
pub(crate) proof fn later_edit(edits:Seq<EditModel>,i:int)
    requires ordered::stream_ordered(edits),0<=i<edits.len(),
    ensures ordered::base(edits[i])>=ordered::base(edits[0]),
    decreases i,
{
    if i>0 {later_edit(edits,i-1);crate::canonical::span_positive(edits[i-1].lod as nat);assert(ordered::end(edits[i-1])<=ordered::base(edits[i]));}
}
pub(crate) proof fn stream_head_hit(head:EditModel,tail:Seq<EditModel>,initial:spec_fn(int)->u32,path:int)
    requires ordered::covers(head,path),ordered::stream_ordered(tail),tail.len()>0 ==> ordered::base(tail[0])>=ordered::end(head),
    ensures ordered::stream_effect(seq![head]+tail,initial,path,tail.len()+1)==ordered::payload_value(head,initial,path),
{
    assert forall|i:int|0<=i<tail.len() implies !#[trigger] ordered::covers(tail[i],path) by {later_edit(tail,i);}
    assert(!stream_hits(tail,path));stream_prepend(head,tail,initial,path);
}
proof fn group_scan(cells:Seq<SetCell>,limit:nat)
    requires 0<limit<=cells.len(),forall|i:int|0<=i<limit ==> crate::morton::proof::value(#[trigger] cells[i].path)>>6==crate::morton::proof::value(cells[0].path)>>6,
    ensures group(cells,end(cells)),
    decreases cells.len()-limit,
{
    if limit==cells.len() || crate::morton::proof::value(cells[limit as int].path)>>6!=crate::morton::proof::value(cells[0].path)>>6 {
        group_found(cells,limit);
    } else {group_scan(cells,limit+1);}
}
proof fn sorted_pair(cells:Seq<SetCell>,left:int,right:int)
    requires patch::sorted(cells),0<=left<=right<cells.len(),
    ensures patch::code(cells[left])<=patch::code(cells[right]),
    decreases right-left,
{
    if left<right {sorted_pair(cells,left,right-1);assert(patch::code(cells[right-1])<=patch::code(cells[right]));}
}
proof fn group_tail(cells:Seq<SetCell>,limit:nat)
    requires patch::sorted(cells),input_domain(cells),group(cells,limit),
    ensures patch::sorted(cells.subrange(limit as int,cells.len() as int)),input_domain(cells.subrange(limit as int,cells.len() as int)),
        forall|i:int|limit<=i<cells.len() ==> patch::code(#[trigger] cells[i]) as int/64>patch::code(cells[0]) as int/64,
{
    let tail=cells.subrange(limit as int,cells.len() as int);
    assert forall|i:int|0<=i && i+1<tail.len() implies patch::code(#[trigger] tail[i])<=patch::code(tail[i+1]) by {
        assert(patch::code(cells[limit as int+i])<=patch::code(cells[limit as int+i+1]));
    }
    if limit<cells.len() {
        code_parts(patch::code(cells[0]));code_parts(patch::code(cells[limit as int]));sorted_pair(cells,0,limit as int);
        assert forall|i:int|limit<=i<cells.len() implies patch::code(#[trigger] cells[i]) as int/64>patch::code(cells[0]) as int/64 by {
            sorted_pair(cells,limit as int,i);
        }
    }
}
pub(crate) open(super) spec fn head_edit(cells:Seq<SetCell>)->EditModel {
    EditModel{path:patch::code(cells[0]),lod:1,payload:Payload::Patch(|before:Seq<u32>|patch::apply(before,cells.take(end(cells) as int),end(cells)))}
}
proof fn decompose(cells:Seq<SetCell>)
    requires cells.len()>0,patch::sorted(cells),input_domain(cells),
    ensures group(cells,end(cells)),ordered::edit_shape(head_edit(cells)),
        patch::sorted(cells.subrange(end(cells) as int,cells.len() as int)),input_domain(cells.subrange(end(cells) as int,cells.len() as int)),
        stream(cells)==seq![head_edit(cells)]+stream(cells.subrange(end(cells) as int,cells.len() as int)),
        ordered::base(head_edit(cells))==patch::code(cells[0]) as int/64*64,ordered::end(head_edit(cells))==ordered::base(head_edit(cells))+64,
        forall|i:int|0<=i<end(cells) ==> patch::code(#[trigger] cells[i]) as int/64==patch::code(cells[0]) as int/64,
        forall|i:int|end(cells)<=i<cells.len() ==> patch::code(#[trigger] cells[i]) as int>=ordered::end(head_edit(cells)),
{
    group_scan(cells,1);let limit=end(cells);group_tail(cells,limit);
    reveal_with_fuel(crate::state::span,2);code_parts(patch::code(cells[0]));
    assert forall|i:int|0<=i<limit implies patch::code(#[trigger] cells[i]) as int/64==patch::code(cells[0]) as int/64 by {code_parts(patch::code(cells[i]));}
    assert forall|i:int|limit<=i<cells.len() implies patch::code(#[trigger] cells[i]) as int>=ordered::end(head_edit(cells)) by {group_address(patch::code(cells[i]),patch::code(cells[i]) as int/64*64);}
    assert(cells.take(limit as int) =~= cells.subrange(0,limit as int));
}
pub(crate) proof fn stream_valid(cells:Seq<SetCell>)
    requires patch::sorted(cells),input_domain(cells),
    ensures ordered::stream_ordered(stream(cells)),
        forall|i:int|0<=i<stream(cells).len() ==> !(#[trigger] stream(cells)[i].payload is Built)
            && ordered::base(stream(cells)[i])>=patch::code(cells[0]) as int/64*64,
    decreases cells.len(),
{
    if cells.len()>0 {
        decompose(cells);let limit=end(cells);let rest=cells.subrange(limit as int,cells.len() as int);
        stream_valid(rest);let head=head_edit(cells);let tail=stream(rest);let base=ordered::base(head);
        assert forall|i:int|0<=i<tail.len() implies !(#[trigger] tail[i].payload is Built) && ordered::base(tail[i])>=ordered::end(head) by {
            assert(!(tail[i].payload is Built));
            assert(ordered::base(tail[i])>=patch::code(rest[0]) as int/64*64);
            assert(patch::code(rest[0]) as int>=ordered::end(head));
            vstd::arithmetic::div_mod::lemma_fundamental_div_mod(patch::code(rest[0]) as int,64);
        }
        stream_cons_ordered(head,tail);
        assert forall|i:int|0<=i<stream(cells).len() implies !(#[trigger] stream(cells)[i].payload is Built)
            && ordered::base(stream(cells)[i])>=base by {if i>0{assert(stream(cells)[i]==tail[i-1]);}}
    }
}
proof fn input_head_hit(cells:Seq<SetCell>,initial:spec_fn(int)->u32,path:int)
    requires patch::sorted(cells),input_domain(cells),cells.len()>0,ordered::covers(head_edit(cells),path),
    ensures effect(cells,initial,path,cells.len())==effect(cells.take(end(cells) as int),initial,path,end(cells)),
{
    hide(effect);decompose(cells);let group=cells.take(end(cells) as int);let rest=cells.subrange(end(cells) as int,cells.len() as int);
    assert(cells =~= group+rest);effect_concat(group,rest,initial,path);effect_absent(rest,|p:int|effect(group,initial,p,group.len()),path,rest.len());
}
proof fn input_head_miss(cells:Seq<SetCell>,initial:spec_fn(int)->u32,path:int)
    requires patch::sorted(cells),input_domain(cells),cells.len()>0,!ordered::covers(head_edit(cells),path),
    ensures effect(cells,initial,path,cells.len())==effect(cells.subrange(end(cells) as int,cells.len() as int),initial,path,(cells.len()-end(cells)) as nat),
{
    hide(effect);decompose(cells);let group=cells.take(end(cells) as int);let rest=cells.subrange(end(cells) as int,cells.len() as int);let base=ordered::base(head_edit(cells));
    assert(cells =~= group+rest);effect_concat(group,rest,initial,path);
    assert forall|i:int|0<=i<group.len() implies patch::code(#[trigger] group[i]) as int!=path by {group_address(patch::code(group[i]),base);}
    effect_absent(group,initial,path,group.len());effect_same_initial(rest,|p:int|effect(group,initial,p,group.len()),initial,path,rest.len());
}
proof fn head_payload(cells:Seq<SetCell>,initial:spec_fn(int)->u32,path:int)
    requires patch::sorted(cells),input_domain(cells),cells.len()>0,ordered::covers(head_edit(cells),path),
    ensures ordered::payload_value(head_edit(cells),initial,path)==effect(cells.take(end(cells) as int),initial,path,end(cells)),
{
    hide(effect);decompose(cells);let group=cells.take(end(cells) as int);let base=ordered::base(head_edit(cells));
    leaf_group(group,initial,base,path-base);leaf_apply_length(Seq::new(64,|i:int|initial(base+i)),group,group.len());
}
proof fn value_join(cells:Seq<SetCell>,initial:spec_fn(int)->u32,path:int)
    requires patch::sorted(cells),input_domain(cells),cells.len()>0,group(cells,end(cells)),
        stream(cells)==seq![head_edit(cells)]+stream(cells.subrange(end(cells) as int,cells.len() as int)),
        ordered::stream_ordered(stream(cells.subrange(end(cells) as int,cells.len() as int))),
        stream(cells.subrange(end(cells) as int,cells.len() as int)).len()>0 ==>
            ordered::base(stream(cells.subrange(end(cells) as int,cells.len() as int))[0])>=ordered::end(head_edit(cells)),
        ordered::stream_effect(stream(cells.subrange(end(cells) as int,cells.len() as int)),initial,path,stream(cells.subrange(end(cells) as int,cells.len() as int)).len())
            ==effect(cells.subrange(end(cells) as int,cells.len() as int),initial,path,(cells.len()-end(cells)) as nat),
    ensures ordered::stream_effect(stream(cells),initial,path,stream(cells).len())==effect(cells,initial,path,cells.len()),
{
    hide(stream);hide(effect);hide(head_edit);hide(ordered::payload_value);hide(ordered::stream_effect);
    let rest=cells.subrange(end(cells) as int,cells.len() as int);
    let head=head_edit(cells);let tail=stream(rest);
    if ordered::covers(head,path) {
        head_payload(cells,initial,path);input_head_hit(cells,initial,path);
        stream_head_hit(head,tail,initial,path);
    }else{input_head_miss(cells,initial,path);stream_skip_head(head,tail,initial,path);}
}
proof fn tail_order(cells:Seq<SetCell>)
    requires patch::sorted(cells),input_domain(cells),cells.len()>0,
    ensures ordered::stream_ordered(stream(cells.subrange(end(cells) as int,cells.len() as int))),
        stream(cells.subrange(end(cells) as int,cells.len() as int)).len()>0 ==>
            ordered::base(stream(cells.subrange(end(cells) as int,cells.len() as int))[0])>=ordered::end(head_edit(cells)),
{
    hide(stream);decompose(cells);let rest=cells.subrange(end(cells) as int,cells.len() as int);stream_valid(rest);
    if rest.len()==0{reveal(stream);}else{
        let tail=stream(rest);if tail.len()>0{assert(!(tail[0].payload is Built));
            assert(ordered::base(tail[0])>=patch::code(rest[0]) as int/64*64);
            assert(patch::code(rest[0]) as int>=ordered::end(head_edit(cells)));
            vstd::arithmetic::div_mod::lemma_fundamental_div_mod(patch::code(rest[0]) as int,64);
            assert(patch::code(rest[0]) as int/64*64>=ordered::end(head_edit(cells)));}
    }
}
pub(crate) proof fn stream_value(cells:Seq<SetCell>,initial:spec_fn(int)->u32,path:int)
    requires patch::sorted(cells),input_domain(cells),
    ensures ordered::stream_effect(stream(cells),initial,path,stream(cells).len())==effect(cells,initial,path,cells.len()),
    decreases cells.len(),
{
    hide(stream);hide(effect);
    if cells.len()>0 {decompose(cells);stream_value(cells.subrange(end(cells) as int,cells.len() as int),initial,path);tail_order(cells);value_join(cells,initial,path);}
    else{reveal(stream);reveal(effect);}
}
pub(crate) proof fn stream_contract(cells:Seq<SetCell>,initial:spec_fn(int)->u32,path:int)
    requires patch::sorted(cells),input_domain(cells),
    ensures ordered::stream_ordered(stream(cells)),ordered::stream_effect(stream(cells),initial,path,stream(cells).len())==effect(cells,initial,path,cells.len()),
{stream_valid(cells);stream_value(cells,initial,path);}
impl crate::HashDag {
    pub open(crate) spec fn set_cells_post(&self,before:&Self,root:crate::RootId,cells:&[SetCell],result:Result<(),Error>)->bool {
        crate::write::proof::input_written(before,self,root,patch::sorted(cells@),
            |initial:spec_fn(int)->u32,path:int|effect(cells@,initial,path,cells@.len()),result)
    }
}
pub(crate) open spec fn effect(cells:Seq<SetCell>,initial:spec_fn(int)->u32,path:int,count:nat)->u32
    decreases count,
{
    if count==0 {initial(path)} else {
        let cell=cells[(count-1) as int];
        if crate::morton::proof::value(cell.path) as int==path {cell.data}
        else {effect(cells,initial,path,(count-1) as nat)}
    }
}
impl crate::write::proof::BatchModel for Cells<'_> {
    open(crate) spec fn accepted(&self)->bool {patch::sorted(self.inputs()) && input_domain(self.inputs())}
    open(crate) spec fn effect(&self,initial:spec_fn(int)->u32,path:int)->u32 {effect(self.inputs(),initial,path,self.inputs().len())}
}
impl<'a> Cells<'a> {
    #[verifier::type_invariant]
    pub(crate) open(super) spec fn ordered(self)->bool {patch::sorted(self.cells@) && input_domain(self.cells@)}
    pub(crate) open(super) spec fn inputs(&self)->Seq<SetCell> {self.cells@}
}
pub(super) open spec fn group(cells: Seq<SetCell>, end: nat) -> bool {
    0 < end <= cells.len()
        && (forall|i:int| 0 <= i < end ==> crate::morton::proof::value(#[trigger] cells[i].path) >> 6
            == crate::morton::proof::value(cells[0].path) >> 6)
        && (end == cells.len() || crate::morton::proof::value(cells[end as int].path) >> 6
            != crate::morton::proof::value(cells[0].path) >> 6)
}
pub(super) open spec fn end(cells: Seq<SetCell>) -> nat { choose|limit:nat| group(cells, limit) }

pub(super) proof fn group_found(cells: Seq<SetCell>, limit: nat)
    requires group(cells, limit),
    ensures end(cells) == limit,
{
    assert(exists|candidate:nat| group(cells, candidate));
    let selected = end(cells);
    assert(group(cells, selected));
    if selected < limit {
        assert(crate::morton::proof::value(cells[selected as int].path) >> 6 == crate::morton::proof::value(cells[0].path) >> 6);
    } else if limit < selected {
        assert(crate::morton::proof::value(cells[limit as int].path) >> 6 == crate::morton::proof::value(cells[0].path) >> 6);
    }
}

pub(crate) open(super) spec fn stream(cells: Seq<SetCell>) -> Seq<EditModel>
    decreases cells.len(),
{
    if cells.len() == 0 { Seq::empty() } else {
        let limit = end(cells);
        if 0 < limit <= cells.len() {
            let group = cells.subrange(0, limit as int);
            seq![EditModel { path: crate::morton::proof::value(cells[0].path), lod: 1,
                payload: Payload::Patch(|before:Seq<u32>| patch::apply(before, group, group.len())) }]
                + stream(cells.subrange(limit as int, cells.len() as int))
        } else { Seq::empty() }
    }
}
impl CellEdits<'_> {
    pub(super) open spec fn stream(&self) -> Seq<EditModel> { stream(self.pending@) }
}
impl StreamModel for CellEdits<'_> {
    open(super) spec fn stream_valid(&self)->bool {true}
    open(super) spec fn remaining(&self)->Seq<EditModel> {self.stream()}
}
}
