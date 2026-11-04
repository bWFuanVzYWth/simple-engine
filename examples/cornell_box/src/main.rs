//! Cornell box 离屏参考渲染；场景、积分策略和输出由应用所有。
//!
//! # 参考定义
//!
//! 实测毫米几何、相机和单面矩形灯来自
//! [Cornell 数据](https://bowers.cornell.edu/computer-graphics/data)，RGB 反射率与灯辐射值采用
//! [Ray Tracing: The Rest of Your Life](https://raytracing.github.io/books/RayTracingTheRestOfYourLife.html)。
//! 这是 linear sRGB 的 RGB 近似，不与实测光谱或 CCD 照片作直接定量比较。
//!
//! 全部表面 Lambertian、黑色环境；面光源 NEE、power heuristic MIS 与 Russian roulette，
//! 灯面继续反射，不使用降噪或缓存。float32 传输及顺序累加；几何射线采用引擎误差界，
//! 连接方向不归一化，PDF 使用未偏移点。路径与阴影在 raygen 顺序追踪，hit/miss 不递归。
//! 深度护栏触顶或出现非有限辐射时整次失败，不保存截断估计。
//!
//! 固定 seed、像素与绝对样本索引决定采样序列，batch 仅控制 dispatch 分组；
//! 同设备/构建保证分批等价，不承诺跨 GPU 或编译器逐位一致。原生路径直接执行 Slang SPIR-V；
//! 每批上传/提交，最终统一等待和读回，主机计时包含这些成本，不能视为纯 GPU 时间。
//! shader_blake3 包含入口及共享模块，spirv_blake3 按 raygen/miss/closest-hit 记录实际字节码。
#![forbid(unsafe_code)]

#[cfg(all(feature = "mimalloc", not(target_arch = "wasm32")))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod output;
mod scene;

use clap::Parser;
use simple_engine::{gpu_log, vulkan, wgpu};
use std::{path::PathBuf, process::ExitCode, sync::mpsc, time::Instant};
use vulkan::interop::{RayShader, RayTracingPipeline, SharedBuffer, WgpuContext};

