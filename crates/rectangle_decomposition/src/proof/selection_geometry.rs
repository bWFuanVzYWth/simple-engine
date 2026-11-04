//! 原生 chord 索引、good 最大独立集与 raw cut 字段的连接。

use super::{
    HorizontalCut, SparseOptimalScratch64, ValuedChord, VerticalCut, chord_proof as chords,
    cut_proof, mask_proof,
};
use crate::optimal::{graph::proof as graph, matching::proof as matching};
use crate::partition::{good, optimality};
use vstd::prelude::*;

verus! {

pub(super) open spec fn image(chords: Seq<ValuedChord>,indices: Set<int>) -> Set<int> {
    indices.map(|i: int|chords[i].id())
}

pub(super) open spec fn selected(horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,left: Seq<u16>,right: Seq<u16>) -> Set<int> {
    image(horizontal,matching::index_set(left)).union(image(vertical,matching::index_set(right)))
}

pub(super) open spec fn bounded(chords: Seq<ValuedChord>,indices: Set<int>) -> bool {
    forall|i: int|#[trigger] indices.contains(i) ==> 0<=i<chords.len()
}

pub(super) open spec fn complete(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>) -> bool {
    chords::all_good(labels,false,horizontal) && chords::all_good(labels,true,vertical)
        && chords::ids(horizontal).no_duplicates() && chords::ids(vertical).no_duplicates()
        && chords::ids(horizontal).to_set()==good::chords(labels,false)
        && chords::ids(vertical).to_set()==good::chords(labels,true)
}

impl SparseOptimalScratch64 {
    pub(super) open spec fn chosen(&self) -> Set<int> {
        cut_proof::horizontal_ids(self.horizontal_cuts@).union(cut_proof::vertical_ids(self.vertical_cuts@))
    }

    pub(super) open spec fn cuts_ready(&self) -> bool {
        self.horizontal_cuts.valid() && self.vertical_cuts.valid() && self.matching.selection_ready()
            && mask_proof::horizontal_input(self.horizontal_cuts@) && mask_proof::vertical_input(self.vertical_cuts@)
            && cut_proof::selected_cuts(self.chosen(),self.horizontal_cuts@,self.vertical_cuts@)
            && good::maximum_independent(self.rows.labels(),self.chosen())
            && optimality::canonical_choice(self.rows.labels(),self.chosen())
    }
}

pub(super) open spec fn frame(before: &SparseOptimalScratch64,after: &SparseOptimalScratch64) -> bool {
    before.row_buckets==after.row_buckets && before.column_buckets==after.column_buckets
        && before.rows==after.rows && before.columns==after.columns && before.dense_labels==after.dense_labels
        && before.chord_groups==after.chord_groups && before.horizontal_cut_masks==after.horizontal_cut_masks
        && before.vertical_cut_masks==after.vertical_cut_masks && before.partition==after.partition && before.rectangles==after.rectangles
}

proof fn image_size(chords: Seq<ValuedChord>,indices: Set<int>)
    requires chords::ids(chords).no_duplicates(),bounded(chords,indices),
    ensures image(chords,indices).len()==indices.len(),
{
    let f=|i: int|chords[i].id();
    assert(indices.injective_on(f)) by {
        assert forall|a: int,b: int|indices.contains(a) && indices.contains(b) && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            assert(chords::ids(chords)[a]==chords::ids(chords)[b]);
        }
    }
    vstd::set_lib::lemma_map_size(indices,image(chords,indices),f);
}

proof fn image_good(labels: Seq<u16>,vertical: bool,chords: Seq<ValuedChord>,indices: Set<int>)
    requires chords::all_good(labels,vertical,chords),bounded(chords,indices),
    ensures image(chords,indices).subset_of(good::chords(labels,vertical)),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    assert forall|id: int|#[trigger] image(chords,indices).contains(id) implies good::chords(labels,vertical).contains(id) by {
        let i=choose|i: int|indices.contains(i) && #[trigger] chords[i].id()==id;
        chords::native_id(labels,vertical,chords[i]);
    }
}

