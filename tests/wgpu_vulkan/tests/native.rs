//! 通用 native owner 的 any-hit 资源访问、RT→compute 同步和 wgpu 提交租约。
#![forbid(unsafe_code)]
#![recursion_limit = "256"]

use simple_engine::{
    gpu_log,
    vulkan::{
        interop::{RayShader, WgpuContext},
        native::{self, Binding},
    },
    wgpu,
};
use std::{error::Error, sync::mpsc};

#[test]
#[ignore = "需要真实Vulkan EXT SER设备；独占GPU"]
fn hit_object_dispatch_keeps_hit_and_callable_indices_independent() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::new())?;
        let geometry = context
            .native()
            .triangle_blas(&[[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]], true)?;
        let scene = context.native().acceleration_instances(&[
            simple_engine::vulkan::TriangleInstance {
                geometry,
                transform: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]],
                custom_index: 17,
            },
        ])?;
        let output = native::Buffer::new(context.native(), 64 * 16, true)?;
        let bindings = native::Bindings::new(
            context.native(),
            &[Binding::Acceleration(scene), Binding::StorageBuffer(output)],
        )?;
        macro_rules! shader {
            ($name:literal) => {
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/", $name, ".spv")),
                    entry: std::ffi::CStr::from_bytes_with_nul(concat!($name, "\0").as_bytes())
                        .unwrap(),
                }
            };
        }
        let program = native::RayTracingPipeline::with_hit_groups(
            context.native(),
            bindings,
            shader!("hit_groups_raygen"),
            shader!("hit_groups_miss"),
            &[
                shader!("hit_groups_first"),
                shader!("hit_groups_second"),
                shader!("hit_groups_third"),
            ],
            None,
            &[shader!("hit_groups_add"), shader!("hit_groups_scale")],
        )?;
        let mut commands = native::Commands::new(context.native())?;
        commands.trace(program, [64, 1])?;
        commands
            .finish(context.native())?
            .submit_and_wait(context.native())?;
        let mut bytes = vec![0; 64 * 16];
        output.read(context.native(), 0, &mut bytes)?;
        for (index, record) in bytes.as_chunks::<16>().0.iter().enumerate() {
            let value = (index as u32 % 3 + 1) * 10;
            let expected = if index % 2 == 0 { value + 3 } else { value * 2 };
            let words: [u32; 4] =
                std::array::from_fn(|i| u32::from_le_bytes(record.as_chunks::<4>().0[i]));
            assert_eq!(words, [expected, 1f32.to_bits(), 17, 0], "lane {index}");
        }
        Ok(())
    })
}

#[test]
fn reorder_shader_uses_ext_contract() {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/reorder_raygen.spv"));
    let words: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect();
    let mut capabilities = Vec::new();
    let mut reorder = Vec::new();
    let mut cursor = 5;
    while cursor < words.len() {
        let count = (words[cursor] >> 16) as usize;
        let opcode = words[cursor] & 0xffff;
        assert!(count > 0 && count <= words.len() - cursor);
        if opcode == 17 {
            capabilities.push(words[cursor + 1]);
        } else if matches!(opcode, 5279 | 5280 | 5314 | 5315) {
            reorder.push(opcode);
        }
        cursor += count;
    }
    assert!(capabilities.contains(&5388));
    assert!(!capabilities.contains(&5383));
    assert_eq!(reorder, [5314]);
}

