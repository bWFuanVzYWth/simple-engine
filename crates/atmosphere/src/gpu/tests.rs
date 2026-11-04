use super::*;
use serde::Deserialize;
use std::{collections::BTreeMap, error::Error, sync::mpsc};
use vulkan::interop::WgpuContext;

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

#[derive(Deserialize)]
struct Sample {
    at: [u32; 2],
    value: [f32; 4],
}
#[derive(Deserialize)]
struct SkyOracle {
    altitude_km: f32,
    sun_elevation: f32,
    samples: Vec<Sample>,
}
#[derive(Deserialize)]
struct Oracle {
    fields: BTreeMap<String, Vec<Sample>>,
    skies: Vec<SkyOracle>,
    transport: Vec<TransportOracle>,
}
#[derive(Deserialize)]
struct TransportOracle {
    ray: [f32; 4],
    sun: [f32; 4],
    light: [f32; 4],
    transmittance: [f32; 4],
}

fn shaders(context: &WgpuContext) -> Result<Shaders> {
    let modules: Vec<_> = SHADERS
        .iter()
        .map(|shader| context.shader(shader.spirv, shader.entry, shader.workgroup))
        .collect::<std::result::Result<_, _>>()?;
    Ok(Shaders {
        modules: modules.try_into().map_err(|_| "shader count")?,
    })
}

fn create(context: &WgpuContext) -> Result<Atmosphere> {
    Ok(Atmosphere::new(
        context.device(),
        context.queue(),
        shaders(context)?,
    )?)
}

fn complete(context: &WgpuContext, submission: wgpu::SubmissionIndex) -> Result {
    context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    Ok(())
}

fn read_points(
    context: &WgpuContext,
    texture: &wgpu::Texture,
    coordinates: &[[u32; 3]],
) -> Result<Vec<[f32; 4]>> {
    let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("atmosphere sampled oracle"),
        size: coordinates.len() as u64 * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = context.device().create_command_encoder(&Default::default());
    for (index, coordinate) in coordinates.iter().enumerate() {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: coordinate[0],
                    y: coordinate[1],
                    z: coordinate[2],
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: index as u64 * 16,
                    bytes_per_row: None,
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }
    let submission = context.queue().submit([encoder.finish()]);
    let (sender, receiver) = mpsc::sync_channel(1);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    complete(context, submission)?;
    receiver.recv()??;
    let mapped = readback.slice(..).get_mapped_range()?;
    let half = texture.format() == wgpu::TextureFormat::Rgba16Float;
    let result = mapped
        .as_chunks::<16>()
        .0
        .iter()
        .map(|bytes| {
            std::array::from_fn(|channel| {
                if half {
                    half::f16::from_le_bytes([bytes[channel * 2], bytes[channel * 2 + 1]]).to_f32()
                } else {
                    f32::from_le_bytes(bytes[channel * 4..channel * 4 + 4].try_into().unwrap())
                }
            })
        })
        .collect();
    drop(mapped);
    readback.unmap();
    Ok(result)
}

fn compare(
    context: &WgpuContext,
    texture: &wgpu::Texture,
    expected: &[Sample],
    name: &str,
) -> Result {
    let coordinates: Vec<_> = expected
        .iter()
        .map(|sample| [sample.at[0], sample.at[1], 0])
        .collect();
    let actual = read_points(context, texture, &coordinates)?;
    for (sample, actual) in expected.iter().zip(actual) {
        for channel in 0..4 {
            let target = sample.value[channel];
            let tolerance = if name == "sky" || name == "mean" {
                0.025
            } else {
                2e-4 + target.abs() * 0.01
            };
            assert!(
                actual[channel].is_finite() && (actual[channel] - target).abs() <= tolerance,
                "{name} {:?} channel{channel}: {} != {target}, tolerance {tolerance}",
                sample.at,
                actual[channel]
            );
        }
    }
    Ok(())
}

