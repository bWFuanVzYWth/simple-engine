//! 原生 4K 泛光。GPU 时间戳覆盖生产录制器的高光提取、卷积、裁边和合成。
//! 加法核共用线性 HDR 输入，两种标定光瞳与快速圆形模式使用同一太阳盘。输入准备、初始化、录制和回读分别计量。
#![forbid(unsafe_code)]

mod support;

use simple_engine::fft_bloom::gpu::PROGRAMS;
use simple_engine::{
    circular_bloom,
    fft_bloom::{
        self as bloom, Aperture, Kernel, Settings,
        gpu::{Filter, Lens},
    },
    gpu_test::{self, Capability, Gpu},
    vulkan::{
        self,
        native::{Image, ImageFormat},
    },
};
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

const EXTENT: [u32; 2] = [3840, 2160];
const STAR_RADIUS: u32 = 64;
const FRAMES: u32 = 4;
const ROUNDS: usize = 9;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Gpu(#[from] gpu_test::Error),
    #[error(transparent)]
    Vulkan(#[from] vulkan::Error),
    #[error(transparent)]
    Parameters(#[from] bloom::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("泛光 GPU 计时无效")]
    Timing,
    #[error("泛光输出校验失败：{0}")]
    Output(&'static str),
    #[error(
        "核宽 {width}，像素 ({x},{y}) 通道 {channel} 的 GPU 输出 {actual}，空间卷积为 {reference}"
    )]
    Probe {
        width: usize,
        x: usize,
        y: usize,
        channel: usize,
        actual: f32,
        reference: f64,
    },
}

fn scene() -> Vec<[f32; 4]> {
    let [width, height] = EXTENT;
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let grain = ((x ^ y) % 32) as f32 / 32.0;
            // 四边留出最大核半径，可直接用全图通量校验归一化。
            let interior = (STAR_RADIUS..width - STAR_RADIUS).contains(&x)
                && (STAR_RADIUS..height - STAR_RADIUS).contains(&y);
            pixels.push(if interior && x % 163 < 13 && y % 107 < 7 {
                [32.0, 8.0 + grain, 12.0 * grain, 1.0]
            } else {
                [0.0625 + 0.125 * grain, 0.25 * grain, 0.375, 1.0]
            });
        }
    }
    pixels
}

fn star_kernel() -> Result<Kernel, bloom::Error> {
    let radius = STAR_RADIUS as i32;
    let side = 2 * STAR_RADIUS + 1;
    let mut weights = Vec::with_capacity((side * side) as usize);
    for y in -radius..=radius {
        for x in -radius..=radius {
            weights.push(if x == 0 || y == 0 || x.abs() == y.abs() {
                1.0 / (1 + x.abs().max(y.abs())) as f32
            } else {
                0.0
            });
        }
    }
    Kernel::new([side; 2], weights)
}

fn verify(pixels: &[[f32; 4]], output: &[u8], settings: Settings) -> Result<[f64; 3], Error> {
    if output.len() != size_of_val(pixels) {
        return Err(Error::Output("输出尺寸不同"));
    }
    let [threshold, intensity] = settings.to_array().map(f64::from);
    let mut expected = [0.0; 3];
    let optical = settings == Settings::OPTICAL;
    let mut measured = [0.0; 3];
    let mut halo = false;
    for (base, &bytes) in pixels.iter().zip(output.as_chunks::<16>().0) {
        let result: [f32; 4] = bytemuck::cast(bytes);
        if !result.iter().all(|v| v.is_finite()) || result[3] != base[3] {
            return Err(Error::Output("非有限颜色或 alpha 改变"));
        }
        let peak = f64::from(base[0].max(base[1]).max(base[2]));
        let gain = if peak > threshold {
            intensity * (1.0 - threshold / peak)
        } else {
            0.0
        };
        for c in 0..3 {
            let delta = f64::from(result[c]) - f64::from(base[c]);
            measured[c] += if optical { f64::from(result[c]) } else { delta };
            expected[c] += if optical {
                f64::from(base[c])
            } else {
                gain * f64::from(base[c])
            };
            halo |= peak <= threshold && delta > 0.001;
        }
    }
    let relative = std::array::from_fn(|c| (measured[c] - expected[c]) / expected[c]);
    if !halo || !relative.iter().all(|v| v.abs() < 1e-4) {
        return Err(Error::Output("高光扩散或各通道总通量不符"));
    }
    Ok(relative)
}

