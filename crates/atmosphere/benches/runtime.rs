//! 固定 Earth/balanced 资源路径的主机墙钟；需要真实 Vulkan GPU，非 GPU timestamp。
//!
//! initialization_complete 排除设备、shader/pipeline 和常驻资源创建，包含 scratch 创建、
//! optical+8 轮散射录制、提交、完成等待和 scratch 释放；owner 析构在计时外。
//! observer_update_complete 包含 CPU chart 逆映射、上传、SkyView/Aerial 派发和完成等待；
//! aerial_update_complete 只改变视锥。steady_complete 不派发计算，包含参数判定、空录制、
//! 提交和完成等待；它不衡量积分器中的查表成本。初始化和预热不计入更新／稳态样本。
#![forbid(unsafe_code)]

use atmosphere::{
    Update, View,
    gpu::{Atmosphere, SHADERS, Shaders},
};
use criterion::{Criterion, criterion_group, criterion_main};
use std::{
    error::Error,
    hint::black_box,
    time::{Duration, Instant},
};
use vulkan::interop::WgpuContext;

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

fn create(context: &WgpuContext) -> Result<Atmosphere> {
    let modules: Vec<_> = SHADERS
        .iter()
        .map(|shader| context.shader(shader.spirv, shader.entry, shader.workgroup))
        .collect::<std::result::Result<_, _>>()?;
    let modules = modules.try_into().map_err(|_| "shader count")?;
    Ok(Atmosphere::new(
        context.device(),
        context.queue(),
        Shaders { modules },
    )?)
}

fn complete(context: &WgpuContext, submission: wgpu::SubmissionIndex) -> Result {
    context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    Ok(())
}

fn initialize(context: &WgpuContext, atmosphere: &mut Atmosphere) -> Result {
    let mut encoder = context.device().create_command_encoder(&Default::default());
    atmosphere.initialize(&mut encoder)?;
    complete(context, context.queue().submit([encoder.finish()]))?;
    atmosphere.release_initialization();
    Ok(())
}

fn update(context: &WgpuContext, atmosphere: &mut Atmosphere, view: View) -> Result<Update> {
    let expected = atmosphere.prepare(view)?;
    let mut encoder = context.device().create_command_encoder(&Default::default());
    let actual = atmosphere.record(&mut encoder);
    assert_eq!(expected, actual);
    complete(context, context.queue().submit([encoder.finish()]))?;
    Ok(actual)
}

fn benchmarks(criterion: &mut Criterion) -> Result {
    let context = pollster::block_on(WgpuContext::compute())?;
    eprintln!("atmosphere runtime: {:?}", context.adapter_info());
    let mut group = criterion.benchmark_group("atmosphere/initialization_complete");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(100));
    group.measurement_time(Duration::from_secs(1));
    group.bench_function("earth_balanced_8_iterations", |bench| {
        bench.iter_custom(|iterations| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iterations {
                let mut atmosphere = create(&context).unwrap();
                let started = Instant::now();
                initialize(&context, &mut atmosphere).unwrap();
                elapsed += started.elapsed();
                black_box(atmosphere.memory());
            }
            elapsed
        });
    });
    group.finish();
    let mut atmosphere = create(&context)?;
    initialize(&context, &mut atmosphere)?;
    let view = View {
        altitude_km: 0.2,
        sun_direction: [0.0, 0.6, 0.8],
        ..View::default()
    };
    for _ in 0..3 {
        update(&context, &mut atmosphere, view)?;
    }
    eprintln!("atmosphere retained payload: {:?}", atmosphere.memory());
    let mut group = criterion.benchmark_group("atmosphere/runtime_complete");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(1));
    group.bench_function("observer_update_sky_and_aerial", |bench| {
        bench.iter_custom(|iterations| {
            update(&context, &mut atmosphere, view).unwrap();
            let started = Instant::now();
            for iteration in 0..iterations {
                let next = View {
                    altitude_km: if iteration % 2 == 0 { 0.21 } else { 0.2 },
                    ..view
                };
                black_box(update(&context, &mut atmosphere, next).unwrap());
            }
            started.elapsed()
        });
    });
    group.bench_function("projection_update_aerial_only", |bench| {
        bench.iter_custom(|iterations| {
            update(&context, &mut atmosphere, view).unwrap();
            let started = Instant::now();
            for iteration in 0..iterations {
                let next = View {
                    right: [if iteration % 2 == 0 { 1.1 } else { 1.0 }, 0.0, 0.0],
                    ..view
                };
                black_box(update(&context, &mut atmosphere, next).unwrap());
            }
            started.elapsed()
        });
    });
    group.bench_function("steady_no_dispatch", |bench| {
        bench.iter_custom(|iterations| {
            update(&context, &mut atmosphere, view).unwrap();
            let started = Instant::now();
            for _ in 0..iterations {
                assert_eq!(
                    update(&context, &mut atmosphere, black_box(view)).unwrap(),
                    Update::default()
                );
            }
            started.elapsed()
        });
    });
    group.finish();
    Ok(())
}

fn entry(criterion: &mut Criterion) {
    gpu_log::checked(|| benchmarks(criterion)).unwrap();
}
criterion_group!(benches, entry);
criterion_main!(benches);
