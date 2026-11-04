//! 生产几何 shader 的负载测量与失败输入重放 CLI。
//!
//! 每种分布热身三批，再保存指定轮数的计时样本。每个 dispatch 对同一批三角端点执行
//! 生产重建／连接构造并覆盖输出射线，不含硬件求交或串行反馈，不能外推整帧渲染成本。
//! 每种分布另执行一次回读，只检查输出有限且连接有效，不把结果检验计入测量轮。
//!
//! 逐轮计时沿用 [`Gpu::bench_compute`]：GPU timestamp 覆盖整批 dispatch 及测量边界，
//! host 包含录制、提交、查询 resolve、等待与查询回读，prepare 包含管线查找与 IO 准备。
//! CPU 输入生成、设备初始化、累计管线创建分别记录；不是单个函数的隔离延迟。
//! 报告保留输入、源码、编译、SPIR-V、可执行文件身份及保留 buffer 容量，不设性能通过阈值。
//! CLI 不改变 validation 设置；A/B 环境设置与版本保留由外层运行脚本负责。
//!
//! 重放交给 [`shader_tests::run::replay`]，先保存原／现 shader 指纹与结果，仍失败时返回非零。
#![forbid(unsafe_code)]

use shader_tests::{
    input::{self, Distribution},
    run,
};
use simple_engine::gpu_test::{self, Capability, DEFAULT_SEED, Failure, Gpu};
use std::{path::PathBuf, process::ExitCode, time::Instant};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Gpu(#[from] gpu_test::Error),
    #[error(transparent)]
    Run(#[from] run::Error),
    #[error(transparent)]
    Record(#[from] gpu_test::record::RecordError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Seed(#[from] gpu_test::input::SeedError),
    #[error("参数错误：{0}")]
    Argument(String),
    #[error("连接负载输出无效")]
    Output,
    #[error("重放仍复现性质失败，详情已输出")]
    Replay,
}

struct Args {
    count: u32,
    dispatches: u32,
    rounds: u32,
    adapter: usize,
    scale: f32,
    distribution: String,
    output: PathBuf,
    replay: Option<PathBuf>,
    seed: [u8; 32],
}

fn parse<T: std::str::FromStr>(value: &str) -> Result<T, Error> {
    value.parse().map_err(|_| Error::Argument(value.into()))
}

fn args() -> Result<Option<Args>, Error> {
    let mut result = Args {
        count: 262_144,
        dispatches: 256,
        rounds: 7,
        adapter: 0,
        scale: 1.0,
        distribution: "all".into(),
        output: format!(
            "temp/shader-tests/bench-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
        .into(),
        replay: None,
        seed: DEFAULT_SEED,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(key) = arguments.next() {
        if key == "--help" {
            println!(
                "shader_bench --count N --dispatches N --rounds N --scale F --distribution all|coherent|incoherent|grazing --adapter N --seed HEX64 --output JSON\nshader_bench --replay FAILURE_JSON --output JSON"
            );
            return Ok(None);
        }
        let value = arguments
            .next()
            .ok_or_else(|| Error::Argument(format!("{key} 缺少值")))?;
        match key.as_str() {
            "--count" => result.count = parse(&value)?,
            "--dispatches" => result.dispatches = parse(&value)?,
            "--rounds" => result.rounds = parse(&value)?,
            "--adapter" => result.adapter = parse(&value)?,
            "--scale" => result.scale = parse(&value)?,
            "--distribution" => result.distribution = value,
            "--output" => result.output = value.into(),
            "--replay" => result.replay = Some(value.into()),
            "--seed" => result.seed = gpu_test::input::parse_seed(&value)?,
            _ => return Err(Error::Argument(key)),
        }
    }
    if result.count == 0
        || result.dispatches == 0
        || result.rounds == 0
        || !result.scale.is_finite()
        || result.scale <= 0.0
    {
        return Err(Error::Argument(
            "count/dispatches/rounds/scale 须为有限正数".into(),
        ));
    }
    Ok(Some(result))
}

fn save(path: &std::path::Path, value: &serde_json::Value) -> Result<(), Error> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn run() -> Result<(), Error> {
    let Some(args) = args()? else {
        return Ok(());
    };
    if let Some(path) = args.replay {
        let record = run::replay(&Failure::load(path)?)?;
        save(&args.output, &record)?;
        println!(
            "{}：passed={}，{}",
            record["case"],
            record["passed"],
            args.output.display()
        );
        return if record["passed"].as_bool() == Some(true) {
            Ok(())
        } else {
            Err(Error::Replay)
        };
    }
    let choices = match args.distribution.as_str() {
        "all" => vec![
            ("connection_coherent", Distribution::Coherent),
            ("connection_incoherent", Distribution::Incoherent),
            ("connection_grazing", Distribution::Grazing),
        ],
        "coherent" => vec![("connection_coherent", Distribution::Coherent)],
        "incoherent" => vec![("connection_incoherent", Distribution::Incoherent)],
        "grazing" => vec![("connection_grazing", Distribution::Grazing)],
        _ => return Err(Error::Argument(args.distribution)),
    };
    let start = Instant::now();
    let mut gpu = Gpu::new(Capability::Compute, args.adapter, true)?;
    let setup_seconds = start.elapsed().as_secs_f64();
    let info = gpu.context().adapter_info();
    if u64::from(args.count) * 80
        > gpu
            .context()
            .device()
            .limits()
            .max_storage_buffer_binding_size
    {
        return Err(Error::Argument(
            "count 对应的输入工作集超过设备 storage 范围".into(),
        ));
    }
    let shader = run::shader("connection_load_main")?;
    let mut workloads = Vec::new();
    for (name, distribution) in choices {
        let start = Instant::now();
        let words = input::connections(args.count, distribution, args.scale, args.seed, 10);
        let input = run::bytes(&words);
        drop(words);
        let prepare_cpu_seconds = start.elapsed().as_secs_f64();
        let groups = [args.count.div_ceil(64), 1, 1];
        let output_bytes = args.count as usize * 16;
        for _ in 0..3 {
            gpu.bench_compute(shader, &input, output_bytes, groups, args.dispatches)?;
        }
        let mut samples = Vec::new();
        for _ in 0..args.rounds {
            let sample =
                gpu.bench_compute(shader, &input, output_bytes, groups, args.dispatches)?;
            samples.push(serde_json::json!({ "gpu_seconds": sample.gpu_seconds,
                "host_seconds": sample.host_seconds, "prepare_seconds": sample.prepare_seconds }));
        }
        let actual = gpu.compute(shader, &input, output_bytes, groups)?;
        if !actual.as_chunks::<16>().0.iter().all(|row| {
            let values: [f32; 4] = std::array::from_fn(|i| {
                f32::from_le_bytes(row[i * 4..i * 4 + 4].try_into().unwrap())
            });
            values.iter().all(|value| value.is_finite()) && values[3] > 0.0
        }) {
            return Err(Error::Output);
        }
        workloads.push(serde_json::json!({ "name": name, "input_blake3": blake3::hash(&input).to_hex().as_str(),
            "count": args.count, "dispatches": args.dispatches,
            "work_items": u64::from(args.count) * u64::from(args.dispatches), "samples": samples,
            "input_bytes": input.len(), "output_bytes": output_bytes,
            "prepare_cpu_seconds": prepare_cpu_seconds, "gpu_buffer_capacity_bytes": gpu.stats().buffer_capacity_bytes }));
    }
    gpu.check()?;
    let mut executable = blake3::Hasher::new();
    executable.update_reader(std::fs::File::open(std::env::current_exe()?)?)?;
    let result = serde_json::json!({ "device": info.name, "driver": info.driver_info,
        "slang": shader_tests::SLANG_VERSION, "source_blake3": shader_tests::SOURCE_BLAKE3,
        "shader_build_blake3": shader_tests::SHADER_BUILD_BLAKE3,
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "rng": gpu_test::input::RNG_PROTOCOL,
        "spirv_blake3": blake3::hash(shader.spirv).to_hex().as_str(), "executable_blake3": executable.finalize().to_hex().as_str(),
        "seed": args.seed, "stream": 10, "scale": args.scale,
        "workload": "生产 geometry 三角端点重建与连接构造；不含硬件求交或循环反馈",
        "timing": "GPU timestamp 包围整批 dispatch；host 包含录制/提交/等待；prepare 为 IO 更新；不含读回检查",
        "setup_seconds": setup_seconds, "pipeline_seconds": gpu.stats().pipeline_seconds,
        "workloads": workloads });
    gpu.finish()?;
    save(&args.output, &result)?;
    println!("{}：{}", info.name, args.output.display());
    Ok(())
}

fn main() -> ExitCode {
    let result = run();
    let shutdown = gpu_test::shutdown_shared();
    if let Err(error) = &result {
        eprintln!("{error}");
    }
    if let Err(error) = &shutdown {
        eprintln!("{error}");
    }
    if result.is_ok() && shutdown.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
