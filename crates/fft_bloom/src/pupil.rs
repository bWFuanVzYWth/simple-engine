//! 可组合的振幅光瞳。长度以名义孔径直径为单位，二维坐标的 y 轴朝下。
//! 以名义直径为边长的固定方形区域均匀放置灰尘和纤维，调整开口时颗粒保持就位。
//! 基础通光率上先乘灰尘的透射振幅，再乘纤维薄相位屏。每层的形状取并集，两层可重叠。
pub use crate::pupil_outline::{Blades, Edge, Iris};
use crate::{optical_rng::Pcg, pupil_outline::ANGLES};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// 光瞳构造或光瞳采样失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 数值、图像尺寸或振幅超出声明的范围，或遮罩完全遮光。
    #[error("光瞳参数、尺寸或透射振幅无效")]
    Domain,
    /// 引擎的梯度噪声采样失败。
    #[error(transparent)]
    Noise(#[from] perlin_noise::NoiseError),
    /// 有界采样未获得有效的随机尺寸。
    #[error("当前种子未能生成有效的颗粒尺寸")]
    Sampling,
}
/// 基础开口。组合模式取两张光阑的交集，关闭时使用名义直径的通光圆。
pub enum Opening {
    /// 无附加轮廓的名义圆孔。
    Clear,
    /// 带直边或圆弧边的 N 叶光圈。
    Blades(Blades),
    /// 有圆周 fBm 边界的瞳孔。
    Iris(Iris),
    /// 光圈与噪声瞳孔同时限制通光范围。
    Intersection(Blades, Iris),
    /// 线性实振幅，行优先，最长边对应名义直径。每轴 1–1024，样本须在 0–1 且至少一个透光。
    Mask {
        /// 宽、高。
        extent: [u32; 2],
        /// 逐行正方形像素的振幅。
        amplitude: Vec<f32>,
    },
}
/// 一层圆形灰尘，中心在名义孔径的外接正方形内均匀分布。半径从条件为正的高斯分布取样。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Dust {
    /// 颗粒数，0–1024。
    pub count: u32,
    /// 原高斯分布的平均半径，0.0002–0.15 个孔径。
    pub radius_mean: f64,
    /// 原高斯分布的半径标准差，0–0.15，零产生等半径。
    pub radius_sigma: f64,
    /// 圆内的振幅透射率，0–1。
    pub transmission: f64,
    /// 控制粒径与位置的种子。
    pub seed: u32,
}
impl Default for Dust {
    fn default() -> Self {
        Self {
            count: 24,
            radius_mean: 0.004,
            radius_sigma: 0.0015,
            transmission: 0.24,
            seed: 73421,
        }
    }
}
impl Dust {
    /// 所有参数及其组合均在上述物理取值范围内时返回真。
    pub fn is_valid(self) -> bool {
        self.count <= 1024
            && (0.0002..=0.15).contains(&self.radius_mean)
            && (0.0..=0.15).contains(&self.radius_sigma)
            && (0.0..=1.).contains(&self.transmission)
    }
}
/// 一层随机取向的细长矩形。全层宽度一致，长度取条件为至少一个宽度的高斯分布。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fibers {
    /// 纤维数，0–1024。
    pub count: u32,
    /// 所有矩形的全宽，0.0002–0.03 个孔径。
    pub width: f64,
    /// 原高斯分布的平均全长，至少为宽度，至多 1 个孔径。
    pub length_mean: f64,
    /// 原高斯分布的长度标准差，0–0.5。
    pub length_sigma: f64,
    /// 纤维区域的振幅透射率，0–1。
    pub transmission: f64,
    /// 薄相位屏相对干净区域的额外光程，0–300 nm。
    pub opd_nm: f64,
    /// 控制位置、取向与长度的种子。
    pub seed: u32,
}
impl Default for Fibers {
    fn default() -> Self {
        Self {
            count: 16,
            width: 0.00135,
            length_mean: 0.18,
            length_sigma: 0.032,
            transmission: 0.61,
            opd_nm: 39.,
            seed: 20261120,
        }
    }
}
impl Fibers {
    /// 所有参数及其组合均在上述物理取值范围内时返回真。
    pub fn is_valid(self) -> bool {
        self.count <= 1024
            && (0.0002..=0.03).contains(&self.width)
            && (self.width..=1.).contains(&self.length_mean)
            && (0.0..=0.5).contains(&self.length_sigma)
            && (0.0..=1.).contains(&self.transmission)
            && (0.0..=300.).contains(&self.opd_nm)
    }
}

