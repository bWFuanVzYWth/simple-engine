//! 验证生产槽表；不导入 Vulkan FFI、设备或错误包装。
#![cfg_attr(verus_only, feature(proc_macro_hygiene))]

#[path = "../src/arena.rs"]
mod arena;

use vstd::prelude::*;

verus! {
fn cursor_keeps_first_value<T>(arena: &arena::Arena<T>)
    requires arena.values_view().len() > 0,
{
    let mut cursor = arena.values_cursor();
    let ghost original = cursor@;
    let found = cursor.advance();
    assert(found is Some && *found->Some_0 == original[0]);
    #[cfg(reject_cursor_skips_live_value)]
    assert(found is None);
}

fn main() {
    let mut arena = arena::Arena::<u32>::new();
    let key = match arena.try_insert(7, 1) {
        Ok(key) => key,
        Err(_) => return,
    };
    let current = arena.at(key.slot as usize, 7);
    assert(current is Some && current->Some_0.0 == key && *current->Some_0.1 == 1);
    let length = arena.len();
    assert(length > key.slot);
    arena.remove(key);
    let _ = arena.try_insert(7, 2);
    proof { arena::revoked_absent(&arena, key); }
    assert(arena.lookup(key) is None);
    #[cfg(reject_stale_arena_key)]
    assert(arena.lookup(key) is Some);
}
}
