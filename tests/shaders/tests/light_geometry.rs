//! 光源连接的独立f64几何量与整数选择器分布。
use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn run(
    entry: &'static str,
    spirv: &[u8],
    rows: &[[f32; 4]],
    count: usize,
) -> Result<Vec<[f32; 4]>> {
    let input: Vec<_> = rows
        .iter()
        .flatten()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let output = gpu.compute(
        Shader {
            spirv,
            entry,
            workgroup: [64, 1, 1],
        },
        &input,
        count * 16,
        [(count as u32).div_ceil(64), 1, 1],
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

#[test]
#[ignore = "真实Vulkan compute；面积与立体角测度"]
fn light_pdf_matches_independent_geometric_measure() -> Result {
    let mut input = Vec::new();
    let mut expected = Vec::new();
    for distance in [1e-10_f32, 0.1, 1., 1000., 1e10] {
        for density in [1e-20_f32, 0.25, 1e10] {
            for (normal, sides) in [
                ([0., 0., 1.], 1_u32),
                ([0., 0., -1.], 1),
                ([0., 0., -1.], 2),
                ([0., 0.6, 0.8], 3),
            ] {
                input.extend([
                    [0., 0., distance, density],
                    [0., 0., 0., 0.125],
                    [normal[0], normal[1], normal[2], f32::from_bits(sides)],
                ]);
                let cosine = f64::from(normal[2]);
                let side = if cosine >= 0. { 1 } else { 2 };
                expected.push(if sides & side != 0 {
                    f64::from(density) * 0.125 * f64::from(distance).powi(2) / cosine.abs()
                } else {
                    0.
                });
            }
        }
    }
    input.extend([
        [0., 0., 0., 1.],
        [0., 0., 0., 1.],
        [0., 0., 1., f32::from_bits(3)],
    ]);
    expected.push(0.);
    let out = run(
        "light_geometry_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/light_geometry_main.spv")),
        &input,
        expected.len(),
    )?;
    for (row, reference) in out.iter().zip(expected) {
        for &value in &row[..2] {
            assert!(value.is_finite());
            assert!(
                (f64::from(value) - reference).abs() <= 3e-6 * reference.abs().max(1e-30),
                "{value} != {reference}"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "真实Vulkan compute；均匀灯面的24-bit离散质量"]
fn uniform_emitter_quantizes_the_area_selector_once() -> Result {
    const N: u32 = 1 << 24;
    let mut input = Vec::new();
    let mut expected = Vec::new();
    for split in [0, 1, N / 2 - 1, N / 2, N - 1, N] {
        for selector in [0, 1, N / 2 - 1, N / 2, N - 1] {
            input.extend([
                [f32::from_bits(split), 0.5, 0.25, 0.],
                [selector as f32 / N as f32, 0.375, 0., 0.],
            ]);
            let second = selector >= split;
            let mass = if second { N - split } else { split };
            let start = if second { split } else { 0 };
            let residual = (f64::from(selector - start) + 0.5) / f64::from(mass);
            let root = residual.sqrt();
            expected.push((second, root * 0.625, root * 0.375));
        }
    }
    let out = run(
        "uniform_area_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/uniform_area_main.spv")),
        &input,
        expected.len(),
    )?;
    for (row, (second, x, y)) in out.iter().zip(expected) {
        assert_eq!(row[0].to_bits(), u32::from(second));
        assert!((f64::from(row[1]) - x).abs() < 2e-7 && (f64::from(row[2]) - y).abs() < 2e-7);
        assert_eq!(row[3], if second { 0.25 } else { 0.5 });
    }
    Ok(())
}

#[test]
#[ignore = "真实Vulkan compute；实际灯面连接的距离与PDF"]
fn emitter_connection_matches_unoffset_endpoint_measure() -> Result {
    let input: Vec<_> = [0.001_f32, 1., 1000., 1e6]
        .into_iter()
        .flat_map(|distance| [0.1, 0.7].map(|u| [distance, u, 0.375, 0.]))
        .collect();
    let output = run(
        "emitter_connection_main",
        include_bytes!(concat!(env!("OUT_DIR"), "/emitter_connection_main.spv")),
        &input,
        input.len(),
    )?;
    for (p, row) in input.iter().zip(output) {
        let squared =
            f64::from(p[0]).powi(2) + f64::from(row[2]).powi(2) + f64::from(row[3]).powi(2);
        let distance = squared.sqrt();
        let pdf = 0.25 * f64::from(0.4_f32) * squared * distance / f64::from(p[0]);
        assert!((f64::from(row[0]) / distance - 1.).abs() < 2e-6);
        assert!((f64::from(row[1]) / pdf - 1.).abs() < 3e-6);
    }
    Ok(())
}
