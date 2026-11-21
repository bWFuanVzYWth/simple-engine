//! 原生衍射核工作台，与无窗口批量导出的入口。
#![forbid(unsafe_code)]
#[cfg(not(target_arch = "wasm32"))]
mod controls;
#[cfg(not(target_arch = "wasm32"))]
mod desktop;
#[cfg(not(target_arch = "wasm32"))]
mod panel;
#[cfg(not(target_arch = "wasm32"))]
mod preview;
#[cfg(not(target_arch = "wasm32"))]
mod recipe;
#[cfg(not(target_arch = "wasm32"))]
mod surface;
#[cfg(not(target_arch = "wasm32"))]
mod worker;

#[cfg(not(target_arch = "wasm32"))]
fn run() -> recipe::Result<()> {
    use clap::{Parser, Subcommand};
    use recipe::{Recipe, stage_kernel};
    use simple_engine::{fft_bloom::diffraction::Solver, vulkan::interop::WgpuContext};
    use std::{num::NonZeroU32, path::PathBuf, time::Instant};
    #[derive(Parser)]
    #[command(about = "原生衍射核工作台")]
    struct Options {
        /// 打开保存的工作台参数，默认使用七叶圆弧光圈、圆形灰尘与纤维。
        #[arg(long, global = true)]
        recipe: Option<PathBuf>,
        /// Vulkan 计算适配器序号。
        #[arg(long, default_value_t = 0, global = true)]
        adapter: usize,
        #[command(subcommand)]
        command: Option<Action>,
    }
    #[derive(Subcommand)]
    enum Action {
        /// 从实际光瞳重算并原子写入核。使用 .ktx2 导出 Zstd 无损纹理，.rgb32le 导出原始浮点。
        Bake {
            output: PathBuf,
            /// 用同一设备独立重算多次，逐次报告包含读回的耗时。
            #[arg(long, default_value = "1")]
            repeat: NonZeroU32,
            /// 保存含太阳盘边距的预览积分，观察者模式输出 XYZ。
            #[arg(long)]
            preview: bool,
        },
    }
    let opt = Options::parse();
    let recipe = opt
        .recipe
        .as_deref()
        .map(Recipe::open)
        .transpose()?
        .unwrap_or_default();
    if let Some(Action::Bake {
        output,
        repeat,
        preview,
    }) = opt.command
    {
        let start = Instant::now();
        let device = pollster::block_on(WgpuContext::with_index(opt.adapter, false))?;
        eprintln!("计算设备 {}", device.adapter_info().name);
        let mut solver = Solver::new(device)?;
        eprintln!("设备与管线 {:.3} s", start.elapsed().as_secs_f64());
        for run in 0..repeat.get() {
            let start = Instant::now();
            let pupil = recipe.parameters.pupil()?;
            let f = solver.solve(&pupil, recipe.parameters.request(preview), |_| true)?;
            let sec = start.elapsed().as_secs_f64();
            println!(
                "第 {} 次  {} × {}  {:.3} s  中心 {:?}",
                run + 1,
                f.side,
                f.side,
                sec,
                f.pixels[f.pixels.len() / 2]
            );
            if run + 1 == repeat.get() {
                let write_start = Instant::now();
                stage_kernel(&output, &f)?.persist(&output)?;
                println!(
                    "写入 {}  {:.3} s",
                    output.display(),
                    write_start.elapsed().as_secs_f64()
                );
            }
        }
        Ok(())
    } else {
        desktop::run(recipe, opt.adapter)
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    match simple_engine::gpu_log::checked(run) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}
#[cfg(target_arch = "wasm32")]
fn main() {}
