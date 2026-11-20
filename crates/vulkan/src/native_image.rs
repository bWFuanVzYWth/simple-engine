//! native图像与采样器的创建和传输；描述值不延长资源寿命。
use super::*;

/// 单 mip、单 sample 的图像格式。
#[derive(Clone, Copy)]
pub enum ImageFormat {
    /// 单通道归一化遮罩。
    R8Unorm,
    /// 单通道线性标量。
    R32Float,
    /// 双通道半精度向量。
    Rg16Float,
    /// 双通道全精度，例如离线衍射频谱的实部与虚部。
    Rg32Float,
    /// 线性 HDR。
    Rgba32Float,
    /// 线性 LUT。
    Rgba16Float,
    /// UNORM 输出。
    Rgba8Unorm,
    /// NRD 的打包世界法线、粗糙度和 2 bit 材质编号。
    Rgb10a2Unorm,
    /// 自动解码的 sRGB atlas。
    Rgba8Srgb,
}
impl ImageFormat {
    pub(super) fn raw(self) -> vk::Format {
        match self {
            Self::R8Unorm => vk::Format::R8_UNORM,
            Self::R32Float => vk::Format::R32_SFLOAT,
            Self::Rg16Float => vk::Format::R16G16_SFLOAT,
            Self::Rg32Float => vk::Format::R32G32_SFLOAT,
            Self::Rgba32Float => vk::Format::R32G32B32A32_SFLOAT,
            Self::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
            Self::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
            Self::Rgb10a2Unorm => vk::Format::A2B10G10R10_UNORM_PACK32,
            Self::Rgba8Srgb => vk::Format::R8G8B8A8_SRGB,
        }
    }
    pub(super) const fn bytes(self) -> usize {
        match self {
            Self::R8Unorm => 1,
            Self::Rgba32Float => 16,
            Self::Rgba16Float | Self::Rg32Float => 8,
            _ => 4,
        }
    }
}
/// 原生图像的只读描述；不转移或延长句柄寿命。
///
/// 此值不保活资源。外部命令须使用同设备并维护同步/GENERAL 布局；未登记的外部使用须由调用方将 Image 留在 Context 发布根至完成。
/// `format`、`usage`、`layout` 分别为 Vulkan 对应枚举的原始值。
#[derive(Clone, Copy)]
pub struct RawImage {
    /// VkImage。
    pub image: u64,
    /// VkImageView。
    pub view: u64,
    /// backing VkDeviceMemory。
    pub memory: u64,
    /// 三维 texel 尺寸。
    pub extent: [u32; 3],
    /// VkFormat。
    pub format: u32,
    /// VkImageUsageFlags。
    pub usage: u32,
    /// 交接 VkImageLayout。
    pub layout: u32,
}
pub(crate) struct ImageData {
    pub(super) raw: vk::Image,
    pub(super) view: vk::ImageView,
    pub(super) memory: vk::DeviceMemory,
    pub(super) extent: [u32; 3],
    pub(super) format: ImageFormat,
    pub(super) allocation: u64,
}
pub(super) fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1)
}
impl Image {
    pub(super) fn data<'a>(&self, context: &'a Context) -> Result<&'a ImageData, Error> {
        match context.get(self.key)? {
            Data::Image(data) => Ok(data),
            _ => Err(Error::InvalidArgument("image 标识类型无效")),
        }
    }
    /// 创建图像，同步初始化布局为 GENERAL，不初始化 texel；sRGB 只允许 sampled。
    pub fn new(
        context: &mut Context,
        extent: [u32; 3],
        format: ImageFormat,
    ) -> Result<Self, Error> {
        let data = create_image(context, extent, format)?;
        Ok(Self {
            key: context.insert(Data::Image(data))?,
            extent,
        })
    }
    /// 三维尺寸，2D 深度为1。
    pub const fn extent(&self) -> [u32; 3] {
        self.extent
    }
    /// 借入原始描述；调用方保证外部使用完成及同步。
    pub fn raw(&self, context: &Context) -> Result<RawImage, Error> {
        context.active(self.key)?;
        let inner = self.data(context)?;
        Ok(RawImage {
            image: inner.raw.as_raw(),
            view: inner.view.as_raw(),
            memory: inner.memory.as_raw(),
            extent: inner.extent,
            format: inner.format.raw().as_raw() as u32,
            usage: (vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST
                | if matches!(inner.format, ImageFormat::Rgba8Srgb) {
                    vk::ImageUsageFlags::empty()
                } else {
                    vk::ImageUsageFlags::STORAGE
                })
            .as_raw(),
            layout: vk::ImageLayout::GENERAL.as_raw() as u32,
        })
    }
    /// 同步上传紧密 mip0；调用方须完成此前图像使用。
    pub fn upload(&self, context: &mut Context, bytes: &[u8]) -> Result<(), Error> {
        context.active(self.key)?;
        context.resource_idle(self.key)?;
        let inner = self.data(context)?;
        let expected = inner
            .extent
            .into_iter()
            .try_fold(inner.format.bytes(), |size, n| size.checked_mul(n as usize));
        if expected != Some(bytes.len()) {
            return Err(Error::InvalidArgument("图像上传设备或字节数无效"));
        }
        let staging = Guard::new(
            &context.device,
            buffer::Buffer::new(
                &context.device,
                bytes.len(),
                vk::BufferUsageFlags::TRANSFER_SRC,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?,
        );
        staging.write(&context.device, 0, bytes)?;
        immediate(&context.device, |command| {
            unsafe {
                memory_barrier(&context.device, command);
                context.device.raw.cmd_copy_buffer_to_image(
                    command,
                    staging.raw,
                    inner.raw,
                    vk::ImageLayout::GENERAL,
                    &[vk::BufferImageCopy::default()
                        .image_subresource(
                            vk::ImageSubresourceLayers::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .layer_count(1),
                        )
                        .image_extent(vk::Extent3D {
                            width: inner.extent[0],
                            height: inner.extent[1],
                            depth: inner.extent[2],
                        })],
                );
                memory_barrier(&context.device, command);
            }
            Ok(())
        })
    }
    /// 同步回读紧密 mip0；调用方须完成此前图像使用。
    pub fn download(&self, context: &mut Context) -> Result<Vec<u8>, Error> {
        context.active(self.key)?;
        context.resource_idle(self.key)?;
        let inner = self.data(context)?;
        let len = inner
            .extent
            .into_iter()
            .try_fold(inner.format.bytes(), |size, n| size.checked_mul(n as usize))
            .ok_or(Error::InvalidArgument("图像回读大小溢出"))?;
        let staging = Guard::new(
            &context.device,
            buffer::Buffer::new(
                &context.device,
                len,
                vk::BufferUsageFlags::TRANSFER_DST,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?,
        );
        immediate(&context.device, |command| {
            unsafe {
                memory_barrier(&context.device, command);
                context.device.raw.cmd_copy_image_to_buffer(
                    command,
                    inner.raw,
                    vk::ImageLayout::GENERAL,
                    staging.raw,
                    &[vk::BufferImageCopy::default()
                        .image_subresource(
                            vk::ImageSubresourceLayers::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .layer_count(1),
                        )
                        .image_extent(vk::Extent3D {
                            width: inner.extent[0],
                            height: inner.extent[1],
                            depth: inner.extent[2],
                        })],
                );
                memory_barrier(&context.device, command);
            }
            Ok(())
        })?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len)?;
        bytes.resize(len, 0);
        unsafe {
            let mapped = context.device.raw.map_memory(
                staging.memory,
                0,
                len as u64,
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(mapped.cast::<u8>(), bytes.as_mut_ptr(), len);
            context.device.raw.unmap_memory(staging.memory);
        }
        Ok(bytes)
    }
}
impl Destroy for ImageData {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_image_view(self.view, None);
            device.raw.destroy_image(self.raw, None);
            device.raw.free_memory(self.memory, None);
        }
        device
            .buffer_bytes
            .fetch_sub(self.allocation, Ordering::Relaxed);
    }
}
pub(super) fn create_image(
    context: &Context,
    extent: [u32; 3],
    format: ImageFormat,
) -> Result<ImageData, Error> {
    context.device.ensure_active()?;
    let limits = context.device.physical.properties().limits;
    let limit = if extent[2] > 1 {
        limits.max_image_dimension3_d
    } else {
        limits.max_image_dimension2_d
    };
    if extent.contains(&0) || extent.into_iter().any(|n| n > limit) {
        return Err(Error::InvalidArgument("native 图像尺寸无效"));
    }
    let mut result = Guard::new(
        &context.device,
        ImageData {
            raw: vk::Image::null(),
            view: vk::ImageView::null(),
            memory: vk::DeviceMemory::null(),
            extent,
            format,
            allocation: 0,
        },
    );
    let usage = vk::ImageUsageFlags::SAMPLED
        | vk::ImageUsageFlags::TRANSFER_DST
        | vk::ImageUsageFlags::TRANSFER_SRC
        | if matches!(format, ImageFormat::Rgba8Srgb) {
            vk::ImageUsageFlags::empty()
        } else {
            vk::ImageUsageFlags::STORAGE
        };
    let raw = &context.device.raw;
    unsafe {
        result.raw = raw.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(if extent[2] > 1 {
                    vk::ImageType::TYPE_3D
                } else {
                    vk::ImageType::TYPE_2D
                })
                .format(format.raw())
                .extent(vk::Extent3D {
                    width: extent[0],
                    height: extent[1],
                    depth: extent[2],
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        )?;
        let requirements = raw.get_image_memory_requirements(result.raw);
        let properties = context
            .device
            .physical
            .instance
            .raw
            .get_physical_device_memory_properties(context.device.physical.raw);
        let kind = (0..properties.memory_type_count)
            .find(|&i| {
                requirements.memory_type_bits & (1 << i) != 0
                    && properties.memory_types[i as usize]
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .ok_or(Error::Unsupported("图像缺少 DEVICE_LOCAL 内存"))?;
        result.memory = raw.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(kind),
            None,
        )?;
        result.allocation = requirements.size;
        context
            .device
            .buffer_bytes
            .fetch_add(requirements.size, Ordering::Relaxed);
        raw.bind_image_memory(result.raw, result.memory, 0)?;
        result.view = raw.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(result.raw)
                .view_type(if extent[2] > 1 {
                    vk::ImageViewType::TYPE_3D
                } else {
                    vk::ImageViewType::TYPE_2D
                })
                .format(format.raw())
                .subresource_range(color_range()),
            None,
        )?;
    }
    immediate(&context.device, |command| {
        unsafe {
            raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(result.raw)
                    .subresource_range(color_range())],
            );
        }
        Ok(())
    })?;
    Ok(result.finish())
}
pub(crate) struct SamplerData {
    pub(super) raw: vk::Sampler,
}
impl Sampler {
    /// 创建 clamp-to-edge sampler。
    pub fn new(context: &mut Context, linear: bool) -> Result<Self, Error> {
        context.device.ensure_active()?;
        let filter = if linear {
            vk::Filter::LINEAR
        } else {
            vk::Filter::NEAREST
        };
        let raw = unsafe {
            context.device.raw.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(filter)
                    .min_filter(filter)
                    .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .max_lod(0.0),
                None,
            )?
        };
        Ok(Self {
            key: context.insert(Data::Sampler(SamplerData { raw }))?,
        })
    }
    pub(super) fn data<'a>(&self, context: &'a Context) -> Result<&'a SamplerData, Error> {
        match context.get(self.key)? {
            Data::Sampler(data) => Ok(data),
            _ => Err(Error::InvalidArgument("sampler 标识类型无效")),
        }
    }
}
impl Destroy for SamplerData {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_sampler(self.raw, None);
        }
    }
}
