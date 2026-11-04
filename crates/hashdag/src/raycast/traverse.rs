use crate::ZOrder20x3;

use crate::hashdag::DAG_DEPTH;
use crate::node::Child;
use crate::pool::Pool;

use super::dda::DdaState;
use super::math::voxel_in_root;
use super::prepare::PreparedRay;
use super::types::{
    AbortReason, BlockedReason, HitBehavior, MissReason, RaySample, RaycastResult, VoxelHit,
};
use super::{CACHE_LEVELS, SAFETY_COUNTER};

struct UniformSample {
    value: u32,
    lod: u8,
}

struct TraversalCache {
    path: ZOrder20x3,
    children: [Child; CACHE_LEVELS],
    min_valid_lod: u8,
}

pub(super) fn traverse<F>(
    pools: &[Pool; DAG_DEPTH as usize],
    root: Child,
    prepared: PreparedRay,
    mut predicate: F,
) -> RaycastResult
where
    F: FnMut(RaySample) -> HitBehavior,
{
    let mut state = DdaState::init(
        prepared.origin,
        prepared.dir,
        prepared.t_enter,
        prepared.face_axis,
    );
    let mut step_count = 0u32;
    let mut cache = TraversalCache::new(root);

    loop {
        if state.exceeds_reach(prepared.max_relative_t) {
            return RaycastResult::Miss {
                reason: MissReason::ReachExceeded,
            };
        }

        let Some(voxel) = state.voxel.to_voxel_coord() else {
            return RaycastResult::Miss {
                reason: MissReason::BoundaryExceeded,
            };
        };
        if !voxel_in_root(voxel) {
            return RaycastResult::Miss {
                reason: MissReason::BoundaryExceeded,
            };
        }
        if step_count >= SAFETY_COUNTER {
            return RaycastResult::Aborted {
                reason: AbortReason::SafetyExhausted,
            };
        }
        step_count += 1;
        // voxel_in_root 已保证坐标在范围内
        let path = ZOrder20x3::encode_unchecked(voxel.0, voxel.1, voxel.2);

        let sample = cache.uniform_at_path(pools, path);
        let Some(sample) = sample else {
            return RaycastResult::Aborted {
                reason: AbortReason::DagCorruption,
            };
        };
        let ray_sample = RaySample {
            voxel,
            value: sample.value,
            lod: sample.lod,
            step_count,
        };

        match predicate(ray_sample) {
            HitBehavior::Hit => {
                return RaycastResult::Hit(VoxelHit {
                    voxel,
                    value: sample.value,
                    lod: sample.lod,
                    face_axis: state.face_axis,
                    t: state.t_current(prepared.t_enter),
                    prev_empty: state.prev_cell().to_signed_tuple(),
                });
            }
            HitBehavior::Block => {
                return RaycastResult::Blocked {
                    by_voxel: voxel,
                    value: sample.value,
                    lod: sample.lod,
                    t: state.t_current(prepared.t_enter),
                    reason: BlockedReason::PredicateBlock,
                };
            }
            HitBehavior::Pass => {
                if sample.lod == 0 {
                    state.advance();
                } else {
                    state.skip_lod_cell(sample.lod);
                }
            }
        }
    }
}

impl TraversalCache {
    fn new(root: Child) -> Self {
        let mut children = [Child::Leaf(0); CACHE_LEVELS];
        if let Some(child) = children.get_mut(usize::from(DAG_DEPTH)) {
            *child = root;
        }
        Self {
            path: ZOrder20x3::ZERO,
            children,
            min_valid_lod: DAG_DEPTH,
        }
    }

    fn uniform_at_path(
        &mut self,
        pools: &[Pool; DAG_DEPTH as usize],
        path: ZOrder20x3,
    ) -> Option<UniformSample> {
        let mut current_lod = self.reusable_lod(path);
        let mut child = *self.children.get(usize::from(current_lod))?;

        loop {
            match child {
                Child::Leaf(value) => {
                    self.path = path;
                    self.min_valid_lod = current_lod;
                    return Some(UniformSample {
                        value,
                        lod: current_lod * 2,
                    });
                }
                Child::Ptr(idx) => {
                    if current_lod == 0 {
                        return None;
                    }
                    let pool_lod = current_lod - 1;
                    child = pools
                        .get(usize::from(pool_lod))?
                        .get_live(idx)?
                        .child_at(path, pool_lod);
                    current_lod = pool_lod;
                    let cached = self.children.get_mut(usize::from(current_lod))?;
                    *cached = child;
                }
            }
        }
    }

    fn reusable_lod(&self, path: ZOrder20x3) -> u8 {
        if self.min_valid_lod >= DAG_DEPTH {
            return DAG_DEPTH;
        }

        let diff = self.path.value() ^ path.value();
        if diff == 0 {
            return self.min_valid_lod;
        }

        let highest_diff_bit = (u64::BITS - 1) - diff.leading_zeros();
        let reusable_lod = u8::try_from(highest_diff_bit / 6 + 1).unwrap_or(DAG_DEPTH);
        reusable_lod.clamp(self.min_valid_lod, DAG_DEPTH)
    }
}
