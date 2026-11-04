//! 写入前完整扫描区域约束；不修改 DAG 或申请容量。
use super::input::{DenseRegion, UniformFill};
use crate::{Error, ZOrder20x3};
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "../proof/write_validate.rs"]
#[cfg(verus_only)]
pub(super) mod proof;

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == proof::dense_count(lod), no_unwind
))]
pub(super) const fn dense_value_count(lod: u8) -> Option<usize> {
    1_usize.checked_shl(lod as u32 * 3)
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result is Ok <==> proof::fills_prefix(fills@, fills@.len()), no_unwind
))]
pub(super) fn validate_fills(fills: &[UniformFill]) -> Result<(), Error> {
    let mut previous_end = 0u64;
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= fills@.len(), proof::fills_prefix(fills@, index as nat),
            index == 0 ==> previous_end == 0,
            index > 0 ==> previous_end as int == proof::fill_end(fills@[(index - 1) as int]),
        decreases fills@.len() - index,
    ))]
    while index < fills.len() {
        let fill = &fills[index];
        if fill.lod > ZOrder20x3::BIT_DEPTH {
            #[cfg(verus_only)]
            proof! { proof::fill_failure(fills@, index as nat); }
            return Err(Error::InvalidLod(fill.lod));
        }
        #[cfg(verus_only)]
        proof! { proof::span_safe(fill.lod); }
        let size = 1u64 << (fill.lod as u32 * 3);
        let base = fill.path.value() & !(size - 1);
        if base < previous_end {
            #[cfg(verus_only)]
            proof! { proof::fill_failure(fills@, index as nat); }
            return Err(Error::InvalidInput);
        }
        previous_end = match base.checked_add(size) {
            Some(end) => end,
            None => {
                #[cfg(verus_only)]
                proof! { proof::fill_failure(fills@, index as nat); }
                return Err(Error::CapacityOverflow);
            }
        };
        #[cfg(verus_only)]
        proof! { proof::fill_validated(fills@, index as nat); }
        index += 1;
    }
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result is Ok <==> proof::dense_prefix(regions@, regions@.len()), no_unwind
))]
pub(super) fn dense(regions: &[DenseRegion<'_>]) -> Result<(), Error> {
    let mut previous_end = 0u64;
    let mut index = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant index <= regions@.len(), proof::dense_prefix(regions@, index as nat),
            index == 0 ==> previous_end == 0,
            index > 0 ==> previous_end as int == proof::dense_end(regions@[(index - 1) as int]),
        decreases regions@.len() - index,
    ))]
    while index < regions.len() {
        let region = &regions[index];
        if region.lod > ZOrder20x3::BIT_DEPTH {
            #[cfg(verus_only)]
            proof! { proof::dense_failure(regions@, index as nat); }
            return Err(Error::InvalidLod(region.lod));
        }
        if Some(region.values.len()) != dense_value_count(region.lod) {
            #[cfg(verus_only)]
            proof! { proof::dense_failure(regions@, index as nat); }
            return Err(Error::InvalidDenseValues);
        }
        #[cfg(verus_only)]
        proof! { proof::span_safe(region.lod); }
        let span = 1u64 << (region.lod as u32 * 3);
        let base = region.path.value();
        if base & (span - 1) != 0 || base < previous_end {
            #[cfg(verus_only)]
            proof! { proof::dense_failure(regions@, index as nat); }
            return Err(Error::InvalidInput);
        }
        previous_end = match base.checked_add(span) {
            Some(end) => end,
            None => {
                #[cfg(verus_only)]
                proof! { proof::dense_failure(regions@, index as nat); }
                return Err(Error::CapacityOverflow);
            }
        };
        #[cfg(verus_only)]
        proof! { proof::dense_validated(regions@, index as nat); }
        index += 1;
    }
    Ok(())
}