/// 在亮点、FFT 分块的上下边缘和末尾条带，以 f64 直接空间卷积核对像素位置。
fn verify_optical_probes(pixels: &[[f32; 4]], output: &[u8], kernel: &Kernel) -> Result<(), Error> {
    let [width, height] = EXTENT.map(|n| n as usize);
    let [kw, kh] = kernel.extent().map(|n| n as usize);
    let probes = if kw > 513 {
        let [cx, cy] = [width / 2, height / 2];
        [
            [cx, cy],
            [cx, cy - 257],
            [cx, cy - 480],
            [2175, cy],
            [2176, cy],
            [cx, cy - 930],
        ]
    } else {
        [
            [523, 383],
            [1046, 766],
            [523, 511],
            [523, 512],
            [1569, 1535],
            [523, 2048],
        ]
    };
    for [x, y] in probes {
        let actual: [f32; 4] = bytemuck::cast(output.as_chunks::<16>().0[y * width + x]);
        for (c, weights) in kernel.channels().into_iter().enumerate() {
            let mut reference = 0.0;
            for (ky, row) in weights.chunks_exact(kw).enumerate() {
                let Some(sy) = (y + kh / 2).checked_sub(ky).filter(|&sy| sy < height) else {
                    continue;
                };
                for (kx, &weight) in row.iter().enumerate() {
                    if let Some(sx) = (x + kw / 2).checked_sub(kx).filter(|&sx| sx < width) {
                        reference += f64::from(pixels[sy * width + sx][c]) * f64::from(weight);
                    }
                }
            }
            if (f64::from(actual[c]) - reference).abs() > 0.0002 + 2e-6 * reference.abs() {
                return Err(Error::Probe {
                    width: kw,
                    x,
                    y,
                    channel: c,
                    actual: actual[c],
                    reference,
                });
            }
        }
    }
    Ok(())
}

enum Workload {
    Kernel(Settings, fn() -> Result<Kernel, bloom::Error>),
    Lens(Aperture),
    CircularSun,
    CircularTexture,
}

