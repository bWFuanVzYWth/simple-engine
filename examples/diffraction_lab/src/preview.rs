//! 带足额边距的太阳盘卷积和显示变换。点源光量与均匀面亮度太阳盘共用线性像面。
use crate::recipe::{Channels, Parameters, View};
use simple_engine::{
    color::{LinearRec2020Rgba, LinearSrgbRgba, spectrum::xyz_to_linear_srgb},
    fft_bloom::diffraction::Frame,
};
use std::sync::OnceLock;

pub(crate) const THUMB: u32 = 192;
pub(crate) struct Preview {
    pub point: Vec<[f32; 3]>,
    pub sun: Vec<[f32; 3]>,
    pub side: u32,
    pub pupil: Vec<u8>,
    pub xyz: [f64; 3],
    pub flux: [f64; 3],
    pub observer: bool,
    pub source: simple_engine::fft_bloom::calibration::Source,
}
impl Preview {
    pub fn new(
        f: &Frame,
        p: &Parameters,
        pupil: Vec<u8>,
        valid: impl Fn() -> bool,
    ) -> Option<Self> {
        let n = p.preview_side as usize;
        let raw = f.side as usize;
        let offset = (raw as i32 - n as i32) / 2;
        let mut point = vec![[0.; 3]; n * n];
        let mut flux = [0_f64; 3];
        let mut prefix = vec![[0_f64; 3]; raw * (raw + 1)];
        for (y, row) in f.pixels.chunks_exact(raw).enumerate() {
            let mut sum = [0_f64; 3];
            for (x, &v) in row.iter().enumerate() {
                for c in 0..3 {
                    sum[c] += f64::from(v[c]);
                }
                prefix[y * (raw + 1) + x + 1] = sum;
                let dx = x as i32 - offset;
                let dy = y as i32 - offset;
                if (0..n as i32).contains(&dx) && (0..n as i32).contains(&dy) {
                    point[dy as usize * n + dx as usize] = v;
                    for c in 0..3 {
                        flux[c] += f64::from(v[c]);
                    }
                }
            }
        }
        // 每行的覆盖率是若干等权区间的和。以 f64 行前缀做差，保留强中心附近的暗翼。
        let r = p.sun_radius as i32;
        let mut spans = Vec::new();
        for dy in -r..=r {
            let mut previous = 0;
            for w in (0..=r).rev() {
                let mut covered = 0;
                for a in 0..8 {
                    for b in 0..8 {
                        let x = 16 * w + 2 * a - 7;
                        let y = 16 * dy + 2 * b - 7;
                        if x * x + y * y <= 256 * r * r {
                            covered += 1;
                        }
                    }
                }
                if covered > previous {
                    spans.push((dy, w, f64::from(covered - previous) / 64.));
                    previous = covered;
                }
            }
        }
        let mut sun = Vec::with_capacity(n * n);
        let mut row = vec![[0_f64; 3]; n];
        for y in 0..n {
            if y % 16 == 0 && !valid() {
                return None;
            }
            row.fill([0.; 3]);
            for &(dy, w, weight) in &spans {
                let sy = y as i32 + offset + dy;
                if !(0..raw as i32).contains(&sy) {
                    continue;
                }
                let pre = &prefix[sy as usize * (raw + 1)..(sy as usize + 1) * (raw + 1)];
                let lo = (w - offset).max(0).min(n as i32) as usize;
                let hi = (raw as i32 - w - offset).max(0).min(n as i32) as usize;
                for (x, v) in row.iter_mut().enumerate() {
                    let center = x as i32 + offset;
                    let (l, h) = if (lo..hi).contains(&x) {
                        ((center - w) as usize, (center + w + 1) as usize)
                    } else {
                        (
                            (center - w).clamp(0, raw as i32) as usize,
                            (center + w + 1).clamp(0, raw as i32) as usize,
                        )
                    };
                    for c in 0..3 {
                        v[c] += (pre[h][c] - pre[l][c]) * weight;
                    }
                }
            }
            sun.extend(row.iter().map(|v| v.map(|n| n as f32)));
        }
        Some(Self {
            point,
            sun,
            side: p.preview_side,
            pupil,
            flux,
            xyz: f.source_xyz,
            observer: p.response == Channels::Observer,
            source: p.light(),
        })
    }
    pub fn rgba(&self, view: View, ev: f32) -> Vec<u8> {
        static DISPLAY: OnceLock<[u8; 16385]> = OnceLock::new();
        let lut = DISPLAY.get_or_init(|| {
            std::array::from_fn(|i| {
                let t = i as f32 / 16384.;
                LinearSrgbRgba::new(t, t, t, 1.).to_srgb8().to_array()[0]
            })
        });
        let basis = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        let matrix = if self.observer {
            basis.map(|v| xyz_to_linear_srgb(v).map(|x| x as f32))
        } else {
            basis.map(|[r, g, b]| {
                let v = LinearRec2020Rgba::new(r as f32, g as f32, b as f32, 1.)
                    .to_linear_srgb()
                    .to_array();
                [v[0], v[1], v[2]]
            })
        };
        let sun = view == View::Sun;
        let gain = ev.exp2() / if sun { 64. } else { 1. };
        let sky = if sun { [0.17, 0.28, 0.45] } else { [0.; 3] };
        let plane = if sun { &self.sun } else { &self.point };
        let mut rgba = Vec::with_capacity(plane.len() * 4);
        for &[x, y, z] in plane {
            let rgb: [f32; 3] = std::array::from_fn(|i| {
                gain * (matrix[0][i] * x + matrix[1][i] * y + matrix[2][i] * z) + sky[i]
            });
            let scale = 16384. / (1. + rgb[0].max(rgb[1]).max(rgb[2]).max(0.));
            rgba.extend(rgb.map(|k| lut[(k * scale).round().clamp(0., 16384.) as usize]));
            rgba.push(255);
        }
        rgba
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::Result;
    use simple_engine::fft_bloom::calibration::{Plan, Response};
    #[test]
    fn sun_convolution_preserves_covered_disk_area() {
        let p = Parameters {
            preview_side: 15,
            sun_radius: 3,
            ..Default::default()
        };
        let mut pixels = vec![[0.; 3]; 21 * 21];
        pixels[10 * 21 + 10] = [1.; 3];
        let f = Frame {
            pixels,
            side: 21,
            source_xyz: [1.; 3],
        };
        let solar = Preview::new(&f, &p, Vec::new(), || true).unwrap().sun;
        for (i, pixel) in solar.iter().enumerate() {
            let x = (i % 15) as i32 - 7;
            let y = (i / 15) as i32 - 7;
            let covered = (0..64)
                .filter(|j| {
                    let u = (x * 16 + 2 * (j % 8) - 7).pow(2);
                    let v = (y * 16 + 2 * (j / 8) - 7).pow(2);
                    u + v <= 48 * 48
                })
                .count();
            assert_eq!(*pixel, [covered as f32 / 64.; 3]);
        }
    }
    #[test]
    fn d65_observer_preview_is_neutral() -> Result<()> {
        let xyz = Plan::new(
            simple_engine::fft_bloom::calibration::Source::D65,
            Response::Observer,
            36,
        )?
        .source_xyz;
        let f = Frame {
            pixels: vec![xyz.map(|n| n as f32)],
            side: 1,
            source_xyz: xyz,
        };
        let p = Parameters {
            preview_side: 1,
            response: Channels::Observer,
            ..Default::default()
        };
        let v = Preview::new(&f, &p, Vec::new(), || true)
            .unwrap()
            .rgba(View::Point, 0.);
        assert!(v[0].abs_diff(v[1]) <= 1 && v[2].abs_diff(v[1]) <= 1);
        Ok(())
    }
}
