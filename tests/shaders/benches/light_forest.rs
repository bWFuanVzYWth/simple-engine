//! 驻留 quad 上对照完整发布与单点改灯。GPU 时间覆盖构建提交，
//! 主机完成时间另含校验、分配、录制与等待。输入上传和输出检查在计时外。
#![forbid(unsafe_code)]

use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use serde_json::json;
use simple_engine::{
    gpu_test::{self, Capability, Gpu},
    vulkan::{
        self,
        interop::{CommandBuffer, RayShader, WgpuContext},
        native::{
            Binding, Bindings, Buffer, Commands, ComputePipeline, LightRegionInput, LightWorld,
            LightWorldInput, TimestampQueries,
        },
    },
    wgpu,
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Gpu(#[from] gpu_test::Error),
    #[error(transparent)]
    Vulkan(#[from] vulkan::Error),
    #[error(transparent)]
    Poll(#[from] wgpu::PollError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("构建提交的时间戳尚未就绪")]
    MissingTimestamp,
    #[error("根节点总功率为 {actual}，预期 {count}")]
    RootPower { actual: f32, count: u32 },
    #[error("世界灯表没有保持预期的区块复用")]
    RegionReuse,
}

fn marker(context: &mut WgpuContext, query: TimestampQueries) -> Result<CommandBuffer, Error> {
    let mut commands = Commands::new(context.native())?;
    commands.reset_timestamps(query)?;
    commands.write_timestamp(query, 0)?;
    let recorded = commands.finish(context.native())?;
    Ok(context.record_native(recorded)?)
}

const fn chunk(index: u32) -> [i32; 3] {
    [
        (index % 64 / 16) as i32,
        (index / 64 % 64 / 16) as i32,
        (index / 4096 / 16) as i32,
    ]
}

fn read_power(
    context: &mut WgpuContext,
    world: LightWorld,
    readback: Buffer,
) -> Result<[u32; 4], Error> {
    let native = context.native();
    let bindings = Bindings::new(
        native,
        &[
            Binding::StorageBuffer(readback),
            Binding::StorageBuffer(readback),
            Binding::LightWorld(world),
        ],
    )?;
    let program = ComputePipeline::new(
        native,
        bindings,
        RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/light_world_power.spv")),
            entry: c"light_world_power",
        },
    )?;
    let mut read = Commands::new(native)?;
    read.dispatch(program, [1, 1, 1])?;
    let command = read.finish(native)?;
    let recorded = context.record_native(command)?;
    let submission = context.submit([recorded])?;
    context.parts().1.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(Duration::from_secs(20)),
    })?;
    let mut outcome = [0u32; 4];
    readback.read(context.native(), 0, bytemuck::bytes_of_mut(&mut outcome))?;
    Ok(outcome)
}