fn compare_transport(
    context: &WgpuContext,
    atmosphere: &Atmosphere,
    expected: &[TransportOracle],
) -> Result {
    let device = context.device();
    let bytes = expected.len() as u64 * 32;
    let queries = buffer(
        device,
        "source transport queries",
        bytes,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    );
    let rows: Vec<_> = expected
        .iter()
        .flat_map(|query| [query.ray, query.sun])
        .collect();
    context
        .queue()
        .write_buffer(&queries, 0, bytemuck::cast_slice(&rows));
    let output = buffer(
        device,
        "source transport output",
        bytes,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let uniform = uniform(
        device,
        context.queue(),
        bytemuck::bytes_of(&atmosphere.base),
    );
    let layout = layout(
        device,
        &[
            Kind::Uniform(400),
            Kind::Read,
            Kind::Filtered,
            Kind::Filtered,
            Kind::Unfiltered,
            Kind::Unfiltered,
            Kind::Sampler,
            Kind::Unfiltered,
            Kind::Read,
            Kind::Write,
        ],
    );
    let group = group(
        device,
        &layout,
        &[
            uniform.as_entire_binding(),
            atmosphere.data.as_entire_binding(),
            tv(&atmosphere.tau),
            tv(&atmosphere.field.source),
            tv(&atmosphere.field.mean),
            tv(&atmosphere.field.ground),
            wgpu::BindingResource::Sampler(&atmosphere.sampler),
            tv(&atmosphere.field.high),
            queries.as_entire_binding(),
            output.as_entire_binding(),
        ],
    );
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("source transport oracle"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let module = context.shader(
        include_bytes!(concat!(env!("OUT_DIR"), "/transport_probe.spv")),
        "transport_probe",
        [64, 1, 1],
    )?;
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("source transport oracle"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("transport_probe"),
        compilation_options: Default::default(),
        cache: None,
    });
    let staging = buffer(
        device,
        "source transport readback",
        bytes,
        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    dispatch(
        &mut encoder,
        &pipeline,
        &group,
        [(expected.len() as u32).div_ceil(64), 1, 1],
    );
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, bytes);
    let submission = context.queue().submit([encoder.finish()]);
    let (sender, receiver) = mpsc::sync_channel(1);
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    complete(context, submission)?;
    receiver.recv()??;
    let mapped = staging.slice(..).get_mapped_range()?;
    for (index, (actual, expected)) in mapped.as_chunks::<32>().0.iter().zip(expected).enumerate() {
        let actual: [f32; 8] = bytemuck::pod_read_unaligned(actual);
        for channel in 0..4 {
            assert!(
                (actual[channel] - expected.light[channel]).abs()
                    <= 1e-6 + expected.light[channel].abs() * 0.01,
                "transport {index} L channel{channel}: {} != {}",
                actual[channel],
                expected.light[channel]
            );
            assert!(
                (actual[channel + 4] - expected.transmittance[channel]).abs() <= 5e-4,
                "transport {index} T channel{channel}: {} != {}",
                actual[channel + 4],
                expected.transmittance[channel]
            );
        }
    }
    drop(mapped);
    staging.unmap();
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan GPU；主任务统一调度"]
fn eight_scattering_iterations_match_source() -> Result {
    gpu_log::checked(|| {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/reference.json");
        let oracle: Oracle = serde_json::from_slice(&std::fs::read(path)?)?;
        let context = pollster::block_on(WgpuContext::compute())?;
        let mut atmosphere = create(&context)?;
        let mut encoder = context.device().create_command_encoder(&Default::default());
        atmosphere.initialize(&mut encoder)?;
        complete(&context, context.queue().submit([encoder.finish()]))?;
        atmosphere.release_initialization();
        for (name, samples) in &oracle.fields {
            let texture = match name.as_str() {
                "tau" => &atmosphere.tau.texture,
                "source" => &atmosphere.field.source.texture,
                "mean" => &atmosphere.field.mean.texture,
                "ground" => &atmosphere.field.ground.texture,
                "high" => &atmosphere.field.high.texture,
                _ => return Err(format!("未知 reference texture {name}").into()),
            };
            compare(&context, texture, samples, name)?;
        }
        compare_transport(&context, &atmosphere, &oracle.transport)?;
        for sky in oracle.skies {
            let elevation = sky.sun_elevation;
            let view = View {
                altitude_km: sky.altitude_km,
                sun_direction: [0.0, elevation.sin(), elevation.cos()],
                ..View::default()
            };
            atmosphere.prepare(view)?;
            let mut encoder = context.device().create_command_encoder(&Default::default());
            atmosphere.record(&mut encoder);
            complete(&context, context.queue().submit([encoder.finish()]))?;
            compare(&context, &atmosphere.sky.texture, &sky.samples, "sky")?;
        }
        Ok(())
    })
}

#[test]
#[ignore = "需要真实 Vulkan GPU；主任务统一调度"]
fn aerial_color_scale_preserves_transmittance() -> Result {
    gpu_log::checked(|| {
        let context = pollster::block_on(WgpuContext::compute())?;
        let mut atmosphere = create(&context)?;
        let mut encoder = context.device().create_command_encoder(&Default::default());
        atmosphere.initialize(&mut encoder)?;
        complete(&context, context.queue().submit([encoder.finish()]))?;
        atmosphere.release_initialization();
        let view = View {
            altitude_km: 0.2,
            sun_direction: [0.0, 0.6, 0.8],
            ..View::default()
        };
        atmosphere.prepare(view)?;
        let mut encoder = context.device().create_command_encoder(&Default::default());
        atmosphere.record(&mut encoder);
        complete(&context, context.queue().submit([encoder.finish()]))?;
        let points = [[0, 0, 0], [15, 15, 15], [31, 31, 31]];
        let transmittance = read_points(&context, &atmosphere.aerial_t.texture, &points)?;
        let light = read_points(&context, &atmosphere.aerial_light.texture, &points)?;
        assert_eq!(
            atmosphere.prepare(View {
                radiance_scale: [0.5, 1.0, 2.0],
                ..view
            })?,
            Update {
                sky: false,
                aerial: true
            }
        );
        let mut encoder = context.device().create_command_encoder(&Default::default());
        atmosphere.record(&mut encoder);
        complete(&context, context.queue().submit([encoder.finish()]))?;
        assert_eq!(
            read_points(&context, &atmosphere.aerial_t.texture, &points)?,
            transmittance
        );
        let scaled = read_points(&context, &atmosphere.aerial_light.texture, &points)?;
        for (original, actual) in light.iter().zip(scaled) {
            for channel in 0..3 {
                let expected = original[channel] * [0.5, 1.0, 2.0][channel];
                assert!(
                    actual[channel].is_finite()
                        && (actual[channel] - expected).abs() <= expected.abs() * 0.002 + 1e-6
                );
            }
        }
        Ok(())
    })
}

fn read_half_volume(context: &WgpuContext, texture: &wgpu::Texture) -> Result<Vec<[f32; 4]>> {
    let row_pitch = (texture.width() * 8).div_ceil(256) * 256;
    let byte_count = u64::from(row_pitch)
        * u64::from(texture.height())
        * u64::from(texture.depth_or_array_layers());
    let staging = buffer(
        context.device(),
        "Aerial boundary readback",
        byte_count,
        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    );
    let mut encoder = context.device().create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_pitch),
                rows_per_image: Some(texture.height()),
            },
        },
        texture.size(),
    );
    let submission = context.queue().submit([encoder.finish()]);
    let (sender, receiver) = mpsc::sync_channel(1);
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    complete(context, submission)?;
    receiver.recv()??;
    let mapped = staging.slice(..).get_mapped_range()?;
    let mut result = Vec::new();
    for z in 0..texture.depth_or_array_layers() {
        for y in 0..texture.height() {
            let offset = ((z * texture.height() + y) * row_pitch) as usize;
            let row = &mapped[offset..offset + texture.width() as usize * 8];
            result.extend(row.as_chunks::<8>().0.iter().map(|pixel| {
                std::array::from_fn(|channel| {
                    half::f16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]]).to_f32()
                })
            }));
        }
    }
    drop(mapped);
    staging.unmap();
    Ok(result)
}

