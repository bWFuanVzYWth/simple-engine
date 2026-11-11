//! wgpu 大气资源 owner；不创建平台设备、不提交队列、不隐式等待。
//!
//! [`Atmosphere::initialize`] 将 optical depth 和八轮散射记录到 caller encoder。
//! 提交、等待完成后可用 [`Atmosphere::release_initialization`] 释放第二场与 scratch；
//! 最终场不再写入。各轮参数独立，避免 queue write 覆盖尚未执行的 dispatch。
//!
//! 每帧依次 `prepare`、`record`、submit，再开始下一次 `prepare`；同一 owner 的输出
//! 纹理会原位更新，不能作为互不排序的多视图缓存。多视图需要独立 owner。
//! caller 可在同一 encoder 后续 pass 采样，或按同一队列提交顺序接入其他后端。
//! 丢弃已记录的更新 encoder 后须重建 owner，不能把尚未执行的内容当作有效缓存。
//!
//! native 应由 Vulkan 安全入口直通本模块的 SPIR-V；这里仅接收已创建的模块。
//! `device/queue/shaders` 必须来自同一设备；shader 接口及 GPU 错误由受控产物与
//! caller error scope 共同维护。RGBA32F 只做手动双线性采样，不要求 float32 filtering。
//! 固定 GPU payload 常驻约 8.4 MiB；初始化另有约 36.3 MiB 的 bank/scratch/参数。
//! 构造期间还临时解包 199584 B 物理数据，上传后释放；wgpu staging 与驱动分配另计。

use crate::{
    AERIAL_SIZE, Error, FrameState, MAPPING_ROWS, Memory, Model, Parameters, SKY_SIZE, Update, View,
};
use std::num::NonZeroU64;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

pub use crate::{SHADERS, ShaderBinary};

/// caller 在同一设备创建的八个生产模块。
pub struct Shaders {
    /// 顺序严格对应 [`SHADERS`]，仅接受同批构建的产物。
    pub modules: [wgpu::ShaderModule; 8],
}

struct Image {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

fn image(device: &wgpu::Device, label: &str, size: [u32; 3], format: wgpu::TextureFormat) -> Image {
    let texture = device.create_texture(&image_descriptor(label, size, format));
    output_image(texture)
}

fn image_descriptor(
    label: &str,
    size: [u32; 3],
    format: wgpu::TextureFormat,
) -> wgpu::TextureDescriptor<'_> {
    let dimension = if size[2] == 1 {
        wgpu::TextureDimension::D2
    } else {
        wgpu::TextureDimension::D3
    };
    wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: size[2],
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    }
}

struct Bank {
    source: Image,
    mean: Image,
    ground: Image,
    high: Image,
}

