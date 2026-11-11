//! 缓存于逻辑设备的无描述符计算管线；每次录制仅借用该批拥有的缓冲地址。
use crate::{
    Device, Error, ShaderModule,
    owner::{Destroy, Guard},
    vk,
};

// 紧凑输入、输出列和身份属于同一份经过容量校验的构建计划。
pub(crate) struct QuadDispatch {
    pub addresses: [u64; 8],
    pub count: u32,
    pub region: [i32; 3],
    pub first_token: u64,
    pub opacity: [u64; 2],
}

pub(crate) struct Expansion {
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
}
impl Expansion {
    pub(crate) fn new(device: &Device, cache: vk::PipelineCache) -> Result<Self, Error> {
        Self::with_shader(
            device,
            cache,
            include_bytes!(concat!(env!("OUT_DIR"), "/expand_quads.spv")),
            c"expand_quads",
            112,
        )
    }
    pub(crate) fn with_shader(
        device: &Device,
        cache: vk::PipelineCache,
        bytes: &[u8],
        entry: &std::ffi::CStr,
        size: u32,
    ) -> Result<Self, Error> {
        let module = ShaderModule::new(device, bytes)?;
        let range = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .size(size)];
        // SAFETY: 着色器和 push constant 来自同批构建的私有 ABI。
        let layout = unsafe {
            device.raw.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&range),
                None,
            )?
        };
        let mut result = Guard::new(
            device,
            Self {
                layout,
                pipeline: vk::Pipeline::null(),
            },
        );
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(module.raw)
            .name(entry);
        let create = vk::ComputePipelineCreateInfo::default()
            .stage(stage)
            .layout(layout);
        result.pipeline =
            match unsafe { device.raw.create_compute_pipelines(cache, &[create], None) } {
                Ok(pipelines) => pipelines[0],
                Err((partial, error)) => {
                    for pipeline in partial {
                        unsafe { device.raw.destroy_pipeline(pipeline, None) };
                    }
                    return Err(error.into());
                }
            };
        Ok(result.finish())
    }
    pub(crate) fn dispatch(
        &self,
        device: &Device,
        command: vk::CommandBuffer,
        push: &[u8],
        groups: u32,
    ) {
        unsafe {
            device
                .raw
                .cmd_bind_pipeline(command, vk::PipelineBindPoint::COMPUTE, self.pipeline);
            device.raw.cmd_push_constants(
                command,
                self.layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                push,
            );
            device.raw.cmd_dispatch(command, groups, 1, 1);
        }
    }

    pub(crate) fn record(
        &self,
        device: &Device,
        command: vk::CommandBuffer,
        parameters: QuadDispatch,
    ) {
        let QuadDispatch {
            addresses,
            count,
            region,
            first_token: id,
            opacity,
        } = parameters;
        let limit = device
            .physical
            .properties()
            .limits
            .max_compute_work_group_count[0];
        // SAFETY: 调用方在此前校验坐标流、偏移及输出容量，完成前保留全部源、目标与本管线。
        unsafe {
            device
                .raw
                .cmd_bind_pipeline(command, vk::PipelineBindPoint::COMPUTE, self.pipeline);
            let mut push = [0u8; 112];
            for (slot, addr) in push[..64].as_chunks_mut::<8>().0.iter_mut().zip(addresses) {
                slot.copy_from_slice(&addr.to_ne_bytes());
            }
            push[68..72].copy_from_slice(&count.to_ne_bytes());
            push[72..80].copy_from_slice(&id.to_ne_bytes());
            for (slot, axis) in push[80..92].as_chunks_mut::<4>().0.iter_mut().zip(region) {
                slot.copy_from_slice(&axis.to_ne_bytes());
            }
            let mask = addresses[3..]
                .iter()
                .enumerate()
                .fold(u32::from(opacity[0] != 0) << 5, |v, (i, &a)| {
                    v | (u32::from(a != 0) << i)
                });
            push[92..96].copy_from_slice(&mask.to_ne_bytes());
            for (dst, addr) in push[96..].as_chunks_mut::<8>().0.iter_mut().zip(opacity) {
                dst.copy_from_slice(&addr.to_ne_bytes());
            }
            let mut first = 0;
            while first < count {
                let groups = (count - first).div_ceil(64).min(limit);
                push[64..68].copy_from_slice(&first.to_ne_bytes());
                device.raw.cmd_push_constants(
                    command,
                    self.layout,
                    vk::ShaderStageFlags::COMPUTE,
                    0,
                    &push,
                );
                device.raw.cmd_dispatch(command, groups, 1, 1);
                first = first.saturating_add(groups.saturating_mul(64));
            }
        }
    }
}
impl Destroy for Expansion {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_pipeline(self.pipeline, None);
            device.raw.destroy_pipeline_layout(self.layout, None);
        }
    }
}
