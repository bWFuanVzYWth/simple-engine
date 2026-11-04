use crate::raycast::types::Axis;

use super::math::{
    DVec3, IVec3, UVec3, add_axis_i32, axis_exit_crossings, axis_value_i32, axis_value_u64,
    cell_bounds, voxel_at_entry_point,
};
use super::{
    DDA_FIXED_ONE, DDA_FIXED_ONE_U128, DDA_FIXED_SHIFT, DDA_FIXED_SHIFT_I32, F64_EXP_BIAS,
    F64_MANTISSA_BITS,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DdaTime {
    pub(super) dist: u64,
    pub(super) speed: u64,
}

impl DdaTime {
    pub(super) const fn zero() -> Self {
        Self { dist: 0, speed: 1 }
    }

    pub(super) const fn infinity() -> Self {
        Self {
            dist: u64::MAX,
            speed: 0,
        }
    }

    pub(super) const fn is_finite(self) -> bool {
        self.speed != 0
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "命中距离只用于输出，DDA 判定保留整数精度"
    )]
    pub(super) fn to_f64(self) -> f64 {
        if self.is_finite() {
            self.dist as f64 / self.speed as f64
        } else {
            f64::INFINITY
        }
    }

    pub(super) const fn exceeds_fixed(self, limit: u64) -> bool {
        if !self.is_finite() || limit == u64::MAX {
            return false;
        }
        (self.dist as u128) * DDA_FIXED_ONE_U128 > (limit as u128) * (self.speed as u128)
    }

    pub(super) const fn le(self, rhs: Self) -> bool {
        if !self.is_finite() {
            return !rhs.is_finite();
        }
        if !rhs.is_finite() {
            return true;
        }
        (self.dist as u128) * (rhs.speed as u128) <= (rhs.dist as u128) * (self.speed as u128)
    }

    pub(super) const fn lt(self, rhs: Self) -> bool {
        if !self.is_finite() {
            return false;
        }
        if !rhs.is_finite() {
            return true;
        }
        (self.dist as u128) * (rhs.speed as u128) < (rhs.dist as u128) * (self.speed as u128)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DdaState {
    pub(super) voxel: IVec3,
    pub(super) step: IVec3,
    pub(super) first_dist: UVec3,
    pub(super) dir_abs: UVec3,
    pub(super) crossings: IVec3,
    pub(super) t_current: DdaTime,
    pub(super) face_axis: Option<Axis>,
}

impl DdaState {
    pub(super) fn init(origin: DVec3, dir: DVec3, start_t: f64, face_axis: Option<Axis>) -> Self {
        let point = origin + dir * start_t;
        let voxel = voxel_at_entry_point(point, dir);

        let origin_local = DVec3::new(
            point.x - f64::from(voxel.x),
            point.y - f64::from(voxel.y),
            point.z - f64::from(voxel.z),
        );
        let (step_x, first_x, dir_x) = axis_init_fixed(dir.x, origin_local.x);
        let (step_y, first_y, dir_y) = axis_init_fixed(dir.y, origin_local.y);
        let (step_z, first_z, dir_z) = axis_init_fixed(dir.z, origin_local.z);

        Self {
            voxel,
            step: IVec3::new(step_x, step_y, step_z),
            first_dist: UVec3::new(first_x, first_y, first_z),
            dir_abs: UVec3::new(dir_x, dir_y, dir_z),
            crossings: IVec3::new(0, 0, 0),
            t_current: DdaTime::zero(),
            face_axis,
        }
    }

    pub(super) fn advance(&mut self) {
        let axis = self.next_axis();
        self.t_current = self.axis_crossing_time(axis, 0);
        self.crossings = add_axis_i32(self.crossings, axis, 1);
        self.voxel = add_axis_i32(self.voxel, axis, axis_value_i32(self.step, axis));
        self.face_axis = Some(Axis::from_usize(axis));
    }

    pub(super) fn prev_cell(&self) -> IVec3 {
        self.face_axis.map_or(self.voxel, |axis| {
            let a = axis as usize;
            add_axis_i32(self.voxel, a, -axis_value_i32(self.step, a))
        })
    }

    /// 以整数时间跳出均匀 cell；不分配、不调度，仅更新当前状态，每轴为 O(1)。
    pub(super) fn skip_lod_cell(&mut self, lod: u8) {
        let cell = cell_bounds(self.voxel, lod);
        let exit_steps = IVec3::new(
            axis_exit_crossings(self.step.x, self.voxel.x, cell.min.x, cell.max.x),
            axis_exit_crossings(self.step.y, self.voxel.y, cell.min.y, cell.max.y),
            axis_exit_crossings(self.step.z, self.voxel.z, cell.min.z, cell.max.z),
        );
        let exit_t = (
            self.axis_crossing_time(0, exit_steps.x - 1),
            self.axis_crossing_time(1, exit_steps.y - 1),
            self.axis_crossing_time(2, exit_steps.z - 1),
        );
        let face_axis = argmin3_time(exit_t);
        let t_enter = axis_value_time(exit_t, face_axis);
        let crossings = IVec3::new(
            self.axis_skip_crossings(t_enter, exit_steps.x, 0, face_axis),
            self.axis_skip_crossings(t_enter, exit_steps.y, 1, face_axis),
            self.axis_skip_crossings(t_enter, exit_steps.z, 2, face_axis),
        );

        self.voxel = IVec3::new(
            self.voxel.x + self.step.x * crossings.x,
            self.voxel.y + self.step.y * crossings.y,
            self.voxel.z + self.step.z * crossings.z,
        );
        self.t_current = t_enter;
        self.crossings = IVec3::new(
            self.crossings.x + crossings.x,
            self.crossings.y + crossings.y,
            self.crossings.z + crossings.z,
        );
        self.face_axis = Some(Axis::from_usize(face_axis));
    }

    pub(super) fn t_current(self, t_enter: f64) -> f64 {
        t_enter + self.t_current.to_f64()
    }

    pub(super) const fn exceeds_reach(self, max_relative_t: u64) -> bool {
        self.t_current.exceeds_fixed(max_relative_t)
    }

    fn next_axis(&self) -> usize {
        argmin3_time((
            self.axis_crossing_time(0, 0),
            self.axis_crossing_time(1, 0),
            self.axis_crossing_time(2, 0),
        ))
    }

    fn axis_crossing_time(&self, axis: usize, extra_crossings: i32) -> DdaTime {
        if extra_crossings < 0 || axis_value_i32(self.step, axis) == 0 {
            return DdaTime::infinity();
        }
        let Some(crossing_count) =
            axis_value_i32(self.crossings, axis).checked_add(extra_crossings)
        else {
            return DdaTime::infinity();
        };
        let Ok(crossing_count) = u64::try_from(crossing_count) else {
            return DdaTime::infinity();
        };
        let dist = axis_value_u64(self.first_dist, axis) + (crossing_count << DDA_FIXED_SHIFT);
        let speed = axis_value_u64(self.dir_abs, axis);
        if speed == 0 {
            DdaTime::infinity()
        } else {
            DdaTime { dist, speed }
        }
    }

    fn axis_skip_crossings(
        &self,
        t_enter: DdaTime,
        exit_crossings: i32,
        axis: usize,
        face_axis: usize,
    ) -> i32 {
        if axis == face_axis {
            return exit_crossings;
        }

        self.axis_catch_up_crossings(t_enter, axis < face_axis, exit_crossings - 1, axis)
    }

    fn axis_catch_up_crossings(
        &self,
        t_enter: DdaTime,
        include_enter: bool,
        max_crossings: i32,
        axis: usize,
    ) -> i32 {
        if max_crossings <= 0 || axis_value_i32(self.step, axis) == 0 {
            return 0;
        }
        let first = self.axis_crossing_time(axis, 0);
        if (include_enter && !first.le(t_enter)) || (!include_enter && !first.lt(t_enter)) {
            return 0;
        }

        if !t_enter.is_finite() && include_enter {
            return max_crossings;
        }

        let finite_max = max_crossings.min(
            i32::MAX
                .saturating_sub(axis_value_i32(self.crossings, axis))
                .saturating_add(1),
        );
        if !t_enter.is_finite() || finite_max == 1 {
            return finite_max;
        }

        // 每次 crossing 增加 2^40 距离。严格比较在交叉乘积中减 1，保留轴序 tie。
        let budget = u128::from(t_enter.dist) * u128::from(first.speed)
            - u128::from(first.dist) * u128::from(t_enter.speed)
            - u128::from(!include_enter);
        // 方向已归一化，first.speed <= 2^40；右移后的预算不超过 u64::MAX。
        // floor(budget / (2^40 * speed)) 等于 floor((budget >> 40) / speed)。
        let whole_dist = u64::try_from(budget >> DDA_FIXED_SHIFT).unwrap_or(u64::MAX);
        let crossings = i32::try_from(whole_dist / t_enter.speed).unwrap_or(i32::MAX);
        crossings.saturating_add(1).min(finite_max)
    }
}

fn axis_init_fixed(dir_axis: f64, origin_local: f64) -> (i32, u64, u64) {
    let dir_abs = fixed_from_unit(dir_axis.abs());
    if dir_axis > 0.0 {
        (
            1,
            DDA_FIXED_ONE.saturating_sub(fixed_from_unit_interval(origin_local)),
            dir_abs,
        )
    } else if dir_axis < 0.0 {
        (-1, fixed_from_unit_interval(origin_local), dir_abs)
    } else {
        (0, 0, 0)
    }
}

fn fixed_from_unit(value: f64) -> u64 {
    let fixed = fixed_from_nonnegative(value);
    if value > 0.0 && fixed == 0 { 1 } else { fixed }
}

fn fixed_from_unit_interval(value: f64) -> u64 {
    fixed_from_nonnegative(value.clamp(0.0, 1.0)).min(DDA_FIXED_ONE)
}

pub(super) fn fixed_from_nonnegative(value: f64) -> u64 {
    if value <= 0.0 {
        return 0;
    }
    if !value.is_finite() {
        return u64::MAX;
    }

    let bits = value.to_bits();
    let exponent_bits = u16::try_from((bits >> F64_MANTISSA_BITS) & 0x7ff).unwrap_or(0);
    let fraction_mask = (1u64 << F64_MANTISSA_BITS) - 1;
    let fraction = bits & fraction_mask;
    let (mantissa, exponent) = if exponent_bits == 0 {
        (
            u128::from(fraction),
            1 - F64_EXP_BIAS - i32::try_from(F64_MANTISSA_BITS).unwrap_or(0),
        )
    } else {
        (
            u128::from((1u64 << F64_MANTISSA_BITS) | fraction),
            i32::from(exponent_bits) - F64_EXP_BIAS - i32::try_from(F64_MANTISSA_BITS).unwrap_or(0),
        )
    };

    let fixed = shifted_u128_rounded(mantissa, exponent + DDA_FIXED_SHIFT_I32);
    u64::try_from(fixed).unwrap_or(u64::MAX)
}

fn shifted_u128_rounded(value: u128, shift: i32) -> u128 {
    if value == 0 {
        return 0;
    }
    if shift >= 0 {
        let Ok(left_shift) = u32::try_from(shift) else {
            return u128::MAX;
        };
        if left_shift >= u128::BITS {
            return u128::MAX;
        }
        return value.checked_shl(left_shift).unwrap_or(u128::MAX);
    }

    let right_shift = shift.unsigned_abs();
    if right_shift == 0 {
        return value;
    }
    if right_shift > u128::BITS {
        return 0;
    }
    if right_shift == u128::BITS {
        return u128::from(value >= (1u128 << (u128::BITS - 1)));
    }

    let base = value >> right_shift;
    let round_bit = (value >> (right_shift - 1)) & 1;
    base.saturating_add(round_bit)
}

const fn axis_value_time(v: (DdaTime, DdaTime, DdaTime), axis: usize) -> DdaTime {
    match axis {
        0 => v.0,
        1 => v.1,
        _ => v.2,
    }
}

const fn argmin3_time(v: (DdaTime, DdaTime, DdaTime)) -> usize {
    if v.0.le(v.1) && v.0.le(v.2) {
        0
    } else if v.1.le(v.2) {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn repeated_skip(mut state: DdaState, lod: u8) -> DdaState {
        let cell = cell_bounds(state.voxel, lod);
        let mut remaining = 3 * (1u32 << lod);
        loop {
            assert!(remaining > 0, "非零 DDA 方向必须离开 cell");
            remaining -= 1;
            state.advance();
            if state.voxel.x < cell.min.x
                || state.voxel.x >= cell.max.x
                || state.voxel.y < cell.min.y
                || state.voxel.y >= cell.max.y
                || state.voxel.z < cell.min.z
                || state.voxel.z >= cell.max.z
            {
                return state;
            }
        }
    }

    fn linear_catch_up(state: DdaState, limit: DdaTime, include_enter: bool, max: i32) -> i32 {
        let mut count = 0;
        for crossing in 0..max {
            let time = state.axis_crossing_time(0, crossing);
            if (include_enter && time.le(limit)) || (!include_enter && time.lt(limit)) {
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    #[test]
    fn skip_matches_steps_at_ties_and_all_root_lods() {
        for lod in 0..=20 {
            for dir in [
                DVec3::new(1.0, 1.0, 1.0),
                DVec3::new(-1.0, -1.0, -1.0),
                DVec3::new(1.0, 0.0, 1.0),
                DVec3::new(1.0, 0.731, 0.193),
                DVec3::new(-1.0, -0.731, -0.193),
            ] {
                let edge = f64::from(1u32 << lod);
                let center = edge.mul_add(0.5, 0.25);
                let mut skipped =
                    DdaState::init(DVec3::new(center, center, center), dir, 0.0, None);
                let repeated = repeated_skip(skipped, lod);
                skipped.skip_lod_cell(lod);
                assert_eq!(skipped, repeated, "lod={lod}, dir={dir:?}");
            }
        }
    }

    #[test]
    fn skip_matches_steps_for_quantized_fine_directions() {
        for fine in [f64::from_bits(1), 1e-13, 1e-12, 1e-11] {
            for sign in [-1.0, 1.0] {
                let dir = DVec3::new(1.0, sign * fine, 0.731);
                let mut skipped = DdaState::init(DVec3::new(63.25, 31.5, 15.75), dir, 0.0, None);
                let repeated = repeated_skip(skipped, 8);
                skipped.skip_lod_cell(8);
                assert_eq!(skipped, repeated, "dir={dir:?}");
            }
        }
    }

    #[test]
    fn catch_up_stops_before_crossing_count_overflow() {
        let mut state = DdaState::init(
            DVec3::new(0.5, 0.5, 0.5),
            DVec3::new(1.0, 1.0, 1.0),
            0.0,
            None,
        );
        state.crossings = IVec3::new(i32::MAX - 3, 0, 0);
        let limit = DdaTime {
            dist: u64::MAX,
            speed: 1,
        };
        assert_eq!(state.axis_catch_up_crossings(limit, true, 16, 0), 4);
        assert_eq!(state.axis_catch_up_crossings(limit, false, 16, 0), 4);
        assert_eq!(
            state.axis_catch_up_crossings(DdaTime::infinity(), true, 16, 0),
            16
        );
        assert_eq!(
            state.axis_catch_up_crossings(DdaTime::infinity(), false, 16, 0),
            4
        );
    }

    #[test]
    fn catch_up_preserves_exact_crossing_boundaries() {
        let mut state = DdaState::init(
            DVec3::new(0.5, 0.5, 0.5),
            DVec3::new(1.0, 0.0, 0.0),
            0.0,
            None,
        );
        for first in [0, 1, DDA_FIXED_ONE - 1, DDA_FIXED_ONE] {
            state.first_dist.x = first;
            for speed in [1, DDA_FIXED_ONE - 1, DDA_FIXED_ONE] {
                state.dir_abs.x = speed;
                for prior in [0, 1, (1 << 20) - 1] {
                    state.crossings.x = prior;
                    for crossing in 0..16 {
                        let time = state.axis_crossing_time(0, crossing);
                        for dist in [time.dist.saturating_sub(1), time.dist, time.dist + 1] {
                            let limit = DdaTime { dist, speed };
                            for include_enter in [false, true] {
                                assert_eq!(
                                    state.axis_catch_up_crossings(limit, include_enter, 16, 0),
                                    linear_catch_up(state, limit, include_enter, 16),
                                    "first={first}, speed={speed}, prior={prior}, crossing={crossing}, dist={dist}, include={include_enter}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    proptest! {
        #[test]
        fn catch_up_matches_linear_oracle_for_integer_times(
            first_dist in 0u64..=DDA_FIXED_ONE,
            axis_speed in 1u64..=DDA_FIXED_ONE,
            limit_dist in 0u64..(1 << 61),
            limit_speed in 1u64..=DDA_FIXED_ONE,
            prior in 0i32..(1 << 20),
            max in 0i32..128,
            include_enter in any::<bool>(),
        ) {
            let mut state = DdaState::init(DVec3::new(0.5, 0.5, 0.5), DVec3::new(1.0, 0.0, 0.0), 0.0, None);
            state.first_dist.x = first_dist;
            state.dir_abs.x = axis_speed;
            state.crossings.x = prior;
            let limit = DdaTime { dist: limit_dist, speed: limit_speed };
            prop_assert_eq!(
                state.axis_catch_up_crossings(limit, include_enter, max, 0),
                linear_catch_up(state, limit, include_enter, max),
            );
        }

        #[test]
        fn skips_match_steps_after_prior_crossings(
            coords in prop::array::uniform3(64u16..1024),
            fraction in prop::array::uniform3(0u16..1024),
            direction in prop::array::uniform3(-1024i16..=1024),
            prior_steps in 0u8..32,
            lods in prop::array::uniform4(0u8..9),
        ) {
            prop_assume!(direction != [0; 3]);
            let [x, y, z] = coords.map(f64::from);
            let [fx, fy, fz] = fraction.map(|part| f64::from(part) / 1024.0);
            let [dx, dy, dz] = direction.map(|part| f64::from(part) / 1024.0);
            let mut skipped = DdaState::init(DVec3::new(x + fx, y + fy, z + fz), DVec3::new(dx, dy, dz), 0.0, None);
            for _ in 0..prior_steps {
                skipped.advance();
            }
            let mut repeated = skipped;
            for lod in lods {
                repeated = repeated_skip(repeated, lod);
                skipped.skip_lod_cell(lod);
                prop_assert_eq!(skipped, repeated);
            }
        }
    }
}
