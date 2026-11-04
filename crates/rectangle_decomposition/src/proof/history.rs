//! 当前标签图的规范选线与实际位图扫描序列；不依赖 scratch 的旧字段。

use super::{
    AxisIntervals, HorizontalCut, VerticalCut, cut_proof as cuts, mask_proof as masks, scan,
};
use crate::partition::{optimality, scan_model};
use vstd::prelude::*;

verus! {

pub(super) proof fn prepared(rows: &AxisIntervals,selected: Set<int>,horizontal: Seq<HorizontalCut>,vertical: Seq<VerticalCut>,
    hm: Seq<u64>,vm: Seq<u128>,boundaries: u64)
    requires rows.valid(),rows.lines@.len()==64,rows.boundaries_complete(),
        optimality::canonical_choice(rows.labels(),selected),cuts::selected_cuts(selected,horizontal,vertical),
        masks::built(horizontal,vertical,hm,vm,boundaries),
    ensures scan_model::canonical(rows.labels(),scan::proof::selection(rows.labels(),vm),scan::proof::block_fn(hm))
        ==optimality::result_for(rows.labels()),
{
    cuts::ready(rows,selected,horizontal,vertical,hm,vm,boundaries);
    optimality::selected_for_unique(rows.labels(),selected);
    let actual=scan::proof::block_fn(hm);
    let expected=optimality::canonical_block(selected);
    assert forall|y: int,a: int,b: int|#[trigger] actual(y,a,b)==expected(y,a,b) by {
        if 0<=y<64 && 0<=a<b<=64 { cuts::blocked(horizontal,vertical,hm,vm,boundaries,selected,y,a,b); }
    }
    assert(actual =~=expected);
}

}