#[test]
#[ignore = "需要真实 Vulkan GPU；主任务统一调度"]
fn aerial_at_surface_does_not_add_planet_boundary_radiance() -> Result {
    gpu_log::checked(|| {
        let context = pollster::block_on(WgpuContext::compute())?;
        let outputs = Outputs::new(context.device());
        let mut atmosphere = Atmosphere::new_with_outputs(
            context.device(),
            context.queue(),
            shaders(&context)?,
            outputs,
        )?;
        let mut encoder = context.device().create_command_encoder(&Default::default());
        atmosphere.initialize(&mut encoder)?;
        complete(&context, context.queue().submit([encoder.finish()]))?;
        atmosphere.release_initialization();
        atmosphere.prepare(View {
            altitude_km: 0.0,
            forward: [0.0, -1.0, 0.0],
            right: [0.001, 0.0, 0.0],
            up: [0.0, 0.0, 0.001],
            ..View::default()
        })?;
        let mut encoder = context.device().create_command_encoder(&Default::default());
        atmosphere.record(&mut encoder);
        complete(&context, context.queue().submit([encoder.finish()]))?;
        for pixel in read_half_volume(&context, &atmosphere.aerial_light.texture)? {
            assert_eq!(&pixel[..3], &[0.0; 3]);
        }
        for pixel in read_half_volume(&context, &atmosphere.aerial_t.texture)? {
            assert_eq!(&pixel[..3], &[1.0; 3]);
        }
        Ok(())
    })
}
