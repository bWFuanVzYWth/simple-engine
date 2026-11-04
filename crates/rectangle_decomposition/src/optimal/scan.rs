//! 借用原有双缓冲扫描；固定输出连续写入，sink 逐项交付。

use super::{
    AxisIntervals, EDGE, EDGE_U8, PartitionScratch, SparseQuadError, build_runs_for_row,
    horizontal_cut_overlaps, reserve_exact,
};
use crate::optimal::fixed::FixedVec;
use crate::optimal::types::{ActiveRect, RangeU8, Rectangle, Run};
#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, proof_decl};
#[cfg(verus_only)]
#[path = "../proof/scan_state.rs"]
pub(super) mod proof;

/// 游标仅保存跨交付点的合并进度；不缓存待交付矩形。
#[cfg_attr(verus_only, verus_verify)]
pub(super) struct RectScan<'a> {
    pub(super) rows: &'a AxisIntervals,
    pub(super) horizontal: &'a [u64],
    pub(super) vertical: &'a [u128],
    pub(super) runs: &'a mut FixedVec<Run, EDGE>,
    pub(super) active: &'a mut FixedVec<ActiveRect, EDGE>,
    pub(super) next: &'a mut FixedVec<ActiveRect, EDGE>,
    pub(super) boundaries: u64,
    pub(super) y: u8,
    pub(super) active_index: usize,
    pub(super) run_index: usize,
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> RectScan<'a> {
    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::ready(rows,horizontal@,vertical@,cut_boundaries),
            old(scratch).runs.valid(),old(scratch).active.valid(),old(scratch).next_active.valid(),
        ensures result is Ok,result->Ok_0.valid(),result->Ok_0.emitted()==Seq::empty(),
            result->Ok_0.rows.labels()==rows.labels(),result->Ok_0.horizontal@==horizontal@,
            result->Ok_0.vertical@==vertical@,
    ))]
    #[inline(always)]
    pub(super) fn new(
        rows: &'a AxisIntervals,
        horizontal: &'a [u64],
        vertical: &'a [u128],
        cut_boundaries: u64,
        scratch: &'a mut PartitionScratch,
    ) -> Result<Self, SparseQuadError> {
        scratch.active.clear();
        scratch.next_active.clear();
        let Some(&mask) = vertical.first() else {
            return Err(SparseQuadError::CapacityOverflow);
        };
        build_runs_for_row(rows.line(0), mask, &mut scratch.runs)?;
        proof! { proof::raw_row_math(rows,vertical@,0,scratch.runs@); }
        reserve_exact(&scratch.active, scratch.runs.len())?;
        let mut index = 0;
        #[cfg_attr(verus_only, verus_spec(
            invariant proof::ready(rows,horizontal@,vertical@,cut_boundaries),
                scratch.runs.valid(),scratch.active.valid(),scratch.next_active.valid(),scratch.next_active@.len()==0,
                proof::initial(rows,horizontal@,vertical@,scratch.runs@,scratch.active@,index as int),
            decreases scratch.runs@.len()-index,
        ))]
        while index < scratch.runs.len() {
            let run = crate::optimal::copy(&scratch.runs, index);
            proof_decl! { let ghost before=scratch.active@; }
            scratch
                .active
                .push(ActiveRect::new(run.value, run.x_start, run.x_end, 0));
            proof! { proof::initial_next(rows,horizontal@,vertical@,scratch.runs@,before,index as int); }
            index += 1;
        }
        let output = Self {
            rows,
            horizontal,
            vertical,
            runs: &mut scratch.runs,
            active: &mut scratch.active,
            next: &mut scratch.next_active,
            boundaries: (rows.boundaries | cut_boundaries) & !1,
            y: 0,
            active_index: 0,
            run_index: 0,
        };
        proof! { proof::new_scan(&output,cut_boundaries); }
        Ok(output)
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl RectScan<'_> {
    /// 按边界递增、旧 active 次序返回矩形；内部错误保留原错误语义。
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures result.is_ok(),final(self).valid(),final(self).rows==old(self).rows,
            final(self).horizontal@==old(self).horizontal@,final(self).vertical@==old(self).vertical@,
            match result {
                Ok(Some(rectangle)) =>final(self).emitted()==old(self).emitted().push(rectangle)
                    && rectangle==old(self).expected()[old(self).emitted().len() as int],
                Ok(None) =>final(self).emitted()==old(self).emitted() && old(self).emitted()==old(self).expected(),
                _=>false,
            },
    ))]
    #[inline(always)]
    pub(super) fn next_rect(&mut self) -> Result<Option<Rectangle>, SparseQuadError> {
        proof! { proof::valid_prefix(self); }
        proof_decl! { let ghost original=self.frontier(); }
        #[cfg_attr(verus_only, verus_spec(
            invariant proof::driver(self,original),original==old(self).frontier(),self.rows==old(self).rows,
                self.horizontal@==old(self).horizontal@,self.vertical@==old(self).vertical@,
            decreases 64-self.y,
        ))]
        loop {
            proof_decl! { let ghost round_y=self.y; }
            if self.y == 0 {
                self.begin_boundary()?;
            }
            if self.y == EDGE_U8 {
                let Some(_) = self.active.get(self.active_index) else {
                    proof! { proof::finished(self); }
                    return Ok(None);
                };
                let result = self.close_active();
                proof! { proof::yielded(self,original,result); }
                return Ok(Some(result));
            }
            proof_decl! { let ghost body_y=self.y; }
            let Some(&mask) = self.horizontal.get(usize::from(self.y)) else {
                return Err(SparseQuadError::CapacityOverflow);
            };
            #[cfg_attr(verus_only, verus_spec(
                invariant proof::driver(self,original),original==old(self).frontier(),0<self.y<64,mask==self.horizontal@[self.y as int],
                    self.y==body_y,round_y<=body_y,
                    self.rows==old(self).rows,self.horizontal@==old(self).horizontal@,self.vertical@==old(self).vertical@,
                decreases self.active@.len()-self.active_index+self.runs@.len()-self.run_index,
            ))]
            while self.active_index < self.active.len() && self.run_index < self.runs.len() {
                let active = self.active[self.active_index];
                let run = self.runs[self.run_index];
                let key = run.match_key();
                proof! { super::scan_proof::key_equal(active,run);
                proof::active_item(self.rows.labels(),proof::selection(self.rows.labels(),self.vertical@),self.horizontal@,
                    self.y as int-1,self.active@,self.active_index as int); }
                if active.match_key() == key
                    && !horizontal_cut_overlaps(mask, active.x_start, active.x_end)
                {
                    self.continue_run();
                } else if comes_first(active, run) {
                    let result = self.close_active();
                    proof! { proof::yielded(self,original,result); }
                    return Ok(Some(result));
                } else {
                    self.start_run(run);
                }
            }
            if self.active.get(self.active_index).is_some() {
                let result = self.close_active();
                proof! { proof::yielded(self,original,result); }
                return Ok(Some(result));
            }
            reserve_exact(self.next, self.runs.len().saturating_sub(self.run_index))?;
            #[cfg_attr(verus_only, verus_spec(
                invariant proof::driver(self,original),original==old(self).frontier(),0<self.y<64,self.active_index==self.active@.len(),
                    self.y==body_y,round_y<=body_y,
                    self.rows==old(self).rows,self.horizontal@==old(self).horizontal@,self.vertical@==old(self).vertical@,
                decreases self.runs@.len()-self.run_index,
            ))]
            while self.run_index < self.runs.len() {
                self.start_run(self.runs[self.run_index]);
            }
            proof! { proof::completed_merge(self);
            assert(crate::partition::scan_model::prefix(self.rows.labels(),proof::selection(self.rows.labels(),self.vertical@),
                proof::block_fn(self.horizontal@),original)
                ==crate::partition::scan_model::prefix(self.rows.labels(),proof::selection(self.rows.labels(),self.vertical@),
                proof::block_fn(self.horizontal@),self.y as nat*64)); }
            // 交换借用，两个内联数组不搬移。
            core::mem::swap(&mut self.active, &mut self.next);
            self.begin_boundary()?;
        }
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl RectScan<'_> {
    #[cfg_attr(verus_only, verus_spec(
        requires proof::may_start(old(self),run),
        ensures final(self).valid(),final(self).emitted()==old(self).emitted(),
            final(self).rows==old(self).rows,final(self).horizontal@==old(self).horizontal@,final(self).vertical@==old(self).vertical@,
            final(self).active@==old(self).active@,final(self).runs@==old(self).runs@,final(self).y==old(self).y,
            final(self).boundaries==old(self).boundaries,final(self).active_index==old(self).active_index,
            final(self).run_index==old(self).run_index+1,
    ))]
    #[inline]
    fn start_run(&mut self, run: Run) {
        proof! { proof::new_data(self.rows.labels(),proof::selection(self.rows.labels(),self.vertical@),self.horizontal@,
        self.y as int,self.active@,self.runs@,self.next@,self.active_index as int,self.run_index as int); }
        self.next
            .push(ActiveRect::new(run.value, run.x_start, run.x_end, self.y));
        self.run_index += 1;
    }

    #[cfg_attr(verus_only, verus_spec(
        requires old(self).valid(),0<old(self).y<64,old(self).active_index<old(self).active@.len(),
            old(self).run_index<old(self).runs@.len(),proof::continuable(old(self).active@[old(self).active_index as int],
                old(self).runs@[old(self).run_index as int],old(self).horizontal@[old(self).y as int]),
        ensures final(self).valid(),final(self).emitted()==old(self).emitted(),
            final(self).rows==old(self).rows,final(self).horizontal@==old(self).horizontal@,final(self).vertical@==old(self).vertical@,
            final(self).active@==old(self).active@,final(self).runs@==old(self).runs@,final(self).y==old(self).y,
            final(self).boundaries==old(self).boundaries,final(self).active_index==old(self).active_index+1,
            final(self).run_index==old(self).run_index+1,
    ))]
    #[inline]
    fn continue_run(&mut self) {
        let item = crate::optimal::copy(self.active, self.active_index);
        proof! { proof::continued_data(self.rows.labels(),proof::selection(self.rows.labels(),self.vertical@),self.horizontal@,
        self.y as int,self.active@,self.runs@,self.next@,self.active_index as int,self.run_index as int);
        proof::prefix_step(self.rows.labels(),proof::selection(self.rows.labels(),self.vertical@),self.horizontal@,
            self.y as int,self.active@,self.active_index as int); }
        self.next.push(item);
        self.active_index += 1;
        self.run_index += 1;
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::may_close(old(self)),
        ensures final(self).valid(),final(self).emitted()==old(self).emitted().push(result),
            final(self).rows==old(self).rows,final(self).horizontal@==old(self).horizontal@,final(self).vertical@==old(self).vertical@,
            final(self).active@==old(self).active@,final(self).runs@==old(self).runs@,final(self).y==old(self).y,
            final(self).boundaries==old(self).boundaries,final(self).active_index==old(self).active_index+1,
            final(self).run_index==old(self).run_index,
    ))]
    #[inline]
    fn close_active(&mut self) -> Rectangle {
        let item = crate::optimal::copy(self.active, self.active_index);
        proof! {
            if self.y<64 { proof::closed_data(self.rows.labels(),proof::selection(self.rows.labels(),self.vertical@),self.horizontal@,
                self.y as int,self.active@,self.runs@,self.next@,self.active_index as int,self.run_index as int); }
            proof::closed_form(self.rows.labels(),proof::selection(self.rows.labels(),self.vertical@),self.horizontal@,
                self.y as int,self.active@,self.active_index as int);
            proof::prefix_step(self.rows.labels(),proof::selection(self.rows.labels(),self.vertical@),self.horizontal@,
                self.y as int,self.active@,self.active_index as int);
        }
        let result = closed(item, self.y);
        self.active_index += 1;
        result
    }

    #[cfg_attr(verus_only, verus_spec(result =>
        requires proof::boundary_entry(old(self)),
        ensures result.is_ok(),final(self).valid(),final(self).y>old(self).y,
            final(self).rows==old(self).rows,final(self).horizontal@==old(self).horizontal@,
            final(self).vertical@==old(self).vertical@,final(self).active@==old(self).active@,
            final(self).emitted()==crate::partition::scan_model::prefix(old(self).rows.labels(),
                proof::selection(old(self).rows.labels(),old(self).vertical@),
                proof::block_fn(old(self).horizontal@),old(self).y as nat*64),
    ))]
    #[inline(always)]
    fn begin_boundary(&mut self) -> Result<(), SparseQuadError> {
        proof_decl! { let ghost previous_y=self.y; }
        self.active_index = 0;
        self.run_index = 0;
        self.next.clear();
        if self.boundaries == 0 {
            proof! { proof::pending_empty(self.rows,self.horizontal@,self.vertical@,previous_y as int);
            proof::gap_active(self.rows,self.horizontal@,self.vertical@,previous_y as int,63,self.active@); }
            self.y = EDGE_U8;
            proof! { proof::boundary_done(self,previous_y as int); }
            return Ok(());
        }
        proof! { proof::pending_next(self.rows,self.horizontal@,self.vertical@,previous_y as int,self.boundaries); }
        self.y = self.boundaries.trailing_zeros() as u8;
        self.boundaries &= self.boundaries - 1;
        proof! { proof::gap_active(self.rows,self.horizontal@,self.vertical@,previous_y as int,self.y as int-1,self.active@); }
        let Some(&mask) = self.vertical.get(usize::from(self.y)) else {
            return Err(SparseQuadError::CapacityOverflow);
        };
        build_runs_for_row(self.rows.line(usize::from(self.y)), mask, self.runs)?;
        proof! { proof::raw_row_math(self.rows,self.vertical@,self.y as int,self.runs@);
        proof::boundary_done(self,previous_y as int); }
        Ok(())
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => ensures result.value == active.value,
    result.x.start == active.x_start, result.x.end == active.x_end,
    result.y.start == active.y_start, result.y.end == y_end))]
