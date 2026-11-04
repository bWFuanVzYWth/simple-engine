//! DAG 局部射线；归一化与 root 求交使用 `f64`。
//!
//! root 内的方向、局部边界距离与剩余射程按 Q40 舍入；非零方向至少保留一个单位。
//! 有限射程的支持域见 [`Ray::max_t`]；超出上界不额外检查或修复。
//! 推进比较整数时间比值，等时按 X、Y、Z 顺序跨面。命中身份取整数体素，输出距离只供查询。
//! 均匀叶可整体跳过，predicate 不被拆分或重排；根生命周期错误不属于单射线结果。

use crate::hashdag::{DAG_DEPTH, Error, HashDag, NODE_FANOUT, RootId};
use crate::node::Child;
use crate::pool::Pool;

mod dda;
mod math;
mod prepare;
#[cfg(test)]
mod tests;
mod traverse;
mod types;

pub use types::{
    AbortReason, Axis, BlockedReason, HitBehavior, InvalidInputReason, MissReason, Ray, RaySample,
    RaycastResult, SignedVoxelCoord, VoxelCoord, VoxelHit,
};

const SAFETY_COUNTER: u32 = 8192;
const DIR_ZERO_SQ: f64 = 1e-24;
const ROOT_EDGE: i32 = 1i32 << HashDag::AXIS_BITS;
const ROOT_EDGE_U32: u32 = 1u32 << HashDag::AXIS_BITS;
const CACHE_LEVELS: usize = DAG_DEPTH as usize + 1;
const DDA_FIXED_SHIFT: u32 = 40;
const DDA_FIXED_SHIFT_I32: i32 = 40;
const DDA_FIXED_ONE: u64 = 1u64 << DDA_FIXED_SHIFT;
const DDA_FIXED_ONE_U128: u128 = 1u128 << DDA_FIXED_SHIFT;
const F64_MANTISSA_BITS: u32 = 52;
const F64_EXP_BIAS: i32 = 1023;

impl HashDag {
    /// 沿 root 遍历一条射线，使用与 [`Self::raycast_batch`] 相同的内核。
    ///
    /// 方向在校验后归一化，体素坐标范围为每轴 `0..2^20`。均匀叶只回调一次；
    /// 返回 [`HitBehavior::Pass`] 跳过整个叶。root 内的方向与距离量化为 40 bit 小数，
    /// 边界时间相等时按 X、Y、Z 顺序推进；极小非零方向保留一个量化单位。
    /// 单条最多访问 8192 个均匀叶，之后仍需继续则返回 [`AbortReason::SafetyExhausted`]。
    /// 非有限、负射程及无效方向由 [`RaycastResult::InvalidInput`] 表达；
    /// 射程还须满足 [`Ray::max_t`] 的数值支持域。
    ///
    /// # Errors
    ///
    /// root 属于其他实例或已失效时返回错误，且不调用 predicate；节点损坏由
    /// [`RaycastResult::Aborted`] 表达，不与 root 生命周期错误混用。
    pub fn raycast<F>(&self, root: RootId, ray: Ray, predicate: F) -> Result<RaycastResult, Error>
    where
        F: FnMut(RaySample) -> HitBehavior,
    {
        let child = self.root_child(root)?;
        Ok(cast_one(&self.pools, child, ray, predicate))
    }

    /// 按输入顺序完整遍历每条射线，将结果写入调用者持有的等长 output。
    ///
    /// 先校验 root 与长度；失败时不修改 output、不调用 predicate。单条非法输入写入
    /// [`RaycastResult::InvalidInput`] 后继续后续射线。回调携带输入下标，同一条射线完成后
    /// 才处理下一条；不分配、不调度，不重排有状态回调。每条射线只保留深度有界的祖先缓存。
    /// DDA 的均匀叶跳过、距离与轴序约定见 [`Self::raycast`]。
    ///
    /// # Errors
    ///
    /// root 属于其他实例、已失效或 output 长度不等于 rays 长度时返回错误。
    pub fn raycast_batch<F>(
        &self,
        root: RootId,
        rays: &[Ray],
        output: &mut [RaycastResult],
        mut predicate: F,
    ) -> Result<(), Error>
    where
        F: FnMut(usize, RaySample) -> HitBehavior,
    {
        let child = self.root_child(root)?;
        if output.len() != rays.len() {
            return Err(Error::InvalidInput);
        }
        for (index, (ray, result)) in rays.iter().zip(output.iter_mut()).enumerate() {
            *result = cast_one(&self.pools, child, *ray, |sample| predicate(index, sample));
        }
        Ok(())
    }
}

fn cast_one<F>(
    pools: &[Pool; DAG_DEPTH as usize],
    root: Child,
    ray: Ray,
    predicate: F,
) -> RaycastResult
where
    F: FnMut(RaySample) -> HitBehavior,
{
    let prepared = match prepare::prepare_ray(ray) {
        Ok(prepared) => prepared,
        Err(error) => return error.into_result(),
    };
    traverse::traverse(pools, root, prepared, predicate)
}

#[cfg(test)]
use dda::DdaState;
#[cfg(test)]
use math::{CellBounds, DVec3, IVec3, cell_bounds};

const _: () = assert!(NODE_FANOUT == 64);
