//! 将实际 Slang 程序与独立的光学解比较。需 Vulkan 计算设备。
#![cfg(all(feature = "calibration_gpu", not(target_arch = "wasm32")))]
use fft_bloom::{
    calibration::{Response, Source},
    diffraction::{Error, Quality, Request, Solver},
    pupil::{Blades, Dust, Fibers, Opening, Pupil},
};
use std::f64::consts::{PI, TAU};
type Result = std::result::Result<(), Box<dyn std::error::Error>>;
fn device() -> std::result::Result<Solver, Box<dyn std::error::Error>> {
    gpu_log::init()?;
    Ok(Solver::new(pollster::block_on(
        vulkan::interop::WgpuContext::compute(),
    )?)?)
}
fn detector(radius: u32) -> Request {
    Request {
        radius,
        support: 128,
        f_number: 4.,
        pitch_um: 2.,
        source: Source::Line(500.),
        response: Response::EngineRgb,
        quality: Quality::Calibrated,
    }
}
#[test]
#[ignore = "需要 Vulkan compute"]
fn square_pixel_flux_matches_separable_diffraction() -> Result {
    let frame = device()?.solve(&Pupil::raster([1, 1], vec![1.])?, detector(4), |_| true)?;
    // ∫ sinc²(u) du，分别在中心和完整视场 [-4.5,4.5] 内积分后取二维能量比。
    let integrate = |half: f64| {
        let n = 20000;
        (0..n)
            .map(|i| {
                let x = PI * half * (f64::from(i) + 0.5) / f64::from(n);
                (x.sin() / x).powi(2)
            })
            .sum::<f64>()
            * 2.
            * half
            / f64::from(n)
    };
    let expected = (integrate(0.5) / integrate(4.5)).powi(2);
    assert!((f64::from(frame.pixels[40][0]) - expected).abs() < 1e-6);
    gpu_log::check()?;
    Ok(())
}
#[test]
#[ignore = "需要 Vulkan compute"]
fn rectangular_mask_matches_three_continuous_subapertures() -> Result {
    let mut mask = vec![0.; 15];
    mask[0] = 0.5;
    mask[6] = 1.;
    mask[14] = 0.75;
    let frame = device()?.solve(&Pupil::raster([5, 3], mask)?, detector(7), |_| true)?;
    let sinc = |u: f64| {
        if u.abs() < 1e-10 {
            1.
        } else {
            (PI * u).sin() / (PI * u)
        }
    };
    let mut expected = Vec::new();
    let sub = 48;
    for y in -7..=7 {
        for x in -7..=7 {
            let mut pixel = 0.;
            for j in 0..sub {
                for i in 0..sub {
                    let u = f64::from(x) + (f64::from(i) + 0.5) / f64::from(sub) - 0.5;
                    let v = f64::from(y) + (f64::from(j) + 0.5) / f64::from(sub) - 0.5;
                    let (s1, c1) = (TAU * (u + v) / 5.).sin_cos();
                    let (s2, c2) = (TAU * (4. * u + 2. * v) / 5.).sin_cos();
                    pixel += ((0.5 + c1 + 0.75 * c2).powi(2) + (s1 + 0.75 * s2).powi(2))
                        * (sinc(u / 5.) * sinc(v / 5.)).powi(2);
                }
            }
            expected.push(pixel);
        }
    }
    let sum = expected.iter().sum::<f64>();
    let drift = frame
        .pixels
        .iter()
        .zip(expected)
        .map(|(p, ref_value)| (f64::from(p[0]) - ref_value / sum).abs())
        .sum::<f64>();
    assert!(drift < 0.002, "非对称光瞳的总绝对误差 {drift}");
    gpu_log::check()?;
    Ok(())
}
#[test]
#[ignore = "需要 Vulkan compute"]
fn cancelled_solver_accepts_a_new_pupil() -> Result {
    let mut solver = device()?;
    let pupil = Pupil::new(
        Opening::Blades(Blades::default()),
        Some(Dust::default()),
        Some(Fibers::default()),
    )?;
    let cancelled = solver.solve(&pupil, detector(16), |q| q == 0.);
    assert!(matches!(cancelled, Err(Error::Cancelled)));
    let frame = solver.solve(&Pupil::raster([1, 1], vec![1.])?, detector(16), |_| true)?;
    assert!((frame.pixels.iter().map(|p| f64::from(p[1])).sum::<f64>() - 1.).abs() < 1e-6);
    gpu_log::check()?;
    Ok(())
}
#[test]
#[ignore = "需要 Vulkan compute"]
fn four_straight_blades_match_an_inscribed_square() -> Result {
    use fft_bloom::pupil::Edge;
    let pupil = Pupil::new(
        Opening::Blades(Blades {
            count: 4,
            edge: Edge::Straight,
            rotation_deg: 0.,
            ..Default::default()
        }),
        None,
        None,
    )?;
    let frame = device()?.solve(&pupil, detector(6), |_| true)?;
    let width = 1. / 2_f64.sqrt();
    let axis: Vec<_> = (-6..=6)
        .map(|x| {
            (0..128)
                .map(|i| {
                    let t = PI * width * (f64::from(x) + (f64::from(i) + 0.5) / 128. - 0.5);
                    (t.sin() / t).powi(2)
                })
                .sum::<f64>()
        })
        .collect();
    let mass = axis.iter().sum::<f64>().powi(2);
    let drift = frame
        .pixels
        .iter()
        .enumerate()
        .map(|(i, p)| (f64::from(p[0]) - axis[i % 13] * axis[i / 13] / mass).abs())
        .sum::<f64>();
    assert!(drift < 0.004, "四叶直边与连续方孔的像面误差 {drift}");
    gpu_log::check()?;
    Ok(())
}
#[test]
#[ignore = "需要 Vulkan compute"]
fn png_with_a_phase_fiber_matches_two_continuous_rectangles() -> Result {
    // 在不依赖随机序列的情况下，从不透光缩略图量出这根纤维的中心和方向。
    let fiber = Fibers {
        count: 1,
        width: 0.03,
        length_mean: 0.23,
        length_sigma: 0.,
        transmission: 0.85,
        opd_nm: 125.,
        seed: 20261120,
    };
    let make = |fiber| {
        Pupil::new(
            Opening::Mask {
                extent: [1, 1],
                amplitude: vec![1.],
            },
            None,
            fiber,
        )
    };
    let n = 512;
    let clean = make(None)?.thumbnail(n, 500.)?;
    let blocked = make(Some(Fibers {
        transmission: 0.,
        opd_nm: 0.,
        ..fiber
    }))?
    .thumbnail(n, 500.)?;
    let mut moment = [0.; 6];
    for (i, (&a, b)) in clean.iter().zip(blocked).enumerate() {
        let t = f64::from(a) - f64::from(b);
        let coord = |k: usize| ((k as f64 + 0.5) / f64::from(n) - 0.5) * 1.5;
        let x = coord(i % n as usize);
        let y = coord(i / n as usize);
        for (m, v) in moment.iter_mut().zip([1., x, y, x * x, x * y, y * y]) {
            *m += t * v;
        }
    }
    let [_, cx, cy, xx, xy, yy] = moment.map(|v| v / moment[0]);
    let angle = 0.5 * (2. * (xy - cx * cy)).atan2(xx - cx * cx - yy + cy * cy);
    let (s, c) = angle.sin_cos();
    let frame = device()?.solve(&make(Some(fiber))?, detector(6), |_| true)?;
    let sinc = |x: f64| {
        let t = PI * x;
        if t.abs() < 1e-10 { 1. } else { t.sin() / t }
    };
    let mut reference = Vec::new();
    for y in -6..=6 {
        for x in -6..=6 {
            let mut intensity = 0.;
            for j in 0..32 {
                for i in 0..32 {
                    let u = f64::from(x) + (f64::from(i) + 0.5) / 32. - 0.5;
                    let v = f64::from(y) + (f64::from(j) + 0.5) / 32. - 0.5;
                    let a = sinc(u) * sinc(v);
                    let b = fiber.length_mean
                        * fiber.width
                        * sinc(fiber.length_mean * (c * u + s * v))
                        * sinc(fiber.width * (c * v - s * u));
                    let (im, re) = (-TAU * (cx * u + cy * v)).sin_cos();
                    // 125 nm 光程在 500 nm 下产生正 π/2 的相位。
                    let real = a + b * (-re - fiber.transmission * im);
                    let imaginary = b * (-im + fiber.transmission * re);
                    intensity += real * real + imaginary * imaginary;
                }
            }
            reference.push(intensity);
        }
    }
    let mass = reference.iter().sum::<f64>();
    let measured = frame
        .pixels
        .iter()
        .zip(frame.pixels.iter().rev())
        .map(|(a, b)| f64::from(a[0] - b[0]));
    let expected = reference
        .iter()
        .zip(reference.iter().rev())
        .map(|(a, b)| (a - b) / mass);
    let mut contrast = 0.;
    let mut error = 0.;
    for (a, b) in measured.zip(expected) {
        contrast += b.abs();
        error += (a - b).abs();
    }
    assert!(
        contrast > 0.005 && error < 0.025 * contrast,
        "有符号相位对比 {contrast}，绝对误差 {error}"
    );
    gpu_log::check()?;
    Ok(())
}
