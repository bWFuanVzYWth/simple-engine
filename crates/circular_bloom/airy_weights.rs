//! 构建时标定快速圆形泛光。圆孔夫琅禾费强度为 [2 J₁(x)/x]²，x=πrp/(λN)。
//! 归一化后对方形像素面积和三个传感器波段积分。参见
//! [Wyant 与 Creath，圆孔的点扩散函数](https://wp.optics.arizona.edu/jcwyant/wp-content/uploads/sites/13/2016/08/03-BasicAberrations_and_Optical_Testing.pdf)。

use std::f64::consts::PI;

use color::spectrum::ENGINE_RGB_BANDS_NM;
const TAPS: [(usize, usize); 6] = [(0, 0), (1, 0), (1, 1), (2, 0), (2, 1), (2, 2)];
const MULTIPLICITY: [f64; 6] = [1., 4., 4., 4., 8., 4.];
pub(super) const LEVELS: usize = 9;
pub(super) const BINOMIAL: [f64; 6] =
    [1. / 32., 5. / 32., 10. / 32., 10. / 32., 5. / 32., 1. / 32.];

fn airy_pixel(x: usize, y: usize, [low, high]: [u16; 2]) -> f64 {
    const SPECTRAL: usize = 16;
    const GRID: usize = 48;
    let mut integral = 0.;
    for k in 0..SPECTRAL {
        let nm = f64::from(low) + f64::from(high - low) * (k as f64 + 0.5) / SPECTRAL as f64;
        let scale = (nm * 1e-3) * 4.0 / 5.0; // f/4，5 µm 像素，λN/p 以像素为单位。
        let mut area = 0.;
        for j in 0..GRID {
            for i in 0..GRID {
                let dx = x as f64 + (i as f64 + 0.5) / GRID as f64 - 0.5;
                let dy = y as f64 + (j as f64 + 0.5) / GRID as f64 - 0.5;
                let t = PI * dx.hypot(dy) / scale;
                area += (2. * libm::j1(t) / t).powi(2);
            }
        }
        integral += area * PI / (4. * scale * scale);
    }
    integral / (GRID * GRID * SPECTRAL) as f64
}

pub(super) struct Weights {
    pub(super) core: [[f32; 3]; 6],
    pub(super) wings: [[f32; 3]; LEVELS],
}

pub(super) fn calculate() -> Weights {
    let mut shape = [0.; LEVELS];
    let mut overlap = [[0.; 6]; LEVELS];
    let mut analysis = vec![1.];
    for j in 0..LEVELS {
        let stride = 1 << j;
        let mut next = vec![0.; analysis.len() + 5 * stride];
        for (k, tap) in BINOMIAL.into_iter().enumerate() {
            for (i, v) in analysis.iter().enumerate() {
                next[k * stride + i] += v * tap;
            }
        }
        // 对所有像素相位取平均，分析再重建的点响应等于分析核的自相关。
        let axis: [f64; 3] =
            std::array::from_fn(|d| next[d..].iter().zip(&next).map(|(x, y)| x * y).sum());
        overlap[j] = TAPS.map(|(x, y)| axis[x] * axis[y]);
        // 宽带 Airy 远翼的强度包络按 r⁻³ 衰减，每倍频程的通量按 1/σ 衰减。
        let variance = (2. * 1.25 / 3.) * ((4usize.pow((j + 1) as u32) - 1) as f64);
        shape[j] = 1. / variance.sqrt();
        analysis = next;
    }
    let inside = |weights: [f64; 6]| {
        weights
            .into_iter()
            .zip(MULTIPLICITY)
            .map(|(w, n)| w * n)
            .sum::<f64>()
    };
    let wing_outside: f64 = shape
        .iter()
        .zip(overlap)
        .map(|(w, k)| w * (1. - inside(k)))
        .sum();
    let mut core = [[0f32; 3]; 6];
    let mut wing = [[0f32; 3]; LEVELS];
    for (c, band) in ENGINE_RGB_BANDS_NM.into_iter().enumerate() {
        let ideal = TAPS.map(|(x, y)| airy_pixel(x, y, band));
        let gain = (1. - inside(ideal)) / wing_outside;
        for j in 0..LEVELS {
            wing[j][c] = (shape[j] * gain) as f32;
        }
        for i in 0..6 {
            let tail: f64 = (0..LEVELS).map(|j| gain * shape[j] * overlap[j][i]).sum();
            core[i][c] = (ideal[i] - tail) as f32;
            assert!(core[i][c] >= 0., "圆孔近场残差必须非负");
        }
    }
    Weights { core, wings: wing }
}
