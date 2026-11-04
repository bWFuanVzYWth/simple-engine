//! 公共输出规范及其逻辑推论；这些引理不替代生产分区与最优性证明。

use crate::optimal::{RangeU8, Rectangle};
use vstd::prelude::*;

#[path = "chain_marks.rs"]
pub(crate) mod chain_marks;
#[path = "chain_set.rs"]
pub(crate) mod chain_set;
#[path = "chains.rs"]
pub(crate) mod chains;
#[path = "colored.rs"]
pub(crate) mod colored;
#[path = "conflict_budget.rs"]
pub(crate) mod conflict_budget;
#[path = "continuation_count.rs"]
pub(crate) mod continuation_count;
#[path = "corners.rs"]
pub(crate) mod corners;
#[path = "good.rs"]
pub(crate) mod good;
#[path = "lower_bound.rs"]
pub(crate) mod lower_bound;
#[path = "neighbor_count.rs"]
pub(crate) mod neighbor_count;
#[path = "optimality.rs"]
pub(crate) mod optimality;
#[path = "row_chords.rs"]
pub(crate) mod row_chords;
#[path = "row_count.rs"]
pub(crate) mod row_count;
#[path = "scan_count.rs"]
pub(crate) mod scan_count;
#[path = "scan_model.rs"]
pub(crate) mod scan_model;
#[path = "split_count.rs"]
pub(crate) mod split_count;
#[path = "split_geometry.rs"]
pub(crate) mod split_geometry;
#[path = "upper_bound.rs"]
pub(crate) mod upper_bound;
#[path = "vertical_count.rs"]
pub(crate) mod vertical_count;

