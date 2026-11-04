//! 直接消费同一扫描器；首次 callback 错误立即停止。

use super::{SparseQuadError, scan::RectScan};
use crate::optimal::Rectangle;
use vstd::prelude::proof;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
#[path = "../proof/sink.rs"]
pub(super) mod proof;

/// 事件只在验证时记录；执行期逐项交付，不复制或缓存输出。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires old(scanner).valid(), old(scanner).emitted() == Seq::empty(), old(events)@ == Seq::empty(),
    ensures final(scanner).valid(), final(scanner).expected() == old(scanner).expected(),
        final(scanner).rows == old(scanner).rows,
        final(scanner).horizontal@ == old(scanner).horizontal@, final(scanner).vertical@ == old(scanner).vertical@,
        proof::delivery(old(scanner).expected(), final(events)@, result),
        proof::arguments(final(events)@) == final(scanner).emitted(),
))]
pub(super) fn write<F>(
    scanner: &mut RectScan<'_>,
    mut callback: F,
    #[cfg(verus_only)] events: &mut Ghost<Seq<proof::Event>>,
) -> Result<usize, SparseQuadError>
where
    F: FnMut(Rectangle) -> Result<(), SparseQuadError>,
{
    proof! { super::scan::proof::valid_prefix(scanner); }
    let mut count = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant scanner.valid(), scanner.expected() == old(scanner).expected(),
            scanner.rows == old(scanner).rows,
            scanner.horizontal@ == old(scanner).horizontal@, scanner.vertical@ == old(scanner).vertical@,
            scanner.emitted().len() <= scanner.expected().len() <= 4096,
            events@.len() == count, count == scanner.emitted().len(),
            proof::arguments(events@) == scanner.emitted(), proof::successful(events@),
        decreases scanner.expected().len() - scanner.emitted().len(),
    ))]
    loop {
        let Some(rectangle) = scanner.next_rect()? else {
            proof! { proof::finished(scanner.expected(), events@, count); }
            return Ok(count);
        };
        let outcome = crate::std_adapter::invoke(&mut callback, rectangle);
        proof! {
            events@ = events@.push((rectangle, outcome));
            super::scan::proof::valid_prefix(scanner);
            proof::appended(events@.drop_last(), rectangle, outcome);
            assert(proof::arguments(events@) == scanner.emitted());
            assert(proof::arguments(events@) == scanner.expected().take(events@.len() as int));
        }
        proof! {
            if outcome is Err { proof::failed(scanner.expected(), events@, outcome->Err_0); }
        }
        outcome?;
        count += 1;
    }
}
