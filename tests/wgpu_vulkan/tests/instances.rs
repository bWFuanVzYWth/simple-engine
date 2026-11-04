//! 同一 BLAS 的变换实例、24-bit custom ID 与 native 提交所有权回归。
#![forbid(unsafe_code)]
#![recursion_limit = "256"]

use simple_engine::{
    gpu_log,
    vulkan::{
        OPACITY_FULLY_OPAQUE, OpacityMap, TriangleInstance,
        interop::{RayShader, RayTracingPipeline, WgpuContext},
    },
    wgpu,
};
use std::{error::Error, sync::mpsc};

type TestResult<T> = Result<T, Box<dyn Error>>;

fn transformed_shared_instances() -> TestResult<()> {
    let mut context = pollster::block_on(WgpuContext::new())?;
    let blas = context.triangle_blas(&[
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        [[2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0]],
    ])?;
    let instances = [
        TriangleInstance {
            geometry: blas,
            transform: [
                [2.0, 0.0, 0.0, 10.0],
                [0.0, 3.0, 0.0, 20.0],
                [0.0, 0.0, 1.0, 2.0],
            ],
            custom_index: 7,
        },
        TriangleInstance {
            geometry: blas,
            transform: [
                [0.0, -2.0, 0.0, -10.0],
                [3.0, 0.0, 0.0, 5.0],
                [0.0, 0.0, 0.5, 7.0],
            ],
            custom_index: 0x00ff_fffe,
        },
    ];
    let scene = context.acceleration_instances(&instances)?;
    // 四条射线分别穿过两个实例内的 primitive 0/1；独立给定世界坐标避免重演矩阵打包。
    let origins: [[f32; 4]; 4] = [
        [10.5, 20.75, -3.0, 0.0],
        [14.5, 20.75, -3.0, 0.0],
        [-10.5, 5.75, -3.0, 0.0],
        [-10.5, 11.75, -3.0, 0.0],
    ];
    let directions = [[0.0_f32, 0.0, 1.0, 100.0]; 4];
    let encoded: Vec<_> = origins
        .into_iter()
        .chain(directions)
        .flatten()
        .flat_map(f32::to_ne_bytes)
        .collect();
    let rays = context.buffer("instance rays", &encoded, wgpu::BufferUsages::STORAGE)?;
    let result = context.buffer(
        "instance identities",
        &[0; 48],
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    )?;
    let pipeline = RayTracingPipeline::new_buffers(
        &mut context,
        scene,
        [
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/instance_raygen.spv")),
                entry: c"instance_raygen",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/instance_miss.spv")),
                entry: c"instance_miss",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/instance_closest.spv")),
                entry: c"instance_closest",
            },
        ],
        &[result.clone(), rays.clone()],
    )?;
    drop(rays);
    let command = pipeline.encode(&mut context, [4, 1, 1])?;
    // 不再发布场景身份，已录制批次仍须保留 TLAS/BLAS 与其依赖。
    context.native().collect(&[])?;

    let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("instance identities readback"),
        size: 48,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut copy = context.device().create_command_encoder(&Default::default());
    copy.copy_buffer_to_buffer(result.buffer(), 0, &readback, 0, 48);
    drop(result);
    let submission = context.submit([command, copy.finish().into()])?;
    let (send, receive) = mpsc::sync_channel(1);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    receive.recv()??;
    let mapped = readback.slice(..).get_mapped_range()?;
    let actual: Vec<_> = mapped
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_ne_bytes(*word))
        .collect();
    drop(mapped);
    readback.unmap();
    let expected = [
        7,
        7,
        0x00ff_fffe,
        0x00ff_fffe,
        0,
        1,
        0,
        1,
        5.0_f32.to_bits(),
        5.0_f32.to_bits(),
        10.0_f32.to_bits(),
        10.0_f32.to_bits(),
    ];
    if actual != expected {
        return Err(format!("变换实例的身份/距离不符：实际 {actual:?}，预期 {expected:?}").into());
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan KHR RT；由 GPU 验收串行运行"]
fn shared_blas_keeps_transforms_custom_ids_and_submission_owners() -> TestResult<()> {
    gpu_log::checked(transformed_shared_instances)
}

fn binary_opacity_survives_owner_release() -> TestResult<()> {
    let mut context = pollster::block_on(WgpuContext::new())?;
    let micromap = {
        let bits = vec![0b0101];
        context.opacity_micromap(&[OpacityMap {
            subdivision_level: 1,
            data: &bits,
        }])?
    };
    let blas = {
        let triangles: Vec<_> = [0.0, 2.0, 4.0]
            .map(|x| [[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 1.0, 0.0]])
            .into();
        let indices = vec![0, 0, OPACITY_FULLY_OPAQUE];
        context.triangle_blas_with_opacity(&triangles, micromap, &indices)?
    };

    // micromap 只是身份，资源退休由 context.collect 与命令批次决定。
    let scene = context.acceleration_instances(&[TriangleInstance {
        geometry: blas,
        transform: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]],
        custom_index: 17,
    }])?;

    // blas 只是身份，资源退休由 context.collect 与命令批次决定。
    // 四个远离边界的 barycentric 点分别位于 vertex0、中心、vertex1、vertex2 子三角形。
    let barycentrics = [[0.125, 0.125], [0.375, 0.375], [0.75, 0.125], [0.125, 0.75]];
    let origins: Vec<[f32; 4]> = [0.0, 2.0, 4.0]
        .into_iter()
        .flat_map(|offset| barycentrics.map(|[u, v]| [offset + u, v, -1.0, 0.0]))
        .collect();
    let count = origins.len() as u32;
    let encoded: Vec<_> = origins
        .into_iter()
        .chain(vec![[0.0, 0.0, 1.0, 10.0]; count as usize])
        .flatten()
        .flat_map(f32::to_ne_bytes)
        .collect();
    let rays = context.buffer("OMM rays", &encoded, wgpu::BufferUsages::STORAGE)?;
    let result = context.buffer(
        "OMM identities",
        &vec![0; count as usize * 12],
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    )?;
    let pipeline = RayTracingPipeline::new_buffers(
        &mut context,
        scene,
        [
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/instance_raygen.spv")),
                entry: c"instance_raygen",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/instance_miss.spv")),
                entry: c"instance_miss",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/instance_closest.spv")),
                entry: c"instance_closest",
            },
        ],
        &[result.clone(), rays.clone()],
    )?;

    // scene 只是身份，资源退休由 context.collect 与命令批次决定。
    drop(rays);
    let command = pipeline.encode(&mut context, [count, 1, 1])?;

    context.native().collect(&[])?;
    let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("OMM identities readback"),
        size: u64::from(count) * 12,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut copy = context.device().create_command_encoder(&Default::default());
    copy.copy_buffer_to_buffer(result.buffer(), 0, &readback, 0, readback.size());
    drop(result);
    let submission = context.submit([command, copy.finish().into()])?;
    let (send, receive) = mpsc::sync_channel(1);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    receive.recv()??;
    let mapped = readback.slice(..).get_mapped_range()?;
    let actual: Vec<_> = mapped
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_ne_bytes(*word))
        .collect();
    for ray in 0..count as usize {
        let hit = ray >= 8 || [true, false, true, false][ray % 4];
        assert_eq!(
            actual[ray],
            if hit { 17 } else { u32::MAX },
            "ray {ray} instance"
        );
        assert_eq!(
            actual[count as usize + ray],
            if hit { (ray / 4) as u32 } else { u32::MAX },
            "ray {ray} primitive"
        );
        assert_eq!(
            f32::from_bits(actual[2 * count as usize + ray]),
            if hit { 1.0 } else { 0.0 },
            "ray {ray} distance"
        );
    }
    drop(mapped);
    readback.unmap();
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan 2-state OMM；由 GPU 验收串行运行"]
fn binary_micromap_references_preserve_opacity_after_owner_release() -> TestResult<()> {
    gpu_log::checked(binary_opacity_survives_owner_release)
}
