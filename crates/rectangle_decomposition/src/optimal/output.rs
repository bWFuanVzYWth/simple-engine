//! 借用同一扫描器，直接交付调用方持有的固定容量输出。

use super::{MAX_RECTANGLES, SparseQuadError, scan::RectScan};
use crate::optimal::fixed::FixedVec;
use crate::optimal::types::{PackedRectangles64, Rectangle};
use proof::Model;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(not(verus_only))]
use vstd::view::View;

#[path = "../proof/output.rs"]
pub(super) mod proof;

/// 将完整扫描直接追加到已清空的 AoS；清空和预留仍由原调用位置负责。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(scanner).valid(), old(scanner).emitted() == Seq::empty(),
        old(output).valid(), old(output)@ == Seq::empty(),
    ensures result is Ok, result->Ok_0 == final(output)@.len(), final(output).valid(),
        final(output)@ == old(scanner).expected(), final(scanner).valid(),
        final(scanner).emitted() == old(scanner).expected(),
        final(scanner).rows == old(scanner).rows,
        final(scanner).horizontal@ == old(scanner).horizontal@, final(scanner).vertical@ == old(scanner).vertical@,
))]
#[inline(always)]
pub(super) fn write_aos(
    scanner: &mut RectScan<'_>,
    output: &mut FixedVec<Rectangle, MAX_RECTANGLES>,
) -> Result<usize, SparseQuadError> {
    let count = scanner.write_all(output)?;
    #[cfg(verus_only)]
    proof! { FixedVec::establish(&*output); }
    Ok(count)
}

/// 直接交付同一序列到 packed bounds／labels，不经过 AoS 数组。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(scanner).valid(), old(scanner).emitted() == Seq::empty(), old(output).valid(), old(output)@ == Seq::empty(),
    ensures result is Ok, result->Ok_0 == final(output)@.len(), final(output).valid(),
        final(output)@ == old(scanner).expected(), final(scanner).valid(),
        final(scanner).emitted() == old(scanner).expected(),
        final(scanner).rows == old(scanner).rows,
        final(scanner).horizontal@ == old(scanner).horizontal@, final(scanner).vertical@ == old(scanner).vertical@,
))]
#[inline(always)]
pub(super) fn write_packed(
    scanner: &mut RectScan<'_>,
    output: &mut PackedRectangles64,
) -> Result<usize, SparseQuadError> {
    let count = scanner.write_all(output)?;
    #[cfg(verus_only)]
    proof! { PackedRectangles64::establish(&*output); }
    Ok(count)
}

/// 连续扫描直接提交到原输出所有者；只在这两种表示间静态分派。
#[cfg_attr(verus_only, verus_verify)]
pub(super) trait Rectangles: View<V = Model> {
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == self@.len(), result <= 4096,
    ))]
    fn len(&self) -> usize;
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self)@.len() < 4096,
        ensures result is Ok, final(self)@ == old(self)@.push(rectangle),
    ))]
    fn append(&mut self, rectangle: Rectangle) -> Result<(), SparseQuadError>;
}

#[cfg_attr(verus_only, verus_verify)]
impl Rectangles for FixedVec<Rectangle, MAX_RECTANGLES> {
    #[inline(always)]
    fn len(&self) -> usize {
        #[cfg(verus_only)]
        proof! { FixedVec::establish(self); }
        FixedVec::len(self)
    }
    #[inline(always)]
    fn append(&mut self, rectangle: Rectangle) -> Result<(), SparseQuadError> {
        #[cfg(verus_only)]
        proof! { FixedVec::establish(&*self); }
        FixedVec::push(self, rectangle);
        Ok(())
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl Rectangles for PackedRectangles64 {
    #[inline(always)]
    fn len(&self) -> usize {
        PackedRectangles64::len(self)
    }
    #[inline(always)]
    fn append(&mut self, rectangle: Rectangle) -> Result<(), SparseQuadError> {
        #[cfg(verus_only)]
        proof! { PackedRectangles64::establish(&*self); }
        PackedRectangles64::push(self, rectangle)
    }
}
