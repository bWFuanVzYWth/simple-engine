//! 4K 透视相机主射线与投影。在投影负载中沿每条当前射线独立重投影 32 个世界点，
//! 使用带平移和旋转的前帧相机。设备时间包含派发和同步，不含上传或结果检查。
#![forbid(unsafe_code)]

#[path = "support/compute.rs"]
mod compute;

use simple_engine::gpu_test::{self, Capability, Gpu, Shader};
use std::io::{self, Write};

const EXTENT: [u32; 2] = [3840, 2160];
const WORKGROUP: [u32; 3] = [8, 8, 1];
const GROUPS: [u32; 3] = [EXTENT[0] / WORKGROUP[0], EXTENT[1] / WORKGROUP[1], 1];
const OUTPUT_BYTES: usize = EXTENT[0] as usize * EXTENT[1] as usize * 16;
const DISPATCHES: u32 = 16;
const RAY: Shader<'static> = Shader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/camera-ray.spv")),
    entry: "ray_main",
    workgroup: WORKGROUP,
};
const PROJECT: Shader<'static> = Shader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/camera-project.spv")),
    entry: "project_main",
    workgroup: WORKGROUP,
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Gpu(#[from] gpu_test::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Measurement(#[from] compute::Error),
    #[error("{0} 的相机输出无效")]
    Output(&'static str),
}

fn camera_input() -> Vec<u8> {
    let [w, h] = EXTENT.map(|extent| extent as f32);
    [
        [0.0, 0.0, 0.0, 0.6],
        [0.0, 0.0, 1.0, w / h],
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.25, 0.1, 0.75, 0.62],
        [0.28, 0.0, 0.96, w / h],
        [0.96, 0.0, -0.28, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [1.0 / w, 1.0 / h, 0.25, 0.75],
    ]
    .into_iter()
    .flatten()
    .flat_map(f32::to_le_bytes)
    .collect()
}

fn main() -> Result<(), Error> {
    if !std::env::args_os().any(|argument| argument == "--bench") {
        return Ok(());
    }
    let mut gpu = Gpu::new(Capability::Compute, 0, true)?;
    let adapter = gpu.context().adapter_info();
    let input = camera_input();
    let workloads = [RAY, PROJECT]
        .into_iter()
        .map(|shader| -> Result<_, Error> {
            let trial =
                compute::measure(&mut gpu, shader, &input, OUTPUT_BYTES, GROUPS, DISPATCHES)?;
            if trial.output.first_chunk::<16>() == trial.output.last_chunk::<16>()
                || !trial
                    .output
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|&v| f32::from_le_bytes(v).is_finite())
            {
                return Err(Error::Output(shader.entry));
            }
            Ok(serde_json::json!({
                "name": shader.entry,
                "spirv_blake3": blake3::hash(shader.spirv).to_hex().as_str(),
                "output_blake3": blake3::hash(&trial.output).to_hex().as_str(),
                "median_gpu_ms_per_frame": trial.median_gpu_ms_per_frame,
                "samples": trial.samples,
            }))
        })
        .collect::<Result<Vec<_>, _>>();
    let finish = gpu.finish();
    let workloads = workloads?;
    finish?;
    let mut executable = blake3::Hasher::new();
    executable.update_reader(std::fs::File::open(std::env::current_exe()?)?)?;
    let report = serde_json::json!({
        "device": adapter.name,
        "driver": adapter.driver_info,
        "slang": shader_tests::SLANG_VERSION,
        "source_blake3": shader_tests::SOURCE_BLAKE3,
        "shader_build_blake3": shader_tests::SHADER_BUILD_BLAKE3,
        "executable_blake3": executable.finalize().to_hex().as_str(),
        "resolution": EXTENT,
        "dispatches_per_sample": DISPATCHES,
        "input_blake3": blake3::hash(&input).to_hex().as_str(),
        "workloads": workloads,
    });
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)?;
    Ok(())
}
