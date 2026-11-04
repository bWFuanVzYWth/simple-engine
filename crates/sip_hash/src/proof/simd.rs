//! 八路生产内核与标量规范的逐 lane 对应；标准原语由独立可信适配提供。

use super::super::scalar::proof as scalar;
use super::State;
use sip_hash_std::{view32, view64};
use std::simd::u32x8;
use vstd::prelude::*;

verus! {

broadcast use sip_hash_std::primitives;

pub open spec fn lane(s: State, i: int) -> scalar::State {
    (view64(s.0)[i], view64(s.1)[i], view64(s.2)[i], view64(s.3)[i])
}

pub open spec fn iterated(before: State, after: State, n: nat) -> bool {
    forall|i: int| 0 <= i < 8 ==> #[trigger] lane(after, i) == scalar::rounds(lane(before, i), n)
}

pub open spec fn iterated_view(before: State, after: State, n: nat) -> bool {
    view64(after.0) == Seq::new(8, |i:int| scalar::rounds(lane(before,i),n).0)
    && view64(after.1) == Seq::new(8, |i:int| scalar::rounds(lane(before,i),n).1)
    && view64(after.2) == Seq::new(8, |i:int| scalar::rounds(lane(before,i),n).2)
    && view64(after.3) == Seq::new(8, |i:int| scalar::rounds(lane(before,i),n).3)
}

pub(crate) proof fn rounds_view(before: State, after: State, n: nat)
    requires iterated(before, after, n),
    ensures iterated_view(before, after, n),
{
    assert forall|i:int| 0 <= i < 8 implies
        view64(after.0)[i] == scalar::rounds(lane(before,i),n).0
        && view64(after.1)[i] == scalar::rounds(lane(before,i),n).1
        && view64(after.2)[i] == scalar::rounds(lane(before,i),n).2
        && view64(after.3)[i] == scalar::rounds(lane(before,i),n).3 by {
        assert(lane(after,i) == scalar::rounds(lane(before,i),n));
    }
    assert(view64(after.0) =~= Seq::new(8, |i:int| scalar::rounds(lane(before,i),n).0));
    assert(view64(after.1) =~= Seq::new(8, |i:int| scalar::rounds(lane(before,i),n).1));
    assert(view64(after.2) =~= Seq::new(8, |i:int| scalar::rounds(lane(before,i),n).2));
    assert(view64(after.3) =~= Seq::new(8, |i:int| scalar::rounds(lane(before,i),n).3));
}

pub(crate) proof fn rounds_step(before: State, n: nat)
    ensures forall|i: int| 0 <= i < 8 ==>
        scalar::rounds(lane(before, i), n + 1) == scalar::round(scalar::rounds(lane(before, i), n)),
{
    assert forall|i: int| 0 <= i < 8 implies
        scalar::rounds(lane(before, i), n + 1) == scalar::round(scalar::rounds(lane(before, i), n)) by {
        scalar::rounds_step(lane(before, i), n);
    }
}

pub(crate) proof fn coordinates(s: scalar::State, a: u32x8, b: u32x8, c: u32x8, cr: nat, dr: nat)
    ensures forall|i: int| 0 <= i < 8 ==>
        #[trigger] scalar::coords(s, view32(a)[i], view32(b)[i], view32(c)[i], cr, dr)
        == scalar::finish(scalar::compress(scalar::compress(s,
            (view32(a)[i] as u64) | ((view32(b)[i] as u64) << 32), cr),
            (12u64 << 56) | (view32(c)[i] as u64), cr), dr),
{
    assert forall|i: int| 0 <= i < 8 implies
        scalar::coords(s, view32(a)[i], view32(b)[i], view32(c)[i], cr, dr)
        == scalar::finish(scalar::compress(scalar::compress(s,
            (view32(a)[i] as u64) | ((view32(b)[i] as u64) << 32), cr),
            (12u64 << 56) | (view32(c)[i] as u64), cr), dr) by {
        scalar::coords_words(s, view32(a)[i], view32(b)[i], view32(c)[i], cr, dr);
    }
}

}