fn measure(
    gpu: &mut Gpu,
    input: Image,
    pixels: &[[f32; 4]],
    name: &str,
    workload: Workload,
) -> Result<serde_json::Value, Error> {
    let start = Instant::now();
    let (kernel, settings) = match workload {
        Workload::Kernel(settings, make_kernel) => (Some(make_kernel()?), settings),
        Workload::Lens(shape) => (Some(shape.kernel()?), Settings::OPTICAL),
        Workload::CircularSun | Workload::CircularTexture => (None, Settings::OPTICAL),
    };
    let kernel_seconds = start.elapsed().as_secs_f64();
    let native = gpu.context_mut().native();
    let before = native.allocated_bytes();
    let prepare = Instant::now();
    let (bloom, rectangle, mut roots, filter_seconds, bind_seconds) = match workload {
        Workload::Kernel(..) => {
            let mut filter = Filter::new(
                native,
                PROGRAMS,
                kernel.as_ref().expect("自定义核已加载"),
                settings,
            )?;
            let prepared = prepare.elapsed().as_secs_f64();
            let bind = Instant::now();
            let view = filter.bind(native, input)?;
            let rectangle = view.fft_rectangle();
            (
                view.into(),
                rectangle,
                filter.roots().collect::<Vec<_>>(),
                prepared,
                bind.elapsed().as_secs_f64(),
            )
        }
        Workload::Lens(shape) => {
            let mut lens = Lens::new(native, PROGRAMS)?;
            lens.select(shape);
            let prepared = prepare.elapsed().as_secs_f64();
            let bind = Instant::now();
            let view = lens.bind(native, input)?;
            let rectangle = view.fft_rectangle();
            (
                view.into(),
                rectangle,
                lens.roots().collect(),
                prepared,
                bind.elapsed().as_secs_f64(),
            )
        }
        Workload::CircularSun | Workload::CircularTexture => {
            let filter = circular_bloom::Filter::new(native)?;
            let prepared = prepare.elapsed().as_secs_f64();
            let bind = Instant::now();
            let view = filter.bind(native, input)?;
            (
                view,
                None,
                filter.roots().collect(),
                prepared,
                bind.elapsed().as_secs_f64(),
            )
        }
    };
    let bytes_before_collection = native.allocated_bytes() - before;
    roots.extend(bloom.roots());
    native.collect(&roots)?;
    let resident_bytes = native.allocated_bytes() - before;
    let setup_seconds = start.elapsed().as_secs_f64();
    let record = |commands: &mut _| bloom.record_into(commands);
    let warmup = Instant::now();
    while warmup.elapsed() < Duration::from_millis(800) {
        support::native_frames(gpu, FRAMES, record)?;
    }
    let mut frame_ms = [0.0; ROUNDS];
    let mut samples = Vec::with_capacity(ROUNDS);
    for elapsed in &mut frame_ms {
        let (sample, record_seconds) = support::native_frames(gpu, FRAMES, record)?;
        if !sample.gpu_seconds.is_finite() || sample.gpu_seconds <= 0.0 {
            return Err(Error::Timing);
        }
        *elapsed = 1000.0 * sample.gpu_seconds / f64::from(FRAMES);
        samples.push(serde_json::json!({
            "gpu_seconds": sample.gpu_seconds,
            "host_seconds": sample.host_seconds,
            "record_seconds": record_seconds,
        }));
    }
    frame_ms.sort_unstable_by(f64::total_cmp);
    let start = Instant::now();
    let native = gpu.context_mut().native();
    let output = bloom.output().download(native)?;
    let readback_seconds = start.elapsed().as_secs_f64();
    native.collect(&[input.into()])?;
    let relative = verify(pixels, &output, settings)?;
    if let Some(dir) = std::env::var_os("BLOOM_BENCH_READBACK_DIR") {
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(format!("{name}.rgba32f")), &output)?;
    }
    if settings == Settings::OPTICAL {
        if let Some(kernel) = &kernel {
            verify_optical_probes(pixels, &output, kernel)?;
        } else if !matches!(workload, Workload::CircularTexture) {
            // 半径 9 的太阳盘覆盖 253 个像素，远场取宽带 Airy 的平均强度 s/(π³r³)。
            let [w, h, _] = input.extent().map(|n| n as usize);
            let center = h / 2 * w + w / 2;
            for r in [64usize, 128, 256] {
                let expected = 253.0 * 110_000. * (0.545 * 4. / 5.)
                    / (std::f64::consts::PI * r as f64).powi(3);
                for at in [center + r, center + r * w] {
                    let pixel: [f32; 4] = bytemuck::cast(output.as_chunks::<16>().0[at]);
                    if (f64::from(pixel[1]) / expected - 1.).abs() > 0.30 {
                        return Err(Error::Probe {
                            width: 0,
                            x: at % w,
                            y: at / w,
                            channel: 1,
                            actual: pixel[1],
                            reference: expected,
                        });
                    }
                }
            }
        }
    }
    let kernel_hash = kernel.as_ref().map(|k| {
        let mut digest = blake3::Hasher::new();
        for channel in k.channels() {
            digest.update(bytemuck::cast_slice(channel));
        }
        digest.finalize().to_hex().to_string()
    });
    Ok(serde_json::json!({
        "name": name,
        "resolution": &input.extent()[..2],
        "optical": settings == Settings::OPTICAL,
        "input_blake3": blake3::hash(bytemuck::cast_slice(pixels)).to_hex().as_str(),
        "kernel_extent": kernel.as_ref().map(Kernel::extent),
        "fft_rectangle": rectangle,
        "dispatches_per_frame": bloom.dispatch_count(),
        "kernel_blake3": kernel_hash,
        "output_blake3": blake3::hash(&output).to_hex().as_str(),
        "relative_flux_error_rgb": relative,
        "resident_bytes_excluding_input": resident_bytes,
        "bytes_before_collection_excluding_input": bytes_before_collection,
        "setup_seconds": setup_seconds,
        "kernel_cpu_seconds": kernel_seconds,
        "filter_device_seconds": filter_seconds,
        "view_bind_seconds": bind_seconds,
        "readback_seconds": readback_seconds,
        "median_gpu_ms_per_frame": frame_ms[ROUNDS / 2],
        "samples": samples,
    }))
}

