//! 真实扫描计数与竞争者下界的组合；扫描调用者须证明达到该计数。

use super::{Rectangle, colored, good, lower_bound, optimal, partition, row_chords, row_count};
use vstd::prelude::*;

verus! {

pub(crate) open spec fn vertical_part(selected: Set<int>) -> Set<int> {
    selected.filter(|id: int|good::vertical(id))
}

pub(crate) open spec fn horizontal_part(selected: Set<int>) -> Set<int> {
    selected.filter(|id: int|!good::vertical(id))
}

pub(crate) open spec fn neighbors(labels: Seq<u16>,verticals: Set<int>) -> Set<int> {
    good::chords(labels,false).filter(|h: int|exists|v: int| #[trigger] verticals.contains(v)
        && good::intersects(h,v))
}

/// 左侧可完全匹配时选择全部 V；否则选择所有最大独立集共有的 V。
pub(crate) open spec fn canonical_choice(labels: Seq<u16>,selected: Set<int>) -> bool {
    good::maximum_independent(labels,selected)
        && if good::maximum_independent(labels,good::chords(labels,true)) {
            selected==good::chords(labels,true)
        } else {
            forall|other: Set<int>| #[trigger] good::maximum_independent(labels,other)
                ==> vertical_part(selected).subset_of(vertical_part(other))
        }
}

/// 固定 tie 规则确定选线集合；不要求匹配边或内部搜索轨迹相同。
pub(crate) proof fn canonical_unique(labels: Seq<u16>,first: Set<int>,second: Set<int>)
    requires canonical_choice(labels,first),canonical_choice(labels,second),
    ensures first==second,
{
    broadcast use vstd::set_lib::group_set_properties;
    if !good::maximum_independent(labels,good::chords(labels,true)) {
        assert(vertical_part(first).subset_of(vertical_part(second)));
        assert(vertical_part(second).subset_of(vertical_part(first)));
        assert(vertical_part(first) =~=vertical_part(second));
        maximal_horizontal(labels,first);maximal_horizontal(labels,second);
        assert(horizontal_part(first)==horizontal_part(second));
        assert(first =~=horizontal_part(first).union(vertical_part(first)));
        assert(second =~=horizontal_part(second).union(vertical_part(second)));
    }
}

pub(crate) open spec fn selected_for(labels: Seq<u16>) -> Set<int> {
    choose|selected: Set<int>|canonical_choice(labels,selected)
}

pub(crate) open spec fn canonical_block(selected: Set<int>) -> spec_fn(int,int,int)->bool {
    |y: int,a: int,b: int|0<=y<64 && 0<=a<b<=64 && super::scan_count::horizontal_block(selected,y,a,b)
}

/// 仅由当前标签图定义的输出；存在性由实际已验证 producer 交付的选线见证建立。
pub(crate) open spec fn result_for(labels: Seq<u16>) -> Seq<Rectangle> {
    super::scan_model::canonical(labels,vertical_part(selected_for(labels)),canonical_block(selected_for(labels)))
}

pub(crate) proof fn selected_for_unique(labels: Seq<u16>,selected: Set<int>)
    requires canonical_choice(labels,selected),
    ensures selected_for(labels)==selected,
{
    assert(exists|value: Set<int>|canonical_choice(labels,value));
    canonical_unique(labels,selected_for(labels),selected);
}

/// 最大推出极大：所有不与 S 冲突的 H 都必须已经选中。
pub(crate) proof fn maximal_horizontal(labels: Seq<u16>,selected: Set<int>)
    requires good::maximum_independent(labels,selected),
    ensures horizontal_part(selected) =~=good::chords(labels,false).difference(neighbors(labels,vertical_part(selected))),
{
    broadcast use vstd::set_lib::group_set_properties;
    let hs=horizontal_part(selected);let vs=vertical_part(selected);
    let possible=good::chords(labels,false).difference(neighbors(labels,vs));
    assert forall|h: int| #[trigger] hs.contains(h) implies possible.contains(h) by {
        if neighbors(labels,vs).contains(h) {
            let v=choose|v: int| vs.contains(v) && #[trigger] good::intersects(h,v);
        }
    }
    assert forall|h: int| #[trigger] possible.contains(h) implies hs.contains(h) by {
        if !selected.contains(h) {
            let larger=selected.insert(h);
            assert(good::independent(labels,larger)) by {
                assert forall|id: int| #[trigger] larger.contains(id) implies good::good(labels,id) by {}
                assert forall|a: int,b: int| #[trigger] larger.contains(a) && #[trigger] larger.contains(b)
                    && !good::vertical(a) && good::vertical(b) implies !good::intersects(a,b) by {
                    if a==h {
                        assert(vs.contains(b));
                        if good::intersects(a,b) {
                            assert(exists|v: int|vs.contains(v) && #[trigger] good::intersects(h,v));
                            assert(neighbors(labels,vs).contains(h));
                        }
                    }
                }
            }
            assert(larger.len()>selected.len());
        }
    }
}

pub(crate) proof fn maximum_cardinality(labels: Seq<u16>,selected: Set<int>)
    requires good::maximum_independent(labels,selected),
    ensures selected.len() as int==good::chords(labels,false).len() as int
        -neighbors(labels,vertical_part(selected)).len() as int+vertical_part(selected).len() as int,
{
    broadcast use vstd::set_lib::group_set_properties;
    let hs=horizontal_part(selected);let vs=vertical_part(selected);
    let domain=good::chords(labels,false);let excluded=neighbors(labels,vs);
    maximal_horizontal(labels,selected);
    assert(selected =~= hs.union(vs));assert(hs.intersect(vs) =~= Set::<int>::empty());
    vstd::set_lib::lemma_set_intersect_union_lens(hs,vs);
    vstd::set_lib::lemma_set_difference_len(domain,excluded);
    assert(excluded.subset_of(domain));assert(domain.intersect(excluded) =~=excluded);
}

/// 条件式组合边界：精确覆盖与扫描计数均须由实际生产代码建立。
pub(crate) proof fn from_exact_count(labels: Seq<u16>,rectangles: Seq<Rectangle>,selected: Set<int>)
    requires partition(labels,rectangles),good::maximum_independent(labels,selected),
        4*rectangles.len()+4*selected.len()==colored::b4(labels),
    ensures optimal(labels,rectangles),
{
    assert forall|other: Seq<Rectangle>| #[trigger] partition(labels,other)
        implies rectangles.len()<=other.len() by {
        lower_bound::partition_lower_bound(labels,other,selected);
    }
}

/// 当前扫描器的目标数量式达到角点下界；并非替代其待证明的实际数量后置。
pub(crate) proof fn from_run_count(labels: Seq<u16>,rectangles: Seq<Rectangle>,selected: Set<int>)
    requires partition(labels,rectangles),good::maximum_independent(labels,selected),
        rectangles.len() as int==row_count::total_runs(labels)-row_count::identical(labels)
            -vertical_part(selected).len() as int+neighbors(labels,vertical_part(selected)).len() as int,
    ensures optimal(labels,rectangles),
{
    row_chords::original_run_identity(labels);
    maximum_cardinality(labels,selected);
    from_exact_count(labels,rectangles,selected);
}

}
