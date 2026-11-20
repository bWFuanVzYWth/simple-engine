//! 离线制核的实际求解时间，包含光瞳准备、设备提交、归一化和读回。磁盘写入单独由工作台负责。
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use fft_bloom::{
        calibration::{Response, Source},
        diffraction::{Quality, Request, Solver},
        pupil::{Blades, Dust, Fibers, Iris, Opening, Pupil},
    };
    use std::{hint::black_box, time::Instant};
    let start = Instant::now();
    let device = pollster::block_on(vulkan::interop::WgpuContext::compute())?;
    let name = device.adapter_info().name.clone();
    let mut solver = Solver::new(device)?;
    println!(
        "{name}，建立设备与积分管线 {:.3} s",
        start.elapsed().as_secs_f64()
    );
    let base = Request {
        radius: 912,
        support: 912,
        f_number: 4.,
        pitch_um: 5.,
        source: Source::Equal,
        response: Response::EngineRgb,
        quality: Quality::Calibrated,
    };
    for (name, iris, request) in [
        (
            "七叶叠加预览",
            false,
            Request {
                radius: 103,
                quality: Quality::Draft,
                ..base
            },
        ),
        (
            "七叶叠加默认视场",
            false,
            Request {
                radius: 263,
                quality: Quality::Draft,
                ..base
            },
        ),
        ("七叶叠加导出", false, base),
        (
            "噪声瞳孔导出",
            true,
            Request {
                source: Source::Blackbody(3200.),
                ..base
            },
        ),
    ] {
        for i in 0..6 {
            let start = Instant::now();
            let opening = if iris {
                Opening::Iris(Iris::default())
            } else {
                Opening::Blades(Blades::default())
            };
            let pupil = Pupil::new(opening, Some(Dust::default()), Some(Fibers::default()))?;
            let frame = black_box(solver.solve(&pupil, request, |_| true)?);
            println!(
                "{name} {}² 第 {} 次 {:.3} ms",
                frame.side,
                i + 1,
                1000. * start.elapsed().as_secs_f64()
            );
        }
    }
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