impl Bank {
    fn descriptors(p: &Parameters) -> [(&'static str, [u32; 3], wgpu::TextureFormat); 4] {
        let [h, s, _, _] = p.dims;
        let low = p.offsets1[1];
        [
            (
                "atmosphere source",
                [s * p.solve[1], low * p.solve[2], 1],
                wgpu::TextureFormat::Rgba16Float,
            ),
            (
                "atmosphere mean",
                [s, h, 1],
                wgpu::TextureFormat::Rgba32Float,
            ),
            (
                "atmosphere ground",
                [s, 1, 1],
                wgpu::TextureFormat::Rgba32Float,
            ),
            (
                "atmosphere high",
                [s * 5, h - low + 1, 1],
                wgpu::TextureFormat::Rgba32Float,
            ),
        ]
    }
    fn from_images([source, mean, ground, high]: [Image; 4]) -> Self {
        Self {
            source,
            mean,
            ground,
            high,
        }
    }
    fn new(device: &wgpu::Device, p: &Parameters) -> Self {
        Self::from_images(
            Self::descriptors(p).map(|(label, size, format)| image(device, label, size, format)),
        )
    }
    fn bytes(p: &Parameters) -> u64 {
        let [h, s, _, _] = p.dims.map(u64::from);
        let low = u64::from(p.offsets1[1]);
        low * s * u64::from(p.solve[1]) * u64::from(p.solve[2]) * 8
            + (h - low + 1) * s * 5 * 16
            + h * s * 16
            + s * 16
    }
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

fn uniform(device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) -> wgpu::Buffer {
    let result = buffer(
        device,
        "atmosphere parameters",
        bytes.len() as u64,
        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    );
    queue.write_buffer(&result, 0, bytes);
    result
}

#[derive(Clone, Copy)]
enum Kind {
    Uniform(u64),
    Read,
    Write,
    Filtered,
    Unfiltered,
    Sampler,
    Store16,
    Store32,
    Store3d,
}

const LAYOUTS: [&[Kind]; 8] = {
    use Kind::*;
    [
        &[Uniform(400), Read, Store16],
        &[Uniform(400), Read, Write],
        &[
            Uniform(400),
            Read,
            Filtered,
            Filtered,
            Unfiltered,
            Unfiltered,
            Sampler,
            Write,
            Write,
            Unfiltered,
        ],
        &[Uniform(400), Write, Write, Write, Store32, Store32],
        &[Uniform(400), Read, Store16, Unfiltered, Write, Write, Write],
        &[Uniform(400), Read, Filtered, Sampler, Write, Write, Store32],
        &[
            Uniform(400),
            Read,
            Filtered,
            Filtered,
            Unfiltered,
            Unfiltered,
            Sampler,
            Unfiltered,
            Read,
            Store32,
        ],
        &[
            Uniform(400),
            Read,
            Filtered,
            Filtered,
            Unfiltered,
            Unfiltered,
            Sampler,
            Unfiltered,
            Uniform(80),
            Store3d,
            Store3d,
        ],
    ]
};

fn layout(device: &wgpu::Device, kinds: &[Kind]) -> wgpu::BindGroupLayout {
    let entries: Vec<_> = kinds
        .iter()
        .enumerate()
        .map(|(binding, kind)| {
            let ty = match *kind {
                Kind::Uniform(size) => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(size),
                },
                Kind::Read | Kind::Write => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage {
                        read_only: matches!(kind, Kind::Read),
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                Kind::Filtered | Kind::Unfiltered => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float {
                        filterable: matches!(kind, Kind::Filtered),
                    },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                Kind::Sampler => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                Kind::Store16 | Kind::Store32 | Kind::Store3d => {
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: if matches!(kind, Kind::Store32) {
                            wgpu::TextureFormat::Rgba32Float
                        } else {
                            wgpu::TextureFormat::Rgba16Float
                        },
                        view_dimension: if matches!(kind, Kind::Store3d) {
                            wgpu::TextureViewDimension::D3
                        } else {
                            wgpu::TextureViewDimension::D2
                        },
                    }
                }
            };
            wgpu::BindGroupLayoutEntry {
                binding: binding as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty,
                count: None,
            }
        })
        .collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("atmosphere stage"),
        entries: &entries,
    })
}

fn group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    resources: &[wgpu::BindingResource<'_>],
) -> wgpu::BindGroup {
    let entries: Vec<_> = resources
        .iter()
        .enumerate()
        .map(|(binding, resource)| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: resource.clone(),
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("atmosphere stage"),
        layout,
        entries: &entries,
    })
}

fn dispatch(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    group: &wgpu::BindGroup,
    counts: [u32; 3],
) {
    if counts.contains(&0) {
        return;
    }
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("atmosphere"),
        timestamp_writes: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.dispatch_workgroups(counts[0], counts[1], counts[2]);
}

fn tv(image: &Image) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::TextureView(&image.view)
}

struct Scratch {
    _bank: Bank,
    _buffers: Vec<wgpu::Buffer>,
    bytes: u64,
}

