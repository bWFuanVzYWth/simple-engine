//! 固定缓冲的视图与前缀变换证明。
use super::*;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;

impl<T: Copy, const N: usize> View for FixedVec<T, N> {
    type V = Seq<T>;
    open(super) spec fn view(&self) -> Seq<T> { self.items@.take(self.len as int) }
}

impl<T: Copy, const N: usize> FixedVec<T, N> {
    pub(crate) open(super) spec fn valid(&self) -> bool { self.len <= N }
    pub(crate) open(super) spec fn storage(&self) -> Seq<T> { self.items@ }

    #[verifier::type_invariant]
    pub(super) open spec fn type_invariant(&self) -> bool { self.valid() }

    pub(crate) proof fn establish(tracked value: &Self)
        ensures value.valid(),
    {
        use_type_invariant(value);
    }

}

pub(crate) open spec fn popped<T>(before: Seq<T>, after: Seq<T>, result: Option<T>) -> bool {
    if before.len() == 0 { after == before && result is None }
    else { after == before.drop_last() && result == Some(before.last()) }
}

pub(crate) open spec fn resized<T>(before: Seq<T>, len: int, value: T) -> Seq<T> {
    if len <= before.len() { before.take(len) }
    else { before + Seq::new((len - before.len()) as nat, |_: int| value) }
}

pub(super) proof fn pop_prefix<T>(items: Seq<T>, len: int)
    requires 0 < len <= items.len(),
    ensures items.take(len - 1) == items.take(len).drop_last(),
        items[len - 1] == items.take(len).last(),
{
    assert(items.take(len - 1) =~= items.take(len).drop_last());
}

}
