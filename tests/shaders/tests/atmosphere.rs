//! 球形边界、常系数传输与分段阴影的独立f64数学约束；完整求解oracle在atmosphere crate。

use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const SHADER: Shader<'static> = Shader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/atmosphere_main.spv")),
    entry: "atmosphere_main",
    workgroup: [64, 1, 1],
};

fn run(cases: &[[[u32; 4]; 2]]) -> Result<Vec<[u32; 4]>> {
    run_shader(SHADER, cases)
}

#[test]
#[ignore = "真实 GPU；太阳穿过相机侧面时，每条极线的采样射线都在同一太阳平面"]
fn side_on_sun_preserves_epipolar_planes() -> Result {
    let shader = Shader {
        spirv: include_bytes!(concat!(env!("OUT_DIR"), "/epipolar_plane_main.spv")),
        entry: "epipolar_plane_main",
        workgroup: [64, 1, 1],
    };
    let mut cases = Vec::new();
    for z in [-0.02_f32, -0.0003, -0.00001, 0., 0.00001, 0.0003, 0.02] {
        let sun = [0.6, 0.8, z, 0.].map(f32::to_bits);
        for slice in [1, 41, 127, 219, 254] {
            for lane in [0, 64, 126] {
                cases.push([sun, [lane, slice, 0, 0]]);
            }
        }
    }
    for (case, row) in cases.iter().zip(run_shader(shader, &cases)?) {
        let [x, y, z, error] = row.map(f32::from_bits);
        assert!(z > 0. && (x * x + y * y + z * z - 1.).abs() < 2e-5);
        assert!(
            error.abs() < 2e-5,
            "太阳 {:?}，极线和径向 {:?}，平面偏差 {error}",
            case[0].map(f32::from_bits),
            case[1]
        );
    }
    Ok(())
}

fn run_shader(shader: Shader<'static>, cases: &[[[u32; 4]; 2]]) -> Result<Vec<[u32; 4]>> {
    let bytes: Vec<_> = cases
        .iter()
        .flatten()
        .flatten()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let groups = [(cases.len() as u32).div_ceil(shader.workgroup[0]), 1, 1];
    let result = gpu.compute(shader, &bytes, cases.len() * 16, groups);
    let cleanup = gpu.finish();
    let bytes = result?;
    cleanup?;
    Ok(bytes
        .as_chunks::<16>()
        .0
        .iter()
        .map(|row| {
            std::array::from_fn(|i| u32::from_le_bytes(row[i * 4..i * 4 + 4].try_into().unwrap()))
        })
        .collect())
}