proof fn conflict(labels: Seq<u16>,horizontal: ValuedChord,vertical: ValuedChord)
    requires chords::native_good(labels,false,horizontal),chords::native_good(labels,true,vertical),
    ensures graph::closed_conflict(horizontal,vertical) <==>good::intersects(horizontal.id(),vertical.id()),
{
    chords::native_id(labels,false,horizontal);chords::native_id(labels,true,vertical);
}

proof fn image_parts(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,left: Set<int>,right: Set<int>)
    requires chords::all_good(labels,false,horizontal),chords::all_good(labels,true,vertical),
        bounded(horizontal,left),bounded(vertical,right),
    ensures optimality::horizontal_part(image(horizontal,left).union(image(vertical,right)))==image(horizontal,left),
        optimality::vertical_part(image(horizontal,left).union(image(vertical,right)))==image(vertical,right),
        image(horizontal,left).union(image(vertical,right)).len()==image(horizontal,left).len()+image(vertical,right).len(),
{
    broadcast use vstd::set_lib::group_set_properties;
    image_good(labels,false,horizontal,left);image_good(labels,true,vertical,right);
    let hs=image(horizontal,left);let vs=image(vertical,right);let result=hs.union(vs);
    assert(optimality::horizontal_part(result) =~=hs);
    assert(optimality::vertical_part(result) =~=vs);
    assert(hs.intersect(vs) =~= Set::<int>::empty());
    vstd::set_lib::lemma_set_intersect_union_lens(hs,vs);
}

proof fn image_independent(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,left: Set<int>,right: Set<int>)
    requires chords::all_good(labels,false,horizontal),chords::all_good(labels,true,vertical),
        matching::index_independent(horizontal,vertical,left,right),
    ensures good::independent(labels,image(horizontal,left).union(image(vertical,right))),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let hs=image(horizontal,left);let vs=image(vertical,right);let result=hs.union(vs);
    image_good(labels,false,horizontal,left);image_good(labels,true,vertical,right);
    assert forall|h: int,v: int|#[trigger] result.contains(h) && #[trigger] result.contains(v)
        && !good::vertical(h) && good::vertical(v) implies !good::intersects(h,v) by {
        assert(hs.contains(h) && vs.contains(v));
        let hi=choose|i: int|left.contains(i) && #[trigger] horizontal[i].id()==h;
        let vi=choose|i: int|right.contains(i) && #[trigger] vertical[i].id()==v;
        conflict(labels,horizontal[hi],vertical[vi]);
    }
}

pub(super) open spec fn preimage(chords: Seq<ValuedChord>,ids: Set<int>) -> Set<int> {
    Set::<int>::range(0,chords.len() as int).filter(|i: int|ids.contains(chords[i].id()))
}

proof fn preimage_exact(chords: Seq<ValuedChord>,ids: Set<int>)
    requires chords::ids(chords).no_duplicates(),ids.subset_of(chords::ids(chords).to_set()),
    ensures bounded(chords,preimage(chords,ids)),image(chords,preimage(chords,ids))==ids,
        preimage(chords,ids).len()==ids.len(),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use vstd::seq_lib::group_seq_properties;
    broadcast use Set::lemma_map_contains;
    let indices=preimage(chords,ids);
    assert(bounded(chords,indices));
    assert forall|id: int|#[trigger] ids.contains(id) implies image(chords,indices).contains(id) by {
        assert(chords::ids(chords).contains(id));
        let i=choose|i: int|0<=i<chords.len() && #[trigger] chords::ids(chords)[i]==id;
        assert(indices.contains(i));
    }
    assert(image(chords,indices) =~=ids);
    image_size(chords,indices);
}

proof fn preimage_independent(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,ids: Set<int>)
    requires complete(labels,horizontal,vertical),good::independent(labels,ids),
    ensures matching::index_independent(horizontal,vertical,preimage(horizontal,optimality::horizontal_part(ids)),
        preimage(vertical,optimality::vertical_part(ids))),
{
    broadcast use vstd::set_lib::group_set_properties;
    let left=preimage(horizontal,optimality::horizontal_part(ids));
    let right=preimage(vertical,optimality::vertical_part(ids));
    assert forall|h: int,v: int|left.contains(h) && right.contains(v) implies !graph::closed_conflict(horizontal[h],vertical[v]) by {
        conflict(labels,horizontal[h],vertical[v]);
        assert(ids.contains(horizontal[h].id()) && ids.contains(vertical[v].id()));
    }
}

