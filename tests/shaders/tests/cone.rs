//! 小立体角光源的采样与 PDF；以 f64 球面面积和弦长检验实际 shader。
use simple_engine::gpu_test::{Capability, Gpu, Shader};

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn tiny_cones_preserve_density_support_and_radial_distribution()
-> Result<(), Box<dyn std::error::Error>> {
    let mut cases = Vec::new();
    for angle in [1e-6_f32, 1e-4, 0.00465, 0.2, 1.] {
        for axis in [[0., 0., 1.], [0., 0.6, 0.8]] {
            if angle < 0.004 {
                if axis[1] != 0. {
                    continue;
                }
            }
            for i in 0..1024 {
                let u = (i as f32 + 0.5) / 1024.;
                let v = ((i * 347 % 1024) as f32 + 0.5) / 1024.;
                cases.push(([axis[0], axis[1], axis[2], angle], [u, v, 0., 0.]));
            }
        }
    }
    let input: Vec<_> = cases
        .iter()
        .flat_map(|(axis, u)| axis.iter().chain(u))
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let output = gpu.compute(
        Shader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/cone_main.spv")),
            entry: "cone_main",
            workgroup: [64, 1, 1],
        },
        &input,
        cases.len() * 32,
        [(cases.len() as u32).div_ceil(64), 1, 1],
    );
    let cleanup = gpu.finish();
    let output = output?;
    cleanup?;
    for ((axis, u), bytes) in cases.iter().zip(output.as_chunks::<32>().0) {
        let row: [f32; 8] = std::array::from_fn(|i| {
            f32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap())
        });
        let chord2 = 4. * (0.5 * f64::from(axis[3])).sin().powi(2);
        let pdf = 1. / (std::f64::consts::PI * chord2);
        let tolerance = if axis[3] < 0.2 { 5e-7 } else { 5e-6 };
        assert!(
            (f64::from(row[3]) / pdf - 1.).abs() < tolerance,
            "PDF {axis:?}: {row:?}"
        );
        let observed: f64 = (0..3)
            .map(|i| (f64::from(row[i]) - f64::from(axis[i])).powi(2))
            .sum();
        assert!(
            (observed / chord2 - f64::from(u[0])).abs() < 4e-5,
            "半径分布 {axis:?}: {row:?}"
        );
        assert_eq!(row[4], 1., "采样落在发光支持域之外 {axis:?}: {row:?}");
    }
    Ok(())
}
