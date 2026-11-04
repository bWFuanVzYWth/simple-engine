//! packed 有效前缀、原位追加及四字节编码的对应关系。

use super::*;
use crate::error::SparseQuadError;

verus! {


broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) open spec fn encoded(rectangle: Rectangle) -> u32 {
    (rectangle.x.start as u32) | ((rectangle.x.end as u32) << 8)
        | ((rectangle.y.start as u32) << 16) | ((rectangle.y.end as u32) << 24)
}

pub(crate) open spec fn decoded(bounds: u32, value: u16) -> Rectangle {
    Rectangle {
        value,
        x: RangeU8 { start: bounds as u8, end: (bounds >> 8) as u8 },
        y: RangeU8 { start: (bounds >> 16) as u8, end: (bounds >> 24) as u8 },
    }
}

impl View for PackedRectangles64 {
    type V = Seq<Rectangle>;
    open(super) spec fn view(&self) -> Seq<Rectangle> {
        Seq::new(self.len as nat, |i: int| decoded(self.bounds@[i], self.labels@[i]))
    }
}

impl PackedRectangles64 {
    #[verifier::type_invariant]
    pub(super) open spec fn type_invariant(&self) -> bool { self.valid() }

    pub open(super) spec fn initialized(&self) -> bool {
        self.len == 0 && self.bounds@ == Seq::new(4096, |_: int| 0u32)
            && self.labels@ == Seq::new(4096, |_: int| 0u16)
    }
    pub open(super) spec fn valid(&self) -> bool { self.len <= 4096 }
    pub(crate) proof fn establish(tracked value: &Self)
        ensures value.valid(),
    { use_type_invariant(value); }
    pub open(super) spec fn storage(&self) -> (Seq<u32>, Seq<u16>) { (self.bounds@, self.labels@) }
    pub open(super) spec fn bounds_view(&self) -> Seq<u32> { self.bounds@.take(self.len as int) }
    pub open(super) spec fn labels_view(&self) -> Seq<u16> { self.labels@.take(self.len as int) }
}

impl<'a> PackedIter<'a> {
    #[verifier::type_invariant]
    pub(super) open spec fn type_invariant(&self) -> bool {
        self.bounds@.len() == self.labels@.len() && self.bounds@.len() <= 4096
    }
}

impl<'a> vstd::std_specs::iter::IteratorSpecImpl for PackedIter<'a> {
    open spec fn obeys_prophetic_iter_laws(&self) -> bool { true }

    open(super) spec fn remaining(&self) -> Seq<Rectangle> {
        Seq::new(self.bounds@.len(), |i: int| decoded(self.bounds@[i], self.labels@[i]))
    }

    open spec fn will_return_none(&self) -> bool { true }
    open(super) spec fn decrease(&self) -> Option<nat> { Some(self.bounds@.len()) }

    open(super) spec fn peek(&self, index: int) -> Option<Rectangle> {
        if 0 <= index < self.bounds@.len() { Some(decoded(self.bounds@[index], self.labels@[index])) } else { None }
    }
}

impl<'a> vstd::std_specs::iter::ExactSizeIteratorSpecImpl for PackedIter<'a> {
    open(super) spec fn exact_len(&self) -> usize { self.bounds@.len() as usize }
}

pub(crate) open(super) spec fn pushed(
    before: &PackedRectangles64, after: &PackedRectangles64,
    rectangle: Rectangle, result: Result<(), SparseQuadError>,
) -> bool {
    if before.len == 4096 {
        result is Err && result->Err_0 is CapacityOverflow
            && after.len == before.len && after.storage() == before.storage()
    } else {
        result is Ok && after.len == before.len + 1
            && after.bounds@ == before.bounds@.update(before.len as int, encoded(rectangle))
            && after.labels@ == before.labels@.update(before.len as int, rectangle.value)
    }
}

fn default_matches_new() {
    let default = PackedRectangles64::default();
    let new = PackedRectangles64::new();
    assert(default.storage() == new.storage());
    assert(default@ == new@);
}

pub(super) proof fn zero_storage(output: &PackedRectangles64)
    requires output.len == 0,
        forall|i: int| #![auto] 0 <= i < 4096 ==> output.bounds@[i] == 0 && output.labels@[i] == 0,
    ensures output.initialized(),
{
    assert(output.bounds@ =~= Seq::new(4096, |_: int| 0u32));
    assert(output.labels@ =~= Seq::new(4096, |_: int| 0u16));
}

pub(super) proof fn encoding_roundtrip(rectangle: Rectangle, bounds: u32, value: u16)
    ensures decoded(encoded(rectangle), rectangle.value) == rectangle,
        encoded(decoded(bounds, value)) == bounds,
{
    let x0 = rectangle.x.start;
    let x1 = rectangle.x.end;
    let y0 = rectangle.y.start;
    let y1 = rectangle.y.end;
    let encoded = encoded(rectangle);
    assert(encoded as u8 == x0 && (encoded >> 8) as u8 == x1
        && (encoded >> 16) as u8 == y0 && (encoded >> 24) as u8 == y1) by(bit_vector)
        requires encoded == (x0 as u32) | ((x1 as u32) << 8) | ((y0 as u32) << 16) | ((y1 as u32) << 24);
    assert(((bounds as u8 as u32) | (((bounds >> 8) as u8 as u32) << 8)
        | (((bounds >> 16) as u8 as u32) << 16) | (((bounds >> 24) as u8 as u32) << 24)) == bounds) by(bit_vector);
}

}