#[test]
#[ignore = "需要真实 Vulkan EXT SER、position fetch 与 OMM；主任务统一调度"]
fn invocation_reorder_auto_detection_can_be_disabled() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let automatic = pollster::block_on(WgpuContext::new())?;
        assert!(automatic.native_ref().capabilities().invocation_reorder);
        drop(automatic);
        let disabled = pollster::block_on(WgpuContext::with_capabilities(
            0,
            native::Capabilities {
                ray_tracing: true,
                position_fetch: true,
                opacity_micromap: true,
                ..Default::default()
            },
            wgpu::Features::empty(),
        ))?;
        assert!(!disabled.native_ref().capabilities().invocation_reorder);
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn invocation_reorder_is_disabled_for_compute() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let context = pollster::block_on(WgpuContext::compute())?;
        assert!(!context.native_ref().capabilities().invocation_reorder);
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan EXT SER；主任务统一调度"]
fn invocation_reorder_preserves_dispatch_values() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::with_capabilities(
            0,
            native::Capabilities {
                ray_tracing: true,
                invocation_reorder: true,
                ..Default::default()
            },
            wgpu::Features::empty(),
        ))?;
        assert!(context.native_ref().capabilities().invocation_reorder);
        let values: Vec<_> = (0..256u32)
            .map(|index| index.wrapping_mul(2_654_435_761))
            .collect();
        let source = native::Buffer::upload(
            context.native(),
            &values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>(),
        )?;
        let output = native::Buffer::new(context.native(), values.len() as u64 * 8, true)?;
        let bindings = native::Bindings::new(
            context.native(),
            &[
                Binding::StorageBuffer(output),
                Binding::StorageBuffer(source),
            ],
        )?;
        let pipeline = native::RayTracingPipeline::new(
            context.native(),
            bindings,
            [
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/reorder_raygen.spv")),
                    entry: c"reorder_raygen",
                },
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/reorder_miss.spv")),
                    entry: c"reorder_miss",
                },
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/reorder_closest.spv")),
                    entry: c"reorder_closest",
                },
            ],
            None,
            &[],
        )?;
        let mut commands = native::Commands::new(context.native())?;
        commands.trace(pipeline, [values.len() as u32, 1])?;
        commands
            .finish(context.native())?
            .submit_and_wait(context.native())?;
        let mut bytes = vec![0; values.len() * 8];
        output.read(context.native(), 0, &mut bytes)?;
        for (index, (actual, source)) in bytes.as_chunks::<8>().0.iter().zip(values).enumerate() {
            let mut expected = [0; 8];
            expected[..4].copy_from_slice(&(source ^ 0x1357_9bdf).to_le_bytes());
            expected[4..].copy_from_slice(&(index as u32).to_le_bytes());
            assert_eq!(*actual, expected, "dispatch {index}");
        }
        Ok(())
    })
}

fn ray_program(
    context: &mut native::Context,
    bindings: native::Bindings,
) -> Result<native::RayTracingPipeline, simple_engine::vulkan::Error> {
    native::RayTracingPipeline::new(
        context,
        bindings,
        [
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/native_raygen.spv")),
                entry: c"native_raygen",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/native_miss.spv")),
                entry: c"native_miss",
            },
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/native_closest.spv")),
                entry: c"native_closest",
            },
        ],
        Some(RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/native_any_hit.spv")),
            entry: c"native_any_hit",
        }),
        &[],
    )
}

