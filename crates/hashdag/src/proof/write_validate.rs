//! 公开区域跨字段约束；错误分支给出违反完整输入谓词的见证。
use super::{DenseRegion, UniformFill};
use vstd::prelude::*;

verus! {
pub open spec fn size(lod: u8) -> u64 { 1u64 << (lod as u32 * 3) }
pub open spec fn base(fill: UniformFill) -> u64 {
    crate::morton::proof::value(fill.path) & !((size(fill.lod) - 1) as u64)
}
pub open spec fn fill_end(fill: UniformFill) -> int { base(fill) as int + size(fill.lod) as int }
pub open spec fn fill_at(fills: Seq<UniformFill>, index: nat) -> bool {
    let fill = fills[index as int];
    crate::morton::proof::value(fill.path)<0x1000_0000_0000_0000u64 && fill.lod <= 20 && fill_end(fill) <= u64::MAX
        && (index == 0 || base(fill) as int >= fill_end(fills[(index - 1) as int]))
}
pub open spec fn fills_prefix(fills: Seq<UniformFill>, count: nat) -> bool {
    count <= fills.len() && forall|i:nat| i < count ==> #[trigger] fill_at(fills, i)
}
pub open spec fn dense_count(lod:u8) -> Option<usize> {
    if lod as u32 * 3 < usize::BITS { Some((1usize << (lod as u32 * 3))) } else { None }
}
pub open spec fn dense_end(region: DenseRegion) -> int {
    crate::morton::proof::value(region.path) as int + size(region.lod) as int
}
pub open spec fn dense_at(regions: Seq<DenseRegion>, index: nat) -> bool {
    let region = regions[index as int];
    crate::morton::proof::value(region.path)<0x1000_0000_0000_0000u64 && region.values@.len()<=usize::MAX && region.lod <= 20 && dense_count(region.lod) == Some(region.values@.len() as usize)
        && crate::morton::proof::value(region.path) & ((size(region.lod) - 1) as u64) == 0
        && dense_end(region) <= u64::MAX
        && (index == 0 || crate::morton::proof::value(region.path) as int >= dense_end(regions[(index - 1) as int]))
}
pub open spec fn dense_prefix(regions: Seq<DenseRegion>, count: nat) -> bool {
    count <= regions.len() && forall|i:nat| i < count ==> #[trigger] dense_at(regions, i)
}
pub proof fn span_safe(lod:u8)
    requires lod <= 20,
    ensures size(lod) > 0,
{
    let shift = (lod as u32 * 3) as u32;
    assert((1u64 << shift) > 0) by(bit_vector) requires shift <= 60u32;
}
pub proof fn fill_failure(fills: Seq<UniformFill>, index:nat)
    requires index < fills.len(), !fill_at(fills,index),
    ensures !fills_prefix(fills,fills.len()),
{}
pub proof fn fill_validated(fills: Seq<UniformFill>, index:nat)
    requires index < fills.len(), fills_prefix(fills,index), fill_at(fills,index),
    ensures fills_prefix(fills,index+1),
{}
pub proof fn dense_failure(regions: Seq<DenseRegion>, index:nat)
    requires index < regions.len(), !dense_at(regions,index),
    ensures !dense_prefix(regions,regions.len()),
{}
pub proof fn dense_validated(regions: Seq<DenseRegion>, index:nat)
    requires index < regions.len(), dense_prefix(regions,index), dense_at(regions,index),
    ensures dense_prefix(regions,index+1),
{}
pub proof fn dense_suffix(regions: Seq<DenseRegion>, start:int)
    requires dense_prefix(regions, regions.len()), 0 <= start <= regions.len(),
    ensures dense_prefix(regions.subrange(start, regions.len() as int), (regions.len() - start) as nat),
{
    let rest = regions.subrange(start, regions.len() as int);
    assert forall|i:nat| i < rest.len() implies #[trigger] dense_at(rest, i) by {
        assert(dense_at(regions, (start + i) as nat));
    }
}
}
