//! 搜索初始化与未匹配编号稳定过滤。
use super::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

pub(crate) open spec fn free_prefix(pairs: Seq<u16>, end: int) -> Seq<u16>
    decreases end,
{
    if end <= 0 { Seq::empty() }
    else {
        let before = free_prefix(pairs, end - 1);
        if pairs[end - 1] == u16::MAX { before.push((end - 1) as u16) }
        else { before }
    }
}

pub(super) proof fn prefix_step(pairs: Seq<u16>, left: int)
    requires 0 <= left < pairs.len(),
    ensures free_prefix(pairs, left + 1) == if pairs[left] == u16::MAX {
        free_prefix(pairs, left).push(left as u16)
    } else { free_prefix(pairs, left) },
{
    reveal_with_fuel(free_prefix, 1);
}

pub(crate) open spec fn enumerated(pairs: Seq<u16>, output: Seq<u16>, end: int) -> bool {
    output.len() <= end && output.no_duplicates()
        && (forall|id: u16| #[trigger] output.contains(id) <==> (id as int) < end && pairs[id as int] == u16::MAX)
        && (forall|i: int, j: int| #![auto] 0 <= i < j < output.len() ==> output[i] < output[j])
}

pub(super) proof fn complete(pairs: Seq<u16>, end: int)
    requires 0 <= end <= pairs.len(), end <= u16::MAX,
    ensures enumerated(pairs, free_prefix(pairs, end), end),
    decreases end,
{
    reveal_with_fuel(free_prefix, 1);
    if end > 0 {
        complete(pairs, end - 1);
        let previous = free_prefix(pairs, end - 1);
        let current = free_prefix(pairs, end);
        if pairs[end - 1] == u16::MAX {
            assert(!previous.contains((end - 1) as u16));
            assert forall|id: u16| #[trigger] current.contains(id) <==>
                (id as int) < end && pairs[id as int] == u16::MAX by {
                if current.contains(id) && !previous.contains(id) { assert(id == (end - 1) as u16); }
            }
            assert forall|i: int, j: int| #![auto] 0 <= i < j < current.len()
                implies current[i] < current[j] by {
                if j == previous.len() { assert(previous.contains(previous[i])); }
            }
        }
    }
}

pub(crate) open spec fn valid_ids(pairs: Seq<u16>, ids: Seq<u16>) -> bool {
    forall|i: int| #![auto] 0 <= i < ids.len() ==> ids[i] < pairs.len()
}

pub(crate) open spec fn unmatched_ids(pairs: Seq<u16>, ids: Seq<u16>) -> Seq<u16> {
    ids.filter(|id: u16| pairs[id as int] == u16::MAX)
}

pub(super) open spec fn filter_loop(pairs: Seq<u16>, input: Seq<u16>, current: Seq<u16>,
    read: int, write: int) -> bool
{
    valid_ids(pairs, input) && 0 <= write <= read <= input.len() && current.len() == input.len()
        && current.take(write) == unmatched_ids(pairs, input.take(read))
        && (forall|i: int| #![auto] write <= i < input.len() ==> current[i] == input[i])
}

pub(super) proof fn filter_started(pairs: Seq<u16>, input: Seq<u16>)
    requires valid_ids(pairs, input),
    ensures filter_loop(pairs, input, input, 0, 0),
{
    assert(input.take(0) =~= Seq::<u16>::empty());
    reveal_with_fuel(Seq::filter, 1);
}

pub(super) proof fn filter_step(pairs: Seq<u16>, input: Seq<u16>, before: Seq<u16>, after: Seq<u16>,
    read: int, write: int, next: int)
    requires filter_loop(pairs, input, before, read, write), read < input.len(),
        after == if pairs[input[read] as int] == u16::MAX { before.update(write, input[read]) } else { before },
        next == if pairs[input[read] as int] == u16::MAX { write + 1 } else { write },
    ensures filter_loop(pairs, input, after, read + 1, next),
{
    let prefix = input.take(read);
    assert(input.take(read + 1) =~= prefix.push(input[read]));
    prefix.lemma_filter_push(input[read], |id: u16| pairs[id as int] == u16::MAX);
    assert(after.take(next) =~= unmatched_ids(pairs, input.take(read + 1)));
}

pub(super) proof fn filter_done(pairs: Seq<u16>, input: Seq<u16>, current: Seq<u16>, write: int)
    requires filter_loop(pairs, input, current, input.len() as int, write),
    ensures current.take(write) == unmatched_ids(pairs, input), current.skip(write) == input.skip(write),
{
    assert(input.take(input.len() as int) =~= input);
    assert(current.skip(write) =~= input.skip(write));
}
}