/// 四个输出的固定描述符，顺序为 optical depth、SkyView、Aerial L、Aerial T。
///
/// 除计算和采样用途外包含 COPY_SRC 供诊断；外部 owner 可据此创建共享纹理，
/// 无需复制尺寸、格式或 D2/D3 约定。
pub const fn output_descriptors() -> [wgpu::TextureDescriptor<'static>; 4] {
    const fn descriptor(
        label: &'static str,
        size: [u32; 3],
        format: wgpu::TextureFormat,
    ) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: size[2],
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: if size[2] == 1 {
                wgpu::TextureDimension::D2
            } else {
                wgpu::TextureDimension::D3
            },
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                .union(wgpu::TextureUsages::STORAGE_BINDING)
                .union(wgpu::TextureUsages::COPY_SRC),
            view_formats: &[],
        }
    }
    [
        descriptor(
            "atmosphere optical depth",
            [512, 128, 1],
            wgpu::TextureFormat::Rgba16Float,
        ),
        descriptor(
            "atmosphere SkyView",
            [SKY_SIZE, SKY_SIZE, 1],
            wgpu::TextureFormat::Rgba32Float,
        ),
        descriptor(
            "atmosphere Aerial L",
            [AERIAL_SIZE; 3],
            wgpu::TextureFormat::Rgba16Float,
        ),
        descriptor(
            "atmosphere Aerial T",
            [AERIAL_SIZE; 3],
            wgpu::TextureFormat::Rgba16Float,
        ),
    ]
}

/// 外部 owner 创建的四张输出；移入或克隆 wgpu 句柄不复制显存。
///
/// 纹理须与传给 [`Atmosphere::new_with_outputs`] 的 device 同归属，并符合
/// [`output_descriptors`] 的尺寸、格式、维度、mip/sample 数；COPY_SRC 是可选诊断用途。
/// 本 crate 保活句柄并原位更新，外部不得在使用期间显式 destroy。
pub struct Outputs {
    /// 四波长 optical depth，RGBA16F 512×128 D2。
    pub optical_depth: wgpu::Texture,
    /// 三 chart log Rec.2020，RGBA32F 256² D2。
    pub sky: wgpu::Texture,
    /// 视锥介质散射辐亮度，不含 planet ground 边界，RGBA16F 32³ D3。
    pub aerial_light: wgpu::Texture,
    /// 视锥 RGB 透射近似，RGBA16F 32³ D3。
    pub aerial_transmittance: wgpu::Texture,
}

impl Outputs {
    fn new(device: &wgpu::Device) -> Self {
        let [optical_depth, sky, aerial_light, aerial_transmittance] =
            output_descriptors().map(|desc| device.create_texture(&desc));
        Self {
            optical_depth,
            sky,
            aerial_light,
            aerial_transmittance,
        }
    }
    fn validate(&self) -> Result<(), Error> {
        for (texture, descriptor) in [
            &self.optical_depth,
            &self.sky,
            &self.aerial_light,
            &self.aerial_transmittance,
        ]
        .into_iter()
        .zip(output_descriptors())
        {
            if texture.size() != descriptor.size
                || texture.dimension() != descriptor.dimension
                || texture.format() != descriptor.format
                || texture.mip_level_count() != 1
                || texture.sample_count() != 1
                || !texture.usage().contains(
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
                )
            {
                return Err(Error::Output(descriptor.label.unwrap_or("unknown")));
            }
        }
        Ok(())
    }
}

fn output_image(texture: wgpu::Texture) -> Image {
    Image {
        view: texture.create_view(&Default::default()),
        texture,
    }
}

/// 借用当前 GPU 输出；纹理为 mip 0，单层 D2 或真正 D3，均可直接创建采样视图。
///
/// SkyView 是 log(max(线性 Rec.2020,1e-30))；Aerial L/T 是线性值。
/// 所有查表必须使用同批 `sampling.slang` 与 [`Atmosphere::parameters`] / mapping。
pub struct Bindings<'a> {
    /// RGBA16F 512×128 的四波长 optical depth。
    pub optical_depth: &'a wgpu::Texture,
    /// optical depth 的 D2 视图。
    pub optical_depth_view: &'a wgpu::TextureView,
    /// RGBA32F 256×256 的三 chart SkyView。
    pub sky: &'a wgpu::Texture,
    /// SkyView 的 D2 视图。
    pub sky_view: &'a wgpu::TextureView,
    /// RGBA16F 32³ 的介质散射 RGB，已乘 View::radiance_scale，不含 planet ground。
    pub aerial_light: &'a wgpu::Texture,
    /// Aerial 散射 D3 视图。
    pub aerial_light_view: &'a wgpu::TextureView,
    /// RGBA16F 32³ 的 RGB 透射近似。
    pub aerial_transmittance: &'a wgpu::Texture,
    /// Aerial 透射 D3 视图。
    pub aerial_transmittance_view: &'a wgpu::TextureView,
    /// 线性、clamp-to-edge 采样器；SkyView 手动采样不使用过滤。
    pub sampler: &'a wgpu::Sampler,
    /// 与 CPU mapping 一致的只读 storage buffer。
    pub sky_mapping: &'a wgpu::Buffer,
    /// 与 CPU Parameters 一致的 400 B uniform buffer。
    pub parameters: &'a wgpu::Buffer,
}

