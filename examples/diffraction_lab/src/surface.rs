//! 原生窗口的 sRGB 表面与引擎二维绘制。CPU 图像只在内容改变时上传。
use crate::recipe::Result;
use simple_engine::{
    render2d::{DrawList, ImageKey, Rect, gpu::*, text::TextSystem},
    vulkan::interop::WgpuContext,
    wgpu,
};
use winit::window::Window;

pub(crate) struct Screen<'w> {
    surface: wgpu::Surface<'w>,
    device: WgpuContext,
    renderer: Renderer2d,
    config: wgpu::SurfaceConfiguration,
    images: [Option<(wgpu::Texture, ImageKey)>; 2],
    errors: std::sync::mpsc::Receiver<String>,
}
impl<'w> Screen<'w> {
    pub fn new(w: &'w Window) -> Result<Self> {
        let instance = WgpuContext::instance()?;
        let surface = instance.create_surface(w)?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        let device = WgpuContext::from_adapter(&instance, &adapter, false)?;
        let caps = surface.get_capabilities(&adapter);
        let mut config = surface
            .get_default_config(&adapter, 1, 1)
            .ok_or("无法配置桌面显示表面")?;
        config.format = caps
            .format_capabilities
            .iter()
            .find(|c| c.format.is_srgb() && c.color_spaces.contains(wgpu::SurfaceColorSpaces::SRGB))
            .ok_or("窗口需要 sRGB 显示格式")?
            .format;
        config.color_space = wgpu::SurfaceColorSpace::Srgb;
        config.width = 0;
        config.height = 0;
        let scope = device
            .device()
            .push_error_scope(wgpu::ErrorFilter::Validation);
        let renderer = Renderer2d::new(
            device.device(),
            device.queue(),
            Shaders2d {
                vertex: device.shader(VERTEX_SPIRV, "vertex_main", [0; 3])?,
                fragment: device.shader(FRAGMENT_SPIRV, "fragment_main", [0; 3])?,
            },
            Budgets::default(),
        )?;
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(e.into());
        }
        let (send, errors) = std::sync::mpsc::channel();
        device.on_device_lost(move |reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                let _ = send.send(message);
            }
        });
        Ok(Self {
            surface,
            device,
            renderer,
            config,
            images: [None, None],
            errors,
        })
    }
    pub fn upload(&mut self, index: usize, n: u32, bytes: &[u8]) -> Result<()> {
        if self.images[index]
            .as_ref()
            .is_none_or(|(t, _)| t.width() != n)
        {
            let tex = self
                .device
                .device()
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("衍射工作台图像"),
                    size: wgpu::Extent3d {
                        width: n,
                        height: n,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
            let id = self.renderer.register_image(SampledImage {
                view: tex.create_view(&Default::default()),
                sampler: self
                    .device
                    .device()
                    .create_sampler(&wgpu::SamplerDescriptor {
                        min_filter: wgpu::FilterMode::Linear,
                        mag_filter: wgpu::FilterMode::Linear,
                        ..Default::default()
                    }),
            })?;
            if let Some((_, old)) = self.images[index].replace((tex, id)) {
                self.renderer.remove_image(old)?;
            }
        }
        let (tex, _) = self.images[index].as_ref().expect("刚建立图像");
        self.device.queue().write_texture(
            tex.as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(n * 4),
                rows_per_image: Some(n),
            },
            tex.size(),
        );
        Ok(())
    }
    pub fn keys(&self) -> [Option<ImageKey>; 2] {
        self.images
            .each_ref()
            .map(|item| item.as_ref().map(|(_, id)| *id))
    }
    pub fn draw(
        &mut self,
        extent: [u32; 2],
        logical: Rect,
        scene: &DrawList,
        text: &mut TextSystem,
    ) -> Result<bool> {
        if let Ok(message) = self.errors.try_recv() {
            return Err(message.into());
        }
        self.device.device().poll(wgpu::PollType::Poll)?;
        self.renderer.collect();
        if extent.contains(&0) {
            return Ok(true);
        }
        if [self.config.width, self.config.height] != extent {
            self.config.width = extent[0];
            self.config.height = extent[1];
            self.surface.configure(self.device.device(), &self.config);
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                self.config.width = 0;
                f
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.config.width = 0;
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(true),
            wgpu::CurrentSurfaceTexture::Timeout => return Ok(false),
            wgpu::CurrentSurfaceTexture::Validation => return Err("获取窗口表面失败".into()),
        };
        let attachment = frame.texture.create_view(&Default::default());
        let encoder =
            self.device
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("衍射工作台"),
                });
        let data = self.renderer.prepare_text(scene, text)?;
        let view = self.renderer.prepare_view(
            &data,
            ViewParams::new(logical, extent),
            TargetConfig {
                format: self.config.format,
                samples: 1,
            },
        )?;
        let mut recording = self.renderer.recording(encoder)?;
        recording.encode(
            &view,
            RenderTarget {
                view: &attachment,
                resolve: None,
                extent,
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                timestamp_writes: None,
            },
        )?;
        let commands = recording.finish()?;
        self.renderer.submit(commands)?;
        self.device.queue().present(frame);
        Ok(true)
    }
}
