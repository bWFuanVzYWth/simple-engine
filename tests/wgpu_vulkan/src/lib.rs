//! wgpu → native RT → wgpu 的集成 fixture，验证交接数据、跨帧复用、在途保活和窗口 resize。
//!
//! prepare compute 完整初始化两张共享图像，native 阶段分别写入命中/未命中数据，后置
//! compute 将结果复制到独立回读 buffer。每帧用坐标与帧号编码期望值，避免资源错配被
//! 静态图案掩盖；命令与资源生命周期由生产 `vulkan` 封装维护。
#![forbid(unsafe_code)]
// wgpu 的 Send/Sync 类型图超过默认 trait 求解深度。
#![recursion_limit = "256"]

mod window;

use simple_engine::{gpu_log, vulkan, wgpu};
use std::sync::mpsc;
use vulkan::interop::{RayShader, RayTracingPipeline, SharedTexture, WgpuContext};

/// fixture 初始化、平台呈现、GPU 执行或数据核对失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// native 设备或互操作封装返回失败。
    #[error(transparent)]
    Vulkan(#[from] vulkan::Error),
    /// wgpu 错误作用域捕获的验证错误。
    #[error("wgpu 验证失败：{0}")]
    Validation(#[from] wgpu::Error),
    /// 等待已提交工作失败。
    #[error("等待 GPU 失败：{0}")]
    Poll(#[from] wgpu::PollError),
    /// 回读 buffer 映射失败。
    #[error("映射回读失败：{0}")]
    Map(#[from] wgpu::BufferAsyncError),
    /// 无法取得已映射的回读范围。
    #[error("访问映射范围失败：{0}")]
    MapRange(#[from] wgpu::MapRangeError),
    /// 映射回调通道在返回前断开。
    #[error("回读回调中断：{0}")]
    Callback(#[from] mpsc::RecvError),
    /// 桌面事件循环创建或运行失败。
    #[error(transparent)]
    EventLoop(#[from] winit::error::EventLoopError),
    /// 操作系统窗口创建失败。
    #[error(transparent)]
    Window(#[from] winit::error::OsError),
    /// wgpu 无法为窗口创建表面。
    #[error("创建窗口表面失败：{0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    /// 没有满足窗口兼容要求的适配器。
    #[error("无法获取兼容窗口的适配器：{0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    /// 表面无法提供 fixture 要求的 sRGB 配置。
    #[error("窗口没有可用的 sRGB 呈现配置")]
    NoSurfaceConfiguration,
    /// 表面在验收期间丢失。
    #[error("窗口表面已丢失")]
    SurfaceLost,
    /// 获取表面纹理时报告验证错误。
    #[error("获取窗口纹理时发生验证错误")]
    SurfaceValidation,
    /// 事件循环结束前未完成尺寸变化后的呈现。
    #[error("未观察到尺寸变化后的成功呈现")]
    WindowResize,
    /// 全进程 GPU 日志安装或累计检查失败。
    #[error(transparent)]
    GpuLog(#[from] gpu_log::Error),
    /// 完整交接后像素数据与坐标、帧号编码不符。
    #[error("帧 {frame} 像素 {pixel} 不符：实际 {actual:?}，预期 {expected:?}")]
    Pixel {
        /// 被核对的提交帧号。
        frame: u32,
        /// 行优先的像素索引。
        pixel: usize,
        /// 实际回读的四通道值。
        actual: [f32; 4],
        /// 根据坐标与帧号独立生成的值。
        expected: [f32; 4],
    },
}

/// 跨帧复用的前置初始化与后置回读管线，不持有具体尺寸的图像。
pub struct ComputeStages {
    prepare: wgpu::ComputePipeline,
    prepare_layout: wgpu::BindGroupLayout,
    readback: wgpu::ComputePipeline,
    readback_layout: wgpu::BindGroupLayout,
}

impl ComputeStages {
    /// 在同一 context 中创建 production passthrough compute 管线与固定绑定布局。
    pub fn new(context: &WgpuContext) -> Result<Self, Error> {
        let device = context.device();
        let prepare_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prepare"),
            entries: &[
                storage_texture(0),
                storage_texture(1),
                buffer_entry(2, wgpu::BufferBindingType::Uniform),
            ],
        });
        let readback_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("readback"),
            entries: &[
                storage_texture(0),
                buffer_entry(1, wgpu::BufferBindingType::Storage { read_only: false }),
            ],
        });
        let prepare = compute_pipeline(
            context,
            &prepare_layout,
            include_bytes!(concat!(env!("OUT_DIR"), "/prepare_main.spv")),
            "prepare_main",
        )?;
        let readback = compute_pipeline(
            context,
            &readback_layout,
            include_bytes!(concat!(env!("OUT_DIR"), "/readback_main.spv")),
            "readback_main",
        )?;
        Ok(Self {
            prepare,
            prepare_layout,
            readback,
            readback_layout,
        })
    }
}

/// 某一尺寸的两张共享图像及 native RT 管线；resize 时整体替换。
pub struct FrameTargets {
    textures: [SharedTexture; 2],
    views: [wgpu::TextureView; 2],
    native: RayTracingPipeline,
    size: [u32; 2],
}

impl FrameTargets {
    /// 从同设备场景构造非零尺寸的帧目标；不提交命令，图像由后续 prepare 阶段初始化。
    pub fn new(
        context: &mut WgpuContext,
        scene: vulkan::AccelerationStructure,
        size: [u32; 2],
    ) -> Result<Self, Error> {
        let textures = [
            context.texture("input", size)?,
            context.texture("output", size)?,
        ];
        let views = textures
            .each_ref()
            .map(|texture| texture.texture().create_view(&Default::default()));
        let native = RayTracingPipeline::new(
            context,
            scene,
            [
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/raygen_main.spv")),
                    entry: c"raygen_main",
                },
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/miss_main.spv")),
                    entry: c"miss_main",
                },
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/closest_hit_main.spv")),
                    entry: c"closest_hit_main",
                },
            ],
            &textures,
        )?;
        Ok(Self {
            textures,
            views,
            native,
            size,
        })
    }
}

/// 已提交帧的独立回读所有者，可在目标与管线的应用侧引用释放后继续核对。
pub struct PendingFrame {
    frame: u32,
    size: [u32; 2],
    readback: wgpu::Buffer,
    mapped: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

impl PendingFrame {
    /// 完整核对该帧所有像素，成功后解除映射。
    /// 调用前须驱动设备完成提交和映射回调；本函数阻塞接收回调，不自行轮询设备。
    pub fn check(self) -> Result<(), Error> {
        self.mapped.recv()??;
        let view = self.readback.slice(..).get_mapped_range()?;
        for (pixel, bytes) in view.as_chunks::<16>().0.iter().enumerate() {
            let actual = std::array::from_fn(|i| {
                f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
            });
            let x = pixel as u32 % self.size[0];
            let y = pixel as u32 / self.size[0];
            let expected = [
                (x + 1 + if x % 2 == 0 { 1000 } else { 0 }) as f32,
                (y + 1 + 2000) as f32,
                (self.frame + 3000) as f32,
                1.0,
            ];
            if actual != expected {
                return Err(Error::Pixel {
                    frame: self.frame,
                    pixel,
                    actual,
                    expected,
                });
            }
        }
        drop(view);
        self.readback.unmap();
        println!(
            "帧 {}：{} × {}，命中、未命中和跨阶段数据一致",
            self.frame, self.size[0], self.size[1]
        );
        Ok(())
    }
}

/// 运行需要桌面环境的独立窗口验收，要求 resize 后成功呈现并检查析构日志。
pub fn window_resize_probe() -> Result<(), Error> {
    gpu_log::checked(|| window::run(20))
}

/// 按 prepare → native trace → post compute 顺序提交一帧，不等待 GPU 完成。
///
/// stages、targets 与 context 须属于同一设备；每帧拥有独立结果 buffer。返回后调用方
/// 可释放目标引用，生产提交保活负责 GPU 所需资源；轮询完成后再调用 [`PendingFrame::check`]。
pub fn record_frame(
    context: &mut WgpuContext,
    stages: &ComputeStages,
    targets: &FrameTargets,
    frame: u32,
) -> Result<(vulkan::interop::CommandBuffer, PendingFrame), Error> {
    let device = context.device().clone();
    let size = targets.size;
    let prepare = prepare_frame(context, stages, targets, frame)?;
    let bytes = u64::from(size[0]) * u64::from(size[1]) * 16;
    let pixels = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pixels"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let readback_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("readback"),
        layout: &stages.readback_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&targets.views[1]),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: pixels.as_entire_binding(),
            },
        ],
    });
    let trace = targets.native.encode(context, [size[0], size[1], 1])?;
    let mut post = device.create_command_encoder(&Default::default());
    dispatch(&mut post, &stages.readback, &readback_group, size);
    post.copy_buffer_to_buffer(&pixels, 0, &readback, 0, bytes);
    let (sender, mapped) = mpsc::channel();
    post.map_buffer_on_submit(&readback, wgpu::MapMode::Read, .., move |result| {
        let _ = sender.send(result);
    });
    let command = vulkan::interop::CommandBuffer::from(prepare)
        .then(trace)
        .then(post.finish().into());
    Ok((
        command,
        PendingFrame {
            frame,
            size,
            readback,
            mapped,
        },
    ))
}