#[test]
#[ignore = "需要真实 Vulkan KHR RT；主任务统一调度"]
fn ray_binding_versions_preserve_recorded_resources() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::with_capabilities(
            0,
            native::Capabilities {
                ray_tracing: true,
                ..Default::default()
            },
            wgpu::Features::empty(),
        ))?;
        let geometry = context.native().triangle_blas(
            &[
                [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
                [[0., 0., 1.], [1., 0., 1.], [0., 1., 1.]],
            ],
            false,
        )?;
        let mut bindings = Vec::new();
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        for (index, values) in [[0u32, 11], [7, 0]].into_iter().enumerate() {
            let source = native::Buffer::new(context.native(), 8, true)?;
            source.write(
                context.native(),
                0,
                &values
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )?;
            let output = native::Buffer::new(context.native(), 4, true)?;
            let scene = context.native().acceleration_instances(&[
                simple_engine::vulkan::TriangleInstance {
                    geometry,
                    transform: [
                        [1., 0., 0., 0.],
                        [0., 1., 0., 0.],
                        [0., 0., 1., index as f32 * 0.1],
                    ],
                    custom_index: 0,
                },
            ])?;
            bindings.push(native::Bindings::new(
                context.native(),
                &[
                    Binding::Acceleration(scene),
                    Binding::StorageBuffer(output),
                    Binding::StorageBuffer(source),
                ],
            )?);
            inputs.push(source);
            outputs.push(output);
        }
        let template = ray_program(context.native(), bindings[0])?;
        let second = template.rebind(context.native(), bindings[1])?;
        let retained_program = second;
        let mut first = native::Commands::new(context.native())?;
        first.trace(template, [1, 1])?;
        let first = first.finish(context.native())?;
        let mut second_commands = native::Commands::new(context.native())?;
        second_commands.trace(second, [1, 1])?;
        let second = second_commands.finish(context.native())?;
        drop(bindings);
        // 描述符旧版本只由命令保留；CPU后续读取/写入的资源须显式发布。
        context.native().collect(&[
            retained_program.into(),
            inputs[0].into(),
            outputs[0].into(),
        ])?;
        assert!(inputs[0].write(context.native(), 0, &[0; 4]).is_err());
        let first = context.record_native(first)?;
        let second = context.record_native(second)?;
        let submission = context.submit([first, second])?;
        context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        for (output, expected) in outputs.iter().zip([14, 10]) {
            let mut bytes = [0; 4];
            output.read(context.native(), 0, &mut bytes)?;
            assert_eq!(u32::from_le_bytes(bytes), expected);
        }
        inputs[0].write(context.native(), 0, &[0; 4])?;
        context
            .native()
            .collect(&[retained_program.into(), inputs[0].into()])?;
        let mut bytes = [0; 4];
        assert!(outputs[0].read(context.native(), 0, &mut bytes).is_err());
        outputs[1].read(context.native(), 0, &mut bytes)?;
        assert_eq!(u32::from_le_bytes(bytes), 10);
        context.native().collect(&[])?;
        assert!(outputs[1].read(context.native(), 0, &mut bytes).is_err());
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan KHR RT；主任务统一调度"]
fn ray_rebind_rejects_incompatible_descriptor_layout() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::with_capabilities(
            0,
            native::Capabilities {
                ray_tracing: true,
                ..Default::default()
            },
            wgpu::Features::empty(),
        ))?;
        let source = native::Buffer::new(context.native(), 16, true)?;
        let geometry = context
            .native()
            .triangle_blas(&[[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]], false)?;
        let scene = context.native().acceleration_instances(&[
            simple_engine::vulkan::TriangleInstance {
                geometry,
                transform: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]],
                custom_index: 0,
            },
        ])?;
        let bindings = native::Bindings::new(
            context.native(),
            &[
                Binding::Acceleration(scene),
                Binding::StorageBuffer(source),
                Binding::StorageBuffer(source),
            ],
        )?;
        // 不提交 shader；仅检查重绑定的 Vulkan layout 契约。
        let template = ray_program(context.native(), bindings)?;
        let short = native::Bindings::new(context.native(), &[Binding::StorageBuffer(source)])?;
        assert!(matches!(
            template.rebind(context.native(), short),
            Err(simple_engine::vulkan::Error::InvalidArgument(_))
        ));
        let changed = native::Bindings::new(
            context.native(),
            &[
                Binding::Acceleration(scene),
                Binding::StorageBuffer(source),
                Binding::UniformBuffer(source),
            ],
        )?;
        assert!(matches!(
            template.rebind(context.native(), changed),
            Err(simple_engine::vulkan::Error::InvalidArgument(_))
        ));
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan KHR RT；主任务统一调度"]
fn any_hit_compute_and_shared_resource_retirement() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::with_capabilities(
            0,
            native::Capabilities {
                ray_tracing: true,
                ..Default::default()
            },
            wgpu::Features::empty(),
        ))?;
        let geometry = context.native().triangle_blas(
            &[
                [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
                [[0., 0., 1.], [1., 0., 1.], [0., 1., 1.]],
            ],
            false,
        )?;
        let scene = context.native().acceleration_instances(&[
            simple_engine::vulkan::TriangleInstance {
                geometry,
                transform: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]],
                custom_index: 0,
            },
        ])?;
        let output = context.buffer(
            "native output",
            &[0; 4],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        )?;
        let source = context.buffer(
            "native alpha source",
            &[0, 0, 0, 0, 11, 0, 0, 0],
            wgpu::BufferUsages::STORAGE,
        )?;
        let bindings = native::Bindings::new(
            context.native(),
            &[
                Binding::Acceleration(scene),
                Binding::SharedBuffer(output.clone()),
                Binding::SharedBuffer(source),
            ],
        )?;
        let trace = native::RayTracingPipeline::new(
            context.native(),
            bindings,
            [
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/native_raygen.spv")),
                    entry: c"native_raygen",
                },
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/native_miss.spv")),
                    entry: c"native_miss",
                },
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/native_closest.spv")),
                    entry: c"native_closest",
                },
            ],
            Some(RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/native_any_hit.spv")),
                entry: c"native_any_hit",
            }),
            &[],
        )?;
        let post = native::ComputePipeline::new(
            context.native(),
            bindings,
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/native_post.spv")),
                entry: c"native_post",
            },
        )?;
        let mut commands = native::Commands::new(context.native())?;
        commands.trace(trace, [1, 1])?;
        commands.barrier();
        commands.dispatch(post, [1, 1, 1])?;
        let recorded = commands.finish(context.native())?;
        let command = context.record_native(recorded)?;
        context.native().collect(&[])?;
        let staging = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("native readback"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut copy = context.device().create_command_encoder(&Default::default());
        copy.copy_buffer_to_buffer(output.buffer(), 0, &staging, 0, 4);
        drop(output);
        let submission = context.submit([command, copy.finish().into()])?;
        let (send, receive) = mpsc::sync_channel(1);
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        receive.recv()??;
        let mapped = staging.slice(..).get_mapped_range()?;
        assert_eq!(u32::from_le_bytes(mapped[..4].try_into()?), 29);
        drop(mapped);
        staging.unmap();
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn native_image_upload_download_preserves_srgb_bytes() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let image =
            native::Image::new(context.native(), [2, 1, 1], native::ImageFormat::Rgba8Srgb)?;
        let bytes = [3, 17, 128, 255, 255, 201, 64, 0];
        image.upload(context.native(), &bytes)?;
        assert_eq!(image.download(context.native())?, bytes);
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn srgb_image_rejects_storage_binding() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let image =
            native::Image::new(context.native(), [1, 1, 1], native::ImageFormat::Rgba8Srgb)?;
        assert!(matches!(
            native::Bindings::new(context.native(), &[Binding::StorageImage(image)]),
            Err(simple_engine::vulkan::Error::InvalidArgument(_))
        ));
        native::Bindings::new(context.native(), &[Binding::SampledImage(image)])?;
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn keepalive_does_not_reserve_host_access() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        eprintln!("Vulkan 访问协议设备：{:?}", context.adapter_info());
        let buffer = native::Buffer::new(context.native(), 4, true)?;
        let mut commands = native::Commands::new(context.native())?;
        commands.keepalive(buffer.into())?;
        let recorded = commands.finish(context.native())?;
        buffer.write(context.native(), 0, &[1, 2, 3, 4])?;
        context.native().cancel(recorded)?;
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn keepalive_preserves_binding_dependencies() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let buffer = native::Buffer::new(context.native(), 4, true)?;
        let bindings = native::Bindings::new(context.native(), &[Binding::StorageBuffer(buffer)])?;
        let mut commands = native::Commands::new(context.native())?;
        commands.keepalive(bindings.into())?;
        let recorded = commands.finish(context.native())?;
        context.native().collect(&[])?;
        context.native().collect(&[bindings.into()])?;
        buffer.write(context.native(), 0, &[1, 2, 3, 4])?;
        context.native().cancel(recorded)?;
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn declared_buffer_access_blocks_mapping_until_cancelled() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let buffer = native::Buffer::new(context.native(), 4, true)?;
        let mut commands = native::Commands::new(context.native())?;
        commands.access(buffer.into())?;
        let recorded = commands.finish(context.native())?;
        context.native().collect(&[])?;
        context.native().collect(&[buffer.into()])?;
        assert!(matches!(
            buffer.write(context.native(), 0, &[1, 2, 3, 4]),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        assert!(matches!(
            buffer.read(context.native(), 0, &mut [0; 4]),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        context.native().cancel(recorded)?;
        buffer.write(context.native(), 0, &[1, 2, 3, 4])?;
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn declared_image_access_blocks_transfers_until_cancelled() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let image =
            native::Image::new(context.native(), [1, 1, 1], native::ImageFormat::Rgba8Unorm)?;
        let mut commands = native::Commands::new(context.native())?;
        commands.access(image.into())?;
        let recorded = commands.finish(context.native())?;
        assert!(matches!(
            image.upload(context.native(), &[1, 2, 3, 4]),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        assert!(matches!(
            image.download(context.native()),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        context.native().cancel(recorded)?;
        image.upload(context.native(), &[1, 2, 3, 4])?;
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn buffer_mapping_waits_for_recorded_use_retirement() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let buffer = native::Buffer::new(context.native(), 4, true)?;
        let input = 11u32.to_le_bytes();
        buffer.write(context.native(), 0, &input)?;
        let mut commands = native::Commands::new(context.native())?;
        commands.clear_buffer(buffer)?;
        let recorded = commands.finish(context.native())?;
        assert!(matches!(
            buffer.write(context.native(), 0, &input),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        assert!(matches!(
            buffer.read(context.native(), 0, &mut [0; 4]),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        let mut canceled = native::Commands::new(context.native())?;
        canceled.clear_buffer(buffer)?;
        let canceled = canceled.finish(context.native())?;
        context.native().cancel(canceled)?;
        assert!(matches!(
            buffer.write(context.native(), 0, &input),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        let command = context.record_native(recorded)?;
        let submission = context.submit([command])?;
        context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        let mut actual = [0; 4];
        buffer.read(context.native(), 0, &mut actual)?;
        assert_eq!(actual, [0; 4]);
        buffer.write(context.native(), 0, &input)?;
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn retired_versions_do_not_wait_for_later_batches() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let current = native::Buffer::new(context.native(), 4, true)?;
        let current_bytes = context.allocated_native_bytes();
        let old = native::Buffer::new(context.native(), 4, true)?;
        let mut commands = native::Commands::new(context.native())?;
        commands.clear_buffer(old)?;
        let first = commands.finish(context.native())?;
        context.native().collect(&[current.into()])?;
        let mut commands = native::Commands::new(context.native())?;
        commands.clear_buffer(current)?;
        let later = commands.finish(context.native())?;
        context.native().collect(&[current.into()])?;
        context.native().cancel(first)?;
        assert_eq!(context.allocated_native_bytes(), current_bytes);
        assert!(matches!(
            current.write(context.native(), 0, &[0; 4]),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        context.native().cancel(later)?;
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn republished_resource_uses_new_retirement_waits() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let buffer = native::Buffer::new(context.native(), 4, true)?;
        let allocated = context.allocated_native_bytes();
        let mut commands = native::Commands::new(context.native())?;
        commands.keepalive(buffer.into())?;
        let first = commands.finish(context.native())?;
        context.native().collect(&[])?;
        context.native().collect(&[buffer.into()])?;

        let mut commands = native::Commands::new(context.native())?;
        commands.keepalive(buffer.into())?;
        let later = commands.finish(context.native())?;
        context.native().collect(&[])?;
        context.native().cancel(first)?;
        assert_eq!(context.allocated_native_bytes(), allocated);
        context.native().cancel(later)?;
        assert_eq!(context.allocated_native_bytes(), 0);
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn failed_collect_preserves_resources_and_retirement() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let current = native::Buffer::new(context.native(), 4, true)?;
        let current_bytes = context.allocated_native_bytes();
        let retiring = native::Buffer::new(context.native(), 4, true)?;
        let retiring_bytes = context.allocated_native_bytes() - current_bytes;
        let stale = native::Buffer::new(context.native(), 4, true)?;
        let mut commands = native::Commands::new(context.native())?;
        commands.keepalive(retiring.into())?;
        let recorded = commands.finish(context.native())?;
        context.native().collect(&[current.into()])?;
        let candidate = native::Buffer::new(context.native(), 4, true)?;
        let before = context.allocated_native_bytes();

        assert!(matches!(
            context.native().collect(&[stale.into(), current.into()]),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        assert_eq!(context.allocated_native_bytes(), before);
        current.write(context.native(), 0, &[1, 2, 3, 4])?;
        candidate.write(context.native(), 0, &[5, 6, 7, 8])?;
        context.native().cancel(recorded)?;
        assert_eq!(context.allocated_native_bytes(), before - retiring_bytes);
        context.native().collect(&[current.into()])?;
        assert_eq!(context.allocated_native_bytes(), current_bytes);
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn failed_mixed_batch_cancels_only_its_pending_records() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let mut foreign = pollster::block_on(WgpuContext::compute())?;
        let buffers = [
            native::Buffer::new(context.native(), 4, true)?,
            native::Buffer::new(context.native(), 4, true)?,
        ];
        let unrelated = native::Buffer::new(context.native(), 4, true)?;
        let mut commands = native::Commands::new(context.native())?;
        commands.clear_buffer(unrelated)?;
        let unrelated_record = commands.finish(context.native())?;
        let mut batch = Vec::new();
        for buffer in buffers {
            let mut commands = native::Commands::new(context.native())?;
            commands.clear_buffer(buffer)?;
            let recorded = commands.finish(context.native())?;
            batch.push(context.record_native(recorded)?);
        }
        let foreign_buffer = native::Buffer::new(foreign.native(), 4, true)?;
        let mut commands = native::Commands::new(foreign.native())?;
        commands.clear_buffer(foreign_buffer)?;
        let recorded = commands.finish(foreign.native())?;
        let foreign_command = foreign.record_native(recorded)?;
        let last = batch.pop().expect("已录制末段");
        let first = batch.pop().expect("已录制首段");
        assert!(matches!(
            context.submit([first.then(foreign_command).then(last)]),
            Err(simple_engine::vulkan::Error::InvalidArgument(_))
        ));
        for buffer in buffers {
            buffer.write(context.native(), 0, &11u32.to_le_bytes())?;
        }
        assert!(matches!(
            unrelated.write(context.native(), 0, &[0; 4]),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        assert!(matches!(
            foreign_buffer.write(foreign.native(), 0, &[0; 4]),
            Err(simple_engine::vulkan::Error::InvalidState(_))
        ));
        context.native().cancel(unrelated_record)?;
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn uniform_versions_share_program_and_preserve_bindings() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let inputs = [
            native::Buffer::new(context.native(), 16, true)?,
            native::Buffer::new(context.native(), 16, true)?,
        ];
        let outputs = [
            native::Buffer::new(context.native(), 4, true)?,
            native::Buffer::new(context.native(), 4, true)?,
        ];
        let values = [11u32, 29];
        for (input, value) in inputs.iter().zip(values) {
            input.write(context.native(), 0, &value.to_le_bytes())?;
        }
        let groups = [0, 1]
            .map(|index| {
                native::Bindings::new(
                    context.native(),
                    &[
                        Binding::UniformBuffer(inputs[index]),
                        Binding::StorageBuffer(outputs[index]),
                    ],
                )
            })
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let template = native::ComputePipeline::new(
            context.native(),
            groups[0],
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/uniform_main.spv")),
                entry: c"uniform_main",
            },
        )?;
        let first = template.rebind(context.native(), groups[0])?;
        let second = template.rebind(context.native(), groups[1])?;
        let incompatible = native::Bindings::new(
            context.native(),
            &[
                Binding::StorageBuffer(inputs[0]),
                Binding::StorageBuffer(outputs[0]),
            ],
        )?;
        assert!(template.rebind(context.native(), incompatible).is_err());
        // program/version 仅为身份，由 owner 的根集合与待执行批次保留。
        drop(groups);
        let mut commands = native::Commands::new(context.native())?;
        commands.dispatch(first, [1; 3])?;
        commands.dispatch(second, [1; 3])?;
        let recorded = commands.finish(context.native())?;
        assert!(
            inputs[0].write(context.native(), 0, &[0; 4]).is_err(),
            "uniform 同样受录制租约约束"
        );
        recorded.submit_and_wait(context.native())?;
        for (output, value) in outputs.iter().zip(values) {
            let mut actual = [0; 4];
            output.read(context.native(), 0, &mut actual)?;
            assert_eq!(u32::from_le_bytes(actual), value);
        }
        inputs[0].write(context.native(), 0, &[0; 4])?;
        Ok(())
    })
}
