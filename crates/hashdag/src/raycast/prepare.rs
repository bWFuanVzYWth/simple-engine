use crate::raycast::types::{InvalidInputReason, MissReason, Ray, RaycastResult};

use super::dda::fixed_from_nonnegative;
use super::math::{DVec3, argmax3};
use super::types::Axis;
use super::{DIR_ZERO_SQ, ROOT_EDGE};

struct AabbHit {
    t_enter: f64,
    face_axis: Option<Axis>,
}

#[derive(Clone, Copy)]
pub(super) struct PreparedRay {
    pub(super) origin: DVec3,
    pub(super) dir: DVec3,
    pub(super) t_enter: f64,
    pub(super) max_relative_t: u64,
    pub(super) face_axis: Option<Axis>,
}

pub(super) enum RayPrepError {
    Invalid(InvalidInputReason),
    Miss(MissReason),
}

impl RayPrepError {
    pub(super) const fn into_result(self) -> RaycastResult {
        match self {
            Self::Invalid(reason) => RaycastResult::InvalidInput { reason },
            Self::Miss(reason) => RaycastResult::Miss { reason },
        }
    }
}

pub(super) fn prepare_ray(ray: Ray) -> Result<PreparedRay, RayPrepError> {
    let origin = DVec3::from_array(ray.origin);
    if !origin.is_finite() {
        return Err(RayPrepError::Invalid(InvalidInputReason::OriginNonFinite));
    }

    let dir = DVec3::from_array(ray.dir);
    let dir_len_sq = dir.length_squared();
    if !dir_len_sq.is_finite() {
        return Err(RayPrepError::Invalid(InvalidInputReason::DirNonFinite));
    }
    if dir_len_sq < DIR_ZERO_SQ {
        return Err(RayPrepError::Invalid(InvalidInputReason::DirZero));
    }
    if !ray.max_t.is_finite() || ray.max_t < 0.0 {
        return Err(RayPrepError::Invalid(InvalidInputReason::ReachInvalid));
    }

    let dir = dir / dir_len_sq.sqrt();
    let Some(aabb_hit) = ray_root_aabb(origin, dir, ray.max_t) else {
        return Err(RayPrepError::Miss(MissReason::BoundaryExceeded));
    };
    if aabb_hit.t_enter > ray.max_t {
        return Err(RayPrepError::Miss(MissReason::ReachExceeded));
    }
    Ok(PreparedRay {
        origin,
        dir,
        t_enter: aabb_hit.t_enter,
        max_relative_t: fixed_from_nonnegative((ray.max_t - aabb_hit.t_enter).max(0.0)),
        face_axis: aabb_hit.face_axis,
    })
}

fn ray_root_aabb(origin: DVec3, dir: DVec3, max_t: f64) -> Option<AabbHit> {
    let x = axis_root_interval(origin.x, dir.x)?;
    let y = axis_root_interval(origin.y, dir.y)?;
    let z = axis_root_interval(origin.z, dir.z)?;

    let t_enter = x.near.max(y.near).max(z.near).max(0.0);
    let t_exit = x.far.min(y.far).min(z.far);
    if t_exit < 0.0 || t_enter > t_exit {
        return None;
    }
    if t_enter > max_t {
        return Some(AabbHit {
            t_enter,
            face_axis: None,
        });
    }

    let face_axis = if t_enter == 0.0 {
        None
    } else {
        Some(Axis::from_usize(argmax3(DVec3::new(
            x.near, y.near, z.near,
        ))))
    };
    Some(AabbHit { t_enter, face_axis })
}

struct AxisInterval {
    near: f64,
    far: f64,
}

fn axis_root_interval(origin: f64, dir: f64) -> Option<AxisInterval> {
    let min = 0.0;
    let max = f64::from(ROOT_EDGE);
    if dir > 0.0 {
        Some(AxisInterval {
            near: (min - origin) / dir,
            far: (max - origin) / dir,
        })
    } else if dir < 0.0 {
        Some(AxisInterval {
            near: (max - origin) / dir,
            far: (min - origin) / dir,
        })
    } else if origin >= min && origin < max {
        Some(AxisInterval {
            near: f64::NEG_INFINITY,
            far: f64::INFINITY,
        })
    } else {
        None
    }
}
