//! AS 录制的临时分配由提交记录接管，完成或取消后才归池。

use super::*;

#[derive(Default)]
pub(crate) struct BuildBuffers {
    pub(crate) input: Option<Buffer>,
    pub(crate) scratch: Option<Buffer>,
}

impl BuildBuffers {
    pub(crate) fn memory(&self) -> (u64, usize) {
        [&self.input, &self.scratch]
            .into_iter()
            .filter_map(|buffer| buffer.as_ref())
            .fold((0, 0), |(bytes, count), buffer| {
                (bytes + buffer.allocation_bytes(), count + 1)
            })
    }

    pub(crate) fn recycle(mut self, device: &Device, pool: &mut BufferPool) {
        if let Some(input) = self.input.take() {
            pool.give(device, Class::Input, input);
        }
        if let Some(scratch) = self.scratch.take() {
            pool.give(device, Class::Scratch, scratch);
        }
    }
}

#[cfg(test)]
#[path = "acceleration_record_tests.rs"]
mod tests;

struct Resources<'a> {
    device: &'a Device,
    pool: &'a mut BufferPool,
    buffers: BuildBuffers,
    structure: Option<Structure>,
}

impl Drop for Resources<'_> {
    fn drop(&mut self) {
        if let Some(structure) = self.structure.take() {
            structure.recycle(self.device, self.pool);
        }
        std::mem::take(&mut self.buffers).recycle(self.device, self.pool);
    }
}

impl AccelerationData {
    pub(crate) fn record_instances(
        device: &Device,
        instances: &[(TriangleBlas, ResolvedInstance<'_>)],
        pool: &mut BufferPool,
    ) -> Result<(Self, wgpu::CommandBuffer, BuildBuffers), Error> {
        Self::record_instances_with(device, instances, pool, |encode| {
            crate::interop::record(device, "TLAS build", encode)
        })
    }

    pub(crate) fn record_instances_raw(
        device: &Device,
        instances: &[(TriangleBlas, ResolvedInstance<'_>)],
        pool: &mut BufferPool,
        command: vk::CommandBuffer,
    ) -> Result<(Self, BuildBuffers), Error> {
        Self::record_instances_with(device, instances, pool, |encode| encode(command))
            .map(|(data, (), buffers)| (data, buffers))
    }

    fn record_instances_with<C>(
        device: &Device,
        instances: &[(TriangleBlas, ResolvedInstance<'_>)],
        pool: &mut BufferPool,
        record: impl FnOnce(&dyn Fn(vk::CommandBuffer) -> Result<(), Error>) -> Result<C, Error>,
    ) -> Result<(Self, C, BuildBuffers), Error> {
        let properties = properties(device)?;
        let count =
            u32::try_from(instances.len()).map_err(|_| Error::InvalidArgument("实例数量过多"))?;
        if u64::from(count) > properties.max_instance_count {
            return Err(Error::InvalidArgument("实例数量超过设备限制"));
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(
            instances
                .len()
                .max(1)
                .checked_mul(64)
                .ok_or(Error::InvalidArgument("实例字节数溢出"))?,
        )?;
        let mut bottoms = Vec::new();
        bottoms.try_reserve_exact(instances.len())?;
        for (id, instance) in instances {
            bytes.extend_from_slice(&instance_bytes(instance)?);
            bottoms.push(*id);
        }
        if bytes.is_empty() {
            bytes.resize(64, 0);
        }
        let mut resources = Resources {
            device,
            pool,
            buffers: BuildBuffers::default(),
            structure: None,
        };
        resources.buffers.input = Some(
            resources.pool.take(
                device,
                Class::Input,
                bytes
                    .len()
                    .checked_add(15)
                    .ok_or(Error::InvalidArgument("实例输入对齐溢出"))?,
            )?,
        );
        let input = resources.buffers.input.as_ref().expect("已取得实例输入");
        let address = input.address(device);
        let padding = ((16 - address % 16) % 16) as usize;
        input.write_slices(device, &[(padding, &bytes)])?;
        let data = vk::AccelerationStructureGeometryInstancesDataKHR::default().data(
            vk::DeviceOrHostAddressConstKHR {
                device_address: address + padding as u64,
            },
        );
        let geometries = [vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::INSTANCES)
            .geometry(vk::AccelerationStructureGeometryDataKHR { instances: data })];
        let mut build = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&geometries);
        let loader =
            khr::acceleration_structure::Device::load(&device.physical.instance.raw, &device.raw);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            loader.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &build,
                Some(&[count]),
                &mut sizes,
            );
        }
        resources.structure = Some(Structure::allocate_pooled(
            device,
            resources.pool,
            vk::AccelerationStructureTypeKHR::TOP_LEVEL,
            sizes.acceleration_structure_size,
        )?);
        let alignment = u64::from(properties.min_acceleration_structure_scratch_offset_alignment);
        if !alignment.is_power_of_two() {
            return Err(Error::InvalidArgument("AS scratch 对齐无效"));
        }
        let scratch_len = sizes
            .build_scratch_size
            .max(1)
            .checked_add(alignment - 1)
            .ok_or(Error::InvalidArgument("AS scratch 大小溢出"))?;
        resources.buffers.scratch = Some(
            resources.pool.take(
                device,
                Class::Scratch,
                usize::try_from(scratch_len)
                    .map_err(|_| Error::InvalidArgument("AS scratch 超出地址范围"))?,
            )?,
        );
        let scratch = resources
            .buffers
            .scratch
            .as_ref()
            .expect("已取得实例 scratch");
        let scratch_address = (scratch.address(device) + alignment - 1) & !(alignment - 1);
        build = build
            .dst_acceleration_structure(resources.structure.as_ref().expect("已取得 TLAS").raw)
            .scratch_data(vk::DeviceOrHostAddressKHR {
                device_address: scratch_address,
            });
        let ranges = [vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(count)];
        let command = record(&|command| {
            unsafe {
                device.raw.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::HOST,
                    vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                    vk::DependencyFlags::empty(),
                    &[vk::MemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::HOST_WRITE)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ)],
                    &[],
                    &[],
                );
                loader.cmd_build_acceleration_structures(command, &[build], &[Some(&ranges)]);
                device.raw.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR
                        | vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[vk::MemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                        .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR)],
                    &[],
                    &[],
                );
            }
            Ok(())
        })?;
        let output = Self::new(
            bottoms,
            resources.structure.take().expect("录制成功的 TLAS"),
        );
        let buffers = std::mem::take(&mut resources.buffers);
        Ok((output, command, buffers))
    }
}