const TILES: usize = 32;
const DOMAIN: f64 = 1.5;
// 分块多覆盖半个格，足以包含 GPU 栅格与常用缩略图的抗锯齿足迹。
const BIN_MARGIN: f64 = DOMAIN / (2 * TILES) as f64;
fn tile(v: f64) -> usize {
    ((v / DOMAIN + 0.5) * TILES as f64)
        .floor()
        .clamp(0., (TILES - 1) as f64) as usize
}
fn normal(rng: &mut Pcg, mean: f64, sigma: f64, minimum: f64) -> Result<f64, Error> {
    if sigma == 0. {
        return Ok(mean);
    }
    (0..128)
        .find_map(|_| {
            let z =
                (-2. * (1. - rng.uniform(0., 1.)).ln()).sqrt() * (TAU * rng.uniform(0., 1.)).cos();
            let v = mean + sigma * z;
            (v > minimum).then_some(v)
        })
        .ok_or(Error::Sampling)
}
/// 可重复使用的完整光瞳，持有轮廓、颗粒与加速查找的空间分块。
pub struct Pupil {
    pub(crate) mask: bool,
    pub(crate) extent: [u32; 2],
    pub(crate) outer_radius: f32,
    pub(crate) data: Vec<f32>,
    // 中心 x/y、半长（圆半径）、半宽、长轴余弦/正弦。
    pub(crate) particles: Vec<[f32; 6]>,
    pub(crate) bins: Vec<u32>,
    pub(crate) dust_count: u32,
    pub(crate) dust_transmission: f32,
    pub(crate) fiber_transmission: f64,
    pub(crate) opd_nm: f64,
}
impl Pupil {
    /// 按独立种子生成各层。None 关闭对应污染层。各层的粒子固定在光瞳平面，开口裁掉不透光处。
    /// 越域参数、无透光像素或随机采样失败返回错误。
    pub fn new(
        opening: Opening,
        dust: Option<Dust>,
        fibers: Option<Fibers>,
    ) -> Result<Self, Error> {
        let (mask, extent, data) = match opening {
            Opening::Mask { extent, amplitude } => {
                if extent.iter().any(|n| !(1..=1024).contains(n))
                    || amplitude.len() != (extent[0] * extent[1]) as usize
                    || amplitude.iter().any(|v| !(0.0..=1.).contains(v))
                    || !amplitude.iter().any(|v| *v > 0.)
                {
                    return Err(Error::Domain);
                }
                (true, extent, amplitude)
            }
            base => {
                let (blades, iris) = match base {
                    Opening::Clear => (None, None),
                    Opening::Blades(b) => (Some(b), None),
                    Opening::Iris(i) => (None, Some(i)),
                    Opening::Intersection(b, i) => (Some(b), Some(i)),
                    Opening::Mask { .. } => unreachable!("外层已处理图像光瞳"),
                };
                if blades.is_some_and(|b| !b.is_valid()) || iris.is_some_and(|i| !i.is_valid()) {
                    return Err(Error::Domain);
                }
                let mut radii = match iris {
                    Some(i) => i.radii()?,
                    None => vec![if blades.is_some() { f32::INFINITY } else { 0.5 }; ANGLES],
                };
                if let Some(b) = blades {
                    let step = TAU / radii.len() as f64;
                    for (i, r) in radii.iter_mut().enumerate() {
                        *r = r.min(b.radius(step * i as f64) as f32);
                    }
                }
                (false, [radii.len() as u32, 1], radii)
            }
        };
        if dust.is_some_and(|v| !v.is_valid()) || fibers.is_some_and(|v| !v.is_valid()) {
            return Err(Error::Domain);
        }
        let d = dust.filter(|d| d.count > 0 && d.transmission < 1.);
        let f = fibers.filter(|f| f.count > 0 && (f.transmission < 1. || f.opd_nm != 0.));
        let outer_radius = if mask {
            0.5
        } else {
            data.iter().copied().fold(0., f32::max)
        };
        let mut pupil = Self {
            outer_radius,
            mask,
            extent,
            data,
            particles: Vec::new(),
            bins: Vec::new(),
            dust_count: d.map_or(0, |p| p.count),
            dust_transmission: d.map_or(1., |p| p.transmission as f32),
            fiber_transmission: f.map_or(1., |p| p.transmission),
            opd_nm: f.map_or(0., |p| p.opd_nm),
        };
        if let Some(d) = d {
            let mut pos = Pcg::new(d.seed ^ 0x2654a701);
            let mut sizes = Pcg::new(d.seed ^ 0x8cf9380d);
            for _ in 0..d.count {
                let [x, y] = [pos.uniform(-0.5, 0.5), pos.uniform(-0.5, 0.5)];
                let r = normal(&mut sizes, d.radius_mean, d.radius_sigma, 0.)?;
                pupil.particles.push([x, y, r, r, 1., 0.].map(|v| v as f32));
            }
        }
        if let Some(f) = f {
            let mut pos = Pcg::new(f.seed);
            let mut sizes = Pcg::new(f.seed ^ 0x3ea2b953);
            for _ in 0..f.count {
                let [x, y] = [pos.uniform(-0.5, 0.5), pos.uniform(-0.5, 0.5)];
                let (s, c) = pos.uniform(0., PI).sin_cos();
                let len = normal(&mut sizes, f.length_mean, f.length_sigma, f.width)?;
                pupil
                    .particles
                    .push([x, y, len / 2., f.width / 2., c, s].map(|v| v as f32));
            }
        }
        let mut cells = vec![Vec::new(); TILES * TILES];
        for (id, &[x, y, l, w, c, s]) in pupil.particles.iter().enumerate() {
            let bx = f64::from(l * c.abs() + w * s.abs()) + BIN_MARGIN;
            let by = f64::from(l * s.abs() + w * c.abs()) + BIN_MARGIN;
            for row in tile(f64::from(y) - by)..=tile(f64::from(y) + by) {
                for col in tile(f64::from(x) - bx)..=tile(f64::from(x) + bx) {
                    cells[row * TILES + col].push(id as u32);
                }
            }
        }
        let mut offsets = Vec::with_capacity(TILES * TILES + 1);
        let mut ids = Vec::new();
        for cell in cells {
            offsets.push((TILES * TILES + 1 + ids.len()) as u32);
            ids.extend(cell);
        }
        offsets.push((TILES * TILES + 1 + ids.len()) as u32);
        offsets.append(&mut ids);
        pupil.bins = offsets;
        Ok(pupil)
    }
    /// 从单独的线性振幅图建立方形像素光瞳，输入约束同 [Opening::Mask]。
    pub fn raster(extent: [u32; 2], data: Vec<f32>) -> Result<Self, Error> {
        Self::new(
            Opening::Mask {
                extent,
                amplitude: data,
            },
            None,
            None,
        )
    }
    fn radius(&self, x: f64, y: f64) -> f64 {
        let n = self.data.len();
        let theta = (y.atan2(x) / TAU).rem_euclid(1.) * n as f64;
        let i = theta as usize & (n - 1);
        let a = f64::from(self.data[i]);
        a + (f64::from(self.data[(i + 1) & (n - 1)]) - a) * theta.fract()
    }
    fn basis(&self, x: f64, y: f64, width: f64) -> [f64; 2] {
        let mut a = if self.mask {
            let [w, h] = self.extent;
            let d = f64::from(w.max(h));
            let col = (x * d + f64::from(w) / 2.).floor() as i32;
            let row = (y * d + f64::from(h) / 2.).floor() as i32;
            if col >= 0 && row >= 0 && col < w as i32 && row < h as i32 {
                f64::from(self.data[(row as u32 * w + col as u32) as usize])
            } else {
                0.
            }
        } else {
            let distance = x.hypot(y);
            if distance >= f64::from(self.outer_radius) + 0.5 * width {
                return [0.; 2];
            }
            (0.5 + (self.radius(x, y) - distance) / width).clamp(0., 1.)
        };
        if a == 0. {
            return [0.; 2];
        }
        let index = tile(y) * TILES + tile(x);
        let mut dust = 0_f64;
        let mut fiber = 0_f64;
        let mut cover = |id: u32| {
            let [cx, cy, l, w, c, s] = self.particles[id as usize].map(f64::from);
            let (u, v) = (x - cx, y - cy);
            if id < self.dust_count {
                dust = dust.max(
                    (0.5 + (l - u.hypot(v)) / width)
                        .clamp(0., (PI * l * l / (width * width)).min(1.)),
                );
            } else {
                let edge = |half: f64, d: f64| {
                    (0.5 + (half - d.abs()) / width).clamp(0., (2. * half / width).min(1.))
                };
                fiber = fiber.max(edge(l, c * u + s * v) * edge(w, c * v - s * u));
            }
        };
        if width > BIN_MARGIN {
            (0..self.particles.len() as u32).for_each(cover);
        } else {
            for &id in &self.bins[self.bins[index] as usize..self.bins[index + 1] as usize] {
                cover(id);
            }
        }
        a *= 1. + dust * (f64::from(self.dust_transmission) - 1.);
        [a, a * fiber]
    }
    /// 在 1.5 个孔径宽度内显示合成复振幅的模。入射波长 360–830 nm，输出边长 1–1024，线性 8 位灰度。
    pub fn thumbnail(&self, side: u32, nm: f64) -> Result<Vec<u8>, Error> {
        if !(1..=1024).contains(&side) || !(360.0..=830.).contains(&nm) {
            return Err(Error::Domain);
        }
        let pixel = DOMAIN / f64::from(side);
        let (s, c) = (TAU * self.opd_nm / nm).sin_cos();
        let mut out = Vec::with_capacity((side * side) as usize);
        for y in 0..side {
            for x in 0..side {
                let mut total = 0.;
                for j in [0.25, 0.75] {
                    for i in [0.25, 0.75] {
                        let [a, b] = self.basis(
                            (f64::from(x) + i) * pixel - DOMAIN / 2.,
                            (f64::from(y) + j) * pixel - DOMAIN / 2.,
                            pixel / 2.,
                        );
                        total += (a + (self.fiber_transmission * c - 1.) * b)
                            .hypot(self.fiber_transmission * s * b);
                    }
                }
                out.push((total * 255. / 4.).round().clamp(0., 255.) as u8);
            }
        }
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_composition_intersects_both_outlines() -> Result<(), Error> {
        let blades = Blades {
            count: 3,
            edge: Edge::Straight,
            rotation_deg: 0.,
            ..Default::default()
        };
        let iris = Iris {
            strength: 0.25,
            ..Default::default()
        };
        let a = Pupil::new(Opening::Blades(blades), None, None)?;
        let b = Pupil::new(Opening::Iris(iris), None, None)?;
        let both = Pupil::new(Opening::Intersection(blades, iris), None, None)?;
        assert!(a.data.iter().zip(&b.data).any(|(a, b)| a < b));
        assert!(a.data.iter().zip(&b.data).any(|(a, b)| a > b));
        for ((a, b), r) in a.data.iter().zip(&b.data).zip(&both.data) {
            assert_eq!(*r, a.min(*b));
        }
        Ok(())
    }
    #[test]
    fn dust_disks_follow_the_radius_distribution() -> Result<(), Error> {
        let dust = Dust {
            count: 1024,
            radius_mean: 0.018,
            radius_sigma: 0.003,
            ..Default::default()
        };
        let p = Pupil::new(Opening::Clear, Some(dust), None)?;
        let count = f64::from(dust.count);
        let mut sum = 0.;
        let mut square = 0.;
        for &[x, y, r, w, _, _] in &p.particles {
            assert!(x.abs().max(y.abs()) <= 0.5 && r > 0.);
            assert_eq!(r, w);
            let d = f64::from(r) - dust.radius_mean;
            sum += d;
            square += d * d;
        }
        assert!((sum / count).abs() < 0.16 * dust.radius_sigma);
        assert!(((square / count).sqrt() / dust.radius_sigma - 1.).abs() < 0.09);
        Ok(())
    }
    #[test]
    fn fibers_keep_their_width_and_lengths_when_dust_changes() -> Result<(), Error> {
        let f = Fibers {
            count: 512,
            ..Default::default()
        };
        let plain = Pupil::new(Opening::Clear, None, Some(f))?;
        let dirty = Pupil::new(Opening::Clear, Some(Dust::default()), Some(f))?;
        assert_eq!(
            plain.particles,
            dirty.particles[dirty.dust_count as usize..]
        );
        let mut sum = 0.;
        let mut second = 0.;
        for &[_, _, l, w, c, s] in &plain.particles {
            assert_eq!(2. * w, f.width as f32);
            assert!(l >= w && (c.hypot(s) - 1.).abs() < 1e-6);
            let diff = 2. * f64::from(l) - f.length_mean;
            sum += diff;
            second += diff * diff;
        }
        assert!((sum / f64::from(f.count)).abs() < 0.17 * f.length_sigma);
        assert!(((second / f64::from(f.count)).sqrt() / f.length_sigma - 1.).abs() < 0.1);
        Ok(())
    }
    #[test]
    fn contaminants_multiply_the_imported_amplitude_where_they_overlap() -> Result<(), Error> {
        let dust = Dust {
            count: 64,
            radius_mean: 0.06,
            radius_sigma: 0.,
            transmission: 0.4,
            ..Default::default()
        };
        let fibers = Fibers {
            count: 16,
            width: 0.01,
            length_mean: 0.2,
            length_sigma: 0.,
            ..Default::default()
        };
        let p = Pupil::new(
            Opening::Mask {
                extent: [3, 1],
                amplitude: vec![0.75, 0.75, 0.75],
            },
            Some(dust),
            Some(fibers),
        )?;
        let disks = &p.particles[..p.dust_count as usize];
        let center = p.particles[p.dust_count as usize..]
            .iter()
            .find(|&&[x, y, ..]| {
                y.abs() < 0.16
                    && disks
                        .iter()
                        .any(|&[u, v, r, ..]| (x - u).hypot(y - v) < r - 0.001)
            })
            .expect("固定样例中有一根纤维落在灰尘与 PNG 内部");
        let [a, b] = p.basis(f64::from(center[0]), f64::from(center[1]), 0.0005);
        assert!((a - 0.75 * 0.4).abs() < 1e-6 && (a - b).abs() < 1e-6);
        assert_eq!(p.basis(0., 0.2, 0.001), [0.; 2]);
        Ok(())
    }
    #[test]
    fn changing_the_opening_keeps_contamination_fixed_in_the_pupil_plane() -> Result<(), Error> {
        let build = |opening| Pupil::new(opening, Some(Dust::default()), Some(Fibers::default()));
        let circular = build(Opening::Clear)?;
        let polygon = build(Opening::Blades(Blades {
            count: 3,
            ..Default::default()
        }))?;
        let imported = build(Opening::Mask {
            extent: [5, 1],
            amplitude: vec![0.1, 0., 0., 0., 0.],
        })?;
        assert_eq!(circular.particles, polygon.particles);
        assert_eq!(circular.particles, imported.particles);
        Ok(())
    }
}
