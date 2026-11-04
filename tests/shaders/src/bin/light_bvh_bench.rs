//! 常驻树和查询的 light BVH 吞吐基线；每 lane 查询一次，没有 shader 内重复或输出反馈。
//!
//! grid/clustered × 24/4096/65536 灯，分别测 sample 与 reverse PMF。coherent 每64项
//! 共享接收点，selector/反查灯 ID 仍独立变化；incoherent 仅打乱完全相同的查询集合。
//! 两种顺序按原序号还原后必须逐位相同。输出、输入指纹及几何保存在新目录，供布局改动
//! 核对；有限值检查与最终回读在稳态计时外，布局间差异须按数学契约审查。
//!
//! GPU timestamp 包围整批 dispatch、各 pass 间依赖及边界，主机 encode/submit/completion/
//! timequery 分列。timequery 是16B查询结果的复制、等待与映射；不含完整输出回读。
//! build、buffer 分配和完成上传分别记录。默认请求 timestamp；不支持时可显式选择
//! `--timing host`，此时 GPU 时间为 null，不把主机墙钟当作 GPU timestamp。
#![forbid(unsafe_code)]

use simple_engine::{
    gpu_test::{self, Capability, Gpu},
    light_bvh::{self, Quad, Tree},
    vulkan::{self, interop::CommandBuffer},
    wgpu,
};
use std::{
    collections::TryReserveError,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

type Result<T = ()> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Gpu(#[from] gpu_test::Error),
    #[error(transparent)]
    Native(#[from] vulkan::Error),
    #[error(transparent)]
    Tree(#[from] light_bvh::Error),
    #[error(transparent)]
    Poll(#[from] wgpu::PollError),
    #[error(transparent)]
    Allocation(#[from] TryReserveError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Integer(#[from] std::num::ParseIntError),
    #[error("参数错误：{0}")]
    Argument(String),
    #[error("{0} 的输出或查询顺序不变量失败")]
    Output(String),
}

struct Args {
    count: u32,
    dispatches: u32,
    rounds: u32,
    adapter: usize,
    output: PathBuf,
    filter: String,
    timestamps: bool,
}

const SEED: u32 = 0x1357_2468;
const KERNELS: [(&str, &str, &[u8]); 2] = [
    (
        "sample",
        "light_bvh_sample_bench",
        include_bytes!(concat!(env!("OUT_DIR"), "/light_bvh_sample_bench.spv")),
    ),
    (
        "reverse",
        "light_bvh_reverse_bench",
        include_bytes!(concat!(env!("OUT_DIR"), "/light_bvh_reverse_bench.spv")),
    ),
];

fn args() -> Result<Option<Args>> {
    let mut result = Args {
        count: 65_536,
        dispatches: 8,
        rounds: 3,
        adapter: 0,
        output: format!("temp/light-bvh/query-{}", std::process::id()).into(),
        filter: String::new(),
        timestamps: true,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(key) = arguments.next() {
        if key == "--help" {
            println!(
                "light_bvh_bench --count N --dispatches N --rounds N --adapter N \
                 --case 子串 --timing gpu|host --output 新目录\n\
                 case: grid|clustered / 24|4096|65536 / coherent|incoherent / sample|reverse"
            );
            return Ok(None);
        }
        let value = arguments
            .next()
            .ok_or_else(|| Error::Argument(format!("{key} 缺少值")))?;
        match key.as_str() {
            "--count" => result.count = value.parse()?,
            "--dispatches" => result.dispatches = value.parse()?,
            "--rounds" => result.rounds = value.parse()?,
            "--adapter" => result.adapter = value.parse()?,
            "--case" => result.filter = value,
            "--output" => result.output = value.into(),
            "--timing" => {
                result.timestamps = match value.as_str() {
                    "gpu" => true,
                    "host" => false,
                    _ => return Err(Error::Argument("timing 只支持 gpu/host".into())),
                };
            }
            _ => return Err(Error::Argument(key)),
        }
    }
    if result.count == 0 || result.dispatches == 0 || result.rounds == 0 {
        return Err(Error::Argument("count/dispatches/rounds 须为正数".into()));
    }
    Ok(Some(result))
}

fn mix(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

fn bytes(words: &[u32]) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    result.try_reserve_exact(words.len() * 4)?;
    for word in words {
        result.extend(word.to_le_bytes());
    }
    Ok(result)
}

fn geometry(count: u32, clustered: bool) -> Result<(Vec<Quad>, Vec<u8>)> {
    let mut quads = Vec::new();
    let mut words = Vec::new();
    quads.try_reserve_exact(count as usize)?;
    words.try_reserve_exact(count as usize * 13)?;
    for index in 0..count {
        let [x, y, z] = if clustered {
            [(index % 7) as f32 * 0.01, 0.0, 0.0]
        } else {
            [
                (index % 64) as f32 * 4.0,
                ((index / 64) % 64) as f32 * 4.0,
                (index / 4096) as f32 * 4.0,
            ]
        };
        let quad = Quad {
            vertices: [
                [x, y, z],
                [x + 1.0, y, z],
                [x + 1.0, y + 1.0, z],
                [x, y + 1.0, z],
            ],
            power: 1.0 + (index % 17) as f32,
        };
        for vertex in quad.vertices {
            words.extend(vertex.map(f32::to_bits));
        }
        words.push(quad.power.to_bits());
        quads.push(quad);
    }
    Ok((quads, bytes(&words)?))
}

fn queries(count: u32, lights: u32, clustered: bool, shuffled: bool) -> Result<Vec<u8>> {
    let mut items = Vec::new();
    items.try_reserve_exact(count as usize)?;
    for index in 0..count {
        let tile = index / 64;
        let point = if clustered {
            [
                (tile % 32) as f32 * 0.005 - 0.08,
                ((tile / 32) % 32) as f32 * 0.005 - 0.08,
                0.25 + ((tile / 1024) % 16) as f32 * 0.125,
            ]
        } else {
            [
                (tile % 64) as f32 * 4.0 + 0.5,
                ((tile / 64) % 64) as f32 * 4.0 + 0.5,
                -2.0 + ((tile / 4096) % 8) as f32 * 0.5,
            ]
        };
        let selector = match index % 256 {
            0 => 0,
            1 => 0x00ff_ffff,
            _ => mix(index ^ SEED) & 0x00ff_ffff,
        };
        items.push([
            point[0].to_bits(),
            point[1].to_bits(),
            point[2].to_bits(),
            (selector as f32 * (1.0 / 16_777_216.0)).to_bits(),
            mix(index ^ !SEED) % lights,
            0,
            0,
            index,
        ]);
    }
    if shuffled {
        let mut state = SEED;
        for index in (1..items.len()).rev() {
            state = mix(state.wrapping_add(index as u32));
            items.swap(index, state as usize % (index + 1));
        }
    }
    let mut words = Vec::new();
    words.try_reserve_exact(4 + count as usize * 8)?;
    words.extend([count, lights, 0, 0]);
    for item in items {
        words.extend(item);
    }
    bytes(&words)
}

struct Timestamps {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
}

struct Programs {
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::ComputePipeline; 2],
    timestamps: Option<Timestamps>,
}

impl Programs {
    fn new(gpu: &Gpu, timestamps: bool) -> Result<Self> {
        let context = gpu.context();
        let device = context.device();
        let entries = std::array::from_fn::<_, 3, _>(|index| wgpu::BindGroupLayoutEntry {
            binding: index as u32,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage {
                    read_only: index != 2,
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light BVH resident buffers"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("light BVH query"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let mut pipelines = Vec::new();
        for (_, entry, spirv) in KERNELS {
            let module = context.shader(spirv, entry, [64, 1, 1])?;
            pipelines.push(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
            );
        }
        let timestamps = timestamps.then(|| Timestamps {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("light BVH time"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: buffer(
                gpu,
                "light BVH timestamp resolve",
                16,
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
        });
        Ok(Self {
            layout,
            pipelines: pipelines.try_into().unwrap(),
            timestamps,
        })
    }
}

fn buffer(gpu: &Gpu, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    gpu.context()
        .device()
        .create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
}

fn complete(gpu: &Gpu, submission: wgpu::SubmissionIndex) -> Result {
    gpu.context().device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(Duration::from_secs(30)),
    })?;
    Ok(())
}

fn upload(gpu: &Gpu, destination: &wgpu::Buffer, data: &[u8]) -> Result<serde_json::Value> {
    let start = Instant::now();
    gpu.context().queue().write_buffer(destination, 0, data);
    let written = Instant::now();
    let submission = gpu.context().queue().submit([]);
    let submitted = Instant::now();
    complete(gpu, submission)?;
    let done = Instant::now();
    Ok(serde_json::json!({
        "bytes":data.len(),"write_seconds":(written-start).as_secs_f64(),
        "submit_seconds":(submitted-written).as_secs_f64(),
        "completion_seconds":(done-submitted).as_secs_f64(),
        "total_seconds":(done-start).as_secs_f64()
    }))
}

fn dispatch(
    gpu: &mut Gpu,
    programs: &Programs,
    bindings: &wgpu::BindGroup,
    kernel: usize,
    count: u32,
    dispatches: u32,
) -> Result<serde_json::Value> {
    let start = Instant::now();
    let context = gpu.context_mut();
    let device = context.device();
    let mut commands: Vec<CommandBuffer> = Vec::new();
    if let Some(timestamps) = &programs.timestamps {
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.write_timestamp(&timestamps.set, 0);
        commands.push(encoder.finish().into());
        // 与 Gpu::measure 相同的 BOTTOM_OF_PIPE 起点依赖，防止后续工作越过起点。
        commands.push(context.execution_barrier()?);
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    // 每个 dispatch 独占一个 pass，由 wgpu 在 pass 边界排序重复写 output 的 WAW 依赖。
    for _ in 0..dispatches {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&programs.pipelines[kernel]);
        pass.set_bind_group(0, bindings, &[]);
        pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
    }
    commands.push(encoder.finish().into());
    if let Some(timestamps) = &programs.timestamps {
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.write_timestamp(&timestamps.set, 1);
        encoder.resolve_query_set(&timestamps.set, 0..2, &timestamps.resolve, 0);
        commands.push(encoder.finish().into());
    }
    let encoded = Instant::now();
    let submission = context.submit(commands)?;
    let submitted = Instant::now();
    complete(gpu, submission)?;
    let completed = Instant::now();
    let gpu_seconds = if let Some(timestamps) = &programs.timestamps {
        let raw = gpu.readback(&timestamps.resolve)?;
        let first = u64::from_le_bytes(raw[..8].try_into().unwrap());
        let last = u64::from_le_bytes(raw[8..16].try_into().unwrap());
        let mask = u64::MAX >> (64 - gpu.context().timestamp_valid_bits());
        Some(
            (last.wrapping_sub(first) & mask) as f64
                * f64::from(gpu.context().queue().get_timestamp_period())
                * 1e-9,
        )
    } else {
        None
    };
    let queried = Instant::now();
    gpu.check()?;
    Ok(serde_json::json!({
        "encode_seconds":(encoded-start).as_secs_f64(),
        "submit_seconds":(submitted-encoded).as_secs_f64(),
        "completion_seconds":(completed-submitted).as_secs_f64(),
        "timequery_seconds":(queried-completed).as_secs_f64(),
        "total_seconds":(queried-start).as_secs_f64(),"gpu_seconds":gpu_seconds,
        "dispatches":dispatches,"queries":u64::from(count)*u64::from(dispatches)
    }))
}

fn canonical_output(raw: &[u8], count: u32, lights: u32, name: &str) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut seen = Vec::new();
    result.try_reserve_exact(count as usize * 16)?;
    result.resize(count as usize * 16, 0);
    seen.try_reserve_exact(count as usize)?;
    seen.resize(count as usize, false);
    for row in raw.as_chunks::<16>().0 {
        let words: [u32; 4] = std::array::from_fn(|index| {
            u32::from_le_bytes(row[index * 4..index * 4 + 4].try_into().unwrap())
        });
        let index = words[3].wrapping_sub(1) as usize;
        let pmf = f32::from_bits(words[1]);
        if index >= count as usize
            || seen[index]
            || words[0] >= lights
            || !pmf.is_finite()
            || !(1.0 / 16_777_216.0..=1.0).contains(&pmf)
            || words[2] != 0
        {
            return Err(Error::Output(name.into()));
        }
        seen[index] = true;
        result[index * 16..index * 16 + 16].copy_from_slice(row);
    }
    if seen.iter().any(|&found| !found) {
        return Err(Error::Output(name.into()));
    }
    Ok(result)
}

fn write_json(path: &Path, report: &serde_json::Value) -> Result {
    std::fs::write(path, serde_json::to_vec_pretty(report)?)?;
    Ok(())
}

fn selected(args: &Args, name: &str) -> bool {
    name.contains(&args.filter)
}

fn run_cases(gpu: &mut Gpu, args: &Args) -> Result<serde_json::Value> {
    let limits = gpu.context().device().limits();
    if u64::from(args.count) * 32 + 16 > limits.max_storage_buffer_binding_size
        || u64::from(args.count) * 16 > limits.max_storage_buffer_binding_size
        || args.count.div_ceil(64) > limits.max_compute_workgroups_per_dimension
    {
        return Err(Error::Argument(
            "查询工作集超过设备 binding/dispatch 上限".into(),
        ));
    }
    let start = Instant::now();
    let programs = Programs::new(gpu, args.timestamps)?;
    gpu.check()?;
    let pipeline_seconds = start.elapsed().as_secs_f64();
    let mut cases = Vec::new();
    for kind in ["grid", "clustered"] {
        for lights in [24, 4096, 65_536] {
            let prefix = format!("{kind}/{lights}");
            if !["coherent", "incoherent"].into_iter().any(|order| {
                KERNELS.iter().any(|(operation, _, _)| {
                    selected(args, &format!("{prefix}/{order}/{operation}"))
                })
            }) {
                continue;
            }
            let directory = args.output.join(format!("{kind}-{lights}"));
            std::fs::create_dir(&directory)?;
            let start = Instant::now();
            let (quads, geometry) = geometry(lights, kind == "clustered")?;
            let geometry_seconds = start.elapsed().as_secs_f64();
            let start = Instant::now();
            let tree = Tree::build(&quads)?;
            let build_seconds = start.elapsed().as_secs_f64();
            let tree_bytes = bytes(tree.words())?;
            if tree_bytes.len() as u64 > limits.max_storage_buffer_binding_size {
                return Err(Error::Argument("树超过设备 storage binding 上限".into()));
            }
            let checksum_start = Instant::now();
            let tree_hash = blake3::hash(&tree_bytes).to_hex().to_string();
            let tree_checksum_seconds = checksum_start.elapsed().as_secs_f64();
            std::fs::write(directory.join("quads.bin"), &geometry)?;
            std::fs::write(directory.join("tree.bin"), &tree_bytes)?;
            let start = Instant::now();
            let tree_buffer = buffer(
                gpu,
                "light BVH tree",
                tree_bytes.len() as u64,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            let tree_allocation_seconds = start.elapsed().as_secs_f64();
            let tree_upload = upload(gpu, &tree_buffer, &tree_bytes)?;
            let mut canonical: [Option<Vec<u8>>; 2] = [None, None];
            for order in ["coherent", "incoherent"] {
                let active: [bool; 2] = std::array::from_fn(|index| {
                    selected(args, &format!("{prefix}/{order}/{}", KERNELS[index].0))
                });
                if !active.into_iter().any(|value| value) {
                    continue;
                }
                let start = Instant::now();
                let input = queries(
                    args.count,
                    lights,
                    kind == "clustered",
                    order == "incoherent",
                )?;
                let query_seconds = start.elapsed().as_secs_f64();
                std::fs::write(directory.join(format!("{order}-queries.bin")), &input)?;
                let start = Instant::now();
                let query_buffer = buffer(
                    gpu,
                    "light BVH queries",
                    input.len() as u64,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                );
                let output = buffer(
                    gpu,
                    "light BVH outputs",
                    u64::from(args.count) * 16,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                );
                let bindings =
                    gpu.context()
                        .device()
                        .create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("light BVH resident fixture"),
                            layout: &programs.layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: tree_buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: query_buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: output.as_entire_binding(),
                                },
                            ],
                        });
                let io_allocation_binding_seconds = start.elapsed().as_secs_f64();
                let query_upload = upload(gpu, &query_buffer, &input)?;
                let mut samples: [Vec<serde_json::Value>; 2] = [Vec::new(), Vec::new()];
                for (kernel, &enabled) in active.iter().enumerate() {
                    if enabled {
                        for _ in 0..2 {
                            dispatch(
                                gpu,
                                &programs,
                                &bindings,
                                kernel,
                                args.count,
                                args.dispatches,
                            )?;
                        }
                    }
                }
                for round in 0..args.rounds {
                    for position in 0..2 {
                        let kernel = (position + round as usize) % 2;
                        if active[kernel] {
                            let mut sample = dispatch(
                                gpu,
                                &programs,
                                &bindings,
                                kernel,
                                args.count,
                                args.dispatches,
                            )?;
                            sample["round"] = round.into();
                            sample["position"] = position.into();
                            samples[kernel].push(sample);
                        }
                    }
                }
                for kernel in 0..2 {
                    if !active[kernel] {
                        continue;
                    }
                    let name = format!("{prefix}/{order}/{}", KERNELS[kernel].0);
                    dispatch(gpu, &programs, &bindings, kernel, args.count, 1)?;
                    let start = Instant::now();
                    let raw = gpu.readback(&output)?;
                    let output_readback_seconds = start.elapsed().as_secs_f64();
                    let start = Instant::now();
                    let normalized = canonical_output(&raw, args.count, lights, &name)?;
                    let canonical_hash = blake3::hash(&normalized).to_hex().to_string();
                    let compared_order = canonical[kernel].is_some();
                    if canonical[kernel]
                        .as_ref()
                        .is_some_and(|previous| *previous != normalized)
                    {
                        return Err(Error::Output(name));
                    }
                    canonical[kernel] = Some(normalized);
                    let output_checksum_seconds = start.elapsed().as_secs_f64();
                    let output_file = format!("{order}-{}.bin", KERNELS[kernel].0);
                    std::fs::write(directory.join(&output_file), &raw)?;
                    let report = serde_json::json!({
                        "case":name,"lights":lights,"nodes":tree.node_count(),"queries":args.count,
                        "geometry_blake3":blake3::hash(&geometry).to_hex().as_str(),
                        "query_blake3":blake3::hash(&input).to_hex().as_str(),
                        "tree_blake3":tree_hash,"output_blake3":blake3::hash(&raw).to_hex().as_str(),
                        "canonical_output_blake3":canonical_hash,"order_comparison_bitwise":if compared_order {Some(true)}else{None},
                        "output_file":format!("{kind}-{lights}/{output_file}"),
                        "geometry_seconds":geometry_seconds,"build_seconds":build_seconds,
                        "tree_checksum_seconds":tree_checksum_seconds,"query_seconds":query_seconds,
                        "tree_allocation_seconds":tree_allocation_seconds,"tree_upload":tree_upload,
                        "io_allocation_binding_seconds":io_allocation_binding_seconds,"query_upload":query_upload,
                        "tree_bytes":tree_bytes.len(),"query_bytes":input.len(),"output_bytes":output.size(),
                        "resident_buffer_bytes":tree_buffer.size()+query_buffer.size()+output.size(),
                        "output_readback_seconds":output_readback_seconds,"output_checksum_seconds":output_checksum_seconds,
                        "samples":samples[kernel]
                    });
                    write_json(
                        &directory.join(format!("{order}-{}.json", KERNELS[kernel].0)),
                        &report,
                    )?;
                    eprintln!("{name}: canonical {canonical_hash}");
                    cases.push(report);
                }
            }
        }
    }
    if cases.is_empty() {
        return Err(Error::Argument("case 没有匹配任何负载".into()));
    }
    gpu.check()?;
    let device = gpu.context().adapter_info();
    let mut executable = blake3::Hasher::new();
    executable.update_reader(std::fs::File::open(std::env::current_exe()?)?)?;
    Ok(serde_json::json!({
        "version":1,"layout":"median_packet_block4_v3","seed":SEED,"device":device.name,"driver":device.driver_info,
        "timing":if args.timestamps {"gpu_timestamp_and_host"}else{"host_only_explicit"},
        "count":args.count,"dispatches":args.dispatches,"rounds":args.rounds,"warmup_batches":2,
        "pipeline_seconds":pipeline_seconds,"slang":shader_tests::SLANG_VERSION,
        "source_blake3":shader_tests::SOURCE_BLAKE3,"shader_build_blake3":shader_tests::SHADER_BUILD_BLAKE3,
        "executable_blake3":executable.finalize().to_hex().as_str(),
        "kernels":KERNELS.map(|(name,entry,spirv)|serde_json::json!({"name":name,"entry":entry,"spirv_blake3":blake3::hash(spirv).to_hex().as_str()})),
        "profile":if cfg!(debug_assertions){"debug"}else{"release"},
        "scope":"常驻单层quad light BVH查询；不含TraceRay、BSDF、图像与游戏调度，不外推整帧",
        "query_contract":"4-word header(count,lights,base,reserved)+8 words(point.xyz,sample,reverseID,0,0,originalID)；反查ID固定散列，不是前向查询结果",
        "timing_contract":"build仅Tree::build；upload含write/submit/完成；steady不重传tree/query；每dispatch独立pass，GPU含pass间WAW依赖及timestamp边界；timequery含16B复制/等待/映射；完整结果回读/校验在轮次外",
        "coherence_contract":"coherent每64项共享point，sample/反查ID仍变化；incoherent仅Fisher-Yates打乱同一query集合，不保证路径一致",
        "cases":cases
    }))
}

fn run() -> Result {
    let Some(args) = args()? else { return Ok(()) };
    if let Some(parent) = args
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(&args.output)?;
    let start = Instant::now();
    let mut gpu = Gpu::new(Capability::Compute, args.adapter, args.timestamps)?;
    let setup_seconds = start.elapsed().as_secs_f64();
    let result = run_cases(&mut gpu, &args);
    let cleanup = gpu.finish();
    let mut report = result?;
    cleanup?;
    report["setup_seconds"] = setup_seconds.into();
    write_json(&args.output.join("report.json"), &report)?;
    println!("{}：{}", report["device"], args.output.display());
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
