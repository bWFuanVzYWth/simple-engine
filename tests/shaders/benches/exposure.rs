//! 原生 4K 自动曝光。通过生产录制器计时清零、直方图和时域更新的整条设备路径。
//! 比较 buffer 与 texture 输入的均匀、空间混合和宽亮度分布。上传、录制和读回分别计量。
#![forbid(unsafe_code)]

mod support;

use simple_engine::{
    display::exposure::{
        self, Frame,
        gpu::{Exposure, Programs},
    },
    gpu_test::{self, Capability, Gpu},
    vulkan::{
        self,
        interop::RayShader,
        native::{Binding, Buffer, Image, ImageFormat},
    },
    wgpu,
};
use std::{
    io::{self, Write},
    time::Instant,
};

const EXTENT: [u32; 2] = [3840, 2160];
const FRAMES: u32 = 8;
const ROUNDS: usize = 9;
const BUFFER: RayShader<'static> = RayShader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/exposure.histogram.spv")),
    entry: c"histogram_main",
};
const TEXTURE: RayShader<'static> = RayShader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/exposure.histogram-texture.spv")),
    entry: c"histogram_texture_main",
};
const UPDATE: RayShader<'static> = RayShader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/exposure.update.spv")),
    entry: c"update_main",
};

#[derive(Clone, Copy)]
enum Scene {
    Uniform,
    Mixed,
    Wide,
}

#[derive(Clone, Copy)]
enum Input {
    Buffer,
    Texture,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Gpu(#[from] gpu_test::Error),
    #[error(transparent)]
    Vulkan(#[from] vulkan::Error),
    #[error(transparent)]
    Parameters(#[from] exposure::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("自动曝光的 GPU 计时无效")]
    Timing,
    #[error("{0} 的曝光状态无效")]
    Output(String),
}

fn pixels(scene: Scene) -> Vec<u8> {
    let [width, height] = EXTENT;
    let mut bytes = Vec::with_capacity((width * height) as usize * size_of::<[f32; 4]>());
    let levels: [f32; 256] =
        std::array::from_fn(|bin| (-16.0 + (bin as f32 + 0.5) * (36.0 / 256.0)).exp2());
    for y in 0..height {
        for x in 0..width {
            let pixel = match scene {
                Scene::Uniform => [0.18, 0.18, 0.18, 1.0],
                Scene::Mixed => {
                    let grain = ((x ^ y) % 256) as f32 / 255.0;
                    let gain = if y < height / 3 { 8.0 } else { 0.0625 };
                    let alpha = if y % 65 == 0 { -1.0 } else { 1.0 };
                    [
                        gain * (0.75 + 0.25 * grain),
                        gain * 0.5,
                        gain * (grain - 0.125),
                        alpha,
                    ]
                }
                Scene::Wide => {
                    let bin = x.wrapping_mul(197) ^ y.wrapping_mul(87);
                    let level = levels[(bin & 255) as usize];
                    [level, level, level, 1.0]
                }
            };
            bytes.extend(pixel.into_iter().flat_map(f32::to_le_bytes));
        }
    }
    bytes
}

fn measure(
    gpu: &mut Gpu,
    name: String,
    input: Input,
    pixels: &[u8],
    frame: Frame,
) -> Result<serde_json::Value, Error> {
    let start = Instant::now();
    let context = gpu.context_mut();
    let (binding, histogram) = match input {
        Input::Buffer => {
            let buffer = context.zeroed_buffer(
                "自动曝光 4K 输入",
                pixels.len() as u64,
                wgpu::BufferUsages::STORAGE,
            )?;
            context.queue().write_buffer(buffer.buffer(), 0, pixels);
            (Binding::SharedBuffer(buffer), BUFFER)
        }
        Input::Texture => {
            let native = context.native();
            let texture = Image::new(native, [EXTENT[0], EXTENT[1], 1], ImageFormat::Rgba32Float)?;
            texture.upload(native, pixels)?;
            (Binding::SampledImage(texture), TEXTURE)
        }
    };
    let params: Vec<_> = frame
        .to_words()
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let native = context.native();
    let parameters = Buffer::upload(native, &params)?;
    let exposure = Exposure::new(
        native,
        Programs {
            histogram,
            update: UPDATE,
        },
        binding,
        Binding::StorageBuffer(parameters),
        EXTENT,
    )?;
    let readback = Buffer::new(native, 16, true)?;
    let setup_seconds = start.elapsed().as_secs_f64();

    let record_frame = |commands: &mut _| exposure.record_into(commands);
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

    let native = gpu.context_mut().native();
    let mut state = [0; 16];
    exposure.state().copy_prefix_to(native, &readback, 16)?;
    readback.read(native, 0, &mut state)?;
    native.collect(&[])?;
    let chunks = state.as_chunks::<4>().0;
    let words: [u32; 4] = std::array::from_fn(|i| u32::from_le_bytes(chunks[i]));
    if words[1] != 1
        || ![words[0], words[2], words[3]]
            .into_iter()
            .map(f32::from_bits)
            .all(f32::is_finite)
        || words[3] == 0
    {
        return Err(Error::Output(name));
    }
    let mut input_hash = blake3::Hasher::new();
    input_hash.update(pixels).update(&params);
    Ok(serde_json::json!({
        "name": name,
        "input_blake3": input_hash.finalize().to_hex().as_str(),
        "output_blake3": blake3::hash(&state).to_hex().as_str(),
        "histogram_spirv_blake3": blake3::hash(histogram.spirv).to_hex().as_str(),
        "setup_seconds": setup_seconds,
        "median_gpu_ms_per_frame": frame_ms[ROUNDS / 2],
        "samples": samples,
    }))
}

fn workloads(gpu: &mut Gpu) -> Result<Vec<serde_json::Value>, Error> {
    let mut workloads = Vec::new();
    for (name, scene, bt709) in [
        ("uniform", Scene::Uniform, false),
        ("mixed", Scene::Mixed, true),
        ("wide", Scene::Wide, false),
    ] {
        let pixels = pixels(scene);
        let frame = Frame::new(EXTENT, bt709, 1.0 / 60.0, false, false, 0.65)?;
        for (layout, input) in [("buffer", Input::Buffer), ("texture", Input::Texture)] {
            workloads.push(measure(
                gpu,
                format!("{name}_{layout}"),
                input,
                &pixels,
                frame,
            )?);
        }
    }
    Ok(workloads)
}

fn main() -> Result<(), Error> {
    if !std::env::args_os().any(|argument| argument == "--bench") {
        return Ok(());
    }
    let mut gpu = Gpu::new(Capability::Compute, 0, true)?;
    let adapter = gpu.context().adapter_info();
    let workloads = workloads(&mut gpu);
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
        "update_spirv_blake3": blake3::hash(UPDATE.spirv).to_hex().as_str(),
        "workloads": workloads,
    });
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)?;
    Ok(())
}