/// 任意竞争 good 集完整还原为两向有界索引，故原生最大性保留到唯一像素模型。
pub(super) proof fn maximum(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,left: Seq<u16>,right: Seq<u16>)
    requires complete(labels,horizontal,vertical),matching::maximum_selection(horizontal,vertical,left,right),
    ensures good::maximum_independent(labels,selected(horizontal,vertical,left,right)),
        selected(horizontal,vertical,left,right).len()==left.len()+right.len(),
        image(horizontal,matching::index_set(left))==optimality::horizontal_part(selected(horizontal,vertical,left,right)),
        image(vertical,matching::index_set(right))==optimality::vertical_part(selected(horizontal,vertical,left,right)),
{
    broadcast use vstd::set_lib::group_set_properties;
    let lhs=matching::index_set(left);let rhs=matching::index_set(right);
    matching::index_set_view(left);matching::index_set_view(right);
    image_size(horizontal,lhs);image_size(vertical,rhs);image_parts(labels,horizontal,vertical,lhs,rhs);
    image_independent(labels,horizontal,vertical,lhs,rhs);let result=selected(horizontal,vertical,left,right);
    assert forall|other: Set<int>|#[trigger] good::independent(labels,other) implies other.len()<=result.len() by {
        let hs=optimality::horizontal_part(other);let vs=optimality::vertical_part(other);
        assert(hs.subset_of(chords::ids(horizontal).to_set()));assert(vs.subset_of(chords::ids(vertical).to_set()));
        preimage_exact(horizontal,hs);preimage_exact(vertical,vs);preimage_independent(labels,horizontal,vertical,other);
        assert(preimage(horizontal,hs).len()+preimage(vertical,vs).len()<=left.len()+right.len());
        assert(other =~= hs.union(vs));assert(hs.intersect(vs) =~= Set::<int>::empty());
        vstd::set_lib::lemma_set_intersect_union_lens(hs,vs);
    }
}

proof fn image_maximum(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,left: Set<int>,right: Set<int>)
    requires complete(labels,horizontal,vertical),matching::index_maximum(horizontal,vertical,left,right),
    ensures good::maximum_independent(labels,image(horizontal,left).union(image(vertical,right))),
{
    broadcast use vstd::set_lib::group_set_properties;
    image_size(horizontal,left);image_size(vertical,right);image_parts(labels,horizontal,vertical,left,right);
    image_independent(labels,horizontal,vertical,left,right);
    let result=image(horizontal,left).union(image(vertical,right));
    assert forall|other: Set<int>|#[trigger] good::independent(labels,other) implies other.len()<=result.len() by {
        let hs=optimality::horizontal_part(other);let vs=optimality::vertical_part(other);
        assert(hs.subset_of(chords::ids(horizontal).to_set()));assert(vs.subset_of(chords::ids(vertical).to_set()));
        preimage_exact(horizontal,hs);preimage_exact(vertical,vs);preimage_independent(labels,horizontal,vertical,other);
        assert(preimage(horizontal,hs).len()+preimage(vertical,vs).len()<=left.len()+right.len());
        assert(other =~=hs.union(vs));assert(hs.intersect(vs) =~=Set::<int>::empty());
        vstd::set_lib::lemma_set_intersect_union_lens(hs,vs);
    }
}

proof fn preimage_maximum(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,selected: Set<int>)
    requires complete(labels,horizontal,vertical),good::maximum_independent(labels,selected),
    ensures matching::index_maximum(horizontal,vertical,preimage(horizontal,optimality::horizontal_part(selected)),
        preimage(vertical,optimality::vertical_part(selected))),
{
    broadcast use vstd::set_lib::group_set_properties;
    let hs=optimality::horizontal_part(selected);let vs=optimality::vertical_part(selected);
    assert(hs.subset_of(chords::ids(horizontal).to_set()));assert(vs.subset_of(chords::ids(vertical).to_set()));
    preimage_exact(horizontal,hs);preimage_exact(vertical,vs);preimage_independent(labels,horizontal,vertical,selected);
    assert(selected =~=hs.union(vs));assert(hs.intersect(vs) =~=Set::<int>::empty());
    vstd::set_lib::lemma_set_intersect_union_lens(hs,vs);
    assert forall|left: Set<int>,right: Set<int>|#[trigger] matching::index_independent(horizontal,vertical,left,right)
        implies left.len()+right.len()<=preimage(horizontal,hs).len()+preimage(vertical,vs).len() by {
        image_size(horizontal,left);image_size(vertical,right);image_parts(labels,horizontal,vertical,left,right);
        image_independent(labels,horizontal,vertical,left,right);
    }
}