fn workloads(gpu: &mut Gpu) -> Result<serde_json::Value, Error> {
    let start = Instant::now();
    let mut pixels = scene();
    let native = gpu.context_mut().native();
    let input = Image::new(native, [EXTENT[0], EXTENT[1], 1], ImageFormat::Rgba32Float)?;
    input.upload(native, bytemuck::cast_slice(&pixels))?;
    let input_prepare_seconds = start.elapsed().as_secs_f64();
    let settings = Settings::new(1.0, 0.25)?;
    let gaussian = measure(
        gpu,
        input,
        &pixels,
        "gaussian_33",
        Workload::Kernel(settings, || Kernel::gaussian(16, 6.0)),
    )?;
    let star = measure(
        gpu,
        input,
        &pixels,
        "star_129",
        Workload::Kernel(settings, star_kernel),
    )?;
    let start = Instant::now();
    solar_disc(&mut pixels, EXTENT);
    input.upload(gpu.context_mut().native(), bytemuck::cast_slice(&pixels))?;
    let solar_input_prepare_seconds = start.elapsed().as_secs_f64();
    let camera = measure(
        gpu,
        input,
        &pixels,
        "camara_1825_sun",
        Workload::Lens(Aperture::Camera),
    )?;
    let pupil = measure(
        gpu,
        input,
        &pixels,
        "pupil_1825_sun",
        Workload::Lens(Aperture::Pupil),
    )?;
    let circular = measure(
        gpu,
        input,
        &pixels,
        "fast_circular_sun",
        Workload::CircularSun,
    )?;
    let adapter = gpu.context().adapter_info();
    let mut executable = blake3::Hasher::new();
    executable.update_reader(std::fs::File::open(std::env::current_exe()?)?)?;
    let spirv = [PROGRAMS.compact, PROGRAMS.wide].map(|set| {
        [
            set.kernel_rows,
            set.kernel_columns,
            set.forward_rows,
            set.filter_columns,
            set.composite_rows,
        ]
        .map(|shader| blake3::hash(shader.spirv).to_hex().to_string())
    });
    Ok(serde_json::json!({
        "device": adapter.name,
        "driver": adapter.driver_info,
        "slang": shader_tests::SLANG_VERSION,
        "source_blake3": shader_tests::SOURCE_BLAKE3,
        "shader_build_blake3": shader_tests::SHADER_BUILD_BLAKE3,
        "bloom_spirv_blake3": spirv,
        "rect_spirv_blake3": PROGRAMS.rectangular.into_iter().flat_map(|p| p.shaders())
            .map(|shader| blake3::hash(shader.spirv).to_hex().to_string()).collect::<Vec<_>>(),
        "executable_blake3": executable.finalize().to_hex().as_str(),
        "resolution": EXTENT,
        "frames_per_sample": FRAMES,
        "warmup_min_ms": 800,
        "threshold": settings.to_array()[0],
        "intensity": settings.to_array()[1],
        "circular_spirv_blake3": circular_bloom::Filter::SHADERS.map(|p| blake3::hash(p.spirv).to_hex().to_string()),
        "solar_input_prepare_seconds": solar_input_prepare_seconds,
        "input_prepare_seconds": input_prepare_seconds,
        "workloads": [gaussian, star, camera, pupil, circular],
    }))
}

