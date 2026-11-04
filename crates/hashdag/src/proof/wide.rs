//! SIMD 分块折叠与完整切片同值谓词的连接。
use vstd::prelude::*;
verus! {
pub(super) open spec fn equal_range(a: Seq<u32>, b: Seq<u32>, start: int, end: int) -> bool {
    forall|i: int| start <= i < end ==> #[trigger] a[i] == b[i]
}

pub(super) proof fn lane_equality(a: Seq<u32>, b: Seq<u32>, start: int, left: Seq<u32>, right: Seq<u32>)
    requires a.len() == 64, b.len() == 64, 0 <= start <= 56,
        left == a.subrange(start, start + 8), right == b.subrange(start, start + 8),
    ensures (forall|j: int| 0 <= j < 8 ==> #[trigger] left[j] == right[j])
        <==> equal_range(a, b, start, start + 8),
{
    if forall|j: int| 0 <= j < 8 ==> #[trigger] left[j] == right[j] {
        assert forall|i: int| start <= i < start + 8 implies #[trigger] a[i] == b[i] by {
            assert(left[i - start] == right[i - start]);
        }
    }
    if equal_range(a, b, start, start + 8) {
        assert forall|j: int| 0 <= j < 8 implies #[trigger] left[j] == right[j] by {
            assert(a[start + j] == b[start + j]);
        }
    }
}

pub(super) proof fn equal_step(a: Seq<u32>, b: Seq<u32>, part: nat)
    requires a.len() == 64, b.len() == 64, part < 4,
    ensures equal_range(a, b, 0, ((part + 1) * 16) as int)
        <==> equal_range(a, b, 0, (part * 16) as int)
            && equal_range(a, b, (part * 16) as int, (part * 16 + 8) as int)
            && equal_range(a, b, (part * 16 + 8) as int, ((part + 1) * 16) as int),
{
    let base = (part * 16) as int;
    let end = ((part + 1) * 16) as int;
    if equal_range(a, b, 0, base) && equal_range(a, b, base, base + 8) && equal_range(a, b, base + 8, end) {
        assert forall|i: int| 0 <= i < end implies #[trigger] a[i] == b[i] by {
            if i < base { assert(a[i] == b[i]); }
            else if i < base + 8 { assert(a[i] == b[i]); }
            else { assert(a[i] == b[i]); }
        }
    }
}

pub open spec fn uniform(values: Seq<u32>) -> Option<u32> {
    if values.len() > 0 && forall|i: int| 0 <= i < values.len() ==> #[trigger] values[i] == values[0] {
        Some(values[0])
    } else { None }
}
pub proof fn unequal_block(values: Seq<u32>, chunks: Seq<[u32; 8]>, part: int)
    requires values.len() > 0, 0 <= part < chunks.len(),
        forall|i: int, j: int| 0 <= i < chunks.len() && 0 <= j < 8 ==>
            #[trigger] chunks[i]@[j] == values[i * 8 + j],
        !forall|j: int| 0 <= j < 8 ==> #[trigger] chunks[part]@[j] == values[0],
        chunks.len() == values.len() / 8,
    ensures uniform(values) is None,
{
    let j = choose|j: int| 0 <= j < 8 && chunks[part]@[j] != values[0];
    assert(values[part * 8 + j] != values[0]);
}
pub proof fn block_step(values: Seq<u32>, chunks: Seq<[u32; 8]>, part: int)
    requires values.len() > 0, 0 <= part < chunks.len(),
        forall|i: int, j: int| 0 <= i < chunks.len() && 0 <= j < 8 ==>
            #[trigger] chunks[i]@[j] == values[i * 8 + j],
        forall|i: int| 0 <= i < part * 8 ==> #[trigger] values[i] == values[0],
        forall|j: int| 0 <= j < 8 ==> #[trigger] chunks[part]@[j] == values[0],
    ensures forall|i: int| 0 <= i < (part + 1) * 8 ==> #[trigger] values[i] == values[0],
{
    assert forall|i: int| 0 <= i < (part + 1) * 8 implies #[trigger] values[i] == values[0] by {
        if i >= part * 8 { assert(chunks[part]@[i - part * 8] == values[i]); }
    }
}
}
