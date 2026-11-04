//! 地球太阳的视赤纬与当地时角变换。
//! 原算法版权 © 2026 linlin，GPL-3.0-only。

/// 无法建立太阳轨迹或求取方向的输入错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SolarOrbitError {
    /// 纬度或太阳黄经超出支持的整数度范围。
    #[error("纬度须在 −90..=90°，太阳黄经须在 0..=359°")]
    Observer,
    /// 太阳时角不是有限值。
    #[error("太阳时角须为有限弧度")]
    HourAngle,
}

/// 按观测纬度和太阳黄经预计算的日轨迹。
///
/// 黄经 0°、90°、180° 和 270° 对应北半球的春分、夏至、秋分和冬至。
/// 调用者决定季节与时间的关系，并在观测点或黄经变化时重建轨迹。
#[derive(Clone, Copy, Debug)]
pub struct SolarOrbit {
    latitude_sin: f64,
    latitude_cos: f64,
    declination_sin: f64,
    declination_cos: f64,
}

impl Default for SolarOrbit {
    /// 北纬 30° 的春分轨迹。
    fn default() -> Self {
        Self::from_degrees(30, 0).expect("默认观测点在有效范围内")
    }
}

impl SolarOrbit {
    /// 按给定观测纬度、视太阳黄经和固定的地球轴倾角 23.43928° 准备日轨迹。
    /// # Errors
    /// 纬度超出 −90..=90° 或黄经超出 0..=359° 时返回 [`SolarOrbitError::Observer`]。
    pub fn from_degrees(latitude: i32, ecliptic_longitude: u32) -> Result<Self, SolarOrbitError> {
        if !(-90..=90).contains(&latitude) || ecliptic_longitude >= 360 {
            return Err(SolarOrbitError::Observer);
        }
        let latitude = f64::from(latitude).to_radians();
        let longitude = f64::from(ecliptic_longitude).to_radians();
        let declination = (23.43928_f64.to_radians().sin() * longitude.sin()).asin();
        let (latitude_sin, latitude_cos) = latitude.sin_cos();
        let (declination_sin, declination_cos) = declination.sin_cos();
        Ok(Self {
            latitude_sin,
            latitude_cos,
            declination_sin,
            declination_cos,
        })
    }

    /// 从当地视太阳时角得到朝向太阳的世界单位向量，东为 +X，上为 +Y，南为 +Z。
    /// 时角以弧度计，0 为正午，正值朝西。地平线以下仍返回实际方向。
    /// # Errors
    /// 时角为 NaN 或无穷时返回 [`SolarOrbitError::HourAngle`]。
    pub fn direction(self, hour_angle: f32) -> Result<[f32; 3], SolarOrbitError> {
        if !hour_angle.is_finite() {
            return Err(SolarOrbitError::HourAngle);
        }
        let (sine, cosine) = f64::from(hour_angle).sin_cos();
        Ok([
            (-self.declination_cos * sine) as f32,
            (self.latitude_sin * self.declination_sin
                + self.latitude_cos * self.declination_cos * cosine) as f32,
            (-self.latitude_cos * self.declination_sin
                + self.latitude_sin * self.declination_cos * cosine) as f32,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    #[test]
    fn equinox_in_legacy_coordinates() -> Result<(), SolarOrbitError> {
        let orbit = SolarOrbit::default();
        for (hour, expected) in [
            (0., [0., 0.8660254, 0.5]),
            (-FRAC_PI_2, [1., 0., 0.]),
            (FRAC_PI_2, [-1., 0., 0.]),
            (PI, [0., -0.8660254, -0.5]),
        ] {
            for (actual, expected) in orbit.direction(hour)?.into_iter().zip(expected) {
                assert!((actual - expected).abs() < 1e-6);
            }
        }
        Ok(())
    }

    #[test]
    fn polar_day_and_night_follow_declination() -> Result<(), SolarOrbitError> {
        for (season, sign) in [(90, 1.), (270, -1.)] {
            let orbit = SolarOrbit::from_degrees(90, season)?;
            for angle in [0., FRAC_PI_2, PI] {
                let [x, y, z] = orbit.direction(angle)?;
                assert!((y * sign - 0.39777697).abs() < 1e-6);
                assert!((x * x + y * y + z * z - 1.).abs() < 1e-6);
            }
        }
        Ok(())
    }

    #[test]
    fn rejects_outside_input_domain() {
        assert!(matches!(
            SolarOrbit::from_degrees(-91, 0),
            Err(SolarOrbitError::Observer)
        ));
        assert!(matches!(
            SolarOrbit::from_degrees(30, 360),
            Err(SolarOrbitError::Observer)
        ));
        assert_eq!(
            SolarOrbit::default().direction(f32::NAN),
            Err(SolarOrbitError::HourAngle)
        );
    }
}
