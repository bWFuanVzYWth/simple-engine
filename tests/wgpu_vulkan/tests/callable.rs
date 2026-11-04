//! 叶子 callable 的动态 SBT 索引、payload 与绑定资源生命周期。
#![forbid(unsafe_code)]
#![recursion_limit = "256"]

use simple_engine::{
    gpu_log,
    vulkan::interop::{CommandBuffer, RayShader, RayTracingPipeline, SharedBuffer, WgpuContext},
    wgpu,
};
use std::{error::Error, sync::mpsc};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const EXPECTED: [[f32; 4]; 4] = [
    [2.5, 7.0, 12.0, 4.0],
    [4.25, 5.5, 9.0, 4.0],
    [10.5, 11.0, 16.0, 4.0],
    [10.25, 7.5, 11.0, 4.0],
];

fn fixture(context: &mut WgpuContext) -> TestResult<(RayTracingPipeline, SharedBuffer)> {
    let scene =
        context.acceleration_structure(&[[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]])?;
    let output = context.buffer(
        "callable output",
        &[0; 64],
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    )?;
    let values = [
        [1.0_f32, 2.0, 3.0, 0.0],
        [2.0, 3.0, 4.0, 0.0],
        [3.0, 4.0, 5.0, 0.0],
        [4.0, 5.0, 6.0, 0.0],
    ];
    let bytes: Vec<_> = values
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    let source = context.buffer("callable input", &bytes, wgpu::BufferUsages::STORAGE)?;
    let tint = context.sampled_texture(&wgpu::TextureDescriptor {
        label: Some("callable tint"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })?;
    // 精确 HALF4：(.25,.5,1,1)，线性采样不会引入量化歧义。
    let texel: Vec<_> = [0x3400_u16, 0x3800, 0x3c00, 0x3c00]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    context.queue().write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tint.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &texel,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let pipeline = RayTracingPipeline::new_sampled_buffers_with_callables(
        context,
        scene,
        [
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/callable_raygen.spv")),
                entry: c"callable_raygen",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/callable_miss.spv")),
                entry: c"callable_miss",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/callable_closest.spv")),
                entry: c"callable_closest",
            },
        ],
        &[
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/callable_add.spv")),
                entry: c"callable_add",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/callable_scale.spv")),
                entry: c"callable_scale",
            },
        ],
        &[output.clone(), source],
        &[tint],
    )?;
    Ok((pipeline, output))
}

fn readback(
    context: &mut WgpuContext,
    output: SharedBuffer,
    command: CommandBuffer,
) -> TestResult<[[f32; 4]; 4]> {
    let staging = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("callable readback"),
        size: 64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device().create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(output.buffer(), 0, &staging, 0, 64);
    let copy = encoder.finish();
    drop(output);
    let submission = context.submit([command, copy.into()])?;
    let (sender, receiver) = mpsc::sync_channel(1);
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    receiver.recv()??;
    let mapped = staging.slice(..).get_mapped_range()?;
    let values = std::array::from_fn(|pixel| {
        std::array::from_fn(|channel| {
            let offset = (pixel * 4 + channel) * 4;
            f32::from_le_bytes(mapped[offset..offset + 4].try_into().unwrap())
        })
    });
    drop(mapped);
    staging.unmap();
    Ok(values)
}

#[test]
#[ignore = "需要真实 Vulkan KHR RT；主任务统一调度"]
fn callable_indices_roundtrip_payload_and_resources() -> TestResult {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::new())?;
        let (pipeline, output) = fixture(&mut context)?;
        let command = pipeline.encode(&mut context, [4, 1, 1])?;
        assert_eq!(readback(&mut context, output, command)?, EXPECTED);
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan KHR RT；主任务统一调度"]
fn callable_command_keeps_pipeline_and_sbt_alive() -> TestResult {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::new())?;
        let (pipeline, output) = fixture(&mut context)?;
        let command = pipeline.encode(&mut context, [4, 1, 1])?;

        context.native().collect(&[])?;
        assert_eq!(readback(&mut context, output, command)?, EXPECTED);
        Ok(())
    })
}
