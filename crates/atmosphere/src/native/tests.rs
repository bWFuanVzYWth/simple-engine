use super::*;
use crate::gpu::{Atmosphere as WgpuAtmosphere, Shaders};
use std::{error::Error as StdError, sync::mpsc};
use vulkan::interop::WgpuContext;

type Result<T = ()> = std::result::Result<T, Box<dyn StdError>>;

fn read(context: &WgpuContext, texture: &wgpu::Texture) -> Result<Vec<u8>> {
    let size = texture.size();
    let texel = if texture.format() == wgpu::TextureFormat::Rgba16Float {
        8
    } else {
        16
    };
    let row = size.width * texel;
    let bytes = u64::from(row) * u64::from(size.height) * u64::from(size.depth_or_array_layers);
    let output = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("native atmosphere comparison"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device().create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &output,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    let submission = context.queue().submit([encoder.finish()]);
    let (sender, receiver) = mpsc::sync_channel(1);
    output
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |value| {
            let _ = sender.send(value);
        });
    context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    receiver.recv()??;
    let result = output.slice(..).get_mapped_range()?.to_vec();
    output.unmap();
    Ok(result)
}

#[test]
#[ignore = "需要真实 Vulkan GPU；主任务统一调度"]
fn native_and_wgpu_share_fields_and_view_updates() -> Result {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let modules: Vec<_> = SHADERS
            .iter()
            .map(|s| context.shader(s.spirv, s.entry, s.workgroup))
            .collect::<std::result::Result<_, _>>()?;
        let mut reference = WgpuAtmosphere::new(
            context.device(),
            context.queue(),
            Shaders {
                modules: modules.try_into().map_err(|_| "shader count")?,
            },
        )?;
        let mut native = Atmosphere::new(context.native())?;
        let mut encoder = context.device().create_command_encoder(&Default::default());
        reference.initialize(&mut encoder)?;
        context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(context.queue().submit([encoder.finish()])),
            timeout: None,
        })?;
        native
            .initialize(context.native())?
            .submit_and_wait(context.native())?;
        reference.release_initialization();
        native.release_initialization();
        let first = View {
            altitude_km: 0.2,
            sun_direction: [0.0, 0.6, 0.8],
            max_distance_km: 3.0,
            ..View::default()
        };
        let second = View {
            sun_direction: [0.8, 0.6, 0.0],
            radiance_scale: [0.5, 1.0, 2.0],
            ..first
        };
        for (view, expected) in [
            (
                first,
                Update {
                    sky: true,
                    aerial: true,
                },
            ),
            (
                second,
                Update {
                    sky: false,
                    aerial: true,
                },
            ),
        ] {
            assert_eq!(reference.prepare(view)?, expected);
            assert_eq!(native.prepare(context.native(), view)?, expected);
            let mut encoder = context.device().create_command_encoder(&Default::default());
            assert_eq!(reference.record(&mut encoder), expected);
            context.device().poll(wgpu::PollType::Wait {
                submission_index: Some(context.queue().submit([encoder.finish()])),
                timeout: None,
            })?;
            let recorded = native
                .record(context.native())?
                .ok_or("missing native update")?;
            recorded.submit_and_wait(context.native())?;
            native.submitted();
            assert_eq!(reference.parameters().words(), native.parameters().words());
            assert_eq!(reference.sky_mapping(), native.sky_mapping());
            let a = reference.bindings();
            let b = native.bindings();
            for (name, texture, image) in [
                ("optical", a.optical_depth, b.optical_depth),
                ("sky", a.sky, b.sky),
                ("aerial L", a.aerial_light, b.aerial_light),
                ("aerial T", a.aerial_transmittance, b.aerial_transmittance),
            ] {
                assert_eq!(
                    read(&context, texture)?,
                    image.download(context.native())?,
                    "{name} 在两个 owner 中应逐字节相同"
                );
            }
            assert_eq!(native.prepare(context.native(), view)?, Update::default());
            assert!(native.record(context.native())?.is_none());
        }
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan GPU；主任务统一调度"]
fn rejected_prepare_preserves_sky_mapping_while_aerial_is_recorded() -> Result {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let mut atmosphere = Atmosphere::new(context.native())?;
        atmosphere
            .initialize(context.native())?
            .submit_and_wait(context.native())?;
        atmosphere.release_initialization();
        let first = View {
            sun_direction: [0.0, 0.6, 0.8],
            ..View::default()
        };
        atmosphere.prepare(context.native(), first)?;
        atmosphere
            .record(context.native())?
            .ok_or("first update")?
            .submit_and_wait(context.native())?;
        atmosphere.submitted();
        let second = View {
            sun_direction: [0.8, 0.6, 0.0],
            ..first
        };
        assert_eq!(
            atmosphere.prepare(context.native(), second)?,
            Update {
                sky: false,
                aerial: true
            }
        );
        let held = atmosphere
            .record(context.native())?
            .ok_or("aerial update")?;
        let parameters = *atmosphere.parameters().words();
        let mapping = *atmosphere.sky_mapping();
        let mut sky_uniform = vec![0; 400];
        let mut gpu_mapping = vec![0; MAPPING_ROWS * 16];
        atmosphere
            .sky_uniform
            .read(context.native(), 0, &mut sky_uniform)?;
        atmosphere
            .mapping
            .read(context.native(), 0, &mut gpu_mapping)?;
        assert!(
            atmosphere
                .prepare(
                    context.native(),
                    View {
                        altitude_km: 2.0,
                        ..second
                    }
                )
                .is_err()
        );
        assert_eq!(atmosphere.parameters().words(), &parameters);
        assert_eq!(atmosphere.sky_mapping(), &mapping);
        let mut actual = vec![0; sky_uniform.len()];
        atmosphere
            .sky_uniform
            .read(context.native(), 0, &mut actual)?;
        assert_eq!(actual, sky_uniform);
        let mut actual = vec![0; gpu_mapping.len()];
        atmosphere.mapping.read(context.native(), 0, &mut actual)?;
        assert_eq!(actual, gpu_mapping);
        held.submit_and_wait(context.native())?;
        atmosphere.submitted();
        assert_eq!(
            atmosphere.prepare(context.native(), second)?,
            Update::default()
        );
        assert!(atmosphere.record(context.native())?.is_none());
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan GPU；主任务统一调度"]
fn cancelled_view_update_remains_pending() -> Result {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let mut atmosphere = Atmosphere::new(context.native())?;
        atmosphere
            .initialize(context.native())?
            .submit_and_wait(context.native())?;
        atmosphere.release_initialization();
        atmosphere.prepare(context.native(), View::default())?;
        let cancelled = atmosphere
            .record(context.native())?
            .ok_or("missing first update")?;
        context.native().cancel(cancelled)?;
        let retry = atmosphere
            .record(context.native())?
            .ok_or("cancelled update must remain pending")?;
        retry.submit_and_wait(context.native())?;
        atmosphere.submitted();
        assert!(atmosphere.record(context.native())?.is_none());
        Ok(())
    })
}
