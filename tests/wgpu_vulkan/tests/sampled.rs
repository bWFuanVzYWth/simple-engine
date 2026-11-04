//! 同设备 owner 创建的 D2/D3 采样图像、同步和命令保活。
#![forbid(unsafe_code)]
#![recursion_limit = "256"]

use simple_engine::{
    gpu_log,
    vulkan::{
        self,
        interop::{RayShader, RayTracingPipeline, SampledTexture, WgpuContext},
    },
    wgpu,
};
use std::{error::Error, sync::mpsc};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn texture(context: &WgpuContext, volume: bool) -> TestResult<SampledTexture> {
    Ok(context.sampled_texture(&wgpu::TextureDescriptor {
        label: Some("sampled interop source"),
        size: wgpu::Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: if volume { 2 } else { 1 },
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: if volume {
            wgpu::TextureDimension::D3
        } else {
            wgpu::TextureDimension::D2
        },
        format: if volume {
            wgpu::TextureFormat::Rgba16Float
        } else {
            wgpu::TextureFormat::Rgba32Float
        },
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })?)
}

struct Prepare {
    pipeline: wgpu::ComputePipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

impl Prepare {
    fn new(
        context: &WgpuContext,
        volume: &wgpu::Texture,
        plane: &wgpu::Texture,
    ) -> TestResult<Self> {
        let module = context.shader(
            include_bytes!(concat!(env!("OUT_DIR"), "/sampled_prepare.spv")),
            "sampled_prepare",
            [2, 2, 2],
        )?;
        let mut entries: Vec<_> = [
            wgpu::TextureViewDimension::D3,
            wgpu::TextureViewDimension::D2,
        ]
        .into_iter()
        .enumerate()
        .map(|(binding, dimension)| wgpu::BindGroupLayoutEntry {
            binding: binding as u32,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: if binding == 0 {
                    wgpu::TextureFormat::Rgba16Float
                } else {
                    wgpu::TextureFormat::Rgba32Float
                },
                view_dimension: dimension,
            },
            count: None,
        })
        .collect();
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let layout = context
            .device()
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sampled prepare"),
                entries: &entries,
            });
        let pipeline_layout =
            context
                .device()
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("sampled prepare"),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                });
        let pipeline = context
            .device()
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("sampled prepare"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("sampled_prepare"),
                compilation_options: Default::default(),
                cache: None,
            });
        let uniform = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("sampled base"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = context
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sampled prepare"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &volume.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &plane.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            });
        Ok(Self {
            pipeline,
            group,
            uniform,
        })
    }
    fn encode(&self, context: &WgpuContext, base: f32) -> wgpu::CommandBuffer {
        let values: Vec<_> = [base, 0.0, 0.0, 0.0]
            .into_iter()
            .flat_map(f32::to_ne_bytes)
            .collect();
        context.queue().write_buffer(&self.uniform, 0, &values);
        let mut encoder = context.device().create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("sampled prepare"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.finish()
    }
}

fn pipeline(
    context: &mut WgpuContext,
    volume: &SampledTexture,
    plane: &SampledTexture,
    output: &vulkan::interop::SharedBuffer,
) -> TestResult<RayTracingPipeline> {
    let scene =
        context.acceleration_structure(&[[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]])?;
    let imported = [volume.clone(), plane.clone()];
    Ok(RayTracingPipeline::new_sampled_buffers(
        context,
        scene,
        [
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/sampled_raygen.spv")),
                entry: c"sampled_raygen",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/sampled_miss.spv")),
                entry: c"sampled_miss",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/sampled_closest.spv")),
                entry: c"sampled_closest",
            },
        ],
        std::slice::from_ref(output),
        &imported,
    )?)
}

fn readback(
    context: &mut WgpuContext,
    output: &vulkan::interop::SharedBuffer,
    command: vulkan::interop::CommandBuffer,
) -> TestResult<[[f32; 4]; 4]> {
    let staging = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("sampled readback"),
        size: 64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device().create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(output.buffer(), 0, &staging, 0, 64);
    let submission = context.submit([command, encoder.finish().into()])?;
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
    let values = std::array::from_fn(|row| {
        std::array::from_fn(|channel| {
            let offset = (row * 4 + channel) * 4;
            f32::from_ne_bytes(mapped[offset..offset + 4].try_into().unwrap())
        })
    });
    drop(mapped);
    staging.unmap();
    Ok(values)
}

fn expected(base: f32) -> [[f32; 4]; 4] {
    [
        [base + 0.5, 2.0 * base + 0.5, 3.0 * base + 0.5, 1.0],
        [base, 2.0 * base + 1.0, 3.0 * base + 0.5, 1.0],
        [10.0 * base, 20.0 * base, 30.0 * base, 1.0],
        [10.0 * base + 1.0, 20.0 * base + 1.0, 30.0 * base + 2.0, 1.0],
    ]
}

#[test]
#[ignore = "需要真实 Vulkan KHR RT；主任务统一调度"]
fn sampled_commands_keep_texture_and_pipeline_owners() -> TestResult {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::new())?;
        let volume = texture(&context, true)?;
        let plane = texture(&context, false)?;
        let prepare = Prepare::new(&context, volume.texture(), plane.texture())?;
        let output = context.buffer(
            "sampled output",
            &[0; 64],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        )?;
        let pipeline = pipeline(&mut context, &volume, &plane, &output)?;
        let command = pipeline
            .encode(&mut context, [1, 1, 1])?
            .with_preceding([prepare.encode(&context, 1.0)]);
        drop(prepare);
        drop(volume);
        drop(plane);

        context.native().collect(&[])?;
        assert_eq!(readback(&mut context, &output, command)?, expected(1.0));
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan KHR RT；主任务统一调度"]
fn sampled_images_return_to_wgpu_storage_writes() -> TestResult {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::new())?;
        let volume = texture(&context, true)?;
        let plane = texture(&context, false)?;
        let prepare = Prepare::new(&context, volume.texture(), plane.texture())?;
        let output = context.buffer(
            "sampled output",
            &[0; 64],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        )?;
        let pipeline = pipeline(&mut context, &volume, &plane, &output)?;
        for base in [1.0, 4.0] {
            let command = pipeline
                .encode(&mut context, [1, 1, 1])?
                .with_preceding([prepare.encode(&context, base)]);
            assert_eq!(readback(&mut context, &output, command)?, expected(base));
        }
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan KHR RT；主任务统一调度"]
fn sampled_pipeline_rejects_a_foreign_device_owner() -> TestResult {
    gpu_log::checked(|| {
        let owner = pollster::block_on(WgpuContext::compute())?;
        let mut receiver = pollster::block_on(WgpuContext::new())?;
        let volume = texture(&owner, true)?;
        let plane = texture(&owner, false)?;
        let output = receiver.buffer(
            "foreign-owner output",
            &[0; 64],
            wgpu::BufferUsages::STORAGE,
        )?;
        let result = pipeline(&mut receiver, &volume, &plane, &output);
        let error = result.err().ok_or("错误接受其他设备的 sampled owner")?;
        assert!(matches!(
            error.downcast_ref::<vulkan::Error>(),
            Some(vulkan::Error::InvalidArgument(_))
        ));
        Ok(())
    })
}
