//! 真实节点池、分层表示与引用增量；不装配射线和 GPU。
#![feature(portable_simd, proc_macro_hygiene)]
#![allow(dead_code)]

#[path = "../src/error.rs"]
mod error;
#[path = "../src/layout.rs"]
mod layout;
#[path = "../src/morton.rs"]
mod morton;
#[path = "../src/node.rs"]
mod node;
#[path = "../src/pool.rs"]
mod pool;
#[path = "../src/proof/state.rs"]
mod state;
#[path = "../src/wide.rs"]
mod wide;
mod hashdag {
    pub use crate::{
        error::Error,
        layout::{DAG_DEPTH, NODE_FANOUT, ROOT_LOD},
    };
}

pub use error::Error;
pub use morton::ZOrder20x3;
pub use node::{Child, NodeIdx};
use node::{PoolEntry, proof as node_model};
use pool::{InsertResult, Pool};
use vstd::prelude::*;

verus! {
fn intern_frame(parent:&mut Pool,children:&mut Pool,entry:&PoolEntry,rc:usize)
    requires old(parent).valid(),old(children).valid(),old(children).edge_domain(pool::proof::entry_edges(*entry)),
{
    let ghost old_parent=parent.state();
    let ghost old_children=*children;
    let result=parent.intern(rc,entry,Some(children));
    match result {
        Err(_)=>{
            proof {pool::proof::unchanged_model(&old_children,children);}
            assert(parent.state()==old_parent && children.state()==old_children.state());
        },
        Ok(InsertResult::Existing(index))=>{
            proof {pool::proof::unchanged_model(&old_children,children);}
            assert(children.state()==old_children.state());
            assert(old_parent.nodes[node_model::idx(index)]==Some(node_model::children(*entry)));
        },
        Ok(InsertResult::New(index))=>{
            proof {pool::proof::credits_model(&old_children,children,pool::proof::entry_edges(*entry));}
            assert(parent.state().nodes[node_model::idx(index)]==Some(node_model::children(*entry)));
            assert(pool::proof::credited(old_children.state(),children.state(),pool::proof::entry_edges(*entry)));
        },
    }
}

#[cfg(reject_storage_merge)]
fn reject_storage_merge(parent:&mut Pool,entry:&PoolEntry,index:NodeIdx)
    requires old(parent).valid(),state::occupied(old(parent).state(),node_model::idx(index)),
        old(parent).state().nodes[node_model::idx(index)]!=Some(node_model::children(*entry)),
{
    let result=parent.intern(0,entry,None);
    if let Ok(inserted)=result {
        let actual=match inserted {InsertResult::Existing(i)|InsertResult::New(i)=>i};
        assert(actual==index);
    }
}

#[cfg(reject_storage_existing_edges)]
fn reject_storage_existing_edges(parent:&mut Pool,children:&mut Pool,entry:&PoolEntry)
    requires old(parent).valid(),old(children).valid(),old(children).edge_domain(pool::proof::entry_edges(*entry)),
        pool::proof::matched(old(parent).state(),node_model::children(*entry)),
{
    let ghost before=*children;
    let result=parent.intern(0,entry,Some(children));
    if let Ok(_)=result {
        proof {pool::proof::unchanged_model(&before,children);}
        assert(children.state().rcs!=before.state().rcs);
    }
}

#[cfg(reject_storage_rollback)]
fn reject_storage_rollback(parent:&mut Pool,children:&mut Pool,entry:&PoolEntry)
    requires old(parent).valid(),old(children).valid(),old(children).edge_domain(pool::proof::entry_edges(*entry)),
{
    let ghost before=*children;
    let result=parent.intern(0,entry,Some(children));
    if let Err(_)=result {
        proof {pool::proof::unchanged_model(&before,children);}
        assert(children.state()!=before.state());
    }
}
fn main() {}
}
