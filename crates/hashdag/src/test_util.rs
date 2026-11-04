use crate::ZOrder20x3;
use proptest::prelude::*;

use crate::hashdag::DAG_DEPTH;
use crate::node::PoolEntry;
use crate::pool::Pool;

pub fn total_len(pools: &[Pool; DAG_DEPTH as usize]) -> usize {
    pools.iter().map(Pool::len).sum()
}

pub fn arb_entry() -> impl Strategy<Value = PoolEntry> {
    (prop::collection::vec(any::<u32>(), 64), any::<u64>()).prop_map(|(slots, mixed)| {
        let mut arr = [0u32; 64];
        arr.copy_from_slice(&slots);
        PoolEntry::new(&arr, mixed)
    })
}

pub fn path(x: u32, y: u32, z: u32) -> ZOrder20x3 {
    ZOrder20x3::encode_unchecked(x, y, z)
}

pub fn ok<T, E>(result: Result<T, E>) -> Option<T> {
    assert!(result.is_ok());
    result.ok()
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Point {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl Point {
    pub const fn new(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }
    pub fn path(self) -> ZOrder20x3 {
        path(self.x, self.y, self.z)
    }
    pub fn aligned_to(self, lod: u8) -> Self {
        let size = 1u32 << lod;
        let mask = !(size - 1);
        Self {
            x: self.x & mask,
            y: self.y & mask,
            z: self.z & mask,
        }
    }
}

pub fn local_point(index: usize, lod: u8) -> Point {
    let mut point = Point { x: 0, y: 0, z: 0 };
    for bit in 0..lod {
        let shift = usize::from(bit) * 3;
        point.x |= u32::from(((index >> shift) & 1) != 0) << bit;
        point.y |= u32::from(((index >> (shift + 1)) & 1) != 0) << bit;
        point.z |= u32::from(((index >> (shift + 2)) & 1) != 0) << bit;
    }
    point
}
