//! 原生 4K 输入和输出的 5×5 空间重建，测量连续表面及含边界的场景。
//! 使用 float4 HDR、R32F 深度和 RGBA16F 材质导引，上传和回读在计时外。
#![forbid(unsafe_code)]

mod support;

use half::f16;
use simple_engine::{
    gpu_test::{self, Capability, Gpu},
    vulkan::{
        self,
        interop::RayShader,
        native::{Binding, Bindings, Buffer, Commands, ComputePipeline, Image, ImageFormat},
    },
    wgpu,
};
use std::{
    io::{self, Write},
    time::Instant,
};

const EXTENT: [u32; 2] = [3840, 2160];
const PIXELS: usize = EXTENT[0] as usize * EXTENT[1] as usize;
const FRAMES: u32 = 4;
const ROUNDS: usize = 9;
const PROGRAM: RayShader<'static> = RayShader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/denoise-spatial.spv")),
    entry: c"spatial_4k_main",
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Gpu(#[from] gpu_test::Error),
    #[error(transparent)]
    Vulkan(#[from] vulkan::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("空间重建的 GPU 计时无效")]
    Timing,
    #[error("{0} 的空间重建输出无效")]
    Output(&'static str),
}

struct Inputs {
    color: Vec<u8>,
    depth: Vec<u8>,
    normal: Vec<u8>,
    diffuse: Vec<u8>,
    specular: Vec<u8>,
}

impl Inputs {
    fn new(boundaries: bool) -> Self {
        let mut result = Self {
            color: Vec::with_capacity(PIXELS * 16),
            depth: Vec::with_capacity(PIXELS * 4),
            normal: Vec::with_capacity(PIXELS * 8),
            diffuse: Vec::with_capacity(PIXELS * 8),
            specular: Vec::with_capacity(PIXELS * 8),
        };
        let half_bytes = |channels: [f32; 4]| {
            channels
                .map(|value| f16::from_f32(value).to_le_bytes())
                .into_iter()
                .flatten()
        };
        let [width, height] = EXTENT;
        for y in 0..height {
            for x in 0..width {
                let sky = boundaries && y < height / 4;
                let side = boundaries && ((x / 21) ^ (y / 15)) & 1 != 0;
                let invalid = boundaries && (x ^ y) % 127 == 0;
                let noise = ((x.wrapping_mul(197) ^ y.wrapping_mul(87)) & 255) as f32 / 255.0;
                let alpha = if invalid { -1.0 } else { 1.0 };
                let depth = if sky {
                    f32::MAX
                } else if side {
                    24.0
                } else {
                    5.0
                };
                let nz = if sky { 0.0 } else { -1.0 };
                let flags = if sky { 0.0 } else { 1.0 / 255.0 };
                let gain = if side { 0.625 } else { 0.25 };
                result.color.extend(
                    [noise + 0.1, noise * 0.5 + gain, 0.25 + noise * 0.25, alpha]
                        .into_iter()
                        .flat_map(f32::to_le_bytes),
                );
                result.depth.extend(depth.to_le_bytes());
                result.normal.extend(half_bytes([0.0, 0.0, nz, gain]));
                result.diffuse.extend(half_bytes([gain, 0.35, 0.25, flags]));
                result.specular.extend(half_bytes([0.04, 0.06, 0.08, 0.0]));
            }
        }
        result
    }
}

fn measure(gpu: &mut Gpu, name: &'static str, input: Inputs) -> Result<serde_json::Value, Error> {
    let start = Instant::now();
    let context = gpu.context_mut();
    let input_color = context.zeroed_buffer(
        "空间重建输入",
        input.color.len() as u64,
        wgpu::BufferUsages::STORAGE,
    )?;
    context
        .queue()
        .write_buffer(input_color.buffer(), 0, &input.color);
    let output = context.zeroed_buffer(
        "空间重建输出",
        input.color.len() as u64,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    )?;
    let native = context.native();
    let mut upload = |format, bytes: &[u8]| -> Result<Binding, vulkan::Error> {
        let image = Image::new(native, [EXTENT[0], EXTENT[1], 1], format)?;
        image.upload(native, bytes)?;
        Ok(Binding::StorageImage(image))
    };
    let depth = upload(ImageFormat::R32Float, &input.depth)?;
    let normal = upload(ImageFormat::Rgba16Float, &input.normal)?;
    let diffuse = upload(ImageFormat::Rgba16Float, &input.diffuse)?;
    let specular = upload(ImageFormat::Rgba16Float, &input.specular)?;
    let parameters: Vec<_> = [EXTENT[0], EXTENT[1], 1.0f32.to_bits(), 1.0f32.to_bits()]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let frame = Buffer::upload(native, &parameters)?;
    let bindings = Bindings::new(
        native,
        &[
            Binding::StorageBuffer(frame),
            Binding::SharedBuffer(input_color),
            depth,
            normal,
            diffuse,
            specular,
            Binding::SharedBuffer(output.clone()),
        ],
    )?;
    let pipeline = ComputePipeline::new(native, bindings, PROGRAM)?;
    let setup_seconds = start.elapsed().as_secs_f64();
    let mut hash = blake3::Hasher::new();
    for bytes in [
        &input.color,
        &input.depth,
        &input.normal,
        &input.diffuse,
        &input.specular,
        &parameters,
    ] {
        hash.update(bytes);
    }
    drop(input);
    let record_frame = |commands: &mut Commands| {
        commands.barrier();
        commands.dispatch(pipeline, [EXTENT[0] / 8, EXTENT[1] / 8, 1])
    };
    for _ in 0..3 {
        support::native_frames(gpu, FRAMES, record_frame)?;
    }
    let mut samples = Vec::with_capacity(ROUNDS);
    let mut frame_ms = [0.0; ROUNDS];
    for elapsed in &mut frame_ms {
        let (sample, record_seconds) = support::native_frames(gpu, FRAMES, record_frame)?;
        if !sample.gpu_seconds.is_finite() || sample.gpu_seconds <= 0.0 {
            return Err(Error::Timing);
        }
        *elapsed = sample.gpu_seconds * 1000.0 / f64::from(FRAMES);
        samples.push(serde_json::json!({
            "gpu_seconds": sample.gpu_seconds,
            "host_seconds": sample.host_seconds,
            "record_seconds": record_seconds,
        }));
    }
    frame_ms.sort_unstable_by(f64::total_cmp);
    let bytes = gpu.readback(output.buffer())?;
    gpu.context_mut().native().collect(&[])?;
    if !bytes
        .as_chunks::<4>()
        .0
        .iter()
        .all(|&bytes| f32::from_le_bytes(bytes).is_finite())
        || !bytes
            .as_chunks::<16>()
            .0
            .iter()
            .any(|pixel| pixel[12..16] == 1.0f32.to_le_bytes())
    {
        return Err(Error::Output(name));
    }
    Ok(serde_json::json!({
        "name": name,
        "input_blake3": hash.finalize().to_hex().as_str(),
        "output_blake3": blake3::hash(&bytes).to_hex().as_str(),
        "setup_seconds": setup_seconds,
        "median_gpu_ms_per_frame": frame_ms[ROUNDS / 2],
        "samples": samples,
    }))
}

fn main() -> Result<(), Error> {
    if !std::env::args_os().any(|argument| argument == "--bench") {
        return Ok(());
    }
    let mut gpu = Gpu::new(Capability::Compute, 0, true)?;
    let adapter = gpu.context().adapter_info();
    let workloads = [("surface", false), ("boundaries", true)]
        .into_iter()
        .map(|(name, boundaries)| measure(&mut gpu, name, Inputs::new(boundaries)))
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
        "dispatches_per_sample": FRAMES,
        "spirv_blake3": blake3::hash(PROGRAM.spirv).to_hex().as_str(),
        "workloads": workloads,
    });
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)?;
    Ok(())
}
