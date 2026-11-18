// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

//! 4K 原色、sRGB、HSV 和 Prime DRT 的 GPU 时间。各样本重复派发同一图像，
//! 前三项对每个像素的 64 个曝光级分别做一次往返，Prime DRT 对每个像素执行一次。
//! 输入上传、管线构建及结果核验在 GPU 计时外，仍保留主机准备和提交时间。
#![forbid(unsafe_code)]

#[path = "support/compute.rs"]
mod compute;

use simple_engine::{
    display::{self, PrimeDrtSettings},
    gpu_test::{self, Capability, Gpu, Shader},
};
use std::io::{self, Write};

const WIDTH: u32 = 3840;
const HEIGHT: u32 = 2160;
const WORKGROUP: [u32; 3] = [16, 16, 1];
const GROUPS: [u32; 3] = [WIDTH / WORKGROUP[0], HEIGHT / WORKGROUP[1], 1];
const FRAME_BYTES: usize = WIDTH as usize * HEIGHT as usize * size_of::<[f32; 4]>();
const DISPATCHES: u32 = 16;
const WORKLOADS: [Shader<'static>; 4] = [
    shader(
        include_bytes!(concat!(env!("OUT_DIR"), "/color-primaries.spv")),
        "primaries_main",
    ),
    shader(
        include_bytes!(concat!(env!("OUT_DIR"), "/color-srgb.spv")),
        "srgb_main",
    ),
    shader(
        include_bytes!(concat!(env!("OUT_DIR"), "/color-hsv.spv")),
        "hsv_main",
    ),
    shader(
        include_bytes!(concat!(env!("OUT_DIR"), "/color-prime-drt.spv")),
        "prime_drt_main",
    ),
];

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Gpu(#[from] gpu_test::Error),
    #[error(transparent)]
    Display(#[from] display::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Measurement(#[from] compute::Error),
    #[error("{0} 的 GPU 输出无效")]
    Output(&'static str),
}

const fn shader(spirv: &'static [u8], entry: &'static str) -> Shader<'static> {
    Shader {
        spirv,
        entry,
        workgroup: WORKGROUP,
    }
}

fn frame() -> Result<Vec<u8>, display::Error> {
    let parameters = PrimeDrtSettings::DEFAULT.prepare(1.0)?.to_array();
    let mut input = Vec::with_capacity(FRAME_BYTES + size_of_val(&parameters));
    input.extend(parameters.into_iter().flat_map(f32::to_le_bytes));
    let band_height = HEIGHT / 4;
    for y in 0..HEIGHT {
        let gain = [0.004, 0.18, 1.0, 16.0][(y / band_height) as usize];
        let green = (y % band_height) as f32 / band_height as f32 - 0.0625;
        for x in 0..WIDTH {
            let red = x as f32 / WIDTH as f32;
            let blue = ((x ^ y) % 256) as f32 / 255.0;
            let [r, g, b] = if x < WIDTH / 8 {
                [red; 3]
            } else {
                [red, green, blue]
            };
            input.extend(
                [r * gain, g * gain, b * gain, 0.0]
                    .into_iter()
                    .flat_map(f32::to_le_bytes),
            );
        }
    }
    Ok(input)
}

fn measure(
    gpu: &mut Gpu,
    shader: Shader<'static>,
    input: &[u8],
) -> Result<serde_json::Value, Error> {
    let measurement = compute::measure(gpu, shader, input, FRAME_BYTES, GROUPS, DISPATCHES)?;
    let output = measurement.output;
    if output.first_chunk::<16>() == output.last_chunk::<16>()
        || !output
            .as_chunks::<4>()
            .0
            .iter()
            .all(|&value| f32::from_le_bytes(value).is_finite())
    {
        return Err(Error::Output(shader.entry));
    }
    Ok(serde_json::json!({
        "name": shader.entry,
        "spirv_blake3": blake3::hash(shader.spirv).to_hex().as_str(),
        "output_blake3": blake3::hash(&output).to_hex().as_str(),
        "median_gpu_ms_per_frame": measurement.median_gpu_ms_per_frame,
        "samples": measurement.samples,
    }))
}

fn main() -> Result<(), Error> {
    // Cargo 的全目标测试也会运行无 harness 的目标。
    if !std::env::args_os().any(|argument| argument == "--bench") {
        return Ok(());
    }
    let input = frame()?;
    let mut gpu = Gpu::new(Capability::Compute, 0, true)?;
    let adapter = gpu.context().adapter_info();
    let workloads = WORKLOADS
        .into_iter()
        .map(|shader| measure(&mut gpu, shader, &input))
        .collect::<Result<Vec<_>, _>>();
    let stats = gpu.stats().clone();
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
        "resolution": [WIDTH, HEIGHT],
        "dispatches_per_sample": DISPATCHES,
        "input_blake3": blake3::hash(&input).to_hex().as_str(),
        "input_bytes": input.len(),
        "output_bytes": FRAME_BYTES,
        "init_seconds": stats.init_seconds,
        "pipeline_seconds": stats.pipeline_seconds,
        "gpu_buffer_capacity_bytes": stats.buffer_capacity_bytes,
        "workloads": workloads,
    });
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)?;
    Ok(())
}
