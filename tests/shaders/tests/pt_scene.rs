//! 两种命中编码分别使用组合与旧入口；核验发光 MIS、Beer、预算和边界介质的公开行为。

use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
type Path = [[u32; 4]; 7];

fn run(mode: u32) -> Result<Vec<Path>> {
    let samples = [0_u32, 1, 255, 256];
    let mut input = Vec::new();
    for sample in samples {
        input.extend([
            [0.8_f32, 0.6, 0.4, 3.0].map(f32::to_bits),
            [0.5_f32, 0.75, 1.0, 0.5].map(f32::to_bits),
            [
                0.1_f32.to_bits(),
                0.2_f32.to_bits(),
                0.3_f32.to_bits(),
                sample,
            ],
            [
                0.05_f32.to_bits(),
                0.1_f32.to_bits(),
                0.15_f32.to_bits(),
                mode,
            ],
        ]);
    }
    let bytes: Vec<_> = input
        .iter()
        .flatten()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let output = gpu.compute(
        Shader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/pt_scene_main.spv")),
            entry: "pt_scene_main",
            workgroup: [64, 1, 1],
        },
        &bytes,
        samples.len() * 14 * 16,
        [1, 1, 1],
    );
    let cleanup = gpu.finish();
    let output = output?;
    cleanup?;
    let rows: Vec<[u32; 4]> = output
        .as_chunks::<16>()
        .0
        .iter()
        .map(|row| {
            std::array::from_fn(|i| u32::from_le_bytes(row[4 * i..4 * i + 4].try_into().unwrap()))
        })
        .collect();
    rows.as_chunks::<14>()
        .0
        .iter()
        .enumerate()
        .map(|(case, rows)| {
            assert_eq!(&rows[..5], &rows[7..12], "场景适配改变了路径：case={case}");
            for field in 5..7 {
                for component in 0..3 {
                    assert_eq!(
                        f32::from_bits(rows[field][component]),
                        f32::from_bits(rows[field + 7][component]),
                        "场景适配改变了位置：case={case}, field={field}, component={component}"
                    );
                }
            }
            Ok(rows[..7].try_into()?)
        })
        .collect()
}

fn close(actual: u32, expected: f32) {
    let actual = f32::from_bits(actual);
    assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn scene_providers_preserve_emission_mis() -> Result {
    for path in run(0)? {
        let attenuation = [0.1_f32, 0.2, 0.3].map(|v| (-2.0 * v).exp());
        let color = [0.8_f32 * 0.5, 0.6 * 0.75, 0.4];
        for channel in 0..3 {
            close(
                path[1][channel],
                attenuation[channel] * color[channel] * 3.0 * 0.2,
            );
        }
        assert_eq!(path[4], [2, 0, 1, 1]);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn scene_providers_preserve_solid_boundary() -> Result {
    for path in run(1)? {
        let attenuation = [0.1_f32, 0.2, 0.3].map(|v| (-2.0 * v).exp());
        let color = [0.8_f32 * 0.5, 0.6 * 0.75, 0.4];
        for channel in 0..3 {
            close(path[0][channel], attenuation[channel] * color[channel]);
        }
        assert_eq!(path[2], [1.0_f32, 0.05, 0.1, 0.15].map(f32::to_bits));
        assert_eq!(path[3], [0.0_f32, 0.0, -1.0, 0.0].map(f32::to_bits));
        assert_eq!(path[4], [1, 1, 0, 1]);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn scene_termination_clears_geometry_identity() -> Result {
    for path in run(0)? {
        assert_eq!(f32::from_bits(path[0][3]), 0.0);
        for position in &path[5..7] {
            for &bits in &position[..3] {
                assert_eq!(f32::from_bits(bits), 0.0);
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn one_bounce_budget_performs_one_scatter() -> Result {
    for path in run(2)? {
        assert_eq!(path[4], [1, 1, 0, 1]);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn sixty_fourth_bounce_lands_on_terminal_vertex() -> Result {
    for path in run(3)? {
        assert_eq!(path[4], [65, 0, 1, 1]);
        for position in &path[5..7] {
            for &bits in &position[..3] {
                assert_eq!(f32::from_bits(bits), 0.0);
            }
        }
    }
    Ok(())
}
