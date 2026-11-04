use simple_engine::{gpu_log, vulkan, wgpu};
use std::sync::{Arc, OnceLock};

use crate::{ComputeStages, Error, FrameTargets, WgpuContext, prepare_frame};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

pub fn run(frames: u32) -> Result<(), Error> {
    let window_slot = OnceLock::new();
    let mut app = App {
        window_slot: &window_slot,
        window: None,
        renderer: None,
        presented: 0,
        resize_from: None,
        resized: false,
        resized_presented: false,
        error: None,
        limit: frames,
    };
    EventLoop::new()?.run_app(&mut app)?;
    if let Some(error) = app.error {
        return Err(error);
    }
    if app.presented != app.limit || !app.resized_presented {
        return Err(Error::WindowResize);
    }
    Ok(())
}

struct App<'w> {
    window_slot: &'w OnceLock<Window>,
    window: Option<&'w Window>,
    renderer: Option<Renderer<'w>>,
    limit: u32,
    presented: u32,
    resize_from: Option<PhysicalSize<u32>>,
    resized: bool,
    resized_presented: bool,
    error: Option<Error>,
}

impl App<'_> {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl Into<Error>) {
        self.error = Some(error.into());
        event_loop.exit();
    }
}

impl<'w> ApplicationHandler for App<'w> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let slot = self.window_slot;
        if slot.get().is_none() {
            match event_loop.create_window(
                Window::default_attributes()
                    .with_title("wgpu / Vulkan resize 集成测试")
                    .with_inner_size(PhysicalSize::new(480, 320)),
            ) {
                Ok(window) => {
                    let _ = slot.set(window);
                }
                Err(error) => return self.fail(event_loop, error),
            }
        }
        let window = slot.get().expect("已创建窗口");
        match pollster::block_on(Renderer::new(window)) {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(error) => return self.fail(event_loop, error),
        }
        window.request_redraw();
        self.window = Some(window);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(renderer) = self.renderer.take()
            && let Err(error) = renderer.finish()
        {
            self.fail(event_loop, error);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = &self.window else {
            return;
        };
        if window.id() != window_id {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if self.resize_from.is_some_and(|old| size != old)
                    && size.width != 0
                    && size.height != 0
                {
                    self.resized = true;
                }
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size);
                }
            }
            WindowEvent::RedrawRequested => {
                let Some(renderer) = &mut self.renderer else {
                    return;
                };
                match renderer.draw(window, self.presented + 1) {
                    Ok(true) => self.presented += 1,
                    Ok(false) => return,
                    Err(error) => return self.fail(event_loop, error),
                }
                if self.resized
                    && self.resize_from.is_some_and(|old| {
                        renderer.config.width != old.width || renderer.config.height != old.height
                    })
                {
                    self.resized_presented = true;
                }
                if self.presented == self.limit {
                    event_loop.exit();
                } else if self.presented == 5 {
                    self.resize_from = Some(renderer.size);
                    if let Some(size) = window.request_inner_size(PhysicalSize::new(641, 359)) {
                        self.resized = size != renderer.size && size.width != 0 && size.height != 0;
                        renderer.resize(size);
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.renderer.is_some()
            && let Some(window) = &self.window
        {
            let size = window.inner_size();
            if size.width != 0 && size.height != 0 {
                window.request_redraw();
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(renderer) = self.renderer.take()
            && let Err(error) = renderer.finish()
        {
            self.error.get_or_insert(error);
        }
        self.window = None;
        println!(
            "已呈现 {} 帧，resize 后呈现：{}",
            self.presented, self.resized_presented
        );
    }
}

struct Renderer<'w> {
    surface: wgpu::Surface<'w>,
    context: WgpuContext,
    stages: ComputeStages,
    scene: vulkan::AccelerationStructure,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    targets: Option<FrameTargets>,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    dirty: bool,
}

impl<'w> Renderer<'w> {
    async fn new(window: &'w Window) -> Result<Self, Error> {
        let size = window.inner_size();
        let instance = WgpuContext::instance()?;
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await?;
        let mut context = WgpuContext::from_adapter(&instance, &adapter, true)?;
        println!("窗口设备：{}", context.adapter_info().name);
        let device = context.device().clone();
        // wgpu 要求 Arc 错误回调；无资源捕获，device 替换 handler 或销毁时释放。
        device.on_uncaptured_error(Arc::new(|error: wgpu::Error| log::error!("{error}")));
        context.on_device_lost(|reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                log::error!("GPU 已丢失：{reason:?}：{message}");
            }
        });
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut config = surface
            .get_default_config(&adapter, 1, 1)
            .ok_or(Error::NoSurfaceConfiguration)?;
        config.format = surface
            .get_capabilities(&adapter)
            .format_capabilities
            .iter()
            .find(|format| {
                format.format.is_srgb()
                    && format.color_spaces.contains(wgpu::SurfaceColorSpaces::SRGB)
            })
            .map(|format| format.format)
            .ok_or(Error::NoSurfaceConfiguration)?;
        config.color_space = wgpu::SurfaceColorSpace::Srgb;
        let stages = ComputeStages::new(&context)?;
        let scene = context.acceleration_structure(&[[
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]])?;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let vertex = context.shader(
            include_bytes!(concat!(env!("OUT_DIR"), "/vertex_main.spv")),
            "vertex_main",
            [0, 0, 0],
        )?;
        let fragment = context.shader(
            include_bytes!(concat!(env!("OUT_DIR"), "/fragment_main.spv")),
            "fragment_main",
            [0, 0, 0],
        )?;
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(config.format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = scope.pop().await {
            return Err(error.into());
        }
        Ok(Self {
            surface,
            context,
            stages,
            scene,
            pipeline,
            layout,
            targets: None,
            config,
            size,
            dirty: true,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if self.size != size {
            self.size = size;
            self.dirty = true;
        }
    }

    fn finish(&self) -> Result<(), Error> {
        self.context
            .device()
            .poll(wgpu::PollType::wait_indefinitely())?;
        check_gpu()
    }

    fn draw(&mut self, window: &Window, frame: u32) -> Result<bool, Error> {
        check_gpu()?;
        if self.size.width == 0 || self.size.height == 0 {
            return Ok(false);
        }
        let device = self.context.device().clone();
        if self.dirty {
            let limit = device.limits().max_texture_dimension_2d;
            self.config.width = self.size.width.min(limit);
            self.config.height = self.size.height.min(limit);
            self.surface.configure(&device, &self.config);
            let size = [self.config.width, self.config.height];
            if self
                .targets
                .as_ref()
                .is_none_or(|targets| targets.size != size)
            {
                self.targets = Some(FrameTargets::new(&mut self.context, self.scene, size)?);
                self.context.native().collect(&[
                    self.scene.into(),
                    self.targets.as_ref().unwrap().native.into(),
                ])?;
                println!("共享纹理：{} × {}", size[0], size[1]);
            }
            self.dirty = false;
        }
        let (output, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output) => (output, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(output) => (output, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.dirty = true;
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Lost => return Err(Error::SurfaceLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(Error::SurfaceValidation),
        };
        let targets = self.targets.as_ref().unwrap();
        let prepare = prepare_frame(&self.context, &self.stages, targets, frame)?;
        let trace = targets
            .native
            .encode(&mut self.context, [targets.size[0], targets.size[1], 1])?;
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&targets.views[1]),
            }],
        });
        let view = output.texture.create_view(&Default::default());
        let mut post = device.create_command_encoder(&Default::default());
        {
            let mut pass = post.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.context
            .submit([prepare.into(), trace, post.finish().into()])?;
        window.pre_present_notify();
        self.context.queue().present(output);
        self.dirty = suboptimal;
        check_gpu()?;
        Ok(true)
    }
}

fn check_gpu() -> Result<(), Error> {
    gpu_log::check().map_err(Into::into)
}
