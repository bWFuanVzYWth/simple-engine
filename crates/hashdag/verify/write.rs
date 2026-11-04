//! 真实写入、事务与局部构建的完整装配；负例检查同点覆盖顺序。
#![feature(portable_simd, proc_macro_hygiene)]
#![allow(dead_code)]

#[path = "../src/proof/balance.rs"]
mod balance;
#[path = "../src/proof/canonical.rs"]
mod canonical;
#[path = "../src/proof/commit.rs"]
mod commit_proof;
#[path = "../src/diff.rs"]
mod diff;
#[path = "../src/error.rs"]
mod error;
#[path = "../src/hashdag.rs"]
mod hashdag;
#[path = "../src/layers.rs"]
mod layers;
#[path = "../src/layout.rs"]
mod layout;
#[path = "../src/node.rs"]
mod node;
#[path = "../src/pool.rs"]
mod pool;
#[path = "../src/rc.rs"]
mod rc;
#[path = "../src/roots.rs"]
mod roots;
#[path = "../src/proof/state.rs"]
mod state;
#[path = "../src/wide.rs"]
mod wide;

#[path = "../src/morton.rs"]
mod morton;
#[path = "../src/write.rs"]
mod write;

pub use error::Error;
pub use hashdag::HashDag;
pub use morton::ZOrder20x3;
pub use node::{Child, NodeIdx};
pub use roots::RootId;
use vstd::prelude::*;
use write::set_cells::SetCell;

verus! {

fn cell_effect(values: &mut [u32; 64], cells: &[SetCell]) {
    let ghost initial = values@;
    let changed = write::patch::apply_cells(values, cells);
    assert(values@ == write::patch::proof::apply(initial, cells@, cells@.len()));
    assert(!changed ==> values@ == initial);
}

#[cfg(reject_cell_first_writer)]
fn reject_cell_first_writer(values: &mut [u32; 64], path: ZOrder20x3) {
    let cells = [SetCell { path, data: 7 }, SetCell { path, data: 9 }];
    let _ = write::patch::apply_cells(values, &cells);
    let code = path.value();
    proof { write::patch::proof::slot_bits(code); }
    assert(values[(code & 63) as int] == 7);
}

fn main() {}
}
