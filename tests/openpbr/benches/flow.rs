//! 同一 GPU 交替执行修改前后 SPIR-V；输入、pipeline 创建与数值核验在计时外。
//! GPU timestamp 包括 dispatch 边界；host 包括提交、等待与查询回读，另报录制成本。
#![forbid(unsafe_code)]

use simple_engine::{
    gpu_test::{Capability, Gpu},
    openpbr::{GpuEnergy, Material, Medium},
    wgpu,
};
use std::{path::PathBuf, time::Instant};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Args {
    before: PathBuf,
    after: PathBuf,
    output: PathBuf,
    profile: String,
    count: u32,
    dispatches: u32,
    pairs: u32,
}

fn args() -> Result<Args> {
    let mut args = Args {
        before: PathBuf::new(),
        after: PathBuf::new(),
        output: PathBuf::new(),
        profile: "opaque".into(),
        count: 65_536,
        dispatches: 64,
        pairs: 21,
    };
    let mut words = std::env::args().skip(1);
    while let Some(key) = words.next() {
        if key == "--bench" {
            continue;
        }
        let value = words.next().ok_or("参数缺少值")?;
        match key.as_str() {
            "--before" => args.before = value.into(),
            "--after" => args.after = value.into(),
            "--output" => args.output = value.into(),
            "--profile" => args.profile = value,
            "--count" => args.count = value.parse()?,
            "--dispatches" => args.dispatches = value.parse()?,
            "--pairs" => args.pairs = value.parse()?,
            _ => return Err(format!("未知参数 {key}").into()),
        }
    }
    if args.before.as_os_str().is_empty()
        || args.after.as_os_str().is_empty()
        || args.output.as_os_str().is_empty()
        || !(1..=1_048_576).contains(&args.count)
        || !(1..=4096).contains(&args.dispatches)
        || !(3..=1001).contains(&args.pairs)
        || ![
            "opaque",
            "opaque-smooth",
            "subsurface",
            "sss-only",
            "conductor",
            "solid",
            "thin",
            "delta-conductor",
            "delta-solid",
            "delta-thin",
            "mixed",
            "mixed-sss",
            "mixed-smooth",
        ]
        .contains(&args.profile.as_str())
    {
        return Err("需要 before/after/output 与有效批量参数".into());
    }
    Ok(args)
}

fn input(profile: &str, count: u32) -> Result<Vec<u8>> {
    let smooth = profile.starts_with("delta-") || profile.ends_with("smooth");
    let roughness = if smooth { 0.0 } else { 0.35 };
    let material = match profile {
        "solid" | "delta-solid" => Material::dielectric(roughness, None)?,
        "thin" | "delta-thin" => Material::dielectric(roughness, Some(0.0625))?,
        _ => Material::opaque(
            [0.3, 0.6, 0.8],
            roughness,
            1.5,
            match profile {
                "conductor" | "delta-conductor" => 1.0,
                "mixed" | "mixed-sss" | "mixed-smooth" => 0.4,
                _ => 0.0,
            },
            match profile {
                "subsurface" | "mixed-sss" => 0.5,
                "sss-only" => 1.0,
                _ => 0.0,
            },
        )?,
    };
    let mut words = Vec::with_capacity(count as usize * 28);
    let transmitted = Medium::new(1.5, [0.16, 0.32, 0.48])?.to_array();
    for index in 0..count {
        words.extend(material.to_words());
        let cosine = 0.1 + (index & 255) as f32 * (0.9 / 255.0);
        let sample = [17_u32, 37, 59]
            .map(|stride| ((index.wrapping_mul(stride) & 1023) as f32 + 0.5) / 1024.0);
        for row in [
            [(1.0 - cosine * cosine).sqrt(), 0.0, cosine, 0.0],
            [sample[0], sample[1], sample[2], 0.0],
            Medium::VACUUM.to_array(),
            transmitted,
        ] {
            words.extend(row.map(f32::to_bits));
        }
    }
    Ok(words.into_iter().flat_map(u32::to_le_bytes).collect())
}

fn buffer_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn median(values: &[f64]) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) * 0.5
    } else {
        values[middle]
    }
}

