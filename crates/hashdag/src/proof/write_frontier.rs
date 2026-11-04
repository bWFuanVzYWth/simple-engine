//! 活动帧的路径前缀与单槽更新；独立于节点池的所有权维护。
use super::BuildFrame;
use crate::node::proof as node;
use crate::write::commit::proof::image;
use crate::{Child, ZOrder20x3};
use crate::{canonical, state};
use vstd::arithmetic::div_mod::*;
use vstd::arithmetic::power2::*;
use vstd::prelude::*;

verus! {
pub open spec fn contains(frame: BuildFrame, path: ZOrder20x3) -> bool {
    crate::morton::proof::value(path) >> ((frame.pool_lod as u32 + 1) * 6)
        == crate::morton::proof::value(frame.path) >> ((frame.pool_lod as u32 + 1) * 6)
}
pub open spec fn slot(path: ZOrder20x3, level: u8) -> int {
    ((crate::morton::proof::value(path) >> (6 * level)) % 64) as int
}
pub proof fn slot_matches(path: ZOrder20x3, level: u8)
    requires level < 10,
    ensures ((crate::morton::proof::value(path) >> (6 * level)) & 63) as int == slot(path, level),
{
    let code = crate::morton::proof::value(path) >> (6 * level);
    assert(code & 63 == code % 64) by(bit_vector);
}
pub proof fn same_slot(row: Seq<Child>, index: int, child: Child)
    requires 0 <= index < row.len(), row[index] == child,
    ensures row.update(index, child) == row,
{
    assert(row.update(index, child) =~= row);
}

proof fn span_power(height:u8)
    requires height<=10,
    ensures state::span(height as nat)==pow2((height*6) as nat),
    decreases height,
{
    lemma2_to64();
    if height>0 {
        span_power((height-1) as u8);
        lemma_pow2_adds(((height-1)*6) as nat,6);
    }
}

pub(crate) proof fn span_bits(height:u8)
    requires height<=10,
    ensures state::span(height as nat)==(1u64 << (height as u32 * 6)) as nat,
{
    span_power(height);
    vstd::bits::lemma_u64_pow2_no_overflow((height*6) as nat);
    vstd::bits::lemma_u64_shl_is_mul(1,(height*6) as u64);
}

pub(crate) proof fn path_digits(code:u64,height:u8)
    requires height<=10,
    ensures code as int / (state::span(height as nat) as int)==(code>>(height as u32*6)) as int,
        code as int % (state::span(height as nat) as int)
            ==(code & (((1u64<<(height as u32*6))-1) as u64)) as int,
{
    span_bits(height);
    let shift=(height as u32*6) as u32;
    span_power(height);
    vstd::bits::lemma_u64_pow2_no_overflow((height*6) as nat);
    vstd::bits::lemma_u64_shr_is_div(code,shift as u64);
    vstd::bits::lemma_u64_low_bits_mask_is_mod(code,(height*6) as nat);
}

pub(crate) open spec fn covers(frame:BuildFrame,code:int)->bool {
    code/(state::span((frame.pool_lod+1) as nat) as int)
        == crate::morton::proof::value(frame.path) as int/(state::span((frame.pool_lod+1) as nat) as int)
}

pub(crate) proof fn contains_model(frame:BuildFrame,path:ZOrder20x3)
    requires frame.pool_lod<10,
    ensures contains(frame,path)==covers(frame,crate::morton::proof::value(path) as int),
{
    path_digits(crate::morton::proof::value(path),(frame.pool_lod+1) as u8);
    path_digits(crate::morton::proof::value(frame.path),(frame.pool_lod+1) as u8);
}

pub(crate) proof fn slot_model(code:u64,level:u8)
    requires level<10,
    ensures (code as int % (state::span((level+1) as nat) as int)) / (state::span(level as nat) as int) % 64
            == ((code>>(level as u32*6))%64) as int,
        (code as int % (state::span((level+1) as nat) as int)) % (state::span(level as nat) as int)
            == code as int % (state::span(level as nat) as int),
{
    canonical::span_positive(level as nat);
    path_digits(code,level);
    let width=state::span(level as nat) as int;
    let local=code as int % (width*64);
    let digit=code as int / width % 64;
    lemma_mod_bound(code as int,width);
    lemma_mod_bound(code as int/width,64);
    lemma_mod_breakdown(code as int,width,64);
    lemma_fundamental_div_mod_converse(local,width,digit,code as int%width);
    lemma_small_mod(digit as nat,64);
}

pub(crate) open spec fn frame_ready(pools:state::Pools,frame:BuildFrame)->bool {
    frame.pool_lod<10 && state::child_valid(pools,frame.original,(frame.pool_lod+1) as nat)
        && (forall|i:int| 0<=i<64 ==> state::child_valid(pools,#[trigger] node::children(frame.entry)[i],frame.pool_lod as nat))
        && (!frame.any_modified ==> node::children(frame.entry)==image(pools[frame.pool_lod as int],frame.original))
}

pub(crate) open spec fn frame_value(pools:state::Pools,frame:BuildFrame,code:int)->u32 {
    let level=frame.pool_lod as nat;
    let local=code % (state::span(level+1) as int);
    let width=state::span(level) as int;
    state::eval(pools,node::children(frame.entry)[local/width%64],level,local%width)
}

pub(crate) proof fn original_value(pools:state::Pools,frame:BuildFrame,code:int)
    requires state::valid_pools(pools),frame_ready(pools,frame),
        node::children(frame.entry)==image(pools[frame.pool_lod as int],frame.original),
    ensures frame_value(pools,frame,code)
        ==state::eval(pools,frame.original,(frame.pool_lod+1) as nat,code%(state::span((frame.pool_lod+1) as nat) as int)),
{
    canonical::span_positive(frame.pool_lod as nat);
    let width=state::span(frame.pool_lod as nat) as int;
    let local=code%(state::span((frame.pool_lod+1) as nat) as int);
    lemma_mod_bound(local/width,64);
    match frame.original {Child::Leaf(_)=>{},Child::Ptr(index)=>{
        assert(state::occupied(pools[frame.pool_lod as int],node::idx(index)));
    }}
}
pub(crate) proof fn entry_value(pools:state::Pools,frame:BuildFrame,child:Child,code:int)
    requires state::valid_pools(pools),frame_ready(pools,frame),state::child_valid(pools,child,(frame.pool_lod+1) as nat),
        image(pools[frame.pool_lod as int],child)==node::children(frame.entry),
    ensures frame_value(pools,frame,code)==state::eval(pools,child,(frame.pool_lod+1) as nat,code%(state::span((frame.pool_lod+1) as nat) as int)),
{
    let copied=BuildFrame{entry:frame.entry,original:child,path:frame.path,any_modified:false,pool_lod:frame.pool_lod};
    original_value(pools,copied,code);
}

pub(crate) proof fn frame_frozen(before:state::Pools,after:state::Pools,frame:BuildFrame,code:int)
    requires state::valid_pools(before),state::valid_pools(after),state::frozen(before,after),frame_ready(before,frame),
    ensures frame_ready(after,frame),frame_value(before,frame,code)==frame_value(after,frame,code),
{
    canonical::frozen_value(before,after,frame.original,(frame.pool_lod+1) as nat,0);
    assert forall|i:int| 0<=i<64 implies state::child_valid(after,#[trigger] node::children(frame.entry)[i],frame.pool_lod as nat) by {
        canonical::frozen_value(before,after,node::children(frame.entry)[i],frame.pool_lod as nat,0);
    }
    canonical::span_positive(frame.pool_lod as nat);
    let width=state::span(frame.pool_lod as nat) as int;
    let local=code%(state::span((frame.pool_lod+1) as nat) as int);
    lemma_mod_bound(local/width,64);
    canonical::frozen_value(before,after,node::children(frame.entry)[local/width%64],frame.pool_lod as nat,local%width);
}

pub(crate) proof fn replaced_frame(pools:state::Pools,before:BuildFrame,after:BuildFrame,path:ZOrder20x3,child:Child,code:u64)
    requires state::valid_pools(pools),frame_ready(pools,before),
        state::child_valid(pools,child,before.pool_lod as nat),
        after.pool_lod==before.pool_lod,after.path==before.path,after.original==before.original,
        node::children(after.entry)==node::children(before.entry).update(slot(path,before.pool_lod),child),
        after.any_modified==(before.any_modified || node::children(before.entry)[slot(path,before.pool_lod)]!=child),
    ensures frame_ready(pools,after),
        frame_value(pools,after,code as int)==if ((code>>(before.pool_lod as u32*6))%64) as int==slot(path,before.pool_lod) {
            state::eval(pools,child,before.pool_lod as nat,code as int%(state::span(before.pool_lod as nat) as int))
        }else{frame_value(pools,before,code as int)},
{
    slot_matches(path,before.pool_lod);
    slot_model(code,before.pool_lod);
    let target=slot(path,before.pool_lod);
    assert(0<=target<64);
    assert forall|i:int|0<=i<64 implies state::child_valid(pools,#[trigger] node::children(after.entry)[i],after.pool_lod as nat) by {
        if i!=target {assert(node::children(after.entry)[i]==node::children(before.entry)[i]);}
    }
    if !after.any_modified {
        same_slot(node::children(before.entry),target,child);
    }
}

pub(crate) proof fn child_slot(parent:BuildFrame,child:BuildFrame,code:u64)
    requires parent.pool_lod<10,child.pool_lod+1==parent.pool_lod,
        covers(parent,crate::morton::proof::value(child.path) as int),covers(parent,code as int),
    ensures covers(child,code as int) <==> ((code>>(parent.pool_lod as u32*6))%64) as int==slot(child.path,parent.pool_lod),
{
    let width=state::span(parent.pool_lod as nat) as int;
    canonical::span_positive(parent.pool_lod as nat);
    let path=crate::morton::proof::value(child.path);
    path_digits(code,parent.pool_lod);
    path_digits(path,parent.pool_lod);
    lemma_div_denominator(code as int,width,64);
    lemma_div_denominator(path as int,width,64);
    let a=code as int/width;
    let b=path as int/width;
    assert(a/64==b/64);
    lemma_fundamental_div_mod(a,64);
    lemma_fundamental_div_mod(b,64);
}

pub(crate) open spec fn frame_replaced(before:BuildFrame,after:BuildFrame,path:ZOrder20x3,child:Child)->bool {
    after.pool_lod==before.pool_lod && after.path==before.path && after.original==before.original
        && node::children(after.entry)==node::children(before.entry).update(slot(path,before.pool_lod),child)
        && after.any_modified==(before.any_modified || node::children(before.entry)[slot(path,before.pool_lod)]!=child)
}

pub(crate) proof fn parent_closed(pools:state::Pools,parent:BuildFrame,child_frame:BuildFrame,after:BuildFrame,child:Child,code:u64)
    requires state::valid_pools(pools),frame_ready(pools,parent),frame_ready(pools,child_frame),
        child_frame.pool_lod+1==parent.pool_lod,covers(parent,crate::morton::proof::value(child_frame.path) as int),covers(parent,code as int),
        state::child_valid(pools,child,parent.pool_lod as nat),frame_replaced(parent,after,child_frame.path,child),
        frame_value(pools,child_frame,code as int)==state::eval(pools,child,parent.pool_lod as nat,code as int%(state::span(parent.pool_lod as nat) as int)),
    ensures frame_ready(pools,after), frame_value(pools,after,code as int)==if covers(child_frame,code as int) {
        frame_value(pools,child_frame,code as int)
    }else{frame_value(pools,parent,code as int)},
{
    child_slot(parent,child_frame,code);
    replaced_frame(pools,parent,after,child_frame.path,child,code);
}

pub(crate) open spec fn chain_ready(pools:state::Pools,frames:Seq<BuildFrame>)->bool {
    frames.len()<=10
        && (frames.len()==0 ||crate::morton::proof::value(frames[0].path)==0)
        && (forall|i:int|0<=i<frames.len() ==> #[trigger] frame_ready(pools,frames[i])
            && frames[i].pool_lod==9-i)
        && (forall|i:int|0<=i && i+1<frames.len() ==> covers(frames[i],crate::morton::proof::value(#[trigger] frames[i+1].path) as int))
}
pub(crate) proof fn chain_prefix(pools:state::Pools,frames:Seq<BuildFrame>,end:nat)
    requires chain_ready(pools,frames),end<=frames.len(),
    ensures chain_ready(pools,frames.take(end as int)),
{
    assert forall|i:int|0<=i<end implies #[trigger] frame_ready(pools,frames.take(end as int)[i])
        && frames.take(end as int)[i].pool_lod==9-i by {
        assert(frame_ready(pools,frames[i]));
    }
    assert forall|i:int|0<=i && i+1<end implies covers(frames.take(end as int)[i],
        crate::morton::proof::value(#[trigger] frames.take(end as int)[i+1].path) as int) by {
        assert(covers(frames[i],crate::morton::proof::value(frames[i+1].path) as int));
    }
}
pub(crate) proof fn chain_frozen(before:state::Pools,after:state::Pools,frames:Seq<BuildFrame>)
    requires state::valid_pools(before),state::valid_pools(after),state::frozen(before,after),chain_ready(before,frames),
    ensures chain_ready(after,frames),
{
    assert forall|i:int|0<=i<frames.len() implies #[trigger] frame_ready(after,frames[i]) && frames[i].pool_lod==9-i by {
        frame_frozen(before,after,frames[i],0);
    }
}

pub(crate) open spec fn frames_closed(before:Seq<BuildFrame>,after:Seq<BuildFrame>,child:Child)->bool {
    before.len()>=2 && after.len()+1==before.len()
        && (forall|i:int|0<=i && i+1<after.len() ==> #[trigger] after[i]==before[i])
        && frame_replaced(before[before.len()-2],after[after.len()-1],before.last().path,child)
}

pub(crate) proof fn closed_chain(pools:state::Pools,before:Seq<BuildFrame>,after:Seq<BuildFrame>,child:Child)
    requires state::valid_pools(pools),chain_ready(pools,before),frames_closed(before,after,child),
        state::child_valid(pools,child,(before.last().pool_lod+1) as nat),
    ensures chain_ready(pools,after),
{
    let parent=(before.len()-2) as int;
    assert(frame_ready(pools,before[parent]));
    assert(frame_ready(pools,before.last()));
    assert(before[parent].pool_lod==before.last().pool_lod+1);
    replaced_frame(pools,before[parent],after[parent],before.last().path,child,0);
    assert forall|i:int|0<=i<after.len() implies #[trigger] frame_ready(pools,after[i])
        && after[i].pool_lod==9-i by {
        if i!=parent {assert(after[i]==before[i]);assert(frame_ready(pools,before[i]));}
    }
    assert forall|i:int|0<=i && i+1<after.len() implies
        covers(after[i],crate::morton::proof::value(#[trigger] after[i+1].path) as int) by {
        assert(covers(before[i],crate::morton::proof::value(before[i+1].path) as int));
    }
}

pub(crate) proof fn frontier_closed(pools:state::Pools,before:Seq<BuildFrame>,after:Seq<BuildFrame>,child:Child,start:nat,code:u64)
    requires state::valid_pools(pools),chain_ready(pools,before),frames_closed(before,after,child),start<after.len(),
        state::child_valid(pools,child,(before.last().pool_lod+1) as nat),covers(before[start as int],code as int),
        frame_value(pools,before.last(),code as int)
            ==state::eval(pools,child,(before.last().pool_lod+1) as nat,code as int%(state::span((before.last().pool_lod+1) as nat) as int)),
    ensures frontier_value(pools,before,start,code as int)==frontier_value(pools,after,start,code as int),
    decreases after.len()-start,
{
    reveal_with_fuel(frontier_value,2);
    closed_chain(pools,before,after,child);
    let parent=(before.len()-2) as int;
    if start<parent {
        assert(after[start as int]==before[start as int]);
        assert(after[(start+1) as int].path==before[(start+1) as int].path);
        assert(after[(start+1) as int].pool_lod==before[(start+1) as int].pool_lod);
        if covers(before[(start+1) as int],code as int){frontier_closed(pools,before,after,child,start+1,code);}
    }else{
        assert(frame_ready(pools,before[parent]));
        assert(frame_ready(pools,before.last()));
        parent_closed(pools,before[parent],before.last(),after[parent],child,code);
    }
}

pub(crate) open spec fn frontier_value(pools:state::Pools,frames:Seq<BuildFrame>,start:nat,code:int)->u32
    decreases frames.len()-start,
{
    if start>=frames.len(){0}else if start+1<frames.len() && covers(frames[(start+1) as int],code){
        frontier_value(pools,frames,start+1,code)
    }else{frame_value(pools,frames[start as int],code)}
}

pub(crate) open spec fn candidate_value(pools:state::Pools,root:Child,frames:Seq<BuildFrame>,code:int)->u32 {
    if frames.len()==0{state::eval(pools,root,10,code)}else{frontier_value(pools,frames,0,code)}
}

pub(crate) proof fn candidate_frozen(before:state::Pools,after:state::Pools,root:Child,frames:Seq<BuildFrame>,code:int)
    requires state::valid_pools(before),state::valid_pools(after),state::frozen(before,after),
        state::child_valid(before,root,10),chain_ready(before,frames),
    ensures state::child_valid(after,root,10),chain_ready(after,frames),
        candidate_value(before,root,frames,code)==candidate_value(after,root,frames,code),
{
    canonical::frozen_value(before,after,root,10,code);
    chain_frozen(before,after,frames);
    frontier_frozen(before,after,frames,0,code);
}

pub(crate) proof fn image_ready(pools:state::Pools,child:Child,level:u8)
    requires state::valid_pools(pools),level<10,state::child_valid(pools,child,(level+1) as nat),
    ensures forall|i:int|0<=i<64 ==> state::child_valid(pools,#[trigger] image(pools[level as int],child)[i],level as nat),
{
    match child {
        Child::Leaf(_)=>{},
        Child::Ptr(index)=>{
            assert(state::occupied(pools[level as int],node::idx(index)));
            assert(pools[level as int].nodes[node::idx(index)] is Some);
        },
    }
}

pub(crate) proof fn root_opened(pools:state::Pools,root:Child,frame:BuildFrame,code:u64)
    requires state::valid_pools(pools),state::child_valid(pools,root,10),code<0x1000_0000_0000_0000u64,
        frame.pool_lod==9,crate::morton::proof::value(frame.path)==0,frame.original==root,!frame.any_modified,
        node::children(frame.entry)==image(pools[9],root),
    ensures chain_ready(pools,seq![frame]),candidate_value(pools,root,seq![frame],code as int)==state::eval(pools,root,10,code as int),
{
    image_ready(pools,root,9);
    assert(frame_ready(pools,frame));
    span_bits(10);
    assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
    lemma_small_mod(code as nat,state::span(10));
    original_value(pools,frame,code as int);
}

pub(crate) open spec fn fresh_child(parent:BuildFrame,child:BuildFrame,pools:state::Pools)->bool {
    child.pool_lod+1==parent.pool_lod
        && covers(parent,crate::morton::proof::value(child.path) as int)
        && child.original==node::children(parent.entry)[slot(child.path,parent.pool_lod)]
        && !child.any_modified && node::children(child.entry)==image(pools[child.pool_lod as int],child.original)
}

pub(crate) proof fn opened_chain(pools:state::Pools,frames:Seq<BuildFrame>,child:BuildFrame)
    requires state::valid_pools(pools),chain_ready(pools,frames),frames.len()>0,fresh_child(frames.last(),child,pools),
    ensures chain_ready(pools,frames.push(child)),
{
    let parent=frames.last();
    assert(frame_ready(pools,parent));
    assert(parent.pool_lod>0);
    slot_matches(child.path,parent.pool_lod);
    assert(0<=slot(child.path,parent.pool_lod)<64);
    assert(state::child_valid(pools,child.original,(child.pool_lod+1) as nat));
    image_ready(pools,child.original,child.pool_lod);
    assert(frame_ready(pools,child));
    assert forall|i:int|0<=i<frames.push(child).len() implies #[trigger] frame_ready(pools,frames.push(child)[i])
        && frames.push(child)[i].pool_lod==9-i by {
        if i<frames.len(){assert(frames.push(child)[i]==frames[i]);assert(frame_ready(pools,frames[i]));}
    }
}

pub(crate) proof fn frontier_opened(pools:state::Pools,frames:Seq<BuildFrame>,child:BuildFrame,start:nat,code:u64)
    requires state::valid_pools(pools),chain_ready(pools,frames),frames.len()>0,start<frames.len(),
        fresh_child(frames.last(),child,pools),covers(frames[start as int],code as int),
    ensures frontier_value(pools,frames.push(child),start,code as int)==frontier_value(pools,frames,start,code as int),
    decreases frames.len()-start,
{
    reveal_with_fuel(frontier_value,2);
    opened_chain(pools,frames,child);
    if start+1<frames.len(){
        if covers(frames[(start+1) as int],code as int){frontier_opened(pools,frames,child,start+1,code);}
    }else{
        let parent=frames.last();
        assert(frame_ready(pools,parent));
        if covers(child,code as int){
            child_slot(parent,child,code);
            slot_model(code,parent.pool_lod);
            original_value(pools,child,code as int);
            canonical::span_positive(child.pool_lod as nat);
            lemma_mod_mod(code as int,state::span((child.pool_lod+1) as nat) as int,state::span(child.pool_lod as nat) as int);
        }
    }
}

pub(crate) proof fn opening_value(pools:state::Pools,root:Child,frames:Seq<BuildFrame>,child:BuildFrame,code:u64)
    requires state::valid_pools(pools),chain_ready(pools,frames),frames.len()>0,
        fresh_child(frames.last(),child,pools),code<0x1000_0000_0000_0000u64,
    ensures chain_ready(pools,frames.push(child)),
        candidate_value(pools,root,frames.push(child),code as int)==candidate_value(pools,root,frames,code as int),
{
    opened_chain(pools,frames,child);
    span_bits(10);
    assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
    assert(frame_ready(pools,frames[0]));
    assert(frames[0].pool_lod==9);
    assert(crate::morton::proof::value(frames[0].path)==0);
    lemma_fundamental_div_mod_converse(code as int,state::span(10) as int,0,code as int);
    assert(covers(frames[0],code as int));
    frontier_opened(pools,frames,child,0,code);
}

pub(crate) proof fn cover_parent(parent:BuildFrame,child:BuildFrame,code:int)
    requires parent.pool_lod<10,child.pool_lod+1==parent.pool_lod,code>=0,
        covers(parent,crate::morton::proof::value(child.path) as int),covers(child,code),
    ensures covers(parent,code),
{
    canonical::span_positive(parent.pool_lod as nat);
    let width=state::span(parent.pool_lod as nat) as int;
    let path=crate::morton::proof::value(child.path) as int;
    lemma_div_denominator(code,width,64);
    lemma_div_denominator(path,width,64);
}

pub(crate) proof fn chain_cover_last(pools:state::Pools,frames:Seq<BuildFrame>,start:nat,code:int)
    requires chain_ready(pools,frames),start<frames.len(),covers(frames.last(),code),code>=0,
    ensures covers(frames[start as int],code),
    decreases frames.len()-start,
{
    assert(frame_ready(pools,frames[start as int]));
    if start+1<frames.len(){
        chain_cover_last(pools,frames,start+1,code);
        assert(frame_ready(pools,frames[(start+1) as int]));
        cover_parent(frames[start as int],frames[(start+1) as int],code);
    }
}

pub(crate) proof fn frontier_last(pools:state::Pools,frames:Seq<BuildFrame>,start:nat,code:int)
    requires chain_ready(pools,frames),start<frames.len(),covers(frames.last(),code),code>=0,
    ensures frontier_value(pools,frames,start,code)==frame_value(pools,frames.last(),code),
    decreases frames.len()-start,
{
    if start+1<frames.len(){chain_cover_last(pools,frames,start+1,code);frontier_last(pools,frames,start+1,code);}
}

pub(crate) open spec fn last_replaced(before:Seq<BuildFrame>,after:Seq<BuildFrame>,path:ZOrder20x3,child:Child)->bool {
    before.len()>0 && after.len()==before.len() && after==before.update(before.len()-1,after.last())
        && frame_replaced(before.last(),after.last(),path,child)
}

pub(crate) proof fn replaced_chain(pools:state::Pools,before:Seq<BuildFrame>,after:Seq<BuildFrame>,path:ZOrder20x3,child:Child)
    requires state::valid_pools(pools),chain_ready(pools,before),last_replaced(before,after,path,child),
        state::child_valid(pools,child,before.last().pool_lod as nat),
    ensures chain_ready(pools,after),
{
    assert(frame_ready(pools,before.last()));
    replaced_frame(pools,before.last(),after.last(),path,child,0);
    assert forall|i:int|0<=i<after.len() implies #[trigger] frame_ready(pools,after[i]) && after[i].pool_lod==9-i by {
        if i+1<after.len(){assert(after[i]==before[i]);assert(frame_ready(pools,before[i]));}
    }
    assert forall|i:int|0<=i && i+1<after.len() implies covers(after[i],
        crate::morton::proof::value(#[trigger] after[i+1].path) as int) by {
        assert(covers(before[i],crate::morton::proof::value(before[i+1].path) as int));
    }
}

pub(crate) proof fn frontier_replaced(pools:state::Pools,before:Seq<BuildFrame>,after:Seq<BuildFrame>,path:ZOrder20x3,child:Child,start:nat,code:u64)
    requires state::valid_pools(pools),chain_ready(pools,before),last_replaced(before,after,path,child),start<before.len(),
        state::child_valid(pools,child,before.last().pool_lod as nat),covers(before[start as int],code as int),
    ensures frontier_value(pools,after,start,code as int)==if covers(before.last(),code as int)
        && ((code>>(before.last().pool_lod as u32*6))%64) as int==slot(path,before.last().pool_lod) {
            state::eval(pools,child,before.last().pool_lod as nat,code as int%(state::span(before.last().pool_lod as nat) as int))
        }else{frontier_value(pools,before,start,code as int)},
    decreases before.len()-start,
{
    reveal_with_fuel(frontier_value,2);
    if start+1<before.len(){
        assert(after[start as int]==before[start as int]);
        assert(after[(start+1) as int].path==before[(start+1) as int].path);
        assert(after[(start+1) as int].pool_lod==before[(start+1) as int].pool_lod);
        if covers(before[(start+1) as int],code as int){frontier_replaced(pools,before,after,path,child,start+1,code);}
        else if covers(before.last(),code as int){chain_cover_last(pools,before,start+1,code as int);}
    }else{
        assert(frame_ready(pools,before.last()));
        replaced_frame(pools,before.last(),after.last(),path,child,code);
    }
}

pub(crate) proof fn last_frame_replaced(pools:state::Pools,root:Child,before:Seq<BuildFrame>,after:Seq<BuildFrame>,path:ZOrder20x3,child:Child,code:u64)
    requires state::valid_pools(pools),chain_ready(pools,before),last_replaced(before,after,path,child),
        state::child_valid(pools,child,before.last().pool_lod as nat),code<0x1000_0000_0000_0000u64,
    ensures chain_ready(pools,after),candidate_value(pools,root,after,code as int)==if covers(before.last(),code as int)
        && ((code>>(before.last().pool_lod as u32*6))%64) as int==slot(path,before.last().pool_lod) {
            state::eval(pools,child,before.last().pool_lod as nat,code as int%(state::span(before.last().pool_lod as nat) as int))
        }else{candidate_value(pools,root,before,code as int)},
{
    replaced_chain(pools,before,after,path,child);
    span_bits(10);
    assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
    assert(frame_ready(pools,before[0]));
    assert(before[0].pool_lod==9);
    assert(crate::morton::proof::value(before[0].path)==0);
    lemma_fundamental_div_mod_converse(code as int,state::span(10) as int,0,code as int);
    assert(covers(before[0],code as int));
    frontier_replaced(pools,before,after,path,child,0,code);
}

pub(crate) proof fn frontier_frozen(before:state::Pools,after:state::Pools,frames:Seq<BuildFrame>,start:nat,code:int)
    requires state::valid_pools(before),state::valid_pools(after),state::frozen(before,after),start<=frames.len(),
        forall|i:int|0<=i<frames.len() ==> #[trigger] frame_ready(before,frames[i]),
    ensures frontier_value(before,frames,start,code)==frontier_value(after,frames,start,code),
    decreases frames.len()-start,
{
    if start<frames.len(){
        frame_frozen(before,after,frames[start as int],code);
        if start+1<frames.len() && covers(frames[(start+1) as int],code){frontier_frozen(before,after,frames,start+1,code);}
    }
}

pub(crate) open spec fn closed_value(before:state::Pools,after:state::Pools,old_frames:Seq<BuildFrame>,new_frames:Seq<BuildFrame>,child:Child,code:u64)->bool {
    frontier_value(before,old_frames,0,code as int)==if old_frames.len()==1 {
        state::eval(after,child,10,code as int)
    }else{frontier_value(after,new_frames,0,code as int)}
}
pub(crate) proof fn finish_semantics(before:state::Pools,after:state::Pools,old_frames:Seq<BuildFrame>,new_frames:Seq<BuildFrame>,child:Child)
    requires state::build_step(before,after),chain_ready(before,old_frames),old_frames.len()>0,
        state::child_valid(after,child,(old_frames.last().pool_lod+1) as nat),
        image(after[old_frames.last().pool_lod as int],child)==node::children(old_frames.last().entry),
        if old_frames.len()==1 {new_frames.len()==0}else{frames_closed(old_frames,new_frames,child)},
    ensures forall|code:u64|code<0x1000_0000_0000_0000u64 ==> #[trigger] closed_value(before,after,old_frames,new_frames,child,code),
{
    chain_frozen(before,after,old_frames);
    span_bits(10);
    assert((1u64<<60u32)==0x1000_0000_0000_0000u64) by(bit_vector);
    assert(state::span(10)==0x1000_0000_0000_0000u64);
    assert forall|code:u64|code<0x1000_0000_0000_0000u64 implies #[trigger] closed_value(before,after,old_frames,new_frames,child,code) by {
        assert(frame_ready(after,old_frames.last()));
        lemma_fundamental_div_mod_converse(code as int,state::span(10) as int,0,code as int);
        entry_value(after,old_frames.last(),child,code as int);
        frontier_frozen(before,after,old_frames,0,code as int);
        if old_frames.len()==1 {
            assert(frame_ready(after,old_frames[0]));
            assert(old_frames[0].pool_lod==9);
            assert(crate::morton::proof::value(old_frames[0].path)==0);
            lemma_small_mod(code as nat,state::span(10));
        }else{
            assert(frame_ready(after,old_frames[0]));
            assert(covers(old_frames[0],code as int));
            frontier_closed(after,old_frames,new_frames,child,0,code);
        }
    }
}
}
