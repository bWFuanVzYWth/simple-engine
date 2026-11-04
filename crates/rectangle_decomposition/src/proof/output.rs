//! 真实 scanner 的规范前缀与固定输出容量之间的短连接。

#[cfg(verus_only)]
use super::super::scan::proof as scan;
use super::{FixedVec, MAX_RECTANGLES, PackedRectangles64, Rectangle};
#[cfg(verus_only)]
use super::{RectScan, Rectangles};
use vstd::prelude::*;

/// 普通构建只保留 View 的擦除关联类型。
#[cfg(not(verus_only))]
pub(in crate::optimal::sparse) type Model = ();
#[cfg(verus_only)]
pub(in crate::optimal::sparse) type Model = Seq<Rectangle>;

#[cfg(not(verus_only))]
impl View for FixedVec<Rectangle, MAX_RECTANGLES> {
    type V = Model;
}
#[cfg(not(verus_only))]
impl View for PackedRectangles64 {
    type V = Model;
}

#[cfg(verus_only)]
verus! {

pub(in crate::optimal::sparse) open spec fn writing<T: Rectangles>(scanner: &RectScan<'_>, target: &T) -> bool {
    scanner.valid() && target@ == scanner.emitted()
}

pub(in crate::optimal::sparse) proof fn prefix(scanner: &RectScan<'_>)
    requires scanner.valid(),
    ensures scanner.emitted().len() <= scanner.expected().len() <= 4096,
        scanner.expected().take(scanner.emitted().len() as int) == scanner.emitted(),
{
    scan::valid_prefix(scanner);
}

pub(in crate::optimal::sparse) proof fn capacity(scanner: &RectScan<'_>, stored: nat)
    requires scanner.valid(), scanner.emitted().len() == stored + 1,
    ensures stored < 4096, scanner.emitted().len() <= scanner.expected().len() <= 4096,
{
    scan::valid_prefix(scanner);
}

#[cfg(reject_output_aos_count)]
fn reject_output_aos_count(scanner: &mut RectScan<'_>, output: &mut FixedVec<Rectangle, MAX_RECTANGLES>)
    requires old(scanner).valid(), old(scanner).emitted() == Seq::empty(), old(output).valid(), old(output)@ == Seq::empty(),
{
    let result = super::write_aos(scanner, output);
    assert(result is Ok);
    assert(result->Ok_0 != output@.len());
}

#[cfg(reject_output_packed_value)]
fn reject_output_packed_value(scanner: &mut RectScan<'_>, output: &mut PackedRectangles64)
    requires old(scanner).valid(), old(scanner).emitted() == Seq::empty(), old(scanner).expected().len() > 0,
        old(output).valid(), old(output)@ == Seq::empty(),
{
    let ghost first = scanner.expected()[0];
    let result = super::write_packed(scanner, output);
    assert(result is Ok);
    assert(output@[0] != first);
}

}
