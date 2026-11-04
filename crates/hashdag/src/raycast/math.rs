use crate::raycast::types::{SignedVoxelCoord, VoxelCoord};
use std::ops::{Add, Div, Mul};

use super::{ROOT_EDGE, ROOT_EDGE_U32};

#[derive(Clone, Copy, Debug)]
pub(super) struct DVec3 {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) z: f64,
}

impl DVec3 {
    pub(super) const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub(super) const fn from_array(values: [f64; 3]) -> Self {
        let [x, y, z] = values;
        Self::new(x, y, z)
    }

    pub(super) const fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub(super) fn length_squared(self) -> f64 {
        self.x
            .mul_add(self.x, self.y.mul_add(self.y, self.z * self.z))
    }
}

impl Div<f64> for DVec3 {
    type Output = Self;

    fn div(self, rhs: f64) -> Self {
        Self::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

impl Mul<f64> for DVec3 {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Add for DVec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IVec3 {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) z: i32,
}

impl IVec3 {
    pub(super) const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub(super) const fn to_signed_tuple(self) -> SignedVoxelCoord {
        (self.x, self.y, self.z)
    }

    pub(super) fn to_voxel_coord(self) -> Option<VoxelCoord> {
        let x = u32::try_from(self.x).ok()?;
        let y = u32::try_from(self.y).ok()?;
        let z = u32::try_from(self.z).ok()?;
        Some((x, y, z))
    }
}

pub(super) const fn signed_voxel(voxel: VoxelCoord) -> Option<IVec3> {
    if voxel.0 > i32::MAX as u32 || voxel.1 > i32::MAX as u32 || voxel.2 > i32::MAX as u32 {
        return None;
    }
    Some(IVec3::new(voxel.0 as i32, voxel.1 as i32, voxel.2 as i32))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UVec3 {
    pub(super) x: u64,
    pub(super) y: u64,
    pub(super) z: u64,
}

impl UVec3 {
    pub(super) const fn new(x: u64, y: u64, z: u64) -> Self {
        Self { x, y, z }
    }
}

pub(super) fn voxel_at_entry_point(point: DVec3, dir: DVec3) -> IVec3 {
    IVec3::new(
        axis_voxel_at_entry(point.x, dir.x),
        axis_voxel_at_entry(point.y, dir.y),
        axis_voxel_at_entry(point.z, dir.z),
    )
}

fn axis_voxel_at_entry(point: f64, dir: f64) -> i32 {
    if point >= f64::from(ROOT_EDGE) && dir < 0.0 {
        ROOT_EDGE - 1
    } else {
        floor_root_coord(point)
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "root AABB 已保证起点位于 root 范围内"
)]
const fn floor_root_coord(point: f64) -> i32 {
    point.floor() as i32
}

pub(super) const fn axis_exit_crossings(step: i32, voxel: i32, min: i32, max: i32) -> i32 {
    match step {
        1 => max - voxel,
        -1 => voxel - min + 1,
        _ => 0,
    }
}

#[derive(Clone, Copy)]
pub(super) struct CellBounds {
    pub(super) min: IVec3,
    pub(super) max: IVec3,
}

pub(super) const fn cell_bounds(voxel: IVec3, lod: u8) -> CellBounds {
    let edge = 1i32 << lod;
    let mask = !(edge - 1);
    let min = IVec3::new(voxel.x & mask, voxel.y & mask, voxel.z & mask);
    CellBounds {
        min,
        max: IVec3::new(min.x + edge, min.y + edge, min.z + edge),
    }
}

pub(super) const fn voxel_in_root(voxel: VoxelCoord) -> bool {
    voxel.0 < ROOT_EDGE_U32 && voxel.1 < ROOT_EDGE_U32 && voxel.2 < ROOT_EDGE_U32
}

pub(super) const fn axis_value_i32(v: IVec3, axis: usize) -> i32 {
    match axis {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

pub(super) const fn axis_value_u64(v: UVec3, axis: usize) -> u64 {
    match axis {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

pub(super) const fn add_axis_i32(v: IVec3, axis: usize, delta: i32) -> IVec3 {
    match axis {
        0 => IVec3::new(v.x + delta, v.y, v.z),
        1 => IVec3::new(v.x, v.y + delta, v.z),
        _ => IVec3::new(v.x, v.y, v.z + delta),
    }
}

pub(super) const fn argmax3(v: DVec3) -> usize {
    if v.x >= v.y && v.x >= v.z {
        0
    } else if v.y >= v.z {
        1
    } else {
        2
    }
}
