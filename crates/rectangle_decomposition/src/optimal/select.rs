//! 按匹配选择顺序投影两向切线，不分配额外缓冲。

#[cfg(verus_only)]
use super::selection_proof;
use super::{HorizontalCut, SparseQuadError, ValuedChord, VerticalCut};
use crate::optimal::matching::ChordBuffer;
use vstd::prelude::proof;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires crate::optimal::matching::proof::valid_indices(indices@,chords@.len()),indices@.len()<=1953,
        old(output).valid(),old(output)@.len()==0,
    ensures result.is_ok(),final(output).valid(),final(output)@==selection_proof::horizontal_cuts(chords@,indices@),
))]
pub(super) fn fill_horizontal_cuts(
    chords: &[ValuedChord],
    indices: &[u16],
    output: &mut ChordBuffer<HorizontalCut>,
) -> Result<(), SparseQuadError> {
    let mut next = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant next<=indices@.len(),indices@.len()<=1953,
            crate::optimal::matching::proof::valid_indices(indices@,chords@.len()),
            output.valid(),output@==selection_proof::horizontal_cuts(chords@,indices@.take(next as int)),
        decreases indices@.len()-next,
    ))]
    while next < indices.len() {
        let index = usize::from(crate::optimal::copy(indices, next));
        let Some(item) = chords.get(index) else {
            return Err(SparseQuadError::CapacityOverflow);
        };
        let chord = item.chord;
        output.push(HorizontalCut {
            y: chord.y1,
            start: chord.x1,
            end: chord.x2,
        });
        proof! { selection_proof::horizontal_step(chords@,indices@,next as int); }
        next += 1;
    }
    proof! { assert(indices@.take(indices@.len() as int) =~= indices@); }
    Ok(())
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires crate::optimal::matching::proof::valid_indices(indices@,chords@.len()),indices@.len()<=1953,
        old(output).valid(),old(output)@.len()==0,
    ensures result.is_ok(),final(output).valid(),final(output)@==selection_proof::vertical_cuts(chords@,indices@),
))]
pub(super) fn fill_vertical_cuts(
    chords: &[ValuedChord],
    indices: &[u16],
    output: &mut ChordBuffer<VerticalCut>,
) -> Result<(), SparseQuadError> {
    let mut next = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant next<=indices@.len(),indices@.len()<=1953,
            crate::optimal::matching::proof::valid_indices(indices@,chords@.len()),
            output.valid(),output@==selection_proof::vertical_cuts(chords@,indices@.take(next as int)),
        decreases indices@.len()-next,
    ))]
    while next < indices.len() {
        let index = usize::from(crate::optimal::copy(indices, next));
        let Some(item) = chords.get(index) else {
            return Err(SparseQuadError::CapacityOverflow);
        };
        let chord = item.chord;
        output.push(VerticalCut {
            x: chord.x1,
            start: chord.y1,
            end: chord.y2,
        });
        proof! { selection_proof::vertical_step(chords@,indices@,next as int); }
        next += 1;
    }
    proof! { assert(indices@.take(indices@.len() as int) =~= indices@); }
    Ok(())
}
