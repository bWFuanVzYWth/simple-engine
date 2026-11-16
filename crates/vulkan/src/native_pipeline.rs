//! compute/RT管线与SBT准备；运行期只重绑已准备资源。
use super::*;

/// 驱动返回的各RT阶段最大调用栈需求，单位为字节；创建program时查询并缓存。
///
/// 这些值不是寄存器数、实际每ray分配、占用率或GPU内存用量；本类型不修改pipeline栈配置。
/// callable取所有叶子的最大值，不按顺序调用次数相加；无callable时最大值与数量均为0。
/// 来源：[vkGetRayTracingShaderGroupStackSizeKHR](https://docs.vulkan.org/refpages/latest/refpages/source/vkGetRayTracingShaderGroupStackSizeKHR.html)。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RayStackSizes {
    /// Raygen general shader的驱动栈需求。
    pub raygen: u64,
    /// Miss general shader的驱动栈需求。
    pub miss: u64,
    /// 所有三角形hit group中closest-hit shader的最大驱动栈需求。
    pub closest_hit: u64,
    /// 存在any-hit shader时的驱动栈需求；None表示未链接该阶段。
    pub any_hit: Option<u64>,
    /// 所有callable general shader的最大驱动栈需求。
    pub callable_max: u64,
    /// 参与最大值查询的callable数量。
    pub callable_count: u32,
}
pub(crate) struct RayPipelineData {
    pub(super) layout: vk::PipelineLayout,
    pub(super) pipeline: vk::Pipeline,
    pub(super) sbt: Option<buffer::Buffer>,
    pub(super) regions: [vk::StridedDeviceAddressRegionKHR; 4],
    pub(super) loader: khr::ray_tracing_pipeline::Device,
    pub(super) max_dispatch: u32,
    pub(super) stack_sizes: RayStackSizes,
}
impl Destroy for RayPipelineData {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_pipeline(self.pipeline, None);
            device.raw.destroy_pipeline_layout(self.layout, None);
        }
        if let Some(sbt) = &mut self.sbt {
            sbt.destroy(device);
        }
    }
}
impl RayTracingPipeline {
    /// 查询驱动公开的可执行统计；字段和执行单元划分由驱动定义，可能为空。
    ///
    /// 创建 Context 时须显式启用 [`Capabilities::pipeline_statistics`]；重绑定复用
    /// 同一已捕获 program。不会执行或等待 GPU；返回值不借用管线。
    /// # Errors
    /// 未启用能力、标识已回收或不属于此 Context、Vulkan 查询失败时返回错误。
    pub fn executable_statistics(
        &self,
        context: &Context,
    ) -> Result<Vec<ExecutableStatistics>, Error> {
        let program = self.data(context)?.program();
        pipeline_statistics::query(context, context.ray_program(program)?.raw().pipeline)
    }
    /// 返回创建program时缓存的驱动栈需求；descriptor重绑定复用同一份数值。
    /// 标识不属于此Context或资源已回收时返回错误；此查询不录制命令，也不设置dynamic stack。
    pub fn stack_sizes(&self, context: &Context) -> Result<RayStackSizes, Error> {
        let program = self.data(context)?.program();
        Ok(context.ray_program(program)?.raw().stack_sizes)
    }

    /// 不可变program与SBT的资源身份；重绑descriptor时保持相同，可用于复用诊断或保活。
    pub fn program_root(&self, context: &Context) -> Result<Resource, Error> {
        Ok(Resource(self.data(context)?.program()))
    }