fn main() -> Result {
    let args = args()?;
    let mut gpu = Gpu::new(Capability::Compute, 0, true)?;
    let before = std::fs::read(&args.before)?;
    let after = std::fs::read(&args.after)?;
    let data = input(&args.profile, args.count)?;
    let context = gpu.context();
    let device = context.device();
    let energy = GpuEnergy::new(device, context.queue())?;
    let source = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flow input"),
        size: data.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let target = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flow output"),
        size: u64::from(args.count) * 80,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    context.queue().write_buffer(&source, 0, &data);
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("flow ABI"),
        entries: &[
            buffer_entry(0, true),
            buffer_entry(1, false),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("flow data"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: source.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: target.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(energy.view()),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(energy.sampler()),
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("flow pipeline ABI"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipelines: Vec<_> = [&before, &after]
        .into_iter()
        .map(|bytes| -> Result<_> {
            let shader = context.shader(bytes, "main", [64, 1, 1])?;
            Ok(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("flow compared program"),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                }),
            )
        })
        .collect::<Result<_>>()?;
    let record = |gpu: &Gpu, variant: usize, count: u32| {
        let mut encoder = gpu
            .context()
            .device()
            .create_command_encoder(&Default::default());
        for _ in 0..count {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipelines[variant]);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(args.count.div_ceil(64), 1, 1);
        }
        encoder.finish().into()
    };
    let mut outputs = Vec::new();
    for variant in 0..2 {
        let command = record(&gpu, variant, 1);
        gpu.measure(vec![command])?;
        outputs.push(gpu.readback(&target)?);
    }
    let mut max_error = 0.0_f32;
    for (index, (a, b)) in outputs[0]
        .as_chunks::<80>()
        .0
        .iter()
        .zip(outputs[1].as_chunks::<80>().0)
        .enumerate()
    {
        let a: [u32; 20] =
            std::array::from_fn(|i| u32::from_le_bytes(a[4 * i..4 * i + 4].try_into().unwrap()));
        let b: [u32; 20] =
            std::array::from_fn(|i| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap()));
        if a[8] != b[8] || a[10] != index as u32 + 1 || b[10] != index as u32 + 1 {
            return Err("实际执行标识或散射事件不一致".into());
        }
        for row in [0, 1, 3, 4] {
            for channel in 0..4 {
                let a = f32::from_bits(a[row * 4 + channel]);
                let b = f32::from_bits(b[row * 4 + channel]);
                let error = (a - b).abs() / a.abs().max(b.abs()).max(1.0);
                if !a.is_finite() || !b.is_finite() || error > 3e-5 {
                    return Err(format!(
                        "数值不符 record={index},row={row},channel={channel}: {a}/{b}"
                    )
                    .into());
                }
                max_error = max_error.max(error);
            }
        }
    }
    for variant in [0, 1, 1, 0] {
        let command = record(&gpu, variant, args.dispatches);
        gpu.measure(vec![command])?;
    }
    let mut samples = Vec::new();
    let mut ratios = Vec::new();
    for pair in 0..args.pairs {
        let mut times = [0.0; 2];
        for variant in if pair % 2 == 0 { [0, 1] } else { [1, 0] } {
            let started = Instant::now();
            let command = record(&gpu, variant, args.dispatches);
            let encode = started.elapsed().as_secs_f64();
            let measured = gpu.measure(vec![command])?;
            times[variant] = measured.gpu_seconds;
            samples.push(serde_json::json!({ "pair":pair,"variant":variant,"gpu_seconds":measured.gpu_seconds,
                "host_seconds":measured.host_seconds,"encode_seconds":encode }));
        }
        ratios.push(times[1] / times[0]);
    }
    gpu.check()?;
    let report = serde_json::json!({"profile":args.profile,"device":gpu.context().adapter_info().name,
        "driver":gpu.context().adapter_info().driver_info,"count":args.count,"dispatches":args.dispatches,"pairs":args.pairs,
        "before":args.before,"after":args.after,"before_blake3":blake3::hash(&before).to_hex().to_string(),
        "after_blake3":blake3::hash(&after).to_hex().to_string(),"input_blake3":blake3::hash(&data).to_hex().to_string(),
        "median_after_over_before":median(&ratios),"max_scaled_error":max_error,"samples":samples});
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&args.output, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}: GPU after/before={:.4}, max error={max_error}",
        args.profile,
        median(&ratios)
    );
    Ok(())
}
