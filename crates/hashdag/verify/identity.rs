//! 仅标准原子内存单元的逻辑许可与静态对象关联；发行策略在同源 identity.rs 中验证。
use std::sync::atomic::AtomicUsize;
use vstd::prelude::*;
verus! {
#[verifier::external_body]
pub tracked struct Permission { no_copy: vstd::prelude::NoCopy }
pub uninterp spec fn view(permission:Permission)->(int,usize);
pub uninterp spec fn cell(counter:&AtomicUsize)->int;
pub uninterp spec fn global_authority()->vstd::resource::Loc;
}
