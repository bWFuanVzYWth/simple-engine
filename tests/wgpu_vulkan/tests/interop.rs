//! wgpu/native 提交顺序、保活与窗口的集成回归。
#![forbid(unsafe_code)]
#![recursion_limit = "256"]

use simple_engine::{
    gpu_test::{self, Capability, Trial, with_shared},
    vulkan,
};
use std::{sync::Barrier, thread};
use vulkan::interop::WgpuContext;
use wgpu_vulkan_tests::{ComputeStages, Error, FrameTargets, submit_frame};

fn with_gpu<T>(run: impl FnOnce(&mut WgpuContext) -> Result<T, Error>) -> Result<T, String> {
    with_shared(Capability::RayTracing, |gpu| {
        let context = gpu.context_mut();
        let result = run(context);
        context
            .device()
            .poll(simple_engine::wgpu::PollType::wait_indefinitely())?;
        context.native().collect(&[])?;
        Ok(result)
    })
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

fn scene(context: &mut WgpuContext) -> Result<vulkan::AccelerationStructure, Error> {
    Ok(
        context.acceleration_structure(&[[
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]])?,
    )
}

fn roundtrip_preserves_every_pixel() -> Result<(), String> {
    let pending = with_gpu(|context| {
        let stages = ComputeStages::new(context)?;
        let targets = {
            let scene = scene(context)?;
            FrameTargets::new(context, scene, [32, 24])?
        };
        let pending = submit_frame(context, &stages, &targets, 1)?;
        Ok(pending)
    })?;
    pending.check().map_err(|error| error.to_string())
}

fn consecutive_submissions_reuse_targets() -> Result<(), String> {
    let pending = with_gpu(|context| {
        let stages = ComputeStages::new(context)?;
        let targets = {
            let scene = scene(context)?;
            FrameTargets::new(context, scene, [32, 24])?
        };
        let pending = [
            submit_frame(context, &stages, &targets, 1)?,
            submit_frame(context, &stages, &targets, 2)?,
        ];
        Ok(pending)
    })?;
    for frame in pending {
        frame.check().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn replaced_targets_survive_application_release() -> Result<(), String> {
    let pending = with_gpu(|context| {
        let stages = ComputeStages::new(context)?;
        let scene = scene(context)?;
        let old = FrameTargets::new(context, scene, [32, 24])?;
        let before = submit_frame(context, &stages, &old, 1)?;
        let targets = FrameTargets::new(context, scene, [37, 23])?;
        drop(old);
        let after = submit_frame(context, &stages, &targets, 2)?;
        Ok([before, after])
    })?;
    for frame in pending {
        frame.check().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn worker_batches_return_to_one_submit_owner() -> Result<(), String> {
    let pending = with_gpu(|context| {
        let stages = ComputeStages::new(context)?;
        let scene = scene(context)?;
        let targets = [
            FrameTargets::new(context, scene, [32, 24])?,
            FrameTargets::new(context, scene, [37, 23])?,
        ];
        let mut batches = Vec::new();
        for (index, target) in targets.iter().enumerate() {
            batches.push(wgpu_vulkan_tests::record_frame(
                context,
                &stages,
                target,
                index as u32 + 1,
            )?);
        }
        drop(targets);
        context.native().collect(&[])?;
        let barrier = Barrier::new(batches.len());
        let batches = thread::scope(|scope| {
            batches
                .into_iter()
                .map(|batch| {
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        batch
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|thread| thread.join().expect("工作线程崩溃"))
                .collect::<Vec<_>>()
        });
        let mut pending = Vec::new();
        for (command, frame) in batches.into_iter().rev() {
            context.submit([command])?;
            pending.push(frame);
        }
        context
            .device()
            .poll(simple_engine::wgpu::PollType::wait_indefinitely())?;
        Ok(pending)
    })?;
    for frame in pending {
        frame.check().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn window_resize_preserves_presentation() -> Result<(), String> {
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_window_probe"))
        .status()
        .map_err(|error| format!("启动窗口测试失败：{error}"))?;
    if !status.success() {
        return Err(format!("窗口 resize 测试失败：{status}"));
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    eprintln!("默认忽略：离屏用例需要 Vulkan KHR 光追管线；窗口用例还需要桌面。");
    gpu_test::run(vec![
        Trial::test("roundtrip_preserves_every_pixel", || {
            roundtrip_preserves_every_pixel().map_err(Into::into)
        })
        .with_kind("rt")
        .with_ignored_flag(true),
        Trial::test("consecutive_submissions_reuse_targets", || {
            consecutive_submissions_reuse_targets().map_err(Into::into)
        })
        .with_kind("rt")
        .with_ignored_flag(true),
        Trial::test("replaced_targets_survive_application_release", || {
            replaced_targets_survive_application_release().map_err(Into::into)
        })
        .with_kind("rt")
        .with_ignored_flag(true),
        Trial::test("worker_batches_return_to_one_submit_owner", || {
            worker_batches_return_to_one_submit_owner().map_err(Into::into)
        })
        .with_kind("rt")
        .with_ignored_flag(true),
        Trial::test("window_resize_preserves_presentation", || {
            window_resize_preserves_presentation().map_err(Into::into)
        })
        .with_kind("rt-window")
        .with_ignored_flag(true),
    ])
}
