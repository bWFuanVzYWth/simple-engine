//! 相机叶片圆弧的极坐标边界与沿闭合圆周采样的 Perlin fBm。
use crate::pupil::Error;
use perlin_noise::PerlinNoise;
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

pub(crate) const ANGLES: usize = 16384;
/// 叶片两顶点之间的边形状。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    /// 两端点间的弦。
    Straight,
    /// 同弦、向外凸出的圆弧。
    #[default]
    Arc,
}
/// 正 N 叶光圈。外接圆直径为一个名义孔径，旋转角在像面坐标中顺时针为正。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Blades {
    /// 同形叶片数，3–32。
    pub count: u32,
    /// 直边或圆弧边。
    pub edge: Edge,
    /// 弓高相对弦中点到外接圆距离，0 为直边，1 形成圆孔。
    pub curvature: f64,
    /// 第一条边的法线方位，以度表示。
    pub rotation_deg: f64,
}
impl Default for Blades {
    fn default() -> Self {
        Self {
            count: 7,
            edge: Edge::Arc,
            curvature: 0.18,
            rotation_deg: 90.,
        }
    }
}
impl Blades {
    /// 所有参数及其组合均在上述物理取值范围内时返回真。
    pub fn is_valid(self) -> bool {
        (3..=32).contains(&self.count)
            && (0.0..=1.).contains(&self.curvature)
            && self.rotation_deg.is_finite()
    }
    pub(crate) fn radius(self, theta: f64) -> f64 {
        let half = PI / f64::from(self.count);
        let d = (theta - self.rotation_deg.rem_euclid(360.).to_radians() + half)
            .rem_euclid(2. * half)
            - half;
        let chord = 0.5 * half.cos();
        let h = if self.edge == Edge::Arc {
            self.curvature * (0.5 - chord)
        } else {
            0.
        };
        if h < 1e-10 {
            return chord / d.cos();
        }
        let b = 0.5 * half.sin();
        let radius = (b * b + h * h) / (2. * h);
        let tip = chord + h;
        let center = tip - radius;
        let product = tip * (2. * radius - tip);
        let projection = center * d.cos();
        product / ((projection * projection + product).sqrt() - projection)
    }
}
/// 以多倍频梯度柏林噪声扰动圆孔半径。噪声输入绕单位圆一周，首尾连续。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Iris {
    /// 扰动幅度，相对名义半径，范围 0–0.30。
    pub strength: f64,
    /// 单位圆半径对应的初层噪声格数，0.25–12。
    pub frequency: f64,
    /// 噪声倍频层数，1–8。
    pub octaves: u32,
    /// 每层频率乘数，1–3。
    pub lacunarity: f64,
    /// 每层振幅乘数，0–1。
    pub persistence: f64,
    /// 固定梯度场的种子。
    pub seed: u32,
}
impl Default for Iris {
    fn default() -> Self {
        Self {
            strength: 0.075,
            frequency: 1.75,
            octaves: 4,
            lacunarity: 2.,
            persistence: 0.5,
            seed: 371429,
        }
    }
}
impl Iris {
    /// 所有参数及其组合均在上述物理取值范围内时返回真。
    pub fn is_valid(self) -> bool {
        (0.0..=0.3).contains(&self.strength)
            && (0.25..=12.).contains(&self.frequency)
            && (1..=8).contains(&self.octaves)
            && (1.0..=3.).contains(&self.lacunarity)
            && (0.0..=1.).contains(&self.persistence)
    }
    pub(crate) fn radii(self) -> Result<Vec<f32>, Error> {
        if self.strength == 0. {
            return Ok(vec![0.5; ANGLES]);
        }
        // 每周期至少四个角向样本。采样数为二的幂，GPU 与 CPU 均可环绕索引。
        let highest = if self.persistence == 0. {
            self.frequency
        } else {
            self.frequency * self.lacunarity.powi(self.octaves as i32 - 1)
        };
        let angles = ANGLES.max(((4. * TAU * highest).ceil() as usize).next_power_of_two());
        let mut radii = vec![0.5; angles];
        let noise = PerlinNoise::new((u64::from(self.seed), 0x5c3a29b780d61ef4));
        let unit: Vec<_> = (0..angles)
            .map(|i| {
                let (s, c) = (TAU * i as f64 / angles as f64).sin_cos();
                [c as f32, s as f32]
            })
            .collect();
        let mut x = vec![0.; angles];
        let mut y = vec![0.; angles];
        let z = vec![0.375; angles];
        let mut values = vec![0.; angles];
        let mut sum = vec![0_f64; angles];
        let mut freq = self.frequency;
        let mut gain = 1.;
        let mut mass = 0.;
        for _ in 0..self.octaves {
            for ((x, y), &[c, s]) in x.iter_mut().zip(&mut y).zip(&unit) {
                *x = c * freq as f32;
                *y = s * freq as f32;
            }
            noise.sample_3d_batch(&x, &y, &z, &mut values)?;
            for (v, &n) in sum.iter_mut().zip(&values) {
                *v += gain * f64::from(n);
            }
            mass += gain;
            freq *= self.lacunarity;
            gain *= self.persistence;
            if gain == 0. {
                break;
            }
        }
        for (r, value) in radii.iter_mut().zip(sum) {
            *r = (0.5 * (1. + self.strength * value / mass)) as f32;
        }
        Ok(radii)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn square_edges_meet_at_the_circumcircle() {
        let blades = Blades {
            count: 4,
            edge: Edge::Straight,
            rotation_deg: 17.,
            ..Default::default()
        };
        let normal = 17_f64.to_radians();
        for k in 0..4 {
            let angle = normal + f64::from(k) * PI / 2.;
            assert!((blades.radius(angle) - 0.5 / 2_f64.sqrt()).abs() < 1e-12);
            assert!((blades.radius(angle + PI / 4.) - 0.5).abs() < 1e-12);
        }
    }
    #[test]
    fn curved_blades_follow_circular_arcs_through_the_vertices() {
        let blades = Blades {
            count: 5,
            edge: Edge::Arc,
            curvature: 0.47,
            rotation_deg: 0.,
        };
        let angle = PI / 5.;
        let (x, y) = (0.5 * angle.cos(), 0.5 * angle.sin());
        let mid = x + blades.curvature * (0.5 - x);
        let center = (mid * mid - x * x - y * y) / (2. * (mid - x));
        for t in [-angle, -angle / 2., 0., angle / 3., angle] {
            let r = blades.radius(t);
            assert!(((r * t.cos() - center).hypot(r * t.sin()) - (mid - center)).abs() < 1e-12);
        }
        let circular = Blades {
            curvature: 1.,
            ..blades
        };
        assert!((circular.radius(angle * 0.67) - 0.5).abs() < 1e-12);
    }
    #[test]
    fn closed_iris_uses_reproducible_octaves() -> Result<(), Error> {
        let iris = Iris::default();
        let radii = iris.radii()?;
        assert_eq!(radii, iris.radii()?);
        for alt in [
            Iris {
                seed: iris.seed + 1,
                ..iris
            },
            Iris { octaves: 1, ..iris },
        ] {
            let other = alt.radii()?;
            assert!(radii.iter().zip(other).any(|(a, b)| (a - b).abs() > 1e-3));
        }
        let max_step = radii
            .windows(2)
            .map(|w| (w[0] - w[1]).abs())
            .fold(0., f32::max);
        assert!((radii[0] - radii[radii.len() - 1]).abs() <= 1.01 * max_step && max_step < 0.001);
        Ok(())
    }
}