proof fn all_vertical(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>)
    requires complete(labels,horizontal,vertical),
    ensures preimage(horizontal,optimality::horizontal_part(good::chords(labels,true)))==Set::<int>::empty(),
        preimage(vertical,optimality::vertical_part(good::chords(labels,true)))==matching::index_domain(vertical.len()),
        image(horizontal,Set::<int>::empty()).union(image(vertical,matching::index_domain(vertical.len())))==good::chords(labels,true),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use vstd::seq_lib::group_seq_properties;
    let domain=matching::index_domain(vertical.len());let sequence=Seq::new(vertical.len(),|i: int|i);
    assert forall|i: int|#[trigger] domain.contains(i) <==>0<=i<vertical.len() by {
        if domain.contains(i) {let j=choose|j: int|0<=j<sequence.len() && #[trigger] sequence[j]==i;}
        if 0<=i<vertical.len() {assert(sequence[i]==i);}
    }
    assert(preimage(horizontal,optimality::horizontal_part(good::chords(labels,true))) =~=Set::<int>::empty());
    assert forall|i: int|0<=i<vertical.len() implies optimality::vertical_part(good::chords(labels,true)).contains((#[trigger] vertical[i]).id()) by {
        chords::native_id(labels,true,vertical[i]);
    }
    assert(preimage(vertical,optimality::vertical_part(good::chords(labels,true))) =~=domain);
    preimage_exact(vertical,good::chords(labels,true));
    assert(preimage(vertical,good::chords(labels,true)) =~=domain);
    assert(image(horizontal,Set::<int>::empty()) =~=Set::<int>::empty());
}

/// Kőnig 极值选择经完整 chord 双射映射为同一标签图的唯一规范选线。
pub(super) proof fn canonical(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,left: Seq<u16>,right: Seq<u16>)
    requires complete(labels,horizontal,vertical),matching::canonical_selection(horizontal,vertical,left,right),
    ensures optimality::canonical_choice(labels,selected(horizontal,vertical,left,right)),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    maximum(labels,horizontal,vertical,left,right);all_vertical(labels,horizontal,vertical);
    let result=selected(horizontal,vertical,left,right);let all=good::chords(labels,true);
    if good::maximum_independent(labels,all) {
        preimage_maximum(labels,horizontal,vertical,all);
        assert(matching::index_maximum(horizontal,vertical,Set::empty(),matching::index_domain(vertical.len())));
        assert(matching::index_set(left) =~=Set::<int>::empty());
        assert(result==all);
    } else {
        if matching::index_maximum(horizontal,vertical,Set::empty(),matching::index_domain(vertical.len())) {
            image_maximum(labels,horizontal,vertical,Set::empty(),matching::index_domain(vertical.len()));
        }
        assert forall|other: Set<int>|#[trigger] good::maximum_independent(labels,other)
            implies optimality::vertical_part(result).subset_of(optimality::vertical_part(other)) by {
            preimage_maximum(labels,horizontal,vertical,other);
            let vs=optimality::vertical_part(other);let indices=preimage(vertical,vs);
            assert(matching::index_set(right).subset_of(indices));
            assert forall|id: int|#[trigger] optimality::vertical_part(result).contains(id) implies vs.contains(id) by {
                let i=choose|i: int|matching::index_set(right).contains(i) && #[trigger] vertical[i].id()==id;
                assert(indices.contains(i));
            }
        }
    }
}

pub(super) open spec fn horizontal_cut(chord: ValuedChord) -> HorizontalCut {
    HorizontalCut{y:chord.chord.y1,start:chord.chord.x1,end:chord.chord.x2}
}