#[derive(Parser)]
#[command(about = "Cornell box 离屏参考渲染器")]
struct Args {
    #[arg(long, default_value_t = 512, value_parser = clap::value_parser!(u32).range(1..))]
    size: u32,
    #[arg(long, default_value_t = 1024, value_parser = clap::value_parser!(u32).range(1..))]
    samples: u32,
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u32).range(1..))]
    batch: u32,
    #[arg(long, default_value_t = 0)]
    seed: u32,
    /// 路径安全上限；触顶报错，不输出截断图像。
    #[arg(long, default_value_t = 4096, value_parser = clap::value_parser!(u32).range(1..))]
    max_depth: u32,
    /// Vulkan 设备枚举索引。
    #[arg(long, default_value_t = 0)]
    device: usize,
    /// 输出文件前缀。
    #[arg(long, default_value = "target/cornell_box/reference")]
    output: PathBuf,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    GpuLog(#[from] gpu_log::Error),
    #[error(transparent)]
    Vulkan(#[from] vulkan::Error),
    #[error("等待 GPU 失败：{0}")]
    Poll(#[from] wgpu::PollError),
    #[error("映射回读失败：{0}")]
    Map(#[from] wgpu::BufferAsyncError),
    #[error("访问映射范围失败：{0}")]
    MapRange(#[from] wgpu::MapRangeError),
    #[error("回读回调中断：{0}")]
    Callback(#[from] mpsc::RecvError),
    #[error("图像尺寸过大")]
    ImageSize,
    #[error("像素 {0} 达到路径深度上限；请提高 --max-depth")]
    PathLimit(usize),
    #[error("像素 {0} 出现非有限值或无效辐射")]
    InvalidRadiance(usize),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("EXR 编码失败：{0}")]
    Exr(#[from] exr::error::Error),
    #[error("PNG 编码失败：{0}")]
    Png(#[from] png::EncodingError),
    #[error("元数据编码失败：{0}")]
    Metadata(#[from] serde_json::Error),
}

struct Renderer {
    context: WgpuContext,
    pipeline: RayTracingPipeline,
    parameters: SharedBuffer,
    accumulation: SharedBuffer,
}

impl Renderer {
    fn new(args: &Args) -> Result<Self, Error> {
        gpu_log::init()?;
        let mut context = pollster::block_on(WgpuContext::with_index(args.device, true))?;
        let device = context.device();
        let triangles = context.buffer(
            "triangles",
            &scene::triangles_bytes(),
            wgpu::BufferUsages::STORAGE,
        )?;
        let materials = context.buffer(
            "materials",
            &scene::materials_bytes(),
            wgpu::BufferUsages::STORAGE,
        )?;
        let parameters = context.buffer(
            "parameters",
            &[0; 144],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        )?;
        let bytes = u64::from(args.size)
            .checked_mul(u64::from(args.size))
            .and_then(|pixels| pixels.checked_mul(16))
            .filter(|&bytes| {
                bytes <= device.limits().max_buffer_size
                    && bytes <= device.limits().max_storage_buffer_binding_size
            })
            .ok_or(Error::ImageSize)?;
        let accumulation = context.buffer(
            "accumulation",
            &vec![0; bytes as usize],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        )?;
        let geometry: Vec<_> = scene::triangles()
            .map(|triangle| triangle.map(|vertex| [vertex[0], vertex[1], vertex[2]]))
            .collect();
        let scene = context.acceleration_structure(&geometry)?;
        let pipeline = RayTracingPipeline::new_buffers(
            &mut context,
            scene,
            [
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/render.spv")),
                    entry: c"render_main",
                },
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/miss.spv")),
                    entry: c"miss_main",
                },
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/closest_hit.spv")),
                    entry: c"closest_hit_main",
                },
            ],
            &[
                triangles,
                materials,
                parameters.clone(),
                accumulation.clone(),
            ],
        )?;
        Ok(Self {
            context,
            pipeline,
            parameters,
            accumulation,
        })
    }

    fn render(&mut self, args: &Args) -> Result<Vec<[f32; 3]>, Error> {
        let mut first = 0;
        while first < args.samples {
            let count = args.batch.min(args.samples - first);
            self.context.queue().write_buffer(
                self.parameters.buffer(),
                0,
                &parameters(args, first, count),
            );
            let command = self
                .pipeline
                .encode(&mut self.context, [args.size, args.size, 1])?;
            self.context.submit([command])?;
            first += count;
        }
        let bytes = readback(&self.context, self.accumulation.buffer())?;
        gpu_log::check()?;
        pixels(&bytes, args.samples)
    }
}

fn readback(context: &WgpuContext, source: &wgpu::Buffer) -> Result<Vec<u8>, Error> {
    let device = context.device();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &buffer, 0, source.size());
    let submission = context.queue().submit([encoder.finish()]);
    let (sender, receiver) = mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    receiver.recv()??;
    let bytes = buffer.slice(..).get_mapped_range()?.to_vec();
    buffer.unmap();
    Ok(bytes)
}

// std430：两个 uint4，随后为相机和灯面的七个 float4。
fn parameters(args: &Args, first: u32, count: u32) -> [u8; 144] {
    let mut bytes = [0; 144];
    let mut words = bytes.as_chunks_mut::<4>().0.iter_mut();
    for value in [
        args.size,
        args.size,
        first,
        count,
        args.seed,
        args.max_depth,
        scene::TRIANGLE_COUNT as u32,
        scene::MATERIAL_COUNT as u32,
    ] {
        words.next().unwrap().copy_from_slice(&value.to_le_bytes());
    }
    for (vector, w) in [
        (scene::CAMERA_ORIGIN, 0.0),
        (scene::CAMERA_FORWARD, 0.0),
        (scene::CAMERA_RIGHT, 0.0),
        (scene::CAMERA_UP, 0.0),
        (scene::LIGHT_CORNER, scene::LIGHT as f32),
        (scene::LIGHT_EDGE_U, 0.0),
        (scene::LIGHT_EDGE_V, 0.0),
    ] {
        for value in [vector[0], vector[1], vector[2], w] {
            words.next().unwrap().copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

fn pixels(bytes: &[u8], samples: u32) -> Result<Vec<[f32; 3]>, Error> {
    let mut pixels = Vec::with_capacity(bytes.len() / 16);
    for (index, pixel) in bytes.as_chunks::<16>().0.iter().enumerate() {
        let values: [f32; 4] = std::array::from_fn(|i| {
            f32::from_le_bytes(pixel[i * 4..i * 4 + 4].try_into().unwrap())
        });
        if values[3] == 1.0 {
            return Err(Error::PathLimit(index));
        }
        if values[3] != 0.0 || values[..3].iter().any(|v| !v.is_finite() || *v < 0.0) {
            return Err(Error::InvalidRadiance(index));
        }
        pixels.push(std::array::from_fn(|i| {
            (f64::from(values[i]) / f64::from(samples)) as f32
        }));
    }
    Ok(pixels)
}

fn run(args: Args) -> Result<(), Error> {
    let mut renderer = Renderer::new(&args)?;
    let info = renderer.context.adapter_info();
    let device = &info.name;
    println!(
        "{device}：{}×{}，{} spp",
        args.size, args.size, args.samples
    );
    let started = Instant::now();
    let pixels = renderer.render(&args)?;
    let seconds = started.elapsed().as_secs_f64();
    let mean: f64 = pixels
        .iter()
        .map(|p| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]))
        .sum::<f64>()
        / pixels.len() as f64;
    let mut scene_hash = blake3::Hasher::new();
    scene_hash.update(&scene::triangles_bytes());
    scene_hash.update(&scene::materials_bytes());
    scene_hash.update(&parameters(&args, 0, args.samples)[32..]);
    let mut executable_hash = blake3::Hasher::new();
    executable_hash.update_reader(std::fs::File::open(std::env::current_exe()?)?)?;
    let mut spirv_hash = blake3::Hasher::new();
    for module in [
        include_bytes!(concat!(env!("OUT_DIR"), "/render.spv")).as_slice(),
        include_bytes!(concat!(env!("OUT_DIR"), "/miss.spv")),
        include_bytes!(concat!(env!("OUT_DIR"), "/closest_hit.spv")),
    ] {
        spirv_hash.update(module);
    }
    let metadata = serde_json::json!({
        "scene": "Cornell measured geometry, RGB diffuse approximation",
        "geometry_source": "https://bowers.cornell.edu/computer-graphics/data",
        "materials_source": "https://raytracing.github.io/books/RayTracingTheRestOfYourLife.html",
        "integrator": "Lambertian path tracing, area-light NEE, power MIS, Russian roulette",
        "color_space": "linear sRGB / D65",
        "precision": "Vulkan hardware triangle intersection, float32 transport and sequential accumulation",
        "size": args.size, "samples": args.samples, "seed": args.seed,
        "max_depth_guard": args.max_depth, "batch": args.batch,
        "device": device, "vendor_id": info.vendor, "device_id": info.device,
        "driver": info.driver, "driver_info": info.driver_info,
        "slang_version": env!("SLANG_VERSION"),
        "scene_blake3": scene_hash.finalize().to_hex().as_str(),
        "shader_blake3": env!("CORNELL_SHADER_BLAKE3"),
        "spirv_blake3": spirv_hash.finalize().to_hex().as_str(),
        "dependency_lock_blake3": blake3::hash(include_bytes!("../../../Cargo.lock")).to_hex().as_str(),
        "executable_blake3": executable_hash.finalize().to_hex().as_str(),
        "backend": "Slang SPIR-V -> Vulkan KHR ray tracing pipeline / wgpu interop",
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "render_wall_seconds": seconds, "mean_luminance": mean,
        "png": "sRGB encoded, clipped to [0,1]; EXR is unclipped float32",
    });
    output::write(&args.output, args.size, &pixels, &metadata)?;
    println!(
        "完成：{}.exr / .png / .json（{seconds:.2} 秒）",
        args.output.display()
    );
    Ok(())
}

fn main() -> ExitCode {
    match gpu_log::checked(|| run(Args::parse())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "需要 Vulkan KHR 光追管线"]
    fn batching_preserves_samples() {
        gpu_log::checked(|| {
            let mut args = Args::parse_from([
                "cornell_box",
                "--size",
                "16",
                "--samples",
                "8",
                "--batch",
                "1",
            ]);
            let mut renderer = Renderer::new(&args).unwrap();
            let single = renderer.render(&args).unwrap();
            args.batch = 4;
            assert_eq!(single, renderer.render(&args).unwrap());
            Ok::<_, Error>(())
        })
        .unwrap();
    }

    #[test]
    #[ignore = "需要 Vulkan KHR 光追管线"]
    fn depth_guard_rejects_truncated_paths() {
        gpu_log::checked(|| {
            let args = Args::parse_from([
                "cornell_box",
                "--size",
                "8",
                "--samples",
                "1",
                "--max-depth",
                "1",
            ]);
            assert!(matches!(
                Renderer::new(&args).unwrap().render(&args),
                Err(Error::PathLimit(_))
            ));
            Ok::<_, Error>(())
        })
        .unwrap();
    }
}
