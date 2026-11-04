//! 解析 Lambert oracle；实际 GPU 执行共享 initial/shift 与双向 GRIS，CPU 只检查积分结果。

use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
type Sample = [[f32; 4]; 8];
const COUNT: usize = 65_536;
const ALBEDO: [f32; 3] = [0.8, 0.6, 0.3];
const RADIANCE: f32 = 1.7;

fn input(mode: u32, count: u32, roughness: f32, distance: f32, extinction: f32) -> Vec<u8> {
    [
        [mode, count, 0, 0],
        [roughness, distance, extinction, 0.9].map(f32::to_bits),
        [ALBEDO[0], ALBEDO[1], ALBEDO[2], RADIANCE].map(f32::to_bits),
    ]
    .into_iter()
    .flatten()
    .flat_map(u32::to_le_bytes)
    .collect()
}

fn run(mode: u32, roughness: f32, distance: f32, extinction: f32) -> Result<Vec<Sample>> {
    simple_engine::gpu_log::checked(|| run_inner(mode, roughness, distance, extinction))
}

fn run_inner(mode: u32, roughness: f32, distance: f32, extinction: f32) -> Result<Vec<Sample>> {
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let output = gpu.compute(
        Shader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/restir_mean_main.spv")),
            entry: "restir_mean_main",
            workgroup: [64, 1, 1],
        },
        &input(mode, COUNT as u32, roughness, distance, extinction),
        COUNT * 8 * 16,
        [COUNT.div_ceil(64) as u32, 1, 1],
    );
    let cleanup = gpu.finish();
    let output = output?;
    cleanup?;
    let values: Vec<_> = output
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| f32::from_le_bytes(*word))
        .collect();
    let samples: Vec<_> = values
        .as_chunks::<32>()
        .0
        .iter()
        .map(|rows| std::array::from_fn(|row| std::array::from_fn(|col| rows[4 * row + col])))
        .collect();
    for (index, sample) in samples.iter().enumerate() {
        assert!(
            sample.iter().flatten().all(|value| value.is_finite()),
            "非有限结果：mode={mode}, sample={index}, {sample:?}"
        );
        assert!(
            sample[1][..3].iter().all(|error| *error < 2e-5),
            "恒等 shift 改变 integrand：mode={mode}, sample={index}, {:?}",
            sample[1]
        );
    }
    Ok(samples)
}

