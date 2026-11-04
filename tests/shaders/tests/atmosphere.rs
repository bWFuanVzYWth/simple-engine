//! 球形大气边界与常系数源积分的独立 f64 数学约束；完整八阶段 oracle 在 atmosphere crate。

use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const SHADER: Shader<'static> = Shader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/atmosphere_main.spv")),
    entry: "atmosphere_main",
    workgroup: [64, 1, 1],
};

fn run(cases: &[[[u32; 4]; 2]]) -> Result<Vec<[u32; 4]>> {
    let bytes: Vec<_> = cases
        .iter()
        .flatten()
        .flatten()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let result = gpu.compute(
        SHADER,
        &bytes,
        cases.len() * 16,
        [(cases.len() as u32).div_ceil(64), 1, 1],
    );
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
        for sigma in [[0.0_f32, 1e-10, 1e-6, 1e-3], [0.01, 0.1, 1.0, 20.0]] {
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