    pub(super) fn data<'a>(&self, context: &'a Context) -> Result<&'a RayVersion, Error> {
        match context.get(self.key)? {
            Data::Ray(data) => Ok(data),
            _ => Err(Error::InvalidArgument("RT pipeline 标识类型无效")),
        }
    }
    /// 创建单层 RT pipeline；shader 的 payload/调用图契约由受控构建保证。
    pub fn new(
        context: &mut Context,
        bindings: Bindings,
        shaders: [RayShader<'_>; 3],
        any_hit: Option<RayShader<'_>>,
        callables: &[RayShader<'_>],
    ) -> Result<Self, Error> {
        let [raygen, miss, closest_hit] = shaders;
        Self::with_hit_groups(
            context,
            bindings,
            raygen,
            miss,
            &[closest_hit],
            any_hit,
            callables,
        )
    }

    /// 创建单层 RT pipeline，按切片顺序建立三角形 Hit Group 的 SBT 记录。
    ///
    /// `closest_hits` 至少含一项；所有记录共享可选覆盖率过滤。普通追踪及
    /// HitObject 的表索引均相对此切片，callable 索引独立从零开始。
    /// shader 的 payload、索引范围及无环调用图由受控构建保证；本函数不保留借用。
    ///
    /// # Errors
    /// 空命中表、数量或布局溢出、设备不支持及 Vulkan 创建失败时返回错误。
    pub fn with_hit_groups(
        context: &mut Context,
        bindings: Bindings,
        raygen: RayShader<'_>,
        miss: RayShader<'_>,
        closest_hits: &[RayShader<'_>],
        any_hit: Option<RayShader<'_>>,
        callables: &[RayShader<'_>],
    ) -> Result<Self, Error> {
        context.active(bindings.key)?;
        let data = create_ray_program(
            context,
            bindings,
            raygen,
            miss,
            closest_hits,
            any_hit,
            callables,
        )?;
        let program = context.insert(Data::RayProgram(data))?;
        match context.insert(Data::Ray(RayVersion::new(bindings, program))) {
            Ok(key) => Ok(Self { key }),
            Err(error) => {
                context.discard(program);
                Err(error)
            }
        }
    }
    /// 复用 program/SBT，仅创建绑定版本；旧命令依赖不变。
    pub fn rebind(&self, context: &mut Context, bindings: Bindings) -> Result<Self, Error> {
        context.active(self.key)?;
        context.active(bindings.key)?;
        let old = self.data(context)?;
        if !old
            .bindings()
            .data(context)?
            .compatible(bindings.data(context)?)
        {
            return Err(Error::InvalidArgument("RT 重绑定 layout 不兼容"));
        }
        let program = old.program();
        Ok(Self {
            key: context.insert(Data::Ray(RayVersion::new(bindings, program)))?,
        })
    }
}
pub(crate) struct ComputePipelineData {
    pub(super) layout: vk::PipelineLayout,
    pub(super) pipeline: vk::Pipeline,
}
impl Destroy for ComputePipelineData {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_pipeline(self.pipeline, None);
            device.raw.destroy_pipeline_layout(self.layout, None);
        }
    }
}
impl ComputePipeline {
    /// 不可变compute program的资源身份；重绑descriptor时保持相同。
    pub fn program_root(&self, context: &Context) -> Result<Resource, Error> {
        Ok(Resource(self.data(context)?.program()))
    }

    pub(super) fn data<'a>(&self, context: &'a Context) -> Result<&'a ComputeVersion, Error> {
        match context.get(self.key)? {
            Data::Compute(data) => Ok(data),
            _ => Err(Error::InvalidArgument("compute pipeline 标识类型无效")),
        }
    }
    /// 创建直接消费 SPIR-V 的 compute program。
    pub fn new(
        context: &mut Context,
        bindings: Bindings,
        shader: RayShader<'_>,
    ) -> Result<Self, Error> {
        Self::create(context, bindings, shader, None)
    }

    /// 创建固定 subgroup 规模的 compute program，不依赖设备默认规模。
    ///
    /// # Errors
    /// size 非 2 幂、超出范围，或宿主未启用 compute 规模控制时返回错误；其余同 [`Self::new`]。
    pub fn with_subgroup_size(
        context: &mut Context,
        bindings: Bindings,
        shader: RayShader<'_>,
        size: u32,
    ) -> Result<Self, Error> {
        let properties = context.subgroup_properties();
        if !size.is_power_of_two()
            || !properties.compute_size_control
            || size < properties.min_size
            || size > properties.max_size
        {
            return Err(Error::Unsupported("compute subgroup 规模不受设备支持"));
        }
        Self::create(context, bindings, shader, Some(size))
    }

    pub(super) fn create(
        context: &mut Context,
        bindings: Bindings,
        shader: RayShader<'_>,
        subgroup_size: Option<u32>,
    ) -> Result<Self, Error> {
        context.active(bindings.key)?;
        let data = create_compute_program(context, bindings, shader, subgroup_size)?;
        let program = context.insert(Data::ComputeProgram(data))?;
        match context.insert(Data::Compute(ComputeVersion::new(bindings, program))) {
            Ok(key) => Ok(Self { key }),
            Err(error) => {
                context.discard(program);
                Err(error)
            }
        }
    }
    /// 复用 program，仅创建不可变 descriptor 版本。
    pub fn rebind(&self, context: &mut Context, bindings: Bindings) -> Result<Self, Error> {
        context.active(self.key)?;
        context.active(bindings.key)?;
        let old = self.data(context)?;
        if !old
            .bindings()
            .data(context)?
            .compatible(bindings.data(context)?)
        {
            return Err(Error::InvalidArgument("compute 重绑定 layout 不兼容"));
        }
        let program = old.program();
        Ok(Self {
            key: context.insert(Data::Compute(ComputeVersion::new(bindings, program)))?,
        })
    }
}
pub(super) fn create_ray_program(
    context: &Context,
    bindings: Bindings,
    raygen: RayShader<'_>,
    miss: RayShader<'_>,
    closest_hits: &[RayShader<'_>],
    any_hit: Option<RayShader<'_>>,
    callables: &[RayShader<'_>],
) -> Result<RayProgram, Error> {
    let bindings = bindings.data(context)?;
    let set_layout = bindings.layout();
    if closest_hits.is_empty() {
        return Err(Error::InvalidArgument("RT 命中表不能为空"));
    }
    let callable_group_base = closest_hits
        .len()
        .checked_add(2)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or(Error::InvalidArgument("RT hit group 数量溢出"))?;
    let group_count = callables
        .len()
        .checked_add(callable_group_base as usize)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or(Error::InvalidArgument("RT shader group 数量溢出"))?;
    if any_hit.is_some() && group_count == u32::MAX {
        return Err(Error::InvalidArgument("RT shader stage 数量溢出"));
    }
    if !context.device.ray_pipeline {
        return Err(Error::InvalidArgument("RT 能力或绑定设备无效"));
    }
    let loader = khr::ray_tracing_pipeline::Device::load(
        &context.device.physical.instance.raw,
        &context.device.raw,
    );
    let mut result = Guard::new(
        &context.device,
        RayPipelineData {
            layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            sbt: None,
            regions: Default::default(),
            loader,
            max_dispatch: 0,
            stack_sizes: RayStackSizes::default(),
        },
    );
    result.layout = unsafe {
        context.device.raw.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&[context.layout(bindings.layout())?.raw]),
            None,
        )?
    };
    let mut kinds = vec![
        vk::ShaderStageFlags::RAYGEN_KHR,
        vk::ShaderStageFlags::MISS_KHR,
    ];
    kinds.extend(std::iter::repeat_n(
        vk::ShaderStageFlags::CLOSEST_HIT_KHR,
        closest_hits.len(),
    ));
    let mut sources = vec![&raygen, &miss];
    sources.extend(closest_hits);
    let any_hit_stage = any_hit.as_ref().map(|_| sources.len() as u32);
    if let Some(ref shader) = any_hit {
        sources.push(shader);
        kinds.push(vk::ShaderStageFlags::ANY_HIT_KHR);
    }
    let callable_base = sources.len() as u32;
    sources.extend(callables);
    kinds.extend(std::iter::repeat_n(
        vk::ShaderStageFlags::CALLABLE_KHR,
        callables.len(),
    ));
    let modules: Vec<_> = sources
        .iter()
        .map(|shader| ShaderModule::new(&context.device, shader.spirv))
        .collect::<Result<_, _>>()?;
    let stages: Vec<_> = sources
        .iter()
        .zip(&modules)
        .zip(kinds)
        .map(|((shader, module), stage)| {
            vk::PipelineShaderStageCreateInfo::default()
                .stage(stage)
                .module(module.raw)
                .name(shader.entry)
        })
        .collect();
    let blank = vk::RayTracingShaderGroupCreateInfoKHR::default()
        .general_shader(vk::SHADER_UNUSED_KHR)
        .closest_hit_shader(vk::SHADER_UNUSED_KHR)
        .any_hit_shader(vk::SHADER_UNUSED_KHR)
        .intersection_shader(vk::SHADER_UNUSED_KHR);
    let mut groups = vec![
        blank
            .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
            .general_shader(0),
        blank
            .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
            .general_shader(1),
    ];
    groups.extend((0..closest_hits.len()).map(|i| {
        blank
            .ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
            .closest_hit_shader(2 + i as u32)
            .any_hit_shader(any_hit_stage.unwrap_or(vk::SHADER_UNUSED_KHR))
    }));
    groups.extend((0..callables.len()).map(|i| {
        blank
            .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
            .general_shader(callable_base + i as u32)
    }));
    let mut flags = if context.device.opacity_micromap {
        vk::PipelineCreateFlags::RAY_TRACING_OPACITY_MICROMAP_KHR
    } else {
        vk::PipelineCreateFlags::empty()
    };
    if context.device.pipeline_statistics {
        flags |= vk::PipelineCreateFlags::CAPTURE_STATISTICS_KHR;
    }
    let mut robustness = vk::PipelineRobustnessCreateInfo::default()
        .storage_buffers(vk::PipelineRobustnessBufferBehavior::DISABLED)
        .uniform_buffers(vk::PipelineRobustnessBufferBehavior::DISABLED)
        .vertex_inputs(vk::PipelineRobustnessBufferBehavior::DISABLED)
        .images(vk::PipelineRobustnessImageBehavior::DISABLED);
    let mut create = vk::RayTracingPipelineCreateInfoKHR::default()
        .flags(flags)
        .stages(&stages)
        .groups(&groups)
        .max_pipeline_ray_recursion_depth(1)
        .layout(result.layout);
    if context.device.pipeline_robustness {
        create = create.push(&mut robustness);
    }
    result.pipeline = match unsafe {
        result.loader.create_ray_tracing_pipelines(
            vk::DeferredOperationKHR::null(),
            context.pipeline_cache(),
            &[create],
            None,
        )
    } {
        Ok(pipelines) => pipelines[0],
        Err((pipelines, error)) => {
            for pipeline in pipelines {
                unsafe {
                    context.device.raw.destroy_pipeline(pipeline, None);
                }
            }
            return Err(error.into());
        }
    };
    // SAFETY: 同一Device成功创建的pipeline；group0/1为general，其后三角形组均含
    // closest-hit及同一个可选any-hit。callable区间独立于shader stage编号。
    result.stack_sizes = unsafe {
        let stack = |group, shader| {
            result
                .loader
                .get_ray_tracing_shader_group_stack_size(result.pipeline, group, shader)
        };
        RayStackSizes {
            raygen: stack(0, vk::ShaderGroupShaderKHR::GENERAL),
            miss: stack(1, vk::ShaderGroupShaderKHR::GENERAL),
            closest_hit: (2..callable_group_base)
                .map(|group| stack(group, vk::ShaderGroupShaderKHR::CLOSEST_HIT))
                .max()
                .unwrap_or(0),
            any_hit: any_hit
                .is_some()
                .then(|| stack(2, vk::ShaderGroupShaderKHR::ANY_HIT)),
            callable_max: (callable_group_base..group_count)
                .map(|group| stack(group, vk::ShaderGroupShaderKHR::GENERAL))
                .max()
                .unwrap_or(0),
            callable_count: group_count - callable_group_base,
        }
    };
    drop(modules);
    let mut properties = vk::PhysicalDeviceRayTracingPipelinePropertiesKHR::default();
    let mut physical = vk::PhysicalDeviceProperties2::default().push(&mut properties);
    unsafe {
        context
            .device
            .physical
            .instance
            .raw
            .get_physical_device_properties2(context.device.physical.raw, &mut physical);
    }
    result.max_dispatch = properties.max_ray_dispatch_invocation_count;
    let handle_size = properties.shader_group_handle_size as usize;
    let stride = handle_size
        .checked_next_multiple_of(properties.shader_group_handle_alignment as usize)
        .ok_or(Error::InvalidArgument("SBT stride 溢出"))?;
    if stride > properties.max_shader_group_stride as usize {
        return Err(Error::InvalidArgument("SBT stride 超限"));
    }
    let spacing = stride
        .checked_next_multiple_of(properties.shader_group_base_alignment as usize)
        .ok_or(Error::InvalidArgument("SBT 对齐溢出"))?;
    let callable_size = stride
        .checked_mul(callables.len())
        .ok_or(Error::InvalidArgument("callable SBT 大小溢出"))?;
    let hit_size = stride
        .checked_mul(closest_hits.len())
        .ok_or(Error::InvalidArgument("hit SBT 大小溢出"))?;
    let callable_offset = spacing
        .checked_mul(2)
        .and_then(|offset| offset.checked_add(hit_size))
        .and_then(|offset| {
            offset.checked_next_multiple_of(properties.shader_group_base_alignment as usize)
        })
        .ok_or(Error::InvalidArgument("SBT 对齐溢出"))?;
    let byte_count = callable_offset
        .checked_add(callable_size)
        .ok_or(Error::InvalidArgument("SBT 大小溢出"))?;
    let handles = unsafe {
        result.loader.get_ray_tracing_shader_group_handles(
            result.pipeline,
            0,
            group_count,
            handle_size
                .checked_mul(groups.len())
                .ok_or(Error::InvalidArgument("SBT handle 溢出"))?,
        )?
    };
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(byte_count)?;
    bytes.resize(byte_count, 0);
    for i in 0..groups.len() {
        let offset = if i < 2 {
            i * spacing
        } else if i < callable_group_base as usize {
            spacing * 2 + (i - 2) * stride
        } else {
            callable_offset + (i - callable_group_base as usize) * stride
        };
        bytes[offset..offset + handle_size]
            .copy_from_slice(&handles[i * handle_size..(i + 1) * handle_size]);
    }
    let (sbt, address) = buffer::Buffer::upload(
        &context.device,
        &bytes,
        vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        properties.shader_group_base_alignment as usize,
    )?;
    result.regions = std::array::from_fn(|i| {
        if i == 3 && callables.is_empty() {
            Default::default()
        } else {
            vk::StridedDeviceAddressRegionKHR {
                device_address: address + if i == 3 { callable_offset } else { i * spacing } as u64,
                stride: stride as u64,
                size: match i {
                    2 => hit_size,
                    3 => callable_size,
                    _ => stride,
                } as u64,
            }
        }
    });
    result.sbt = Some(sbt);
    Ok(RayProgram::new(set_layout, result.finish()))
}
pub(super) fn create_compute_program(
    context: &Context,
    bindings: Bindings,
    shader: RayShader<'_>,
    subgroup_size: Option<u32>,
) -> Result<ComputeProgram, Error> {
    let bindings = bindings.data(context)?;
    let set_layout = bindings.layout();
    let mut result = Guard::new(
        &context.device,
        ComputePipelineData {
            layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
        },
    );
    result.layout = unsafe {
        context.device.raw.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&[context.layout(bindings.layout())?.raw]),
            None,
        )?
    };
    let module = ShaderModule::new(&context.device, shader.spirv)?;
    let mut size = vk::PipelineShaderStageRequiredSubgroupSizeCreateInfo::default()
        .required_subgroup_size(subgroup_size.unwrap_or(0));
    let mut stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(module.raw)
        .name(shader.entry);
    if subgroup_size.is_some() {
        stage = stage.push(&mut size);
    }
    let create = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(result.layout);
    result.pipeline = match unsafe {
        context
            .device
            .raw
            .create_compute_pipelines(context.pipeline_cache(), &[create], None)
    } {
        Ok(pipelines) => pipelines[0],
        Err((pipelines, error)) => {
            for pipeline in pipelines {
                unsafe {
                    context.device.raw.destroy_pipeline(pipeline, None);
                }
            }
            return Err(error.into());
        }
    };
    drop(module);
    Ok(ComputeProgram::new(set_layout, result.finish()))
}
