//! 规范 guide 与单帧重建的 Vulkan 行为；不使用 SDK 或应用场景。
use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn run(entry: &'static str, spirv: &[u8], mode: u32, rows: usize) -> Result<Vec<[f32; 4]>> {
    let input: Vec<_> = [mode as f32, 0., 0., 0.]
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
    assert!((actual - expected).abs() < 5e-4, "{actual} != {expected}");
}

fn psr(mode: u32) -> Result<Vec<[f32; 4]>> {
    run(
        "psr_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/psr_main.spv")),
        mode,
        3,
    )
}

fn guide(mode: u32) -> Result<Vec<[f32; 4]>> {
    run(
        "guide_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/guide_main.spv")),
        mode,
        5,
    )
}

fn spatial(mode: u32) -> Result<[f32; 4]> {
    Ok(run(
        "spatial_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/spatial_main.spv")),
        mode,
        1,
    )?[0])
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn planar_reflection_unfolds_the_physical_endpoint() -> Result {
    let rows = psr(0)?;
    assert_eq!(rows[0], [0., 0., 6., 1.]);
    assert_eq!(rows[1], [0., 0., -1., 1.]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn consecutive_reflections_compose_in_world_order() -> Result {
    let rows = psr(1)?;
    assert_eq!(rows[0], [0., 0., 9., 1.]);
    assert_eq!(rows[1], [0., 0., 1., 0.]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn refraction_uses_a_marked_target_plane_proxy() -> Result {
    let rows = psr(2)?;
    close(rows[0][2], 3.);
    assert_eq!(rows[0][3], 1.);
    assert_eq!(rows[2], [1., 1., 0., 0.]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn guide_survives_terminated_black_reflection_lighting() -> Result {
    let rows = guide(0)?;
    assert_eq!(rows[0], [0., 0., -2., 22.]);
    assert_eq!(rows[2][0], 6.);
    assert_eq!(rows[2][3], 0.);
    assert_eq!(rows[3], [3., 5., 7., 1.]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn guide_transmission_ignores_absorption_and_zero_boundary_filter() -> Result {
    let rows = guide(1)?;
    assert_eq!(rows[0], [0., 0., 6., 22.]);
    assert_eq!(rows[4], [0., 1., 1., 4.]);
    assert_eq!(rows[2][3], 32.);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn rough_surface_has_an_independent_reflection_proxy() -> Result {
    let rows = guide(6)?;
    assert_eq!(rows[0], [0., 0., -2., 22.]);
    assert_eq!(rows[2][0], 6.);
    assert_eq!(rows[4][0], 1.);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn rigid_previous_position_keeps_object_motion() -> Result {
    let rows = run(
        "rigid_motion_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/rigid_motion_main.spv")),
        0,
        2,
    )?;
    assert_eq!(rows[0], [2., -0.75, 4., 1.]);
    for (actual, expected) in rows[1].into_iter().zip([20., 0., 10., 0.]) {
        close(actual, expected);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn unknown_object_pose_uses_a_marked_camera_proxy() -> Result {
    let rows = run(
        "rigid_motion_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/rigid_motion_main.spv")),
        1,
        2,
    )?;
    assert_eq!(rows[1], [0., 0., 10., 4.]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn moving_interface_does_not_claim_known_endpoint_motion() -> Result {
    let rows = run(
        "rigid_motion_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/rigid_motion_main.spv")),
        2,
        2,
    )?;
    assert_eq!(rows[1], [0., 0., 10., 4.]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn total_internal_reflection_selects_the_reflected_guide() -> Result {
    let rows = guide(2)?;
    assert!(rows[1][2] < 0.);
    assert_eq!(rows[4][..3], [1., 0., 0.]);
    assert_eq!(rows[2][3], 0.);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn escape_on_the_last_query_is_complete() -> Result {
    let rows = guide(4)?;
    assert_eq!(rows[1][3], 2.);
    assert_eq!(rows[2][0], f32::MAX);
    assert_eq!(rows[2][3] as u32 & 8, 0);
    assert_ne!(rows[2][3] as u32 & 1, 0);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn exhausted_guide_keeps_an_observed_surface() -> Result {
    let rows = guide(5)?;
    assert_eq!(rows[0], [0., 0., 2., 11.]);
    assert_eq!(rows[2][0], 2.);
    assert_eq!(rows[2][3] as u32 & 11, 10);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn reflection_distance_motion_preserves_zero_distance_and_sky() -> Result {
    for (mode, expected) in [
        (0, [10., 0., 1., 0.]),
        (1, [7., 3., 1., 0.]),
        (2, [0., 0., 1., 0.]),
        (3, [0.; 4]),
    ] {
        let rows = run(
            "reflection_motion_main",
            include_bytes!(concat!(env!("OUT_DIR"), "/reflection_motion_main.spv")),
            mode,
            1,
        )?;
        for (actual, expected) in rows[0].into_iter().zip(expected) {
            close(actual, expected);
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn spatial_reconstruction_preserves_a_constant_field_at_edges() -> Result {
    let row = spatial(0)?;
    for (actual, expected) in row.into_iter().zip([2., 3., 4., 1.]) {
        close(actual, expected);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn spatial_reconstruction_does_not_cross_depth_discontinuities() -> Result {
    let row = spatial(1)?;
    for (actual, expected) in row.into_iter().zip([2., 0., 0., 1.]) {
        close(actual, expected);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn spatial_reconstruction_does_not_cross_material_discontinuities() -> Result {
    let row = spatial(2)?;
    for (actual, expected) in row.into_iter().zip([2., 0., 0., 1.]) {
        close(actual, expected);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn spatial_reconstruction_reduces_noise_with_identical_guides() -> Result {
    let row = spatial(3)?;
    assert!(row[0] > 0. && row[0] < 2.);
    assert!(row[2] > 0. && row[2] < 2.);
    close(row[0] + row[2], 2.);
    assert_eq!(row[3], 1.);
    Ok(())
}
