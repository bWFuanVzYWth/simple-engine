//! 描述符布局与绑定版本；先验证全部资源，再发布不可变绑定。
use super::*;

impl ExternalBinding {
    /// 导入同设备、已初始化资源。
    /// # Safety
    /// handle 匹配 kind，图像为 GENERAL；宿主保持句柄至所有引用批次真正完成。
    pub unsafe fn new(context: &Context, handle: u64, kind: ExternalKind) -> Result<Self, Error> {
        context.device.ensure_active()?;
        if handle == 0 || matches!(kind, ExternalKind::StorageBuffer { len: 0 }) {
            return Err(Error::InvalidArgument("外部描述符句柄或范围为空"));
        }
        Ok(Self {
            owner: context.owner(),
            handle,
            kind,
        })
    }
}
/// 单个 binding 资源，slice 下标直接对应 set0 binding 编号。
#[derive(Clone)]
pub enum Binding {
    /// native storage buffer。
    StorageBuffer(Buffer),
    /// native uniform buffer；范围须符合设备 uniform 限制。
    UniformBuffer(Buffer),
    /// wgpu storage buffer。
    SharedBuffer(SharedBuffer),
    /// native GENERAL storage image。
    StorageImage(Image),
    /// wgpu GENERAL storage image。
    SharedTexture(SharedTexture),
    /// native GENERAL sampled image。
    SampledImage(Image),
    /// wgpu SHADER_READ_ONLY_OPTIMAL sampled image。
    SampledTexture(SampledTexture),
    /// native sampler。
    Sampler(Sampler),
    /// 同设备 AS。
    Acceleration(crate::AccelerationStructure),
    /// 宿主资源。
    External(ExternalBinding),
}
impl Binding {
    pub(super) fn stored(&self) -> StoredBinding {
        match self {
            Self::StorageBuffer(v) => StoredBinding::StorageBuffer(*v),
            Self::UniformBuffer(v) => StoredBinding::UniformBuffer(*v),
            Self::SharedBuffer(v) => StoredBinding::SharedBuffer(v.clone()),
            Self::StorageImage(v) => StoredBinding::StorageImage(*v),
            Self::SharedTexture(v) => StoredBinding::SharedTexture(v.clone()),
            Self::SampledImage(v) => StoredBinding::SampledImage(*v),
            Self::SampledTexture(v) => StoredBinding::SampledTexture(v.clone()),
            Self::Sampler(v) => StoredBinding::Sampler(*v),
            Self::Acceleration(v) => StoredBinding::Acceleration(*v),
            Self::External(v) => StoredBinding::External(*v),
        }
    }
    pub(super) fn owner(&self) -> u64 {
        match self {
            Self::StorageBuffer(v) | Self::UniformBuffer(v) => v.key.owner,
            Self::StorageImage(v) | Self::SampledImage(v) => v.key.owner,
            Self::Sampler(v) => v.key.owner,
            Self::Acceleration(v) => v.key.owner,
            Self::SharedBuffer(v) => v.device,
            Self::SharedTexture(v) => v.device,
            Self::SampledTexture(v) => v.device,
            Self::External(v) => v.owner,
        }
    }
    pub(super) fn dependency(&self) -> Option<Key> {
        match self {
            Self::StorageBuffer(v) | Self::UniformBuffer(v) => Some(v.key),
            Self::StorageImage(v) | Self::SampledImage(v) => Some(v.key),
            Self::Sampler(v) => Some(v.key),
            Self::Acceleration(v) => Some(v.key),
            _ => None,
        }
    }
    pub(super) fn descriptor_type(&self) -> vk::DescriptorType {
        match self {
            Self::UniformBuffer(_) => vk::DescriptorType::UNIFORM_BUFFER,
            Self::StorageBuffer(_)
            | Self::SharedBuffer(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::StorageBuffer { .. },
                ..
            }) => vk::DescriptorType::STORAGE_BUFFER,
            Self::StorageImage(_)
            | Self::SharedTexture(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::StorageImage,
                ..
            }) => vk::DescriptorType::STORAGE_IMAGE,
            Self::SampledImage(_)
            | Self::SampledTexture(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::SampledImage,
                ..
            }) => vk::DescriptorType::SAMPLED_IMAGE,
            Self::Sampler(_) => vk::DescriptorType::SAMPLER,
            Self::Acceleration(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::Acceleration,
                ..
            }) => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
        }
    }
}
impl StoredBinding {
    pub(super) fn descriptor_type(&self) -> vk::DescriptorType {
        match self {
            Self::UniformBuffer(_) => vk::DescriptorType::UNIFORM_BUFFER,
            Self::StorageBuffer(_)
            | Self::SharedBuffer(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::StorageBuffer { .. },
                ..
            }) => vk::DescriptorType::STORAGE_BUFFER,
            Self::StorageImage(_)
            | Self::SharedTexture(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::StorageImage,
                ..
            }) => vk::DescriptorType::STORAGE_IMAGE,
            Self::SampledImage(_)
            | Self::SampledTexture(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::SampledImage,
                ..
            }) => vk::DescriptorType::SAMPLED_IMAGE,
            Self::Sampler(_) => vk::DescriptorType::SAMPLER,
            Self::Acceleration(_)
            | Self::External(ExternalBinding {
                kind: ExternalKind::Acceleration,
                ..
            }) => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
        }
    }
}
pub(crate) struct DescriptorSetData {
    pub(super) views: Vec<wgpu::TextureView>,
    pub(super) pool: vk::DescriptorPool,
    pub(super) set: vk::DescriptorSet,
}
pub(crate) struct DescriptorLayoutData {
    pub(super) raw: vk::DescriptorSetLayout,
}
impl Destroy for DescriptorLayoutData {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_descriptor_set_layout(self.raw, None);
        }
    }
}
impl Destroy for DescriptorSetData {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_descriptor_pool(self.pool, None);
        }
    }
}
impl BindingData {
    pub(super) fn compatible(&self, other: &Self) -> bool {
        self.resources().len() == other.resources().len()
            && self
                .resources()
                .iter()
                .zip(other.resources())
                .all(|(a, b)| a.descriptor_type() == b.descriptor_type())
    }
    pub(crate) fn track(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.transition_resources(
            self.resources()
                .iter()
                .filter_map(|resource| match resource {
                    StoredBinding::SharedBuffer(v) => Some(wgpu::BufferTransition {
                        buffer: &v.buffer,
                        state: wgpu::BufferUses::STORAGE_READ_WRITE,
                    }),
                    _ => None,
                }),
            self.resources()
                .iter()
                .filter_map(|resource| match resource {
                    StoredBinding::SharedTexture(v) => Some(wgpu::TextureTransition {
                        texture: &v.texture,
                        selector: None,
                        state: wgpu::TextureUses::STORAGE_READ_WRITE,
                    }),
                    StoredBinding::SampledTexture(v) => Some(wgpu::TextureTransition {
                        texture: &v.texture,
                        selector: None,
                        state: wgpu::TextureUses::RESOURCE,
                    }),
                    _ => None,
                }),
        );
    }
}
impl Bindings {
    pub(super) fn data<'a>(&self, context: &'a Context) -> Result<&'a BindingData, Error> {
        match context.get(self.key)? {
            Data::Bindings(data) => Ok(data),
            _ => Err(Error::InvalidArgument("bindings 标识类型无效")),
        }
    }
    /// 创建连续set0；先验证资源归属及设备阶段/set数量上限，再建立独立descriptor版本。
    /// 每项对compute及已启用RT阶段可见，不依赖shader静态裁剪来规避容量限制。
    pub fn new(context: &mut Context, resources: &[Binding]) -> Result<Self, Error> {
        context.device.ensure_active()?;
        let limits = context.device.physical.properties().limits;
        for (kind, stage, set) in [
            (
                vk::DescriptorType::STORAGE_BUFFER,
                limits.max_per_stage_descriptor_storage_buffers,
                limits.max_descriptor_set_storage_buffers,
            ),
            (
                vk::DescriptorType::UNIFORM_BUFFER,
                limits.max_per_stage_descriptor_uniform_buffers,
                limits.max_descriptor_set_uniform_buffers,
            ),
            (
                vk::DescriptorType::STORAGE_IMAGE,
                limits.max_per_stage_descriptor_storage_images,
                limits.max_descriptor_set_storage_images,
            ),
            (
                vk::DescriptorType::SAMPLED_IMAGE,
                limits.max_per_stage_descriptor_sampled_images,
                limits.max_descriptor_set_sampled_images,
            ),
            (
                vk::DescriptorType::SAMPLER,
                limits.max_per_stage_descriptor_samplers,
                limits.max_descriptor_set_samplers,
            ),
        ] {
            let count = resources
                .iter()
                .filter(|resource| resource.descriptor_type() == kind)
                .count();
            if count > stage.min(set) as usize {
                return Err(Error::Unsupported("描述符数量超过设备阶段或set上限"));
            }
        }
        for resource in resources {
            if resource.owner() != context.owner() {
                return Err(Error::InvalidArgument("描述符资源属于其他设备"));
            }
        }
        for resource in resources {
            if let Some(key) = resource.dependency() {
                context.active(key)?;
            }
        }
        let stages = vk::ShaderStageFlags::COMPUTE
            | if context.device.ray_pipeline {
                vk::ShaderStageFlags::RAYGEN_KHR
                    | vk::ShaderStageFlags::MISS_KHR
                    | vk::ShaderStageFlags::CLOSEST_HIT_KHR
                    | vk::ShaderStageFlags::ANY_HIT_KHR
                    | vk::ShaderStageFlags::CALLABLE_KHR
            } else {
                vk::ShaderStageFlags::empty()
            };
        let entries: Vec<_> = resources
            .iter()
            .enumerate()
            .map(|(i, resource)| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(i as u32)
                    .descriptor_type(resource.descriptor_type())
                    .descriptor_count(1)
                    .stage_flags(stages)
            })
            .collect();

        let raw = unsafe {
            context.device.raw.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&entries),
                None,
            )?
        };
        let layout = context.insert(Data::Layout(DescriptorLayoutData { raw }))?;
        match create_bindings(context, resources, layout)
            .and_then(|data| context.insert(Data::Bindings(data)))
        {
            Ok(key) => Ok(Self { key }),
            Err(error) => {
                context.discard(layout);
                Err(error)
            }
        }
    }
}
pub(super) fn create_bindings(
    context: &Context,
    resources: &[Binding],
    layout: Key,
) -> Result<BindingData, Error> {
    if resources.len() > u32::MAX as usize
        || resources
            .iter()
            .any(|resource| context.owner() != resource.owner())
    {
        return Err(Error::InvalidArgument("描述符数量或设备归属无效"));
    }
    for resource in resources {
        if let Some(key) = resource.dependency() {
            context.get(key)?;
        }
        if let Binding::StorageImage(image) = resource
            && matches!(image.data(context)?.format, ImageFormat::Rgba8Srgb)
        {
            return Err(Error::InvalidArgument("sRGB 图像不支持 storage 绑定"));
        }
    }
    if resources.iter().any(|resource| matches!(resource, Binding::UniformBuffer(buffer) if buffer.len() > u64::from(context.device.physical.properties().limits.max_uniform_buffer_range))) {
            return Err(Error::InvalidArgument("uniform buffer 范围超出设备限制"));
        }
    let raw = &context.device.raw;
    let mut result = Guard::new(
        &context.device,
        DescriptorSetData {
            views: Vec::new(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
        },
    );
    let mut counts = BTreeMap::new();
    for resource in resources {
        *counts
            .entry(resource.descriptor_type().as_raw())
            .or_insert(0u32) += 1;
    }
    let sizes: Vec<_> = counts
        .into_iter()
        .map(|(kind, count)| {
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::from_raw(kind))
                .descriptor_count(count)
        })
        .collect();
    result.pool = unsafe {
        raw.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(&sizes),
            None,
        )?
    };
    result.set = unsafe {
        raw.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(result.pool)
                .set_layouts(&[context.layout(layout)?.raw]),
        )?
    }[0];
    let resources: Vec<_> = resources.iter().map(Binding::stored).collect();
    for (index, resource) in resources.iter().enumerate() {
        let mut write = vk::WriteDescriptorSet::default()
            .dst_set(result.set)
            .dst_binding(index as u32)
            .descriptor_type(resource.descriptor_type())
            .descriptor_count(1);
        let buffer_info;
        let image_info;
        let handles;
        let mut acceleration;
        match resource {
            StoredBinding::StorageBuffer(v) | StoredBinding::UniformBuffer(v) => {
                buffer_info = [vk::DescriptorBufferInfo::default()
                    .buffer(v.data(context)?.raw)
                    .range(v.len())];
                write = write.buffer_info(&buffer_info);
            }
            StoredBinding::SharedBuffer(v) => {
                let hal = unsafe { v.buffer.as_hal::<Vulkan>() }
                    .ok_or(Error::InvalidState("共享 buffer 已销毁"))?;
                buffer_info = [vk::DescriptorBufferInfo::default()
                    .buffer(vk::Buffer::from_raw(unsafe { hal.raw_handle() }.as_raw()))
                    .range(v.len())];
                write = write.buffer_info(&buffer_info);
            }
            StoredBinding::StorageImage(v) | StoredBinding::SampledImage(v) => {
                image_info = [vk::DescriptorImageInfo::default()
                    .image_view(v.data(context)?.view)
                    .image_layout(vk::ImageLayout::GENERAL)];
                write = write.image_info(&image_info);
            }
            StoredBinding::SharedTexture(v) => {
                let view = v.texture.create_view(&Default::default());
                let hal = unsafe { view.as_hal::<Vulkan>() }
                    .ok_or(Error::InvalidState("共享 view 已销毁"))?;
                image_info = [vk::DescriptorImageInfo::default()
                    .image_view(vk::ImageView::from_raw(
                        unsafe { hal.raw_handle() }.as_raw(),
                    ))
                    .image_layout(vk::ImageLayout::GENERAL)];
                drop(hal);
                result.views.push(view);
                write = write.image_info(&image_info);
            }
            StoredBinding::SampledTexture(v) => {
                let hal = unsafe { v.view.as_hal::<Vulkan>() }
                    .ok_or(Error::InvalidState("采样 view 已销毁"))?;
                image_info = [vk::DescriptorImageInfo::default()
                    .image_view(vk::ImageView::from_raw(
                        unsafe { hal.raw_handle() }.as_raw(),
                    ))
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
                write = write.image_info(&image_info);
            }
            StoredBinding::Sampler(v) => {
                image_info = [vk::DescriptorImageInfo::default().sampler(v.data(context)?.raw)];
                write = write.image_info(&image_info);
            }
            StoredBinding::Acceleration(v) => {
                handles = [context.acceleration(*v)?.handle()];
                acceleration = vk::WriteDescriptorSetAccelerationStructureKHR::default()
                    .acceleration_structures(&handles);
                write = write.push(&mut acceleration);
            }
            StoredBinding::External(v) => match v.kind {
                ExternalKind::StorageBuffer { len } => {
                    buffer_info = [vk::DescriptorBufferInfo::default()
                        .buffer(vk::Buffer::from_raw(v.handle))
                        .range(len)];
                    write = write.buffer_info(&buffer_info);
                }
                ExternalKind::StorageImage | ExternalKind::SampledImage => {
                    image_info = [vk::DescriptorImageInfo::default()
                        .image_view(vk::ImageView::from_raw(v.handle))
                        .image_layout(vk::ImageLayout::GENERAL)];
                    write = write.image_info(&image_info);
                }
                ExternalKind::Acceleration => {
                    handles = [vk::AccelerationStructureKHR::from_raw(v.handle)];
                    acceleration = vk::WriteDescriptorSetAccelerationStructureKHR::default()
                        .acceleration_structures(&handles);
                    write = write.push(&mut acceleration);
                }
            },
        }
        unsafe {
            raw.update_descriptor_sets(&[write], &[]);
        }
    }
    Ok(BindingData::new(layout, resources, result.finish()))
}