pub(super) open spec fn vertical_cut(chord: ValuedChord) -> VerticalCut {
    VerticalCut{x:chord.chord.x1,start:chord.chord.y1,end:chord.chord.y2}
}

pub(super) open spec fn horizontal_cuts(chords: Seq<ValuedChord>,indices: Seq<u16>) -> Seq<HorizontalCut> {
    indices.map_values(|i: u16|horizontal_cut(chords[i as int]))
}

pub(super) open spec fn vertical_cuts(chords: Seq<ValuedChord>,indices: Seq<u16>) -> Seq<VerticalCut> {
    indices.map_values(|i: u16|vertical_cut(chords[i as int]))
}

pub(super) proof fn horizontal_step(chords: Seq<ValuedChord>,indices: Seq<u16>,end: int)
    requires 0<=end<indices.len(),
    ensures horizontal_cuts(chords,indices.take(end+1))==horizontal_cuts(chords,indices.take(end)).push(horizontal_cut(chords[indices[end] as int])),
{
    assert(horizontal_cuts(chords,indices.take(end+1)) =~=horizontal_cuts(chords,indices.take(end)).push(horizontal_cut(chords[indices[end] as int])));
}

pub(super) proof fn vertical_step(chords: Seq<ValuedChord>,indices: Seq<u16>,end: int)
    requires 0<=end<indices.len(),
    ensures vertical_cuts(chords,indices.take(end+1))==vertical_cuts(chords,indices.take(end)).push(vertical_cut(chords[indices[end] as int])),
{
    assert(vertical_cuts(chords,indices.take(end+1)) =~=vertical_cuts(chords,indices.take(end)).push(vertical_cut(chords[indices[end] as int])));
}

pub(super) proof fn horizontal_coordinates(chord: ValuedChord)
    requires chord.well_formed(false),
    ensures mask_proof::horizontal_valid(horizontal_cut(chord)),cut_proof::horizontal_id(horizontal_cut(chord))==chord.id(),
{}

pub(super) proof fn vertical_coordinates(chord: ValuedChord)
    requires chord.well_formed(true),
    ensures mask_proof::vertical_valid(vertical_cut(chord)),cut_proof::vertical_id(vertical_cut(chord))==chord.id(),
{}

proof fn horizontal_projection(labels: Seq<u16>,chords: Seq<ValuedChord>,indices: Seq<u16>)
    requires chords::all_good(labels,false,chords),matching::valid_indices(indices,chords.len()),
    ensures mask_proof::horizontal_input(horizontal_cuts(chords,indices)),
        cut_proof::horizontal_ids(horizontal_cuts(chords,indices))==image(chords,matching::index_set(indices)),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let cuts=horizontal_cuts(chords,indices);let chosen=matching::index_set(indices);
    matching::index_set_view(indices);
    assert forall|p: int|0<=p<cuts.len() implies #[trigger] mask_proof::horizontal_valid(cuts[p]) by {
        assert(chords::native_good(labels,false,chords[indices[p] as int]));
        horizontal_coordinates(chords[indices[p] as int]);
    }
    assert forall|id: int|#[trigger] cut_proof::horizontal_ids(cuts).contains(id) implies image(chords,chosen).contains(id) by {
        let p=choose|p: int|Set::<int>::range(0,cuts.len() as int).contains(p) && #[trigger] cut_proof::horizontal_id(cuts[p])==id;
        assert(chords::native_good(labels,false,chords[indices[p] as int]));
        horizontal_coordinates(chords[indices[p] as int]);assert(chosen.contains(indices[p] as int));
        assert(exists|i: int|chosen.contains(i) && #[trigger] chords[i].id()==id);
    }
    assert forall|id: int|#[trigger] image(chords,chosen).contains(id) implies cut_proof::horizontal_ids(cuts).contains(id) by {
        let i=choose|i: int|chosen.contains(i) && #[trigger] chords[i].id()==id;
        let p=choose|p: int|0<=p<indices.len() && #[trigger] indices[p] as int==i;
        assert(chords::native_good(labels,false,chords[i]));
        horizontal_coordinates(chords[i]);cut_proof::horizontal_member(cuts,p);
    }
}

