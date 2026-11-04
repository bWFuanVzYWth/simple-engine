use crate::{ENERGY_EXTENT, energy_bytes};

/// 创建能量 texture 所需的设备能力不满足；分配和设备错误沿用 wgpu 的错误机制。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GpuError {
    /// 设备的 3D texture 维度上限小于 159。
    #[error("设备 3D texture 维度不足以保存 OpenPBR 能量表")]
    TextureLimit,
    /// 外部纹理不是要求的 44×32×159、RGBA16Float、D3、单 mip／sample，或缺少上传／采样 usage。
    #[error(
        "OpenPBR 能量纹理须为 44×32×159 RGBA16Float D3、单 mip/sample，包含 COPY_DST|TEXTURE_BINDING"
    )]
    TextureDescriptor,
}

/// 同一设备共享的原始 HALF4 3D texture 与线性 clamp sampler 所有者。
///
/// 每 texel 8B，总纹理数据 1,790,976B；不保留 CPU staging 副本。新建会通过 queue 上传一次，
/// 后续相同队列提交可直接消费。该对象本身没有隐式全局缓存或独立 submit／wait。
pub struct GpuEnergy {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

impl GpuEnergy {
    /// 新建并上传 pinned 数据；device 与 queue 须属于同一设备。
    /// # Errors
    /// 3D texture 尺寸不受设备支持时返回错误；其余资源创建失败通过设备的 wgpu 错误处理报告。
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, GpuError> {
        if device.limits().max_texture_dimension_3d < ENERGY_EXTENT[2] {
            return Err(GpuError::TextureLimit);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OpenPBR author GGX energy"),
            size: wgpu::Extent3d {
                width: ENERGY_EXTENT[0],
                height: ENERGY_EXTENT[1],
                depth_or_array_layers: ENERGY_EXTENT[2],
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self::new_with_texture(device, queue, texture)
    }

    /// 接管同设备的外部纹理，上传原始表并创建采样 view／sampler，不重复分配纹理。
    ///
    /// `device`、`queue` 和 `texture` 须属于同一设备，交由 wgpu 维护该设备一致性契约。
    /// 纹理可带额外 usage；原有内容会被能量表覆盖。native wrapper 可保留同一纹理的克隆，
    /// 本接口不导入 native handle，也不证明外部后端的所有权；提交者仍须保活其资源所有者。
    /// # Errors
    /// 尺寸、格式、维度、mip／sample 数量或必要 usage 不符时返回 TextureDescriptor，且不上传。
    /// 其余资源／设备错误通过 wgpu 的错误处理报告。
    pub fn new_with_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: wgpu::Texture,
    ) -> Result<Self, GpuError> {
        if texture.size()
            != (wgpu::Extent3d {
                width: ENERGY_EXTENT[0],
                height: ENERGY_EXTENT[1],
                depth_or_array_layers: ENERGY_EXTENT[2],
            })
            || texture.dimension() != wgpu::TextureDimension::D3
            || texture.format() != wgpu::TextureFormat::Rgba16Float
            || texture.mip_level_count() != 1
            || texture.sample_count() != 1
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(GpuError::TextureDescriptor);
        }
        queue.write_texture(
            texture.as_image_copy(),
            energy_bytes(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(44 * 8),
                rows_per_image: Some(32),
            },
            texture.size(),
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenPBR energy linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            ..Default::default()
        });
        Ok(Self {
            texture,
            view,
            sampler,
        })
    }

    /// 借用原始 3D texture；native 提交所有者须保活其克隆，不得显式销毁在途资源。
    pub const fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
    /// 借用默认 D3 view，采样格式为 Rgba16Float。
    pub const fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
    /// 借用 normalized coordinate、linear、clamp-to-edge、LOD0 sampler。
    pub const fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }
}
