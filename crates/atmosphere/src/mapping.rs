//! sky_tracer 9e9430d 的 fitted SkyView chart；CPU 只在 observer key 改变时求逆。

use crate::{Error, MAPPING_ROWS, SKY_SIZE};
use serde::Deserialize;
use std::f32::consts::PI;

#[derive(Deserialize)]
struct Shape {
    weights: [f32; 4],
    widths: [f32; 3],
}
#[derive(Deserialize)]
struct Charts {
    low: Shape,
    upper: Shape,
    space: Shape,
    ground: Shape,
    ground_near: Shape,
}
#[derive(Deserialize)]
pub(super) struct Calibration {
    sky: Charts,
}

impl Calibration {
    pub(super) fn new() -> Result<Self, Error> {
        Ok(serde_json::from_str(include_str!(
            "../data/mapping_fit.json"
        ))?)
    }
    pub(super) fn cache(&self, height: f32, sun: f32) -> ([[f32; 4]; MAPPING_ROWS], [u32; 4]) {
        let sky_rows = SKY_SIZE * 3 / 4;
        let lower = if height >= 120.0 {
            sky_rows
        } else {
            sky_rows / 2
        };
        let counts = [lower, sky_rows - lower, SKY_SIZE - sky_rows];
        let mut result = [[0.0; 4]; MAPPING_ROWS];
        let mut row = 12;
        for (chart, count) in counts.into_iter().enumerate() {
            let c = Chart::new(self, height, sun, chart);
            result[chart * 4..chart * 4 + 4].copy_from_slice(&c.coefficients());
            for y in 0..count {
                let u = y as f32 / (count - 1) as f32;
                let e = inverse(|e| c.forward(e), u, c.bounds[0], c.bounds[1]);
                result[row] = [e.sin(), e, 0.0, 0.0];
                row += 1;
            }
        }
        (result, [SKY_SIZE, SKY_SIZE, lower, sky_rows])
    }
}

fn inverse(f: impl Fn(f32) -> f32, u: f32, mut lo: f32, mut hi: f32) -> f32 {
    if u <= 0.0 {
        return lo;
    }
    if u >= 1.0 {
        return hi;
    }
    for _ in 0..28 {
        let mid = (lo + hi) * 0.5;
        if f(mid) < u {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

fn soft(x: f32, w: f32) -> f32 {
    x / (x.abs() + w)
}

struct Chart {
    bounds: [f32; 2],
    centers: [f32; 3],
    weights: [f32; 4],
    widths: [f32; 3],
}

impl Chart {
    fn new(calibration: &Calibration, h: f32, sun: f32, chart: usize) -> Self {
        let horizon = (-(h * (2.0 * 6360.0 + h)).max(0.0).sqrt() / (6360.0 + h)).asin();
        let space = h >= 120.0;
        let outer = if space {
            -(6480.0 / (6360.0 + h)).clamp(0.0, 1.0).acos()
        } else {
            horizon
        };
        let bounds = match chart {
            0 => [horizon, if space { outer } else { 0.0 }],
            1 => [0.0, PI * 0.5],
            _ => [-PI * 0.5, horizon],
        };
        let shape = if chart == 2 {
            &calibration.sky.ground
        } else if space {
            &calibration.sky.space
        } else if chart == 1 {
            &calibration.sky.upper
        } else {
            &calibration.sky.low
        };
        let mut weights = shape.weights;
        let mut widths = shape.widths;
        if chart == 2 && !space {
            let x = ((h - 1.0) / 3.0).clamp(0.0, 1.0);
            let t = 1.0 - x * x * (3.0 - 2.0 * x);
            for (j, value) in weights.iter_mut().enumerate() {
                *value = *value * (1.0 - t) + calibration.sky.ground_near.weights[j] * t;
            }
            for (j, value) in widths.iter_mut().enumerate() {
                *value = *value * (1.0 - t) + calibration.sky.ground_near.widths[j] * t;
            }
        }
        Self {
            bounds,
            centers: [horizon, sun, outer],
            weights,
            widths,
        }
    }
    fn forward(&self, e: f32) -> f32 {
        let [lo, hi] = self.bounds;
        let e = e.clamp(lo, hi);
        let mut y = self.weights[0] * (e - lo) / (hi - lo).max(1e-10);
        for j in 0..3 {
            if self.centers[j] < lo || self.centers[j] > hi {
                y += self.weights[j + 1] * (e - lo) / (hi - lo).max(1e-10)
                    * ((hi - self.centers[j]).abs() + self.widths[j])
                    / ((e - self.centers[j]).abs() + self.widths[j]);
            } else {
                let a = soft(lo - self.centers[j], self.widths[j]);
                let b = soft(hi - self.centers[j], self.widths[j]);
                y += self.weights[j + 1] * (soft(e - self.centers[j], self.widths[j]) - a)
                    / (b - a).max(1e-10);
            }
        }
        y.clamp(0.0, 1.0)
    }
    fn coefficients(&self) -> [[f32; 4]; 4] {
        let [lo, hi] = self.bounds;
        let mut c = [[0.0; 4]; 4];
        c[0] = [self.weights[0] / (hi - lo).max(1e-10), 0.0, lo, hi];
        for j in 0..3 {
            let a = soft(lo - self.centers[j], self.widths[j]);
            let b = soft(hi - self.centers[j], self.widths[j]);
            let outside = self.centers[j] < lo || self.centers[j] > hi;
            let scale = if outside {
                self.weights[j + 1] * ((hi - self.centers[j]).abs() + self.widths[j])
                    / (hi - lo).max(1e-10)
            } else {
                self.weights[j + 1] / (b - a).max(1e-10)
            };
            c[j + 1] = [self.centers[j], self.widths[j], scale, a];
        }
        c
    }
}