fn solar_disc(pixels: &mut [[f32; 4]], extent: [u32; 2]) {
    let [w, h] = extent.map(|n| n as usize);
    let [cx, cy] = [w / 2, h / 2];
    pixels.fill([0., 0., 0., 1.]);
    for y in cy - 9..=cy + 9 {
        for x in cx - 9..=cx + 9 {
            if x.abs_diff(cx).pow(2) + y.abs_diff(cy).pow(2) <= 81 {
                pixels[y * w + x] = [120_000., 110_000., 95_000., 1.];
            }
        }
    }
}

/// 稀疏太阳盘与含纹理的全屏 HDR，分别测试实际用到的 4K 与 1440p 视图。
fn circular_cases(gpu: &mut Gpu) -> Result<serde_json::Value, Error> {
    let mut workloads = Vec::new();
    for [w, h] in [EXTENT, [2560, 1440]] {
        let input = Image::new(
            gpu.context_mut().native(),
            [w, h, 1],
            ImageFormat::Rgba32Float,
        )?;
        let mut pixels = vec![[0., 0., 0., 1.]; (w * h) as usize];
        solar_disc(&mut pixels, [w, h]);
        input.upload(gpu.context_mut().native(), bytemuck::cast_slice(&pixels))?;
        workloads.push(measure(
            gpu,
            input,
            &pixels,
            &format!("circular_sun_{w}x{h}"),
            Workload::CircularSun,
        )?);
        let border = h / 12;
        for y in border..h - border {
            for x in border..w - border {
                let hash = (y * w + x).wrapping_mul(0x85eb_ca6b);
                let grain = ((hash ^ hash.rotate_left(13)).wrapping_mul(0x27d4_eb2d) >> 8) as f32
                    / 16777216.;
                let light = if x % 163 < 13 && y % 107 < 7 {
                    200.
                } else {
                    1.
                };
                pixels[(y * w + x) as usize] = [
                    light * (0.1 + grain),
                    light * (0.2 + 0.6 * grain),
                    light * (1. - 0.5 * grain),
                    1.,
                ];
            }
        }
        input.upload(gpu.context_mut().native(), bytemuck::cast_slice(&pixels))?;
        workloads.push(measure(
            gpu,
            input,
            &pixels,
            &format!("circular_texture_{w}x{h}"),
            Workload::CircularTexture,
        )?);
        gpu.context_mut().native().collect(&[])?;
    }
    let mut executable = blake3::Hasher::new();
    executable.update_reader(std::fs::File::open(std::env::current_exe()?)?)?;
    let adapter = gpu.context().adapter_info();
    Ok(serde_json::json!({
        "device": adapter.name,
        "driver": adapter.driver_info,
        "slang": shader_tests::SLANG_VERSION,
        "source_blake3": shader_tests::SOURCE_BLAKE3,
        "executable_blake3": executable.finalize().to_hex().as_str(),
        "circular_spirv_blake3": circular_bloom::Filter::SHADERS
            .map(|program| blake3::hash(program.spirv).to_hex().to_string()),
        "frames_per_sample": FRAMES,
        "warmup_min_ms": 800,
        "workloads": workloads,
    }))
}

fn main() -> Result<(), Error> {
    if !std::env::args_os().any(|argument| argument == "--bench") {
        return Ok(());
    }
    let mut gpu = Gpu::new(Capability::Compute, 0, true)?;
    let report = if std::env::args_os().any(|arg| arg == "--circular-only") {
        circular_cases(&mut gpu)
    } else {
        workloads(&mut gpu)
    };
    let finish = gpu.finish();
    let report = report?;
    finish?;
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)?;
    Ok(())
}
