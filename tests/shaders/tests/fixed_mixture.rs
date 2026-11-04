//! 实际生产随机选择的均值回归；来源估计强相关，不依赖独立 F/R 假设。
use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const COUNT: usize = 65_536;

fn run(probability: f32) -> Result<Vec<[f32; 4]>> {
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let input: Vec<u8> = [probability.to_bits(), 19, COUNT as u32, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let result = gpu.compute(
        Shader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/fixed_mixture_test.spv")),
            entry: "fixed_mixture_main",
            workgroup: [64, 1, 1],
        },
        &input,
        COUNT * 16,
        [(COUNT / 64) as u32, 1, 1],
    );
    let cleanup = gpu.finish();
    let bytes = result?;
    cleanup?;
    Ok(bytes
        .as_chunks::<16>()
        .0
        .iter()
        .map(|row| {
            std::array::from_fn(|c| f32::from_le_bytes(row[4 * c..4 * c + 4].try_into().unwrap()))
        })
        .collect())
}

#[test]
#[ignore = "真实Vulkan compute；固定选择概率及相关来源均值"]
fn fixed_probability_preserves_the_mean_of_correlated_sources() -> Result {
    simple_engine::gpu_log::checked(|| {
        let p = 0.4;
        let samples = run(p)?;
        let selected = samples.iter().map(|v| f64::from(v[3])).sum::<f64>() / COUNT as f64;
        let expected = f64::from(p);
        let selection_error = 6.0 * (expected * (1.0 - expected) / COUNT as f64).sqrt();
        assert!((selected - expected).abs() < selection_error);
        for (channel, (mean, amplitude)) in
            [(1.0, 3.0), (2.0, 4.0), (3.0, 6.0)].into_iter().enumerate()
        {
            let actual = samples.iter().map(|v| f64::from(v[channel])).sum::<f64>() / COUNT as f64;
            let error = 6.0 * amplitude / (COUNT as f64).sqrt();
            assert!(
                (actual - mean).abs() < error,
                "通道{channel}均值{actual}，期望{mean}"
            );
        }
        Ok(())
    })
}

#[test]
#[ignore = "真实Vulkan compute；概率端点不得发生误选"]
fn probability_endpoints_select_only_the_requested_source() -> Result {
    simple_engine::gpu_log::checked(|| {
        for p in [0.0, 1.0] {
            let samples = run(p)?;
            assert!(samples.iter().all(|v| v[3] == p));
        }
        Ok(())
    })
}
