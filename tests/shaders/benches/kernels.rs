//! 4K 纯计算 shader 的定向负载。各入口自行生成有代表性的参数，
//! GPU 时间包括派发与同步，输入上传和结果读回在计时外。
#![forbid(unsafe_code)]

#[path = "support/compute.rs"]
mod compute;

use simple_engine::gpu_test::{self, Capability, Gpu, Shader};
use std::io::{self, Write};

const EXTENT: [u32; 2] = [3840, 2160];
const WORKGROUP: [u32; 3] = [8, 8, 1];
const GROUPS: [u32; 3] = [EXTENT[0] / WORKGROUP[0], EXTENT[1] / WORKGROUP[1], 1];
const OUTPUT_BYTES: usize = EXTENT[0] as usize * EXTENT[1] as usize * 16;
const DISPATCHES: u32 = 8;
const SEED: u32 = 0xdcc9_53b7;

const KERNELS: &[(&str, Shader<'static>)] = &[
    (
        "transport",
        shader(
            include_bytes!(concat!(env!("OUT_DIR"), "/bench-transport.spv")),
            "mis_main",
        ),
    ),
    (
        "bsdf",
        shader(
            include_bytes!(concat!(env!("OUT_DIR"), "/bench-bsdf.spv")),
            "lambert_main",
        ),
    ),
];

const fn shader(spirv: &'static [u8], entry: &'static str) -> Shader<'static> {
    Shader {
        spirv,
        entry,
        workgroup: WORKGROUP,
    }
}

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
    #[error("指定一个已有的 benchmark 目录名")]
    Argument,
    #[error("{0} 的 GPU 输出无效")]
    Output(&'static str),
}

fn main() -> Result<(), Error> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.iter().any(|arg| arg == "--bench") {
        return Ok(());
    }
    let mut filters = args.iter().filter(|arg| *arg != "--bench");
    let filter = filters.next();
    if filters.next().is_some()
        || filter.is_some_and(|name| !KERNELS.iter().any(|(folder, _)| folder == name))
    {
        return Err(Error::Argument);
    }
    let input: Vec<_> = [SEED, 0, 0, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let mut gpu = Gpu::new(Capability::Compute, 0, true)?;
    let adapter = gpu.context().adapter_info();
    let workloads = KERNELS
        .iter()
        .filter(|(folder, _)| filter.is_none_or(|name| folder == name))
        .map(|&(folder, shader)| -> Result<_, Error> {
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
                "name": format!("{folder}/{}", shader.entry),
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
