use simple_engine::gpu_test::{self, Gpu, Shader};

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error(transparent)]
    Gpu(#[from] gpu_test::Error),
    #[error("{0} 的 GPU 计时无效")]
    Timing(String),
}

pub(super) struct Measurement {
    pub output: Vec<u8>,
    pub median_gpu_ms_per_frame: f64,
    pub samples: Vec<serde_json::Value>,
}

pub(super) fn measure(
    gpu: &mut Gpu,
    shader: Shader<'_>,
    input: &[u8],
    output_bytes: usize,
    groups: [u32; 3],
    dispatches: u32,
) -> Result<Measurement, Error> {
    for _ in 0..3 {
        gpu.bench_compute(shader, input, output_bytes, groups, dispatches)?;
    }
    let mut frame_ms = [0.0; 9];
    let mut samples = Vec::with_capacity(frame_ms.len());
    for elapsed in &mut frame_ms {
        let sample = gpu.bench_compute(shader, input, output_bytes, groups, dispatches)?;
        if !sample.gpu_seconds.is_finite() || sample.gpu_seconds <= 0.0 {
            return Err(Error::Timing(shader.entry.into()));
        }
        *elapsed = 1000.0 * sample.gpu_seconds / f64::from(dispatches);
        samples.push(serde_json::json!({
            "gpu_seconds": sample.gpu_seconds,
            "host_seconds": sample.host_seconds,
            "prepare_seconds": sample.prepare_seconds,
        }));
    }
    frame_ms.sort_unstable_by(f64::total_cmp);
    Ok(Measurement {
        output: gpu.compute(shader, input, output_bytes, groups)?,
        median_gpu_ms_per_frame: frame_ms[frame_ms.len() / 2],
        samples,
    })
}