#[test]
#[ignore = "需要真实 GPU"]
fn spherical_boundary_matches_independent_sphere_intersection() -> Result {
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for h in [0.001_f32, 0.2, 35.0, 119.0] {
        let r = 6360.0 + f64::from(h);
        let horizon = -(1.0 - (6360.0 / r).powi(2)).sqrt();
        for mu in [
            -1.0_f32,
            -0.1,
            (horizon - 0.001) as f32,
            (horizon + 0.001) as f32,
            0.0,
            0.8,
            1.0,
        ] {
            let ground = f64::from(mu) < horizon;
            let target = if ground { 6360.0 } else { 6480.0 };
            let b = r * f64::from(mu);
            let discriminant = b * b - r * r + target * target;
            let d = if ground {
                -b - discriminant.sqrt()
            } else {
                -b + discriminant.sqrt()
            };
            cases.push([[h.to_bits(), mu.to_bits(), 0, 0], [0, 1, 0, 0]]);
            expected.push((d, if ground { 0.0 } else { 120.0 }, ground));
        }
    }
    let rows = run(&cases)?;
    for (i, (row, (d, h, ground))) in rows.iter().zip(expected).enumerate() {
        assert_eq!(row[3], i as u32 + 1);
        let actual = f64::from(f32::from_bits(row[0]));
        assert!(
            (actual - d).abs() <= 0.003 + 2e-5 * d.abs(),
            "d={actual}/{d}"
        );
        assert!((f64::from(f32::from_bits(row[1])) - h).abs() < 0.003);
        assert_eq!(f32::from_bits(row[2]), f32::from(u8::from(ground)));
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 GPU"]
fn homogeneous_segment_matches_beer_integral_and_vacuum_limit() -> Result {
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for dt in [0.0_f32, 0.001, 1.0, 120.0] {
        for sigma in [
            [0.0_f32, 1e-10, 1e-6, 1e-3],
            [0.000_999_9, 0.001, 0.001_000_1, 0.01],
            [0.01, 0.1, 1.0, 20.0],
        ] {
            cases.push([sigma.map(f32::to_bits), [dt.to_bits(), 0, 0, 0]]);
            expected.push(sigma.map(|s| {
                if s == 0.0 {
                    f64::from(dt)
                } else {
                    -(-f64::from(s) * f64::from(dt)).exp_m1() / f64::from(s)
                }
            }));
        }
    }
    for (row, expected) in run(&cases)?.iter().zip(expected) {
        for (&bits, reference) in row.iter().zip(expected) {
            let actual = f64::from(f32::from_bits(bits));
            assert!(actual.is_finite() && actual >= 0.0);
            assert!(
                (actual - reference).abs() <= 2e-6 + 6e-5 * reference,
                "{actual}/{reference}"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 GPU；透射复用保持Beer定律和真空极限"]
fn homogeneous_transmittance_matches_beer_law() -> Result {
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for dt in [0.0_f32, 0.001, 1.0, 120.0] {
        for sigma in [[0.0_f32, 1e-10, 0.001, 0.1], [0.01, 0.1, 1.0, 20.0]] {
            cases.push([sigma.map(f32::to_bits), [dt.to_bits(), 2, 0, 0]]);
            expected.push(sigma.map(|s| (-f64::from(s) * f64::from(dt)).exp()));
        }
    }
    for (row, expected) in run(&cases)?.iter().zip(expected) {
        for (&bits, reference) in row.iter().zip(expected) {
            let actual = f64::from(f32::from_bits(bits));
            assert!((0.0..=1.0).contains(&actual));
            assert!((actual - reference).abs() <= 2e-6, "{actual}/{reference}");
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 GPU；分段阻挡深度匹配独立射线半空间积分"]
fn shadow_profiles_match_visible_ray_lengths() -> Result {
    const PROFILE: &[(f64, f64, f64)] = &[
        (0.0, 63.5, -0.25),
        (63.5, 95.25, 0.5),
        (95.25, 127.0, 1.25),
        (127.0, 254.0, 0.35),
        (254.0, 508.0, 0.35),
        (508.0, 1016.0, 0.35),
        (1016.0, 2032.0, 0.35),
    ];
    let shader = Shader {
        spirv: include_bytes!(concat!(env!("OUT_DIR"), "/shadow_interval_main.spv")),
        entry: "shadow_interval_main",
        workgroup: [1, 1, 1],
    };
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for direction in [
        [1.0_f32, 0.0, 0.0],
        [1.0, 0.0, 0.005],
        [1.0, 0.0, -0.005],
        [0.25, 0.0, 0.968_245_86],
        [0.0, 0.0, 1.0],
    ] {
        let length = direction.iter().map(|x| x * x).sum::<f32>().sqrt();
        let direction = direction.map(|x| x / length);
        for depth in [-100.0_f32, -0.2, 50.0] {
            for (start, end) in [
                (0.0_f32, 100.0_f32),
                (10.0, 300.0),
                (100.0, 1800.0),
                (2000.0, 2500.0),
            ] {
                cases.push([
                    [direction[0], direction[1], direction[2], depth].map(f32::to_bits),
                    [start.to_bits(), end.to_bits(), 0, 0],
                ]);
                let (start, end) = (f64::from(start), f64::from(end));
                let visible = |low: f64, high: f64, blocker: f64| {
                    let low = low.max(start);
                    let high = high.min(end).min(2048.0);
                    if high <= low {
                        return 0.0;
                    }
                    let threshold = blocker - 0.02 - f64::from(depth);
                    let rate = f64::from(direction[2]);
                    if rate > 0.0 {
                        (high - low.max(threshold / rate)).clamp(0.0, high - low)
                    } else if rate < 0.0 {
                        (high.min(threshold / rate) - low).clamp(0.0, high - low)
                    } else if threshold <= 0.0 {
                        high - low
                    } else {
                        0.0
                    }
                };
                let lit = if direction[0] == 0.0 {
                    visible(start, end, PROFILE[0].2)
                } else {
                    PROFILE
                        .iter()
                        .map(|&(low, high, blocker)| {
                            visible(
                                low / f64::from(direction[0]),
                                high / f64::from(direction[0]),
                                blocker,
                            )
                        })
                        .sum()
                };
                expected.push(lit / (end - start));
            }
        }
    }
    for (index, (row, reference)) in run_shader(shader, &cases)?.iter().zip(expected).enumerate() {
        assert_eq!(row[3], index as u32 + 1);
        let actual = f64::from(f32::from_bits(row[0]));
        assert!((0.0..=1.0).contains(&actual));
        assert!(
            (actual - reference).abs() <= 3e-5,
            "case {index}: {actual}/{reference}"
        );
    }
    Ok(())
}