fn measure(context: &mut WgpuContext, out: &mut impl Write, count: u32) -> Result<(), Error> {
    let mut vertices = Vec::with_capacity(count as usize * 4);
    let mut leaves = Vec::with_capacity(count as usize);
    for index in 0..count {
        // 64m 区块内以 4m 间距布置单位发光面，复用应用采用的区块尺度。
        let x = (index % 64) as f32 * 4.0;
        let y = ((index / 64) % 64) as f32 * 4.0;
        let z = (index / 4096) as f32 * 4.0;
        for [x, y] in [[x, y], [x + 1.0, y], [x + 1.0, y + 1.0], [x, y + 1.0]] {
            vertices.push([x.to_bits(), y.to_bits(), z.to_bits(), 0]);
        }
        leaves.push([index, 1.0f32.to_bits(), index, 0]);
    }
    leaves.sort_unstable_by_key(|row| (chunk(row[0]), row[0]));
    let regions: Vec<_> = leaves
        .chunk_by(|a, b| chunk(a[0]) == chunk(b[0]))
        .map(|chunk_leaves| LightRegionInput::Build {
            grid: chunk(chunk_leaves[0][0]),
            count: chunk_leaves.len() as u32,
        })
        .collect();
    let LightRegionInput::Build { count: changed, .. } = regions[0] else {
        unreachable!("初次构建均为新子树");
    };
    let changed = changed as usize;
    let vertex_buffer = Buffer::upload(context.native(), bytemuck::cast_slice(&vertices))?;
    drop(vertices);
    let readback = Buffer::new(context.native(), 16, true)?;
    let first = TimestampQueries::new(context.native(), 1)?;
    let last = TimestampQueries::new(context.native(), 1)?;
    let persistent = [
        vertex_buffer.into(),
        first.into(),
        last.into(),
        readback.into(),
    ];

    for records in [false, true] {
        leaves[0][1] = 1.0f32.to_bits();
        let original = context.native().build_light_world(LightWorldInput {
            vertices: vertex_buffer,
            leaves: &leaves,
            regions: &regions,
            records,
            source: None,
        })?;
        let old_regions = context.native_ref().light_regions(original)?.to_vec();
        let mut edited = Vec::with_capacity(regions.len());
        edited.push(regions[0]);
        edited.extend(
            old_regions[1..]
                .iter()
                .copied()
                .map(LightRegionInput::Reuse),
        );
        leaves[0][1] = 2.0f32.to_bits();
        let full_source = Buffer::upload(context.native(), bytemuck::cast_slice(&leaves))?;
        let edit_source =
            Buffer::upload(context.native(), bytemuck::cast_slice(&leaves[..changed]))?;
        let [vertices, start, end, result] = persistent;
        let roots = [
            vertices,
            start,
            end,
            result,
            full_source.into(),
            edit_source.into(),
            original.into(),
        ];
        for iteration in 0..8 {
            for point_edit in [iteration % 2 != 0, iteration % 2 == 0] {
                let before = marker(context, first)?;
                let after = marker(context, last)?;
                let (leaves, regions, source) = if point_edit {
                    (&leaves[..changed], &edited[..], edit_source)
                } else {
                    (&leaves[..], &regions[..], full_source)
                };
                let started = Instant::now();
                let (world, recording) = context.record_light_world(LightWorldInput {
                    vertices: vertex_buffer,
                    leaves,
                    regions,
                    records,
                    source: Some(source),
                })?;
                let record_ms = started.elapsed().as_secs_f64() * 1e3;
                let submission = context.submit([before, recording, after])?;
                context.parts().1.poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(20)),
                })?;
                let completed_ms = started.elapsed().as_secs_f64() * 1e3;
                let native = context.native();
                let start = first.read(native)?.ok_or(Error::MissingTimestamp)?[0];
                let finish = last.read(native)?.ok_or(Error::MissingTimestamp)?[0];
                let gpu_ms = first.elapsed_ns(native, start, finish)? as f64 / 1e6;
                let [bits, total, chunks, _] = read_power(context, world, readback)?;
                let power = f32::from_bits(bits);
                if power != count as f32 + 1.0 || total != count {
                    return Err(Error::RootPower {
                        actual: power,
                        count: count + 1,
                    });
                }
                let next = context.native_ref().light_regions(world)?;
                if next[0] == old_regions[0] || (point_edit && next[1..] != old_regions[1..]) {
                    return Err(Error::RegionReuse);
                }
                writeln!(
                    out,
                    "{}",
                    json!({
                        "lights": count, "chunks": chunks, "point_edit": point_edit,
                        "rebuilt_chunks": if point_edit { 1 } else { chunks },
                        "rebuilt_lights": leaves.len(),
                        "emit_records": records, "iteration": iteration,
                        "warmup": iteration < 2, "gpu_ms": gpu_ms,
                        "record_ms": record_ms, "completed_ms": completed_ms,
                        "world_payload_bytes": world.bytes(), "root_power": power,
                    })
                )?;
                context.native().collect(&roots)?;
            }
        }
        context.native().collect(&persistent)?;
    }
    context.native().collect(&[])?;
    Ok(())
}

fn main() -> Result<(), Error> {
    let mut gpu = Gpu::new(Capability::RayTracing, 0, false)?;
    let context = gpu.context_mut();
    context.native().prepare_quad_expansion()?;
    let info = context.adapter_info();
    let mut out = io::stdout().lock();
    writeln!(
        out,
        "{}",
        json!({"device": info.name, "driver": info.driver_info})
    )?;
    for count in [4_096, 1_000_000, 3_000_000] {
        measure(context, &mut out, count)?;
    }
    gpu.finish().map_err(Error::from)
}
