//! 跨调用只保留物理 owner 条件；事件和当前图的语义由每次构建阶段建立。

use super::{SparseOptimalScratch64, cut_proof, mask_proof, scan};
use vstd::prelude::*;

verus! {

impl SparseOptimalScratch64 {
    pub open(in crate::optimal::sparse) spec fn valid(&self) -> bool {
        self.rows.valid() && self.columns.valid()
            && self.chord_groups.horizontal.valid() && self.chord_groups.vertical.valid()
            && self.matching.selection_ready()
            && self.horizontal_cuts.valid() && self.vertical_cuts.valid()
            && self.horizontal_cut_masks.valid() && self.vertical_cut_masks.valid()
            && self.partition.runs.valid() && self.partition.active.valid() && self.partition.next_active.valid()
            && self.rectangles.valid()
    }

    pub closed spec fn initialized(&self) -> bool {
        self.valid() && self.dense_labels@ == Seq::new(4096, |_: int| 0u16)
            && self.rows.lines@.len() == 0 && self.columns.lines@.len() == 0
            && self.chord_groups.horizontal@.len() == 0 && self.chord_groups.vertical@.len() == 0
            && self.matching.selected_horizontal_view().len() == 0 && self.matching.selected_vertical_view().len() == 0
            && self.horizontal_cuts@.len() == 0 && self.vertical_cuts@.len() == 0
            && self.horizontal_cut_masks@.len() == 0 && self.vertical_cut_masks@.len() == 0
            && self.partition.runs@.len() == 0 && self.partition.active@.len() == 0 && self.partition.next_active@.len() == 0
            && self.rectangles@.len() == 0
    }

    pub(in crate::optimal::sparse) open spec fn canonical(&self) -> Seq<crate::optimal::Rectangle> {
        crate::partition::scan_model::canonical(self.rows.labels(),
            scan::proof::selection(self.rows.labels(), self.vertical_cut_masks@),
            scan::proof::block_fn(self.horizontal_cut_masks@))
    }

    pub(in crate::optimal::sparse) open spec fn axes_ready(&self) -> bool {
        self.valid() && self.rows.lines@.len() == 64 && self.rows.boundaries_complete()
            && self.columns.represents(self.rows.labels(), true) && self.columns.boundaries_complete()
    }

    #[verifier::opaque]
    pub open(in crate::optimal::sparse) spec fn produced(&self, labels: Seq<u16>, rectangles: Seq<crate::optimal::Rectangle>) -> bool {
        self.valid() && self.rows.lines@.len() == 64 && self.rows.labels() == labels
            && rectangles == self.canonical() && crate::partition::optimal(labels, rectangles)
            && rectangles == crate::partition::optimality::result_for(labels)
    }

    #[verifier::opaque]
    pub open(in crate::optimal::sparse) spec fn sparse_borrowed(&self, leaves: Seq<super::QuadLeaf64>, result: Result<&[crate::optimal::Rectangle], super::SparseQuadError>) -> bool {
        self.valid() && result.is_ok() == super::input_proof::valid(leaves)
            && match result {
                Ok(rectangles) => rectangles@ == self.rectangles@ && self.produced(super::input_proof::labels(leaves), rectangles@),
                Err(error) => super::input_proof::checked(leaves, Err(error))
                    && super::failure_proof::failure_for(leaves) == Some(error),
            }
    }

    #[verifier::opaque]
    pub open(in crate::optimal::sparse) spec fn sparse_packed(&self, leaves: Seq<super::QuadLeaf64>, output: &crate::optimal::PackedRectangles64, result: Result<usize, super::SparseQuadError>) -> bool {
        self.valid() && output.valid() && result.is_ok() == super::input_proof::valid(leaves)
            && match result {
                Ok(count) => count == output@.len() && self.produced(super::input_proof::labels(leaves), output@),
                Err(error) => super::input_proof::checked(leaves, Err(error)) && output@ == Seq::empty()
                    && super::failure_proof::failure_for(leaves) == Some(error),
            }
    }

    #[verifier::opaque]
    pub open(in crate::optimal::sparse) spec fn dense_borrowed(&self, labels: Seq<u16>, result: Result<&[crate::optimal::Rectangle], super::SparseQuadError>) -> bool {
        self.valid() && match result {
            Ok(rectangles) => rectangles@ == self.rectangles@ && self.produced(labels, rectangles@),
            Err(_) => false,
        }
    }

    #[verifier::opaque]
    pub open(in crate::optimal::sparse) spec fn dense_packed(&self, labels: Seq<u16>, output: &crate::optimal::PackedRectangles64, result: Result<usize, super::SparseQuadError>) -> bool {
        self.valid() && output.valid() && match result {
            Ok(count) => count == output@.len() && self.produced(labels, output@),
            Err(_) => false,
        }
    }
}

pub(super) proof fn new_owner(scratch: &SparseOptimalScratch64)
    requires scratch.rows.valid(), scratch.columns.valid(),
        scratch.chord_groups.horizontal.valid(), scratch.chord_groups.horizontal@.len() == 0,
        scratch.chord_groups.vertical.valid(), scratch.chord_groups.vertical@.len() == 0,
        scratch.matching.selection_ready(), scratch.matching.selected_horizontal_view().len() == 0,
        scratch.matching.selected_vertical_view().len() == 0,
        scratch.horizontal_cuts.valid(), scratch.horizontal_cuts@.len() == 0,
        scratch.vertical_cuts.valid(), scratch.vertical_cuts@.len() == 0,
        scratch.horizontal_cut_masks.valid(), scratch.horizontal_cut_masks@.len() == 0,
        scratch.vertical_cut_masks.valid(), scratch.vertical_cut_masks@.len() == 0,
        scratch.partition.runs.valid(), scratch.partition.runs@.len() == 0,
        scratch.partition.active.valid(), scratch.partition.active@.len() == 0,
        scratch.partition.next_active.valid(), scratch.partition.next_active@.len() == 0,
        scratch.rectangles.valid(), scratch.rectangles@.len() == 0,
        scratch.rows.lines@.len() == 0, scratch.columns.lines@.len() == 0,
        scratch.dense_labels@ == Seq::new(4096, |_: int| 0u16),
    ensures scratch.valid(), scratch.initialized(),
{
    reveal(SparseOptimalScratch64::initialized);
}

pub(super) proof fn establish(tracked scratch: &SparseOptimalScratch64)
    ensures scratch.valid(),
{
    super::AxisIntervals::establish(&scratch.rows);
    super::AxisIntervals::establish(&scratch.columns);
    crate::optimal::fixed::FixedVec::establish(&scratch.chord_groups.horizontal);
    crate::optimal::fixed::FixedVec::establish(&scratch.chord_groups.vertical);
    crate::optimal::matching::MatchingScratch::establish(&scratch.matching);
    crate::optimal::fixed::FixedVec::establish(&scratch.horizontal_cuts);
    crate::optimal::fixed::FixedVec::establish(&scratch.vertical_cuts);
    crate::optimal::fixed::FixedVec::establish(&scratch.horizontal_cut_masks);
    crate::optimal::fixed::FixedVec::establish(&scratch.vertical_cut_masks);
    crate::optimal::fixed::FixedVec::establish(&scratch.partition.runs);
    crate::optimal::fixed::FixedVec::establish(&scratch.partition.active);
    crate::optimal::fixed::FixedVec::establish(&scratch.partition.next_active);
    crate::optimal::fixed::FixedVec::establish(&scratch.rectangles);
}

pub(super) proof fn prepare_output(scratch: &SparseOptimalScratch64, boundaries: u64)
    requires scratch.valid(), scratch.rows.lines@.len() == 64, scratch.rows.boundaries_complete(), scratch.cuts_ready(),
        mask_proof::built(scratch.horizontal_cuts@, scratch.vertical_cuts@,
            scratch.horizontal_cut_masks@, scratch.vertical_cut_masks@, boundaries),
    ensures scan::proof::ready(&scratch.rows, scratch.horizontal_cut_masks@, scratch.vertical_cut_masks@, boundaries),
        crate::partition::optimal(scratch.rows.labels(), scratch.canonical()),
        scratch.canonical() == crate::partition::optimality::result_for(scratch.rows.labels()),
{
    cut_proof::prepared_optimal(&scratch.rows, scratch.chosen(), scratch.horizontal_cuts@,
        scratch.vertical_cuts@, scratch.horizontal_cut_masks@, scratch.vertical_cut_masks@, boundaries);
    super::history_proof::prepared(&scratch.rows, scratch.chosen(), scratch.horizontal_cuts@,
        scratch.vertical_cuts@, scratch.horizontal_cut_masks@, scratch.vertical_cut_masks@, boundaries);
}

pub(super) proof fn represented_rows(rows: &super::AxisIntervals, labels: Seq<u16>)
    requires rows.represents(labels, false),
    ensures rows.labels() == labels,
{
    assert forall|p: int| 0 <= p < 4096 implies #[trigger] rows.labels()[p] == labels[p] by {
        assert(super::proof::line_value(rows.line_spec(p / 64), p % 64) == labels[p / 64 * 64 + p % 64]);
    }
    assert(rows.labels() =~= labels);
}

fn occupancy_phase_owner() -> (result: SparseOptimalScratch64)
    ensures result.valid(), !result.row_buckets@[0].valid(),
{
    let mut scratch = SparseOptimalScratch64::new();
    scratch.row_buckets[0].items[0] = super::Interval { start: 0, end: 0, value: 0 };
    scratch.row_buckets[0].starts = 1;
    assert(scratch.valid());
    let ghost word = scratch.row_buckets@[0].starts;
    assert(word == 1);
    assert((word >> 0u64) & 1 == 1) by(bit_vector) requires word == 1;
    assert(super::proof::bit(word, 0));
    scratch
}

#[cfg(reject_scratch_event_phase)]
fn reject_scratch_event_phase() {
    let scratch = occupancy_phase_owner();
    assert(scratch.row_buckets@[0].valid());
}

#[cfg(reject_scratch_initial_output)]
fn reject_scratch_initial_output() {
    let scratch = SparseOptimalScratch64::default();
    reveal(SparseOptimalScratch64::initialized);
    assert(scratch.rectangles@.len() != 0);
}

}