/// 固定 Earth/balanced 场与一份 observer cache；句柄从 caller 克隆。
pub struct Atmosphere {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: [wgpu::ComputePipeline; 8],
    layouts: [wgpu::BindGroupLayout; 8],
    data: wgpu::Buffer,
    base: Parameters,
    calibration: crate::mapping::Calibration,
    frame: FrameState,
    field: Bank,
    tau: Image,
    sky: Image,
    aerial_light: Image,
    aerial_t: Image,
    sampler: wgpu::Sampler,
    sky_uniform: wgpu::Buffer,
    frame_uniform: wgpu::Buffer,
    aerial_uniform: wgpu::Buffer,
    mapping: wgpu::Buffer,
    sky_group: wgpu::BindGroup,
    aerial_group: wgpu::BindGroup,
    initialized: bool,
    scratch: Option<Scratch>,
    resident_bytes: u64,
}

impl Atmosphere {
    /// 创建资源和 pipeline，不提交或求解；设备、队列和模块必须同归属。
    ///
    /// # Errors
    /// 嵌入物理参数解析或设备 texture/storage buffer 限制不满足时返回错误。
    /// GPU 创建错误通过 caller 的 wgpu error scope 报告。
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shaders: Shaders,
    ) -> Result<Self, Error> {
        Self::create(device, queue, shaders, None, None)
    }

    /// 使用同设备外部分配的输出，其他资源及初始化行为与 [`Self::new`] 一致。
    ///
    /// 输出的设备归属由 caller owner 维护；这里校验可查询的跨字段纹理约束。
    /// [`Memory::resident_bytes`] 包含这些输出的 payload，上层统计不能再重复计入。
    /// # Errors
    /// 输出尺寸、格式、用途、mip/sample 或设备容量不匹配时返回错误。
    pub fn new_with_outputs(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shaders: Shaders,
        outputs: Outputs,
    ) -> Result<Self, Error> {
        Self::new_with_outputs_cached(device, queue, shaders, outputs, None)
    }

    /// 使用外部输出及可选的同设备驱动管线缓存，契约与 [`Self::new_with_outputs`] 一致。
    pub fn new_with_outputs_cached(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shaders: Shaders,
        outputs: Outputs,
        cache: Option<&wgpu::PipelineCache>,
    ) -> Result<Self, Error> {
        Self::create(device, queue, shaders, Some(outputs), cache)
    }

    fn create(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shaders: Shaders,
        outputs: Option<Outputs>,
        cache: Option<&wgpu::PipelineCache>,
    ) -> Result<Self, Error> {
        let model = Model::new()?;
        Self::from_model(device, queue, shaders, outputs, cache, model, None)
    }

    /// 为原生体积光共享同一份已求解散射场。四张输出依次为 optical depth、天空、空气辐亮度和透射。
    /// 调用方提交初始化后，两端按同一设备队列顺序消费；物理场只建立一次。
    /// # Errors
    /// 输出格式、设备容量、分配或原生资源创建失败时返回错误。
    #[cfg(feature = "native")]
    pub fn with_terrain_model(
        context: &mut vulkan::interop::WgpuContext,
        shaders: Shaders,
        outputs: [&vulkan::interop::SampledTexture; 4],
    ) -> Result<(Self, crate::terrain::TerrainModel), Error> {
        use vulkan::native::{Binding, Sampler};
        let model = Model::new()?;
        let p = model.parameters;
        let data = context.buffer(
            "atmosphere physical input",
            bytemuck::cast_slice(&model.data),
            wgpu::BufferUsages::STORAGE,
        )?;
        let mut images = Vec::with_capacity(4);
        for (label, size, format) in Bank::descriptors(&p) {
            images.push(context.sampled_texture(&image_descriptor(label, size, format))?);
        }
        let images: [_; 4] = images
            .try_into()
            .unwrap_or_else(|_| unreachable!("四张固定散射场纹理"));
        let bank = Bank::from_images(images.each_ref().map(|i| output_image(i.texture().clone())));
        let sampler = Sampler::new(context.native(), true)?;
        let [src, mean, ground, high] = images.map(Binding::SampledTexture);
        let fields = crate::terrain::TerrainModel::new(
            context.native(),
            &p,
            [
                Binding::SharedBuffer(data.clone()),
                Binding::SampledTexture(outputs[0].clone()),
                src,
                mean,
                ground,
                Binding::Sampler(sampler),
                high,
            ],
        )?;
        let [optical_depth, sky, aerial_light, aerial_transmittance] =
            outputs.map(|o| o.texture().clone());
        let atmosphere = Self::from_model(
            context.device(),
            context.queue(),
            shaders,
            Some(Outputs {
                optical_depth,
                sky,
                aerial_light,
                aerial_transmittance,
            }),
            context.pipeline_cache(),
            model,
            Some((data.buffer().clone(), bank)),
        )?;
        Ok((atmosphere, fields))
    }

    fn from_model(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shaders: Shaders,
        outputs: Option<Outputs>,
        cache: Option<&wgpu::PipelineCache>,
        model: Model,
        shared: Option<(wgpu::Buffer, Bank)>,
    ) -> Result<Self, Error> {
        let p = model.parameters;
        let limits = device.limits();
        let incident_bytes = 4 * u64::from(p.dims[1]) * u64::from(p.solve[0]) * 16;
        if limits.max_texture_dimension_2d < p.dims[1] * p.solve[1]
            || limits.max_texture_dimension_3d < AERIAL_SIZE
        {
            return Err(Error::Limit("texture dimension"));
        }
        if limits.max_storage_buffer_binding_size < incident_bytes
            || limits.max_buffer_size < incident_bytes
        {
            return Err(Error::Limit("incident buffer"));
        }
        let outputs = outputs.unwrap_or_else(|| Outputs::new(device));
        outputs.validate()?;
        let layouts = std::array::from_fn(|i| layout(device, LAYOUTS[i]));
        let pipelines = std::array::from_fn(|i| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(SHADERS[i].entry),
                bind_group_layouts: &[Some(&layouts[i])],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(SHADERS[i].entry),
                layout: Some(&pipeline_layout),
                module: &shaders.modules[i],
                entry_point: Some(SHADERS[i].entry),
                compilation_options: Default::default(),
                cache,
            })
        });
        let (data, field) = shared.unwrap_or_else(|| {
            let data = buffer(
                device,
                "atmosphere physical input",
                model.data.len() as u64 * 16,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            queue.write_buffer(&data, 0, bytemuck::cast_slice(&model.data));
            (data, Bank::new(device, &p))
        });
        let tau = output_image(outputs.optical_depth);
        let sky = output_image(outputs.sky);
        let aerial_light = output_image(outputs.aerial_light);
        let aerial_t = output_image(outputs.aerial_transmittance);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atmosphere linear clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let sky_uniform = uniform(device, queue, bytemuck::bytes_of(&p));
        let frame_uniform = uniform(device, queue, bytemuck::bytes_of(&p));
        let aerial_uniform = uniform(
            device,
            queue,
            bytemuck::cast_slice(&View::default().aerial_parameters()),
        );
        let mapping = buffer(
            device,
            "atmosphere SkyView mapping",
            (MAPPING_ROWS * 16) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let sky_group = group(
            device,
            &layouts[6],
            &[
                sky_uniform.as_entire_binding(),
                data.as_entire_binding(),
                tv(&tau),
                tv(&field.source),
                tv(&field.mean),
                tv(&field.ground),
                wgpu::BindingResource::Sampler(&sampler),
                tv(&field.high),
                mapping.as_entire_binding(),
                tv(&sky),
            ],
        );
        let aerial_group = group(
            device,
            &layouts[7],
            &[
                frame_uniform.as_entire_binding(),
                data.as_entire_binding(),
                tv(&tau),
                tv(&field.source),
                tv(&field.mean),
                tv(&field.ground),
                wgpu::BindingResource::Sampler(&sampler),
                tv(&field.high),
                aerial_uniform.as_entire_binding(),
                tv(&aerial_light),
                tv(&aerial_t),
            ],
        );
        let resident_bytes = Bank::bytes(&p)
            + model.data.len() as u64 * 16
            + u64::from(p.dims[2]) * u64::from(p.dims[3]) * 8
            + u64::from(SKY_SIZE).pow(2) * 16
            + u64::from(AERIAL_SIZE).pow(3) * 16
            + (MAPPING_ROWS * 16) as u64
            + 880;
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            pipelines,
            layouts,
            data,
            base: p,
            calibration: model.calibration,
            frame: FrameState::new(p),
            field,
            tau,
            sky,
            aerial_light,
            aerial_t,
            sampler,
            sky_uniform,
            frame_uniform,
            aerial_uniform,
            mapping,
            sky_group,
            aerial_group,
            initialized: false,
            scratch: None,
            resident_bytes,
        })
    }

    /// 记录 optical depth 与八轮多次散射；临时场仅在初始化使用。
    ///
    /// 不隐式提交或等待。后续消费者必须排在本 encoder 后；完成后显式释放 scratch。
    /// 每批最多四个高度，incident 与 directions 各约 15 MiB，批间复用且不隐藏并行调度。
    /// # Errors
    /// 同一物理场不允许重复初始化。
    pub fn initialize(&mut self, encoder: &mut wgpu::CommandEncoder) -> Result<(), Error> {
        if self.initialized {
            return Err(Error::AlreadyInitialized);
        }
        let device = &self.device;
        let p = self.base;
        let extra = Bank::new(device, &p);
        let count = 4 * u64::from(p.dims[1]) * u64::from(p.solve[0]);
        let incoming = buffer(
            device,
            "atmosphere incident scratch",
            count * 16,
            wgpu::BufferUsages::STORAGE,
        );
        let directions = buffer(
            device,
            "atmosphere directions scratch",
            count * 16,
            wgpu::BufferUsages::STORAGE,
        );
        let moments = buffer(
            device,
            "atmosphere moments scratch",
            4 * u64::from(p.dims[1]) * 7 * 16,
            wgpu::BufferUsages::STORAGE,
        );
        let initial = uniform(device, &self.queue, bytemuck::bytes_of(&p));
        let optical = group(
            device,
            &self.layouts[0],
            &[
                initial.as_entire_binding(),
                self.data.as_entire_binding(),
                tv(&self.tau),
            ],
        );
        dispatch(
            encoder,
            &self.pipelines[0],
            &optical,
            [p.dims[3].div_ceil(8), p.dims[2].div_ceil(8), 1],
        );
        let ground = group(
            device,
            &self.layouts[5],
            &[
                initial.as_entire_binding(),
                self.data.as_entire_binding(),
                tv(&self.tau),
                wgpu::BindingResource::Sampler(&self.sampler),
                directions.as_entire_binding(),
                incoming.as_entire_binding(),
                tv(&self.field.ground),
            ],
        );
        dispatch(
            encoder,
            &self.pipelines[5],
            &ground,
            [p.dims[1].div_ceil(64), 1, 1],
        );
        let mut uniforms = Vec::new();
        uniforms.try_reserve_exact(84)?;
        uniforms.push(initial);
        let banks = [&self.field, &extra];
        let mut current = 0;
        for iteration in 1..=8 {
            let next = 1 - current;
            for first in (0..p.dims[0]).step_by(4) {
                let mut params = p;
                params.batch = [first, 4.min(p.dims[0] - first), 0, 0];
                params.solve[3] = iteration;
                let uniform = uniform(device, &self.queue, bytemuck::bytes_of(&params));
                let previous = banks[current];
                let target = banks[next];
                let states = params.batch[1] * p.dims[1];
                let direction_group = group(
                    device,
                    &self.layouts[1],
                    &[
                        uniform.as_entire_binding(),
                        self.data.as_entire_binding(),
                        directions.as_entire_binding(),
                    ],
                );
                dispatch(
                    encoder,
                    &self.pipelines[1],
                    &direction_group,
                    [p.solve[0].div_ceil(64), states, 1],
                );
                let incident_group = group(
                    device,
                    &self.layouts[2],
                    &[
                        uniform.as_entire_binding(),
                        self.data.as_entire_binding(),
                        tv(&self.tau),
                        tv(&previous.source),
                        tv(&previous.mean),
                        tv(&previous.ground),
                        wgpu::BindingResource::Sampler(&self.sampler),
                        directions.as_entire_binding(),
                        incoming.as_entire_binding(),
                        tv(&previous.high),
                    ],
                );
                dispatch(
                    encoder,
                    &self.pipelines[2],
                    &incident_group,
                    [p.solve[0].div_ceil(64), states, 1],
                );
                let moments_group = group(
                    device,
                    &self.layouts[3],
                    &[
                        uniform.as_entire_binding(),
                        directions.as_entire_binding(),
                        incoming.as_entire_binding(),
                        moments.as_entire_binding(),
                        tv(&target.high),
                        tv(&target.mean),
                    ],
                );
                dispatch(
                    encoder,
                    &self.pipelines[3],
                    &moments_group,
                    [states.div_ceil(64), 1, 1],
                );
                let source_group = group(
                    device,
                    &self.layouts[4],
                    &[
                        uniform.as_entire_binding(),
                        self.data.as_entire_binding(),
                        tv(&target.source),
                        tv(&target.mean),
                        directions.as_entire_binding(),
                        incoming.as_entire_binding(),
                        moments.as_entire_binding(),
                    ],
                );
                dispatch(
                    encoder,
                    &self.pipelines[4],
                    &source_group,
                    [
                        (p.dims[1] * p.solve[1]).div_ceil(8),
                        (p.offsets1[1].saturating_sub(first).min(params.batch[1]) * p.solve[2])
                            .div_ceil(8),
                        1,
                    ],
                );
                let ground_group = group(
                    device,
                    &self.layouts[5],
                    &[
                        uniform.as_entire_binding(),
                        self.data.as_entire_binding(),
                        tv(&self.tau),
                        wgpu::BindingResource::Sampler(&self.sampler),
                        directions.as_entire_binding(),
                        incoming.as_entire_binding(),
                        tv(&target.ground),
                    ],
                );
                dispatch(
                    encoder,
                    &self.pipelines[5],
                    &ground_group,
                    [p.dims[1].div_ceil(64), 1, 1],
                );
                uniforms.push(uniform);
            }
            current = next;
        }
        let bytes = Bank::bytes(&p)
            + count * 32
            + 4 * u64::from(p.dims[1]) * 7 * 16
            + uniforms.len() as u64 * 400;
        uniforms.extend([incoming, directions, moments]);
        self.scratch = Some(Scratch {
            _bank: extra,
            _buffers: uniforms,
            bytes,
        });
        self.initialized = true;
        Ok(())
    }

    /// 释放初始化专用句柄；应在 caller 已提交并确认初始化完成后调用。
    /// wgpu 自持在途资源，本方法不等待，也不改变最终不可变散射场。
    pub fn release_initialization(&mut self) {
        self.scratch = None;
    }

    /// 校验视图并上传本次参数；只相对上次 `record` 的输入判定失效。
    ///
    /// 不同 sun azimuth 复用轴对称 SkyView；高度或 elevation 改变重建整个 SkyView。
    /// 视锥、距离和颜色倍率仅使 Aerial 失效。失败不改变当前有效帧。
    /// # Errors
    /// 未记录物理初始化或 [`View`] 约束不成立时返回错误。
    pub fn prepare(&mut self, view: View) -> Result<Update, Error> {
        if !self.initialized {
            return Err(Error::NotInitialized);
        }
        let previous_pending = self.frame.pending;
        let update = self.frame.prepare(view, &self.calibration)?;
        if update.sky || previous_pending.sky {
            let mut sky_params = self.frame.parameters;
            sky_params.sun[0] = 0.0;
            self.queue
                .write_buffer(&self.sky_uniform, 0, bytemuck::bytes_of(&sky_params));
            self.queue
                .write_buffer(&self.mapping, 0, bytemuck::cast_slice(&self.frame.mapping));
        }
        if update.aerial || previous_pending.aerial {
            self.queue.write_buffer(
                &self.frame_uniform,
                0,
                bytemuck::bytes_of(&self.frame.parameters),
            );
            self.queue.write_buffer(
                &self.aerial_uniform,
                0,
                bytemuck::cast_slice(
                    &self
                        .frame
                        .view
                        .expect("prepare 已设置视图")
                        .aerial_parameters(),
                ),
            );
        }
        Ok(update)
    }

    /// 记录最新 `prepare` 所需的更新并提交 CPU key；不提交 GPU 队列。
    /// 重复调用或稳态不产生 dispatch；提交顺序与输出复用约定见模块文档。
    pub fn record(&mut self, encoder: &mut wgpu::CommandEncoder) -> Update {
        let update = self.frame.commit();
        if update.sky {
            dispatch(
                encoder,
                &self.pipelines[6],
                &self.sky_group,
                [SKY_SIZE.div_ceil(8), SKY_SIZE.div_ceil(8), 1],
            );
        }
        if update.aerial {
            dispatch(
                encoder,
                &self.pipelines[7],
                &self.aerial_group,
                [AERIAL_SIZE.div_ceil(4); 3],
            );
        }
        update
    }

    /// 取消含视图更新的未提交encoder后使CPU缓存键失效；下次prepare重录SkyView和Aerial。
    /// 不丢弃物理场、不等待GPU；已经提交的命令不能由此撤销。
    pub fn invalidate_view(&mut self) {
        self.frame.invalidate_view();
    }

    /// 借用输出纹理和采样资源；GPU 内容须遵守模块的录制及提交约定。
    pub fn bindings(&self) -> Bindings<'_> {
        Bindings {
            optical_depth: &self.tau.texture,
            optical_depth_view: &self.tau.view,
            sky: &self.sky.texture,
            sky_view: &self.sky.view,
            aerial_light: &self.aerial_light.texture,
            aerial_light_view: &self.aerial_light.view,
            aerial_transmittance: &self.aerial_t.texture,
            aerial_transmittance_view: &self.aerial_t.view,
            sampler: &self.sampler,
            sky_mapping: &self.mapping,
            parameters: &self.frame_uniform,
        }
    }
    /// 借用最新 prepare 的 400 B CPU 镜像，可用于其他后端的 storage 上传。
    pub const fn parameters(&self) -> &Parameters {
        &self.frame.parameters
    }
    /// 借用最新 SkyView mapping 的 CPU 镜像；首次 prepare 前内容尚未生成。
    pub const fn sky_mapping(&self) -> &[[f32; 4]; MAPPING_ROWS] {
        &self.frame.mapping
    }
    /// 当前持有的资源 payload；初始化 scratch 释放后相应字节归零。
    pub fn memory(&self) -> Memory {
        Memory {
            resident_bytes: self.resident_bytes,
            initialization_bytes: self.scratch.as_ref().map_or(0, |s| s.bytes),
            cpu_bytes: std::mem::size_of::<Self>() as u64
                + self.scratch.as_ref().map_or(0, |scratch| {
                    (scratch._buffers.capacity() * std::mem::size_of::<wgpu::Buffer>()) as u64
                }),
        }
    }
}
