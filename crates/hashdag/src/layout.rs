//! 原生节点的固定分层与扇出。

#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg_attr(verus_only, verus_verify)]
pub const DAG_DEPTH: u8 = 10;
#[cfg_attr(verus_only, verus_verify)]
pub const ROOT_LOD: u8 = DAG_DEPTH - 1;
#[cfg_attr(verus_only, verus_verify)]
pub const NODE_FANOUT: usize = 64;