proof fn vertical_projection(labels: Seq<u16>,chords: Seq<ValuedChord>,indices: Seq<u16>)
    requires chords::all_good(labels,true,chords),chords::ids(chords).no_duplicates(),matching::valid_indices(indices,chords.len()),
    ensures mask_proof::vertical_input(vertical_cuts(chords,indices)),
        cut_proof::vertical_ids(vertical_cuts(chords,indices))==image(chords,matching::index_set(indices)),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let cuts=vertical_cuts(chords,indices);let chosen=matching::index_set(indices);
    matching::index_set_view(indices);chords::graph_domain(labels,true,chords);
    assert forall|p: int|0<=p<cuts.len() implies #[trigger] mask_proof::vertical_valid(cuts[p]) by {
        vertical_coordinates(chords[indices[p] as int]);
    }
    assert forall|p: int,q: int|0<=p<q<cuts.len() && cuts[p].x==cuts[q].x
        implies #[trigger] cuts[p].end<#[trigger] cuts[q].start || cuts[q].end<cuts[p].start by {
        let i=indices[p] as int;let j=indices[q] as int;
        assert(i!=j);assert(chords[i].line()==chords[j].line());
        if i<j {}else{assert(j<i);}
    }
    assert forall|id: int|#[trigger] cut_proof::vertical_ids(cuts).contains(id) implies image(chords,chosen).contains(id) by {
        let p=choose|p: int|Set::<int>::range(0,cuts.len() as int).contains(p) && #[trigger] cut_proof::vertical_id(cuts[p])==id;
        vertical_coordinates(chords[indices[p] as int]);assert(chosen.contains(indices[p] as int));
        assert(exists|i: int|chosen.contains(i) && #[trigger] chords[i].id()==id);
    }
    assert forall|id: int|#[trigger] image(chords,chosen).contains(id) implies cut_proof::vertical_ids(cuts).contains(id) by {
        let i=choose|i: int|chosen.contains(i) && #[trigger] chords[i].id()==id;
        let p=choose|p: int|0<=p<indices.len() && #[trigger] indices[p] as int==i;
        vertical_coordinates(chords[i]);cut_proof::vertical_member(cuts,p);
    }
}

pub(super) proof fn raw_cuts(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,left: Seq<u16>,right: Seq<u16>)
    requires complete(labels,horizontal,vertical),matching::maximum_selection(horizontal,vertical,left,right),
    ensures good::maximum_independent(labels,selected(horizontal,vertical,left,right)),
        mask_proof::horizontal_input(horizontal_cuts(horizontal,left)),mask_proof::vertical_input(vertical_cuts(vertical,right)),
        cut_proof::selected_cuts(selected(horizontal,vertical,left,right),horizontal_cuts(horizontal,left),vertical_cuts(vertical,right)),
        good::maximum_independent(labels,cut_proof::horizontal_ids(horizontal_cuts(horizontal,left)).union(cut_proof::vertical_ids(vertical_cuts(vertical,right)))),
{
    maximum(labels,horizontal,vertical,left,right);
    horizontal_projection(labels,horizontal,left);vertical_projection(labels,vertical,right);
    selected_union(selected(horizontal,vertical,left,right),horizontal_cuts(horizontal,left),vertical_cuts(vertical,right));
}

pub(super) proof fn selected_union(ids: Set<int>,horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>)
    requires cut_proof::selected_cuts(ids,horizontal,vertical),
    ensures cut_proof::horizontal_ids(horizontal).union(cut_proof::vertical_ids(vertical))==ids,
{
    broadcast use vstd::set_lib::group_set_properties;
    assert(cut_proof::horizontal_ids(horizontal).union(cut_proof::vertical_ids(vertical)) =~=ids);
}

