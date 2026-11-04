//! 固定四槽的真实借用实现，验证抽象接口可由普通 Rust 精确实现。
#[cfg(verus_only)]
use crate::graph::NodeModel;
use crate::graph::{Adjacency, EdgeModel, Graph, Model};
use crate::owner::Key;
use vstd::prelude::*;

#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Stored {
    pub(crate) generation: u32,
    pub(crate) edges: Vec<Option<Key>>,
}

#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Fixture<'a> {
    pub(crate) nodes: &'a [Option<Stored>; 4],
}

#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct Borrowed<'a> {
    pub(crate) edges: &'a [Option<Key>],
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> Graph for Fixture<'a> {
    type Node = Borrowed<'a>;
    fn owner(&self) -> u64 {
        7
    }
    fn len(&self) -> usize {
        4
    }
    fn node(&self, slot: usize) -> Option<(Key, Self::Node)> {
        let node = self.nodes[slot].as_ref()?;
        Some((
            Key {
                owner: 7,
                slot: slot as u32,
                generation: node.generation,
            },
            Borrowed { edges: &node.edges },
        ))
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl Adjacency for Borrowed<'_> {
    fn len(&self) -> usize {
        self.edges.len()
    }
    fn edge(&self, index: usize) -> Option<Key> {
        self.edges[index]
    }
}

verus! {
impl View for Fixture<'_> {
    type V = Model;
    open(crate) spec fn view(&self) -> Model {
        Model { owner: 7, nodes: Seq::new(4, |i: int|
            match self.nodes@[i] {
                Some(node) => Some(NodeModel { generation: node.generation, edges: node.edges@ }),
                None => None,
            }) }
    }
}
impl View for Borrowed<'_> {
    type V = EdgeModel;
    open(crate) spec fn view(&self) -> EdgeModel {
        self.edges@
    }
}
}