verus! {

pub(crate) open spec fn valid(rectangle: Rectangle) -> bool {
    rectangle.value != 0
        && rectangle.x.start < rectangle.x.end <= 64
        && rectangle.y.start < rectangle.y.end <= 64
}

pub(crate) open spec fn covers(rectangle: Rectangle, pixel: int) -> bool {
    0 <= pixel < 4096
        && rectangle.x.start <= pixel % 64 < rectangle.x.end
        && rectangle.y.start <= pixel / 64 < rectangle.y.end
}

pub(crate) open spec fn partition(labels: Seq<u16>, rectangles: Seq<Rectangle>) -> bool {
    labels.len() == 4096
        && (forall|i: int| 0 <= i < rectangles.len() ==> #[trigger] valid(rectangles[i]))
        && (forall|i: int, p: int| 0 <= i < rectangles.len() && #[trigger] covers(rectangles[i], p)
            ==> labels[p] == rectangles[i].value)
        && (forall|p: int| 0 <= p < 4096 ==> (labels[p] != 0 <==>
            exists|i: int| 0 <= i < rectangles.len() && #[trigger] covers(rectangles[i], p)))
        && (forall|i: int, j: int, p: int| 0 <= i < rectangles.len() && 0 <= j < rectangles.len()
            && #[trigger] covers(rectangles[i], p) && #[trigger] covers(rectangles[j], p) ==> i == j)
}

pub(crate) open spec fn optimal(labels: Seq<u16>, rectangles: Seq<Rectangle>) -> bool {
    partition(labels, rectangles)
        && forall|other: Seq<Rectangle>| #[trigger] partition(labels, other)
            ==> rectangles.len() <= other.len()
}

pub(super) open spec fn unit(value: u16, p: int) -> Rectangle {
    Rectangle { value,
        x: RangeU8 { start: (p % 64) as u8, end: (p % 64 + 1) as u8 },
        y: RangeU8 { start: (p / 64) as u8, end: (p / 64 + 1) as u8 } }
}

proof fn unit_covers(value: u16, p: int, q: int)
    requires 0 <= p < 4096, 0 <= q < 4096,
    ensures covers(unit(value, p), q) <==> p == q,
        value != 0 ==> valid(unit(value, p)),
{
    assert(0 <= p % 64 < 64 && 0 <= p / 64 < 64);
    assert(p == p / 64 * 64 + p % 64);
    assert(q == q / 64 * 64 + q % 64);
}

/// 任意完整标签图均可按非零像素分区，结果规范没有排除孔洞、接触点或标签组合。
pub(crate) proof fn partition_exists(labels: Seq<u16>)
    requires labels.len() == 4096,
    ensures exists|rectangles: Seq<Rectangle>| partition(labels, rectangles),
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use vstd::seq_lib::group_seq_properties;
    let visible = Set::<int>::range(0, 4096).filter(|p: int| labels[p] != 0);
    let pixels = visible.to_seq();
    visible.lemma_to_seq_to_set_id();
    visible.lemma_to_seq_no_duplicates();
    pixels.to_set_ensures();
    vstd::set_lib::lemma_int_range(0, 4096);
    let rectangles = pixels.map_values(|p: int| unit(labels[p], p));
    assert forall|i: int| 0 <= i < rectangles.len() implies #[trigger] valid(rectangles[i]) by {
        assert(visible.contains(pixels[i]));
        unit_covers(labels[pixels[i]], pixels[i], pixels[i]);
    }
    assert forall|i: int, p: int| 0 <= i < rectangles.len() && #[trigger] covers(rectangles[i], p)
        implies labels[p] == rectangles[i].value by {
        assert(visible.contains(pixels[i]));
        unit_covers(labels[pixels[i]], pixels[i], p);
    }
    assert forall|p: int| 0 <= p < 4096 implies (labels[p] != 0 <==>
        exists|i: int| 0 <= i < rectangles.len() && #[trigger] covers(rectangles[i], p)) by {
        if labels[p] != 0 {
            assert(visible.contains(p));
            let i = choose|i: int| 0 <= i < pixels.len() && pixels[i] == p;
            unit_covers(labels[p], p, p);
            assert(covers(rectangles[i], p));
        } else if exists|i: int| 0 <= i < rectangles.len() && #[trigger] covers(rectangles[i], p) {
            let i = choose|i: int| 0 <= i < rectangles.len() && #[trigger] covers(rectangles[i], p);
            assert(valid(rectangles[i]));
            assert(labels[p] == rectangles[i].value);
        }
    }
    assert forall|i: int, j: int, p: int| 0 <= i < rectangles.len() && 0 <= j < rectangles.len()
        && #[trigger] covers(rectangles[i], p) && #[trigger] covers(rectangles[j], p) implies i == j by {
        assert(visible.contains(pixels[i]) && visible.contains(pixels[j]));
        unit_covers(labels[pixels[i]], pixels[i], p);
        unit_covers(labels[pixels[j]], pixels[j], p);
    }
    assert(partition(labels, rectangles));
}

proof fn minimize(labels: Seq<u16>, rectangles: Seq<Rectangle>)
    requires partition(labels, rectangles),
    ensures exists|result: Seq<Rectangle>| optimal(labels, result),
    decreases rectangles.len(),
{
    if !optimal(labels, rectangles) {
        let smaller = choose|other: Seq<Rectangle>| #[trigger] partition(labels, other) && other.len() < rectangles.len();
        minimize(labels, smaller);
    }
}

pub(crate) proof fn optimal_exists(labels: Seq<u16>)
    requires labels.len() == 4096,
    ensures exists|result: Seq<Rectangle>| optimal(labels, result),
{
    partition_exists(labels);
    let rectangles = choose|rectangles: Seq<Rectangle>| partition(labels, rectangles);
    minimize(labels, rectangles);
}

pub(crate) proof fn optimal_counts_agree(labels: Seq<u16>, a: Seq<Rectangle>, b: Seq<Rectangle>)
    requires optimal(labels, a), optimal(labels, b),
    ensures a.len() == b.len(),
{}

pub(crate) proof fn empty_partition(labels: Seq<u16>)
    requires labels.len() == 4096,
    ensures partition(labels, Seq::empty()) <==> forall|p: int| 0 <= p < 4096 ==> labels[p] == 0,
{}

pub(crate) proof fn exact_cover(labels: Seq<u16>, rectangles: Seq<Rectangle>, p: int)
    requires partition(labels, rectangles), 0 <= p < 4096, labels[p] != 0,
    ensures exists|i: int| 0 <= i < rectangles.len() && #[trigger] covers(rectangles[i], p)
        && (forall|j: int| 0 <= j < rectangles.len() && #[trigger] covers(rectangles[j], p) ==> i == j),
{
    let i = choose|i: int| 0 <= i < rectangles.len() && #[trigger] covers(rectangles[i], p);
    assert forall|j: int| 0 <= j < rectangles.len() && #[trigger] covers(rectangles[j], p)
        implies i == j by {}
}

pub(crate) open spec fn origin(rectangle: Rectangle) -> int {
    rectangle.x.start as int + 64 * rectangle.y.start as int
}

pub(crate) proof fn origin_covered(rectangle: Rectangle)
    requires valid(rectangle),
    ensures covers(rectangle, origin(rectangle)),
{
    assert(origin(rectangle) % 64 == rectangle.x.start as int) by(nonlinear_arith)
        requires rectangle.x.start < 64;
    assert(origin(rectangle) / 64 == rectangle.y.start as int) by(nonlinear_arith)
        requires rectangle.x.start < 64;
}

/// 非空矩形的原点给出到可见像素集合的单射，数量上界不依赖最优性。
pub(crate) proof fn rectangle_count_bound(labels: Seq<u16>, rectangles: Seq<Rectangle>)
    requires partition(labels, rectangles),
    ensures rectangles.len() <= Set::<int>::range(0, 4096).filter(|p: int| labels[p] != 0).len()
        <= 4096,
{
    broadcast use vstd::set_lib::group_set_properties;
    broadcast use Set::lemma_map_contains;
    let indices = Set::<int>::range(0, rectangles.len() as int);
    let grid = Set::<int>::range(0, 4096);
    let visible = grid.filter(|p: int| labels[p] != 0);
    let point = |i: int| origin(rectangles[i]);
    let origins = indices.map(point);
    vstd::set_lib::lemma_int_range(0, rectangles.len() as int);
    vstd::set_lib::lemma_int_range(0, 4096);
    assert(indices.injective_on(point)) by {
        assert forall|i: int, j: int| indices.contains(i) && indices.contains(j)
            && #[trigger] point(i) == #[trigger] point(j) implies i == j by {
            origin_covered(rectangles[i]);
            origin_covered(rectangles[j]);
        }
    }
    vstd::set_lib::lemma_map_size(indices, origins, point);
    assert(origins.subset_of(visible)) by {
        assert forall|p: int| #[trigger] origins.contains(p) implies visible.contains(p) by {
            let i = choose|i: int| indices.contains(i) && point(i) == p;
            origin_covered(rectangles[i]);
            assert(labels[p] == rectangles[i].value);
        }
    }
    grid.lemma_len_filter(|p: int| labels[p] != 0);
    vstd::set_lib::lemma_len_subset(origins, visible);
}

}