const fn closed(active: ActiveRect, y_end: u8) -> Rectangle {
    Rectangle {
        value: active.value,
        x: RangeU8::new(active.x_start, active.x_end),
        y: RangeU8::new(active.y_start, y_end),
    }
}

#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result==(proof::order(proof::run_of(active))<=proof::order(run)),
))]
const fn comes_first(active: ActiveRect, run: Run) -> bool {
    active.x_start < run.x_start || active.x_start == run.x_start && active.value <= run.value
}

#[cfg_attr(verus_only, verus_verify)]
impl RectScan<'_> {
    /// 不暂停行合并，直接完成固定输出；仅 sink 使用逐项游标。
    #[inline(always)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires super::output::proof::writing(old(self), old(target)),
        ensures result is Ok, final(self).valid(),
            result->Ok_0 == final(target)@.len(), final(target)@ == old(self).expected(),
            final(self).emitted() == old(self).expected(),
            final(self).rows == old(self).rows, final(self).horizontal@ == old(self).horizontal@,
            final(self).vertical@ == old(self).vertical@,
    ))]
    pub(super) fn write_all<T: super::output::Rectangles>(
        &mut self,
        target: &mut T,
    ) -> Result<usize, SparseQuadError> {
        proof! { super::output::proof::prefix(self); }
        if self.y == 0 {
            self.begin_boundary()?;
        }
        #[cfg_attr(verus_only, verus_spec(
            invariant super::output::proof::writing(self, target), 0 < self.y <= 64,
                self.rows == old(self).rows,
                self.horizontal@ == old(self).horizontal@, self.vertical@ == old(self).vertical@,
            decreases 64 - self.y,
        ))]
        while self.y < EDGE_U8 {
            proof_decl! { let ghost row = self.y; }
            let Some(&mask) = self.horizontal.get(usize::from(self.y)) else {
                return Err(SparseQuadError::CapacityOverflow);
            };
            #[cfg_attr(verus_only, verus_spec(
                invariant super::output::proof::writing(self, target), 0 < self.y < 64,
                    self.y == row, mask == self.horizontal@[self.y as int],
                    self.rows == old(self).rows,
                    self.horizontal@ == old(self).horizontal@, self.vertical@ == old(self).vertical@,
                decreases self.active@.len() - self.active_index + self.runs@.len() - self.run_index,
            ))]
            while self.active_index < self.active.len() && self.run_index < self.runs.len() {
                let active = self.active[self.active_index];
                let run = self.runs[self.run_index];
                proof! { super::scan_proof::key_equal(active,run);
                proof::active_item(self.rows.labels(),proof::selection(self.rows.labels(),self.vertical@),self.horizontal@,
                    self.y as int-1,self.active@,self.active_index as int); }
                if active.match_key() == run.match_key()
                    && !horizontal_cut_overlaps(mask, active.x_start, active.x_end)
                {
                    self.continue_run();
                } else if comes_first(active, run) {
                    let rectangle = self.close_active();
                    proof! { super::output::proof::capacity(self, target@.len()); }
                    target.append(rectangle)?;
                } else {
                    self.start_run(run);
                }
            }
            #[cfg_attr(verus_only, verus_spec(
                invariant super::output::proof::writing(self, target), 0 < self.y < 64, self.y == row,
                    self.active_index < self.active@.len() ==> self.run_index == self.runs@.len(),
                    self.rows == old(self).rows,
                    self.horizontal@ == old(self).horizontal@, self.vertical@ == old(self).vertical@,
                decreases self.active@.len() - self.active_index,
            ))]
            while self.active_index < self.active.len() {
                let rectangle = self.close_active();
                proof! { super::output::proof::capacity(self, target@.len()); }
                target.append(rectangle)?;
            }
            #[cfg_attr(verus_only, verus_spec(
                invariant super::output::proof::writing(self, target), 0 < self.y < 64, self.y == row,
                    self.active_index == self.active@.len(),
                    self.rows == old(self).rows,
                    self.horizontal@ == old(self).horizontal@, self.vertical@ == old(self).vertical@,
                decreases self.runs@.len() - self.run_index,
            ))]
            while self.run_index < self.runs.len() {
                self.start_run(self.runs[self.run_index]);
            }
            proof! { proof::completed_merge(self); }
            core::mem::swap(&mut self.active, &mut self.next);
            self.begin_boundary()?;
        }
        #[cfg_attr(verus_only, verus_spec(
            invariant super::output::proof::writing(self, target), self.y == 64,
                self.rows == old(self).rows,
                self.horizontal@ == old(self).horizontal@, self.vertical@ == old(self).vertical@,
            decreases self.active@.len() - self.active_index,
        ))]
        while self.active_index < self.active.len() {
            let rectangle = self.close_active();
            proof! { super::output::proof::capacity(self, target@.len()); }
            target.append(rectangle)?;
        }
        proof! { proof::finished(self); }
        Ok(target.len())
    }
}
