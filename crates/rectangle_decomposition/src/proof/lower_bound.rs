//! 从任意真实矩形分区显式构造独立集，给出全域最优性下界。

use super::{Rectangle, chain_marks, chain_set, chains, colored, conflict_budget, good, partition};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn removed(labels: Seq<u16>,rectangles: Seq<Rectangle>) -> Set<int> {
    chain_set::chords(labels,rectangles).filter(|h: int|!good::vertical(h)
        && exists|v: int| chain_set::chords(labels,rectangles).contains(v)
            && good::vertical(v) && #[trigger] good::intersects(h,v))
}

pub(crate) open spec fn chosen(labels: Seq<u16>,rectangles: Seq<Rectangle>) -> Set<int> {
    chain_set::chords(labels,rectangles).difference(removed(labels,rectangles))
}

spec fn witness(labels: Seq<u16>,rectangles: Seq<Rectangle>,h: int) -> int {
    let v=choose|v: int| chain_set::chords(labels,rectangles).contains(v)
        && good::vertical(v) && #[trigger] good::intersects(h,v);
    h*524288+v
}

proof fn witness_edge(labels: Seq<u16>,rectangles: Seq<Rectangle>,h: int)
    requires removed(labels,rectangles).contains(h),
    ensures conflict_budget::conflicts(labels,rectangles).contains(witness(labels,rectangles,h)),
        witness(labels,rectangles,h)/524288==h,
{
    chain_set::actual_chord_set(labels,rectangles);
    let v=choose|v: int| chain_set::chords(labels,rectangles).contains(v)
        && good::vertical(v) && #[trigger] good::intersects(h,v);
    let e=h*524288+v;
    assert(e/524288==h && e%524288==v) by(nonlinear_arith) requires 0<=v<524288,e==h*524288+v;
    assert(0<=e<274877906944);
}

/// 删除的每条 H 单射到一条真实冲突边，不按同一 H 的多条冲突重复扣减。
pub(crate) proof fn remove_conflicts(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    ensures good::independent(labels,chosen(labels,rectangles)),
        chosen(labels,rectangles).len() as int >=chain_set::chords(labels,rectangles).len() as int
            -conflict_budget::conflicts(labels,rectangles).len() as int,
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let all=chain_set::chords(labels,rectangles);
    let deleted=removed(labels,rectangles);
    let out=chosen(labels,rectangles);
    chain_set::actual_chord_set(labels,rectangles);
    assert(deleted.subset_of(all));
    vstd::set_lib::lemma_set_difference_len(all,deleted);
    assert(all.intersect(deleted) =~= deleted);
    let f=|h: int|witness(labels,rectangles,h);
    let image=deleted.map(f);
    assert(deleted.injective_on(f)) by {
        assert forall|a: int,b: int| deleted.contains(a) && deleted.contains(b)
            && #[trigger] f(a)==#[trigger] f(b) implies a==b by {
            witness_edge(labels,rectangles,a);witness_edge(labels,rectangles,b);
        }
    }
    vstd::set_lib::lemma_map_size(deleted,image,f);
    assert(image.subset_of(conflict_budget::conflicts(labels,rectangles))) by {
        assert forall|edge: int| #[trigger] image.contains(edge)
            implies conflict_budget::conflicts(labels,rectangles).contains(edge) by {
            let h=choose|h: int|deleted.contains(h) && f(h)==edge;
            witness_edge(labels,rectangles,h);
        }
    }
    vstd::set_lib::lemma_len_subset(image,conflict_budget::conflicts(labels,rectangles));
    assert forall|id: int| #[trigger] out.contains(id) implies good::good(labels,id) by {}
    assert forall|h: int,v: int| #[trigger] out.contains(h) && #[trigger] out.contains(v)
        && !good::vertical(h) && good::vertical(v) implies !good::intersects(h,v) by {
        if good::intersects(h,v) {
            assert(exists|v: int|all.contains(v) && good::vertical(v) && #[trigger] good::intersects(h,v));
            assert(deleted.contains(h));
        }
    }
}

/// 任意竞争者可产生一个实际 good chord 独立集 J，达到角点下界所需的预算。
pub(crate) proof fn competitor_witness(labels: Seq<u16>,rectangles: Seq<Rectangle>)
    requires partition(labels,rectangles),
    ensures good::independent(labels,chosen(labels,rectangles)),
        colored::b4(labels)<=4*rectangles.len()+4*chosen(labels,rectangles).len(),
{
    chains::partition_budget(labels,rectangles);
    chain_marks::global_paired_bound(labels,rectangles);
    chain_set::actual_chord_set(labels,rectangles);
    conflict_budget::actual_conflict_bound(labels,rectangles);
    remove_conflicts(labels,rectangles);
}

/// 最大独立集给出对所有矩形分区的下界；包含孔洞、点接触、T/X 竞争者。
pub(crate) proof fn partition_lower_bound(labels: Seq<u16>,rectangles: Seq<Rectangle>,selected: Set<int>)
    requires partition(labels,rectangles),good::maximum_independent(labels,selected),
    ensures colored::b4(labels)<=4*rectangles.len()+4*selected.len(),
{
    competitor_witness(labels,rectangles);
    assert(good::independent(labels,chosen(labels,rectangles)));
}

}
