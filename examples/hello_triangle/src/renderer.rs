//! 最小窗口渲染链路：wgpu 管理 Surface 与提交，Slang SPIR-V 经 Vulkan 直通。
//!
//! 窗口由 run_app 外的 owner 保活，Surface 只借用，物理尺寸延至绘制时配置；只接受线性 sRGB 输出所需的
//! sRGB 格式与色彩空间。异步设备错误锁存首错，绘制失败后停止该 renderer，
//! 挂起或退出调用 [`TriangleRenderer::finish`] 等待完成，再释放 Surface。

use std::sync::{Arc, OnceLock, mpsc};

use crate::error::Error;
use simple_engine::{gpu_log, vulkan::interop::WgpuContext, wgpu};
use winit::window::Window;

/// 一个窗口的 Surface、设备与三角形管线所有者，不持有应用输入或独立时钟。
pub struct TriangleRenderer<'w> {
    surface: wgpu::Surface<'w>,
    context: WgpuContext,
    pipeline: wgpu::RenderPipeline,
    config: wgpu::SurfaceConfiguration,
    gpu_error: OnceLock<String>,
    errors: mpsc::Receiver<String>,
    size: [u32; 2],
    dirty: bool,
    failed: bool,
}

impl<'w> TriangleRenderer<'w> {
    /// 安装全局日志并同步建立兼容窗口的 Vulkan/wgpu 设备；不请求硬件光追。
    /// 日志、适配器、sRGB 配置、shader 或管线验证失败保留底层原因。
    pub fn new(window: &'w Window) -> Result<Self, Error> {
        gpu_log::init()?;
        pollster::block_on(Self::create(window))
    }

    async fn create(window: &'w Window) -> Result<Self, Error> {
        let size = window.inner_size();
        let instance = WgpuContext::instance()?;
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;
        let context = WgpuContext::from_adapter(&instance, &adapter, false)?;
        let device = context.device();
        let format = surface_format(&surface.get_capabilities(&adapter).format_capabilities)?;
        let mut config = surface
            .get_default_config(&adapter, 1, 1)
            .ok_or(Error::NoSurfaceConfiguration)?;
        config.format = format;
        config.color_space = wgpu::SurfaceColorSpace::Srgb;

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let vertex = context.shader(
            include_bytes!(concat!(env!("OUT_DIR"), "/triangle.vert.spv")),
            "vertex_main",
            [0, 0, 0],
        )?;
        let fragment = context.shader(
            include_bytes!(concat!(env!("OUT_DIR"), "/triangle.frag.spv")),
            "fragment_main",
            [0, 0, 0],
        )?;
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("triangle"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("triangle"),
            layout: Some(&layout),
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
                targets: &[Some(format.into())],
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
        // 永久错误保留首错，后续检查不消费。
        let gpu_error = OnceLock::new();
        let (pending, errors) = mpsc::channel();
        let pending = pending.clone();
        // wgpu 要求 Arc 回调对象；设备与注册表持有至卸载/设备释放，捕获仅持 sender，错误由 renderer 独占锁存。
        device.on_uncaptured_error(Arc::new({
            let pending = pending.clone();
            move |error: wgpu::Error| {
                log::error!("{error}");
                let _ = pending.send(error.to_string());
            }
        }));
        context.on_device_lost(move |reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                log::error!("GPU 已丢失：{reason:?}：{message}");
                let _ = pending.send(format!("{reason:?}：{message}"));
            }
        });
        Ok(Self {
            surface,
            context,
            pipeline,
            config,
            gpu_error,
            errors,
            size: [size.width, size.height],
            dirty: true,
            failed: false,
        })
    }

    /// 零尺寸暂停绘制；交换链在下一次绘制时重建。
    pub fn resize(&mut self, width: u32, height: u32) {
        let size = [width, height];
        if self.size != size {
            self.size = size;
            self.dirty = true;
        }
    }

    /// 提交并排定一次呈现，成功返回 true 不代表屏幕已显示。
    ///
    /// 零尺寸、超时、遮挡或 Outdated 返回 false，Outdated 令下次重配 Surface。
    /// Lost、验证或锁存设备错误返回失败并停止后续绘制，须重新创建 renderer。
    pub fn draw(&mut self) -> Result<bool, Error> {
        if self.failed {
            return Err(Error::RendererStopped);
        }
        let result = self.draw_frame();
        self.failed = result.is_err();
        result
    }

    /// 挂起或退出时无限期等待设备完成并检查首个异步错误，不用于每帧同步。
    /// 等待或设备失败保留来源；不会清除锁存错误或恢复停止状态。
    pub fn finish(&self) -> Result<(), Error> {
        self.context
            .device()
            .poll(wgpu::PollType::wait_indefinitely())?;
        self.check_gpu()
    }

    fn check_gpu(&self) -> Result<(), Error> {
        for error in self.errors.try_iter() {
            let _ = self.gpu_error.set(error);
        }
        match self.gpu_error.get() {
            Some(message) => Err(Error::GpuStopped(message.clone())),
            None => Ok(()),
        }
    }

    fn draw_frame(&mut self) -> Result<bool, Error> {
        self.check_gpu()?;
        if self.size.contains(&0) {
            return Ok(false);
        }
        let device = self.context.device();
        if self.dirty {
            let limit = device.limits().max_texture_dimension_2d;
            self.config.width = self.size[0].min(limit);
            self.config.height = self.size[1].min(limit);
            self.surface.configure(device, &self.config);
            self.dirty = false;
        }
        let (frame, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
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
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.015,
                            g: 0.02,
                            b: 0.035,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.draw(0..3, 0..1);
        }
        self.context.queue().submit([encoder.finish()]);
        self.context.queue().present(frame);
        self.dirty = suboptimal;
        self.check_gpu()?;
        Ok(true)
    }
}

// 线性 sRGB 输出依赖附件编码，其他色彩空间尚无输出变换。
// https://docs.rs/wgpu/30.0.1/wgpu/#surface-color-spaces-and-hdr-output
fn surface_format(
    formats: &[wgpu::SurfaceFormatCapabilities],
) -> Result<wgpu::TextureFormat, Error> {
    formats
        .iter()
        .find(|format| {
            matches!(
                format.format,
                wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Rgba8UnormSrgb
            ) && format.color_spaces.contains(wgpu::SurfaceColorSpaces::SRGB)
        })
        .map(|format| format.format)
        .ok_or(Error::NoSrgbSurfaceFormat)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_format_matches_output_encoding() {
        let formats = [
            wgpu::SurfaceFormatCapabilities {
                format: wgpu::TextureFormat::Bgra8Unorm,
                color_spaces: wgpu::SurfaceColorSpaces::SRGB,
            },
            wgpu::SurfaceFormatCapabilities {
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                color_spaces: wgpu::SurfaceColorSpaces::DISPLAY_P3,
            },
            wgpu::SurfaceFormatCapabilities {
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                color_spaces: wgpu::SurfaceColorSpaces::SRGB,
            },
        ];
        assert_eq!(surface_format(&formats).unwrap(), formats[2].format);
    }

    #[test]
    fn surface_format_rejects_missing_output_transform() {
        let formats = [wgpu::SurfaceFormatCapabilities {
            format: wgpu::TextureFormat::Bgra8Unorm,
            color_spaces: wgpu::SurfaceColorSpaces::SRGB,
        }];
        assert!(matches!(
            surface_format(&formats),
            Err(Error::NoSrgbSurfaceFormat)
        ));
    }
}