/// 单一 owner 录制并提交一帧，独立回读在完成后由调用者核对。
pub fn submit_frame(
    context: &mut WgpuContext,
    stages: &ComputeStages,
    targets: &FrameTargets,
    frame: u32,
) -> Result<PendingFrame, Error> {
    let (command, pending) = record_frame(context, stages, targets, frame)?;
    context.submit([command])?;
    Ok(pending)
}

fn prepare_frame(
    context: &WgpuContext,
    stages: &ComputeStages,
    targets: &FrameTargets,
    frame: u32,
) -> Result<wgpu::CommandBuffer, Error> {
    let device = context.device();
    let parameters = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("frame"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: true,
    });
    parameters
        .slice(..)
        .get_mapped_range_mut()?
        .slice(..4)
        .copy_from_slice(&frame.to_le_bytes());
    parameters.unmap();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("prepare"),
        layout: &stages.prepare_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&targets.views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&targets.views[1]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: parameters.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    dispatch(&mut encoder, &stages.prepare, &group, targets.size);
    encoder.transition_resources(
        std::iter::empty(),
        targets
            .textures
            .iter()
            .map(|texture| wgpu::TextureTransition {
                texture: texture.texture(),
                selector: None,
                state: wgpu::TextureUses::STORAGE_READ_WRITE,
            }),
    );
    Ok(encoder.finish())
}

fn storage_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::ReadWrite,
            format: wgpu::TextureFormat::Rgba32Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn buffer_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn compute_pipeline(
    context: &WgpuContext,
    layout: &wgpu::BindGroupLayout,
    spirv: &[u8],
    entry: &str,
) -> Result<wgpu::ComputePipeline, Error> {
    let device = context.device();
    let module = context.shader(spirv, entry, [8, 8, 1])?;
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    Ok(
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(&layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        }),
    )
}

fn dispatch(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    group: &wgpu::BindGroup,
    size: [u32; 2],
) {
    let mut pass = encoder.begin_compute_pass(&Default::default());
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
}
