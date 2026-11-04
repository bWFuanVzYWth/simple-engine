//! CV 的有限分片积分及精确双色方差；CPU 只检查解析积分和分布矩。
use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn run(mode: u32, rows: usize) -> Result<Vec<[f32; 4]>> {
    simple_engine::gpu_log::checked(|| {
        let input: Vec<_> = [mode as f32, 0., 0., 0.]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
        let output = gpu.compute(
            Shader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/control_variate_test.spv")),
                entry: "cv_main",
                workgroup: [64, 1, 1],
            },
            &input,
            rows * 16,
            [1, 1, 1],
        );
        let cleanup = gpu.finish();
        let output = output?;
        cleanup?;
        Ok(output
            .as_chunks::<16>()
            .0
            .iter()
            .map(|row| {
                std::array::from_fn(|i| {
                    f32::from_le_bytes(row[4 * i..4 * i + 4].try_into().unwrap())
                })
            })
            .collect())
    })
}

fn close(actual: &[f32], expected: &[f32]) {
    for (actual, expected) in actual.iter().zip(expected) {
        assert!((actual - expected).abs() < 3e-5, "{actual} != {expected}");
    }
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn zero_reflectance_channels_produce_finite_bounded_alpha() -> Result {
    close(&run(0, 7)?[0][..3], &[1., 2., 2.]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn missing_sources_do_not_create_neighbor_support() -> Result {
    let rows = run(0, 7)?;
    close(&rows[1], &[3., 5., 7., 0.]);
    close(&rows[2], &[3., 5., 7., 1.]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn zero_target_samples_retain_their_confidence() -> Result {
    close(&run(0, 7)?[3], &[1., 2., 3., 3.]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn finite_negative_corrections_survive_combination() -> Result {
    close(&run(0, 7)?[4], &[-0.5, -0.5, -0.5, 2.]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn temporal_cap_changes_confidence_and_mis_without_scaling_ucw() -> Result {
    let rows = run(0, 7)?;
    // cap=20时a=1/81、b=40/43，差值=5/43-1/27；合成分母为21。
    close(&rows[5][..3], &[8806. / 24381.; 3]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn disabled_history_preserves_the_initial_color() -> Result {
    close(&run(0, 7)?[6][..3], &[3., 5., 7.]);
    Ok(())
}

fn integral_matches(mode: u32) -> Result {
    let rows = run(mode, 4)?;
    // I_c=(f_c0+f_c1)/2，I_n=f_n0+f_n1；与proposal、M、Jacobian和alpha无关。
    close(&rows[0][..3], &[0.625, -1.25, -5.25]);
    for row in &rows[1..] {
        close(&row[..3], &[2.25, 2., 1.25]);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn unequal_measures_and_confidences_preserve_the_rgb_integral() -> Result {
    integral_matches(1)
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn partial_bidirectional_support_preserves_both_domain_integrals() -> Result {
    integral_matches(2)
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn complementary_zero_targets_preserve_the_rgb_integral() -> Result {
    let rows = run(4, 4)?;
    close(&rows[0][..3], &[0.125, -1.1, -5.]);
    for row in &rows[1..] {
        close(&row[..3], &[0.25, 1.5, 1.]);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn accumulated_history_reduces_chroma_variance_without_changing_rgb_mean() -> Result {
    let rows = run(3, 2)?;
    close(&rows[0], &[1., 1., 0., 4.]);
    close(&rows[1], &[1., 1., 0., 4. / 17.]);
    Ok(())
}
