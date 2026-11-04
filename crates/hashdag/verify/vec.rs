//! Vec 容量契约；不假定调用者已预留，push 的容量条件由生产证明核检查。
use vstd::prelude::*;
verus! {
pub uninterp spec fn capacity<T>(values: Vec<T>) -> usize;
}