pub(super) proof fn raw_parts(horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>)
    requires mask_proof::horizontal_input(horizontal),mask_proof::vertical_input(vertical),
    ensures cut_proof::selected_cuts(cut_proof::horizontal_ids(horizontal).union(cut_proof::vertical_ids(vertical)),horizontal,vertical),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let hs=cut_proof::horizontal_ids(horizontal);let vs=cut_proof::vertical_ids(vertical);let ids=hs.union(vs);
    assert forall|id: int|#[trigger] hs.contains(id) implies !good::vertical(id) by {
        let i=choose|i: int|Set::<int>::range(0,horizontal.len() as int).contains(i) && #[trigger] cut_proof::horizontal_id(horizontal[i])==id;
        cut_proof::horizontal_coordinates(horizontal[i]);
    }
    assert forall|id: int|#[trigger] vs.contains(id) implies good::vertical(id) by {
        let i=choose|i: int|Set::<int>::range(0,vertical.len() as int).contains(i) && #[trigger] cut_proof::vertical_id(vertical[i])==id;
        cut_proof::vertical_coordinates(vertical[i]);
    }
    assert(optimality::horizontal_part(ids) =~=hs);assert(optimality::vertical_part(ids) =~=vs);
}

pub(super) proof fn empty(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>)
    requires complete(labels,horizontal,vertical),horizontal.len()==0,vertical.len()==0,
    ensures good::maximum_independent(labels,Set::<int>::empty()),
        optimality::canonical_choice(labels,Set::<int>::empty()),
        mask_proof::horizontal_input(Seq::<HorizontalCut>::empty()),mask_proof::vertical_input(Seq::<VerticalCut>::empty()),
        cut_proof::selected_cuts(Set::<int>::empty(),Seq::<HorizontalCut>::empty(),Seq::<VerticalCut>::empty()),
{
    broadcast use vstd::set_lib::group_set_properties;
    assert(chords::ids(horizontal) =~=Seq::<int>::empty());assert(chords::ids(vertical) =~=Seq::<int>::empty());
    assert(good::chords(labels,false) =~=Set::<int>::empty());assert(good::chords(labels,true) =~=Set::<int>::empty());
    assert forall|other: Set<int>|#[trigger] good::independent(labels,other) implies other.len()==0 by {
        assert forall|id: int|#[trigger] other.contains(id) implies false by {
            assert(good::good(labels,id));
            assert(good::chords(labels,good::vertical(id)).contains(id));
        }
        assert(other =~=Set::<int>::empty());
    }
    assert(cut_proof::horizontal_ids(Seq::<HorizontalCut>::empty()) =~=Set::<int>::empty());
    assert(cut_proof::vertical_ids(Seq::<VerticalCut>::empty()) =~=Set::<int>::empty());
    assert(optimality::horizontal_part(Set::<int>::empty()) =~=Set::<int>::empty());
    assert(optimality::vertical_part(Set::<int>::empty()) =~=Set::<int>::empty());
}

#[cfg(reject_selection_lost_chord)]
proof fn reject_lost_chord(labels: Seq<u16>,horizontal: Seq<ValuedChord>,vertical: Seq<ValuedChord>,left: Seq<u16>,right: Seq<u16>)
    requires complete(labels,horizontal,vertical),matching::maximum_selection(horizontal,vertical,left,right),
{
    maximum(labels,horizontal,vertical,left,right);
    assert(selected(horizontal,vertical,left,right).len()<left.len()+right.len());
}

#[cfg(reject_selection_vertical_field)]
proof fn reject_vertical_field(chord: ValuedChord)
    requires chord.well_formed(true),chord.chord.x1!=chord.chord.y1,
{
    assert(vertical_cut(chord).start==chord.chord.x1);
}

#[cfg(reject_select_horizontal_order)]
fn reject_horizontal_order() {
    let chords=[ValuedChord::on_axis::<true>(3,1,2,7),ValuedChord::on_axis::<true>(4,1,2,7)];
    let indices=[1u16,0u16];
    let mut output=crate::optimal::matching::ChordBuffer::<HorizontalCut>::new(HorizontalCut{y:0,start:0,end:0});
    let result=super::select::fill_horizontal_cuts(&chords,&indices,&mut output);
    assert(result.is_ok());assert(output@[0].y==3);
}

#[cfg(reject_select_vertical_projection)]
fn reject_vertical_projection() {
    let chords=[ValuedChord::on_axis::<false>(2,1,3,7)];let indices=[0u16];
    let mut output=crate::optimal::matching::ChordBuffer::<VerticalCut>::new(VerticalCut{x:0,start:0,end:0});
    let result=super::select::fill_vertical_cuts(&chords,&indices,&mut output);
    assert(result.is_ok());assert(output@[0].start==2);
}

}