fn mean_matches(samples: &[Sample], expected: [f32; 3], label: &str) {
    let operations = if label.starts_with("cavity") {
        &[
            (0, "initial"),
            (2, "temporal"),
            (3, "spatial"),
            (6, "shift"),
        ][..]
    } else {
        &[(0, "initial"), (2, "temporal"), (3, "spatial")][..]
    };
    for &(row, operation) in operations {
        for (channel, expected) in expected.into_iter().enumerate() {
            let count = samples.len() as f64;
            let mean = samples
                .iter()
                .map(|sample| f64::from(sample[row][channel]))
                .sum::<f64>()
                / count;
            let variance = samples
                .iter()
                .map(|sample| (f64::from(sample[row][channel]) - mean).powi(2))
                .sum::<f64>()
                / (count - 1.0);
            let stderr = (variance / count).sqrt();
            let error = (mean - f64::from(expected)).abs();
            eprintln!(
                "{label}/{operation}[{channel}] n={} mean={mean:.8} expected={expected:.8} error={error:.8} stderr={stderr:.8}",
                samples.len()
            );
            assert!(
                error <= 6.0 * stderr + 5e-4,
                "{label}/{operation}[{channel}] 超出解析积分的六倍标准误差"
            );
        }
    }
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn lambert_environment_reuse_preserves_integral() -> Result {
    let samples = run(0, 0.2, 0.0002, 0.0)?;
    mean_matches(
        &samples,
        ALBEDO.map(|value| value * RADIANCE),
        "environment",
    );
    let rejected = samples.iter().filter(|sample| sample[4][2] != 0.0).count();
    assert!(rejected > COUNT / 100, "未覆盖倾斜法线的非可逆支持域");
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn no_reconnection_replays_full_selected_path() -> Result {
    let samples = run(1, 2.0, 1e20, 0.0)?;
    mean_matches(&samples, ALBEDO.map(|value| value * RADIANCE), "noRC");
    assert!(samples.iter().all(|sample| sample[4][0] == 1.0));
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn finite_light_reuse_preserves_area_measure() -> Result {
    let samples = run(2, 0.2, 0.0002, 0.0)?;
    mean_matches(&samples, ALBEDO.map(|value| value * RADIANCE), "areaRC");
    assert!(samples.iter().any(|sample| sample[4][1] == 1.0));
    assert!(
        samples
            .iter()
            .any(|sample| (sample[5][0] - 1.0).abs() > 0.01)
    );
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn forced_nee_and_full_replay_share_unbiased_mis() -> Result {
    let samples = run(3, 2.0, 1e20, 0.0)?;
    mean_matches(&samples, ALBEDO.map(|value| value * RADIANCE), "mixedRC");
    assert!(samples.iter().any(|sample| sample[4][0] == 1.0));
    assert!(samples.iter().any(|sample| sample[4][1] == 1.0));
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn camera_and_connection_beer_are_counted_once() -> Result {
    let samples = run(4, 0.2, 0.0002, 0.3)?;
    mean_matches(
        &samples,
        ALBEDO.map(|value| value * RADIANCE * (-0.9_f32).exp()),
        "Beer",
    );
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn primary_environment_uses_camera_medium() -> Result {
    let samples = run(5, 0.2, 0.0002, 0.3)?;
    mean_matches(&samples, [RADIANCE * (-0.6_f32).exp(); 3], "cameraMiss");
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn environment_reconnection_uses_destination_position() -> Result {
    let samples = run(6, 0.2, 0.0002, 0.0)?;
    mean_matches(
        &samples,
        ALBEDO.map(|value| value * RADIANCE),
        "positionEnvironment",
    );
    Ok(())
}

/// 单段余弦分布的Beer平均为2∫₀¹ μ exp(-σ/μ)dμ；独立散射使每阶因子为ρ乘此积分。
fn cavity_expected(extinction: f32) -> [f32; 3] {
    const N: usize = 4096;
    let sigma = f64::from(extinction);
    let mut integral = 0.0;
    for i in 1..=N {
        let cosine = i as f64 / N as f64;
        let value = 2.0 * cosine * (-sigma / cosine).exp();
        let weight = if i == N {
            1.0
        } else if i % 2 == 0 {
            2.0
        } else {
            4.0
        };
        integral += weight * value;
    }
    integral /= 3.0 * N as f64;
    ALBEDO.map(|albedo| {
        let factor = f64::from(albedo) * integral;
        let sum = (0..=10).map(|k| factor.powi(k)).sum::<f64>();
        (f64::from(RADIANCE) * (-0.5 * sigma).exp() * sum) as f32
    })
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn ten_bounce_hybrid_preserves_ris_and_replayed_prefix() -> Result {
    for extinction in [0.0, 0.3] {
        let samples = run(7, 0.2, 0.0002, extinction)?;
        mean_matches(
            &samples,
            cavity_expected(extinction),
            &format!("cavityRC/sigma={extinction}"),
        );
        assert!(samples.iter().any(|sample| sample[7][2] == 2.0));
        if extinction == 0.0 {
            assert!(samples.iter().any(|sample| sample[7][0] == 10.0));
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn ten_bounce_no_rc_replays_selected_terminal_and_rr_pdf() -> Result {
    for extinction in [0.0, 0.3] {
        let samples = run(8, 2.0, 1e20, extinction)?;
        mean_matches(
            &samples,
            cavity_expected(extinction),
            &format!("cavityPSS/sigma={extinction}"),
        );
        assert!(
            samples
                .iter()
                .all(|sample| sample[5][2] == 0.0 || sample[7][2] == 255.0)
        );
        if extinction == 0.0 {
            assert!(samples.iter().any(|sample| sample[7][0] == 10.0));
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn reverse_footprint_rejects_an_earlier_reconnection() -> Result {
    simple_engine::gpu_log::checked(footprint)
}

fn footprint() -> Result {
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let output = gpu.compute(
        Shader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/restir_footprint_main.spv")),
            entry: "restir_footprint_main",
            workgroup: [64, 1, 1],
        },
        &input(2, 2, 0.2, 0.0002, 0.0),
        32,
        [1, 1, 1],
    );
    let cleanup = gpu.finish();
    let output = output?;
    cleanup?;
    let values: Vec<_> = output
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| f32::from_le_bytes(*word))
        .collect();
    assert_eq!(&values[..4], &[0.0; 4]);
    assert_eq!(values[7], 1.0);
    assert!(values[4..7].iter().all(|value| *value > 0.0));
    Ok(())
}
