//! 实际 callback 返回事件的前缀、计数和首错停止；不定义闭包内部状态。

use super::{Rectangle, SparseQuadError};
use vstd::prelude::*;

verus! {

pub(in crate::optimal::sparse) type Event = (Rectangle, Result<(), SparseQuadError>);

pub(in crate::optimal::sparse) open spec fn arguments(events: Seq<Event>) -> Seq<Rectangle> {
    Seq::new(events.len(), |i: int| events[i].0)
}

pub(in crate::optimal::sparse) open spec fn successful(events: Seq<Event>) -> bool {
    forall|i: int| 0 <= i < events.len() ==> #[trigger] events[i].1 is Ok
}

pub(in crate::optimal::sparse) open spec fn delivery(full: Seq<Rectangle>, events: Seq<Event>, result: Result<usize, SparseQuadError>) -> bool {
    events.len() <= full.len() && arguments(events) == full.take(events.len() as int)
        && match result {
            Ok(count) => events.len() == full.len() && count == events.len() && successful(events),
            Err(error) => events.len() > 0 && events.last().1 == Err::<(), SparseQuadError>(error)
                && successful(events.drop_last()),
        }
}

impl super::super::SparseOptimalScratch64 {
    /// 稀疏输入的真实校验、最优完整序列和实际 callback 前缀；非法输入没有调用事件。
    #[verifier::opaque]
    pub open(in crate::optimal::sparse) spec fn sparse_delivery(&self, leaves: Seq<super::super::QuadLeaf64>, events: Seq<Event>, result: Result<usize, SparseQuadError>) -> bool {
        self.valid() && if super::super::input_proof::valid(leaves) {
            self.rows.labels() == super::super::input_proof::labels(leaves)
                && crate::partition::optimal(super::super::input_proof::labels(leaves), self.canonical())
                && self.canonical() == crate::partition::optimality::result_for(super::super::input_proof::labels(leaves))
                && delivery(self.canonical(), events, result)
        } else { result is Err && events == Seq::empty()
            && super::super::failure_proof::failure_for(leaves) == Some(result->Err_0) }
    }

    /// 连续输入的最优完整序列和实际 callback 前缀；正常返回后仍可复用。
    #[verifier::opaque]
    pub open(in crate::optimal::sparse) spec fn dense_delivery(&self, labels: Seq<u16>, events: Seq<Event>, result: Result<usize, SparseQuadError>) -> bool {
        self.valid() && self.rows.labels() == labels && crate::partition::optimal(labels, self.canonical())
            && self.canonical() == crate::partition::optimality::result_for(labels)
            && delivery(self.canonical(), events, result)
    }
}

pub(super) proof fn appended(before: Seq<Event>, rectangle: Rectangle, outcome: Result<(), SparseQuadError>)
    ensures arguments(before.push((rectangle, outcome))) == arguments(before).push(rectangle),
        outcome is Ok && successful(before) ==> successful(before.push((rectangle, outcome))),
{
    assert(arguments(before.push((rectangle, outcome))) =~= arguments(before).push(rectangle));
}

pub(super) proof fn finished(full: Seq<Rectangle>, events: Seq<Event>, count: usize)
    requires arguments(events) == full, successful(events), count == events.len(),
    ensures delivery(full, events, Ok(count)),
{
    assert(full.take(full.len() as int) =~= full);
}

pub(super) proof fn failed(full: Seq<Rectangle>, events: Seq<Event>, error: SparseQuadError)
    requires events.len() > 0, events.len() <= full.len(),
        arguments(events) == full.take(events.len() as int),
        events.last().1 == Err::<(), SparseQuadError>(error), successful(events.drop_last()),
    ensures delivery(full, events, Err(error)),
{}

/// 同一规范序列上逐次返回相同时，首次失败位置、完整事件与最终返回值均相同。
pub(super) proof fn repeat_delivery(full: Seq<Rectangle>, first: Seq<Event>, second: Seq<Event>,
    first_result: Result<usize, SparseQuadError>, second_result: Result<usize, SparseQuadError>)
    requires delivery(full, first, first_result), delivery(full, second, second_result),
        forall|i: int| 0 <= i < first.len() && i < second.len() ==> (#[trigger] first[i]).1 == (#[trigger] second[i]).1,
    ensures first == second, first_result == second_result,
{
    if first.len() < second.len() {
        assert(first_result is Err);
        assert(first.len() > 0);
        let index = first.len() as int - 1;
        assert(first[index].1 is Err);
        assert(second[index] == second.drop_last()[index]);
        assert(second[index].1 is Ok);
    }
    if second.len() < first.len() {
        assert(second_result is Err);
        assert(second.len() > 0);
        let index = second.len() as int - 1;
        assert(second[index].1 is Err);
        assert(first[index] == first.drop_last()[index]);
        assert(first[index].1 is Ok);
    }
    assert(first.len() == second.len());
    assert(arguments(first) == arguments(second));
    assert forall|i: int| 0 <= i < first.len() implies #[trigger] first[i] == second[i] by {
        assert(first[i].0 == arguments(first)[i]);
        assert(second[i].0 == arguments(second)[i]);
    }
    assert(first =~= second);
    if first_result is Ok {
        if second_result is Err { assert(successful(first)); assert(first.last().1 is Ok); }
    } else if second_result is Ok { assert(successful(second)); assert(second.last().1 is Ok); }
}

#[cfg(reject_sink_count)]
fn reject_sink_count<F: FnMut(Rectangle) -> Result<(), SparseQuadError>>(
    scanner: &mut super::RectScan<'_>, callback: F, events: &mut Ghost<Seq<Event>>,
)
    requires old(scanner).valid(), old(scanner).emitted() == Seq::empty(), old(events)@ == Seq::empty(),
{
    let result = super::write(scanner, callback, events);
    if let Ok(count) = result { assert(count != events@.len()); }
}

#[cfg(reject_sink_error)]
fn reject_sink_error<F: FnMut(Rectangle) -> Result<(), SparseQuadError>>(
    scanner: &mut super::RectScan<'_>, callback: F, events: &mut Ghost<Seq<Event>>,
)
    requires old(scanner).valid(), old(scanner).emitted() == Seq::empty(), old(events)@ == Seq::empty(),
{
    let result = super::write(scanner, callback, events);
    if let Err(error) = result { assert(events@.last().1 != Err::<(), SparseQuadError>(error)); }
}

#[cfg(reject_sink_argument)]
fn reject_sink_argument<F: FnMut(Rectangle) -> Result<(), SparseQuadError>>(
    scanner: &mut super::RectScan<'_>, callback: F, events: &mut Ghost<Seq<Event>>,
)
    requires old(scanner).valid(), old(scanner).emitted() == Seq::empty(), old(events)@ == Seq::empty(),
{
    let ghost full = scanner.expected();
    let _ = super::write(scanner, callback, events);
    assert(events@.len() == 0 || events@[0].0 != full[0]);
}

}
