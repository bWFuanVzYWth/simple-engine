//! 通用降噪数据的实际 GPU 行为；不依赖应用或重建 SDK。

use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn run(entry: &'static str, spirv: &[u8], rows: usize) -> Result<Vec<[f32; 4]>> {
    let input: Vec<_> = [0.8_f32, 0.5, 0.2, 0.4]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let output = gpu.compute(
        Shader {
            spirv,
            entry,
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
            std::array::from_fn(|i| f32::from_le_bytes(row[4 * i..4 * i + 4].try_into().unwrap()))
        })
        .collect())
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 5e-5, "{actual} != {expected}");
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn motion_excludes_jitter_and_sky_parallax() -> Result {
    let rows = run(
        "denoise_motion_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/denoise_motion_main.spv")),
        5,
    )?;
    for row in [&rows[0], &rows[3]] {
        close(row[0], 0.0);
        close(row[1], 0.0);
        assert_eq!(row[3], 1.0);
    }
    close(rows[0][2], 10.0);
    close(rows[1][0], 20.0);
    close(rows[2][0], -20.0);
    assert_eq!(rows[3][2], f32::MAX);
    assert_eq!(rows[4], [0.0; 4]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn albedo_is_directional_energy() -> Result {
    let rows = run(
        "denoise_albedo_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/denoise_albedo_main.spv")),
        7,
    )?;
    assert_eq!(rows[0], [0.0, 0.0, 1.0, 0.4]);
    assert_eq!(rows[1], [0.8, 0.5, 0.2, 1.0]);
    assert_eq!(rows[2], [0.0, 0.0, 0.0, 1.0]);
    for (channel, base) in [0.8, 0.5, 0.2].into_iter().enumerate() {
        assert!(rows[3][channel] > 0.0 && rows[3][channel] < base);
        assert!(rows[4][channel] > 0.0 && rows[4][channel] < 1.0);
        close(rows[3][channel] / base + rows[4][channel], 1.0);
    }
    assert_eq!(rows[5], [1.0; 4]);
    assert_eq!(rows[6], [0.0, 0.0, 0.0, 1.0]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn prefix_observer_keeps_first_surface_and_transport() -> Result {
    let rows = run(
        "denoise_prefix_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/denoise_prefix_main.spv")),
        6,
    )?;
    assert_eq!(rows[0], [0.0, 0.0, 2.0, 11.0]);
    assert_eq!(rows[1], [0.0, 0.0, 4.0, 22.0]);
    assert_eq!(rows[2], rows[3]);
    assert_eq!(rows[4], rows[5]);
    assert_eq!(rows[4], [0.0, 0.0, 1.0, 1.0]);
    Ok(())
}
