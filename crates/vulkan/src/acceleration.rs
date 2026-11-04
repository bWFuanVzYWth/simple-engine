use crate::buffer_pool::{BufferPool, Class};
use crate::{Device, Error, OpacityMicromap, buffer::Buffer, khr, vk};
use crate::{
    micromap::MicromapData,
    owner::{Destroy, Guard},
};
use ash::vk::TaggedStructure as _;

#[path = "acceleration_batch.rs"]
mod batch;
#[cfg(test)]
#[path = "acceleration_validation.rs"]
mod validation;
pub(crate) use batch::{BatchMode, ResolvedBlas};
pub use batch::{BuildTiming, TriangleBlasBatch, TriangleBlasInput, TriangleSceneBatch};

#[path = "acceleration_record.rs"]
mod recorded;
pub(crate) use recorded::BuildBuffers;

#[doc(inline)]
pub use crate::handles::{AccelerationStructure, TriangleBlas};
pub(crate) struct ResolvedInstance<'a> {
    pub(crate) geometry: &'a BlasData,
    pub(crate) transform: [[f32; 4]; 3],
    pub(crate) custom_index: u32,
}
impl Destroy for BlasData {
    fn destroy(&mut self, device: &Device) {
        self.raw_mut().destroy(device);
    }
}
impl Destroy for AccelerationData {
    fn destroy(&mut self, device: &Device) {
        self.raw_mut().destroy(device);
    }
}

/// 可复用的三角形 BLAS；默认 opaque，可附精确 2-state OMM。
/// 设备启用 position fetch 时构建带 `ALLOW_DATA_ACCESS`，hit shader 可从 AS 读取 object-space 顶点。
/// 多份 TLAS 以 ID 复用它；Context 在撤销发布根并确认已有使用结束后退役。
/// <https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_ray_tracing_position_fetch.html>
pub(crate) type BlasData = crate::lifecycle::resource::Blas<Structure>;

/// TLAS 的一个三角形实例；矩阵为行主序 3×4 affine object-to-world。
#[derive(Clone)]
pub struct TriangleInstance<G = TriangleBlas> {
    /// 同设备 BLAS 标识；合并构建允许改为同批的 [`BatchBlas`]。
    pub geometry: G,
    /// 有限且可稳定判定为可逆的 affine 变换；拒绝近奇异矩阵，不包含第四行 `[0, 0, 0, 1]`。
    pub transform: [[f32; 4]; 3],
    /// shader `InstanceID()` 可读的 24-bit 应用编号，不是 BLAS primitive index。
    pub custom_index: u32,
}

/// 合并构建中实例引用的 BLAS，交由 Context 保活。
#[derive(Clone, Copy)]
pub enum BatchBlas {
    /// 已提交的同设备 BLAS。
    Existing(TriangleBlas),
    /// 同批新构建的 BLAS，索引以输入顺序从零开始。
    New(usize),
}

/// 三角形实例 TLAS 及共享 BLAS/OMM；mask 为 0xff，禁用面剔除。
/// <https://docs.vulkan.org/spec/latest/chapters/accelstructures.html>
pub(crate) type AccelerationData = crate::lifecycle::resource::Acceleration<Structure>;

fn properties(
    device: &Device,
) -> Result<vk::PhysicalDeviceAccelerationStructurePropertiesKHR<'static>, Error> {
    device.ensure_active()?;
    if !device.ray_pipeline {
        return Err(Error::Unsupported("设备未启用光追管线"));
    }
    let mut properties = vk::PhysicalDeviceAccelerationStructurePropertiesKHR::default();
    let mut physical = vk::PhysicalDeviceProperties2::default().push(&mut properties);
    unsafe {
        device
            .physical
            .instance
            .raw
            .get_physical_device_properties2(device.physical.raw, &mut physical)
    };
    Ok(properties)
}

impl BlasData {
    pub(crate) fn recycle(self, device: &Device, pool: &mut BufferPool) {
        self.into_raw().recycle(device, pool);
    }

    pub(crate) fn native(
        device: &Device,
        triangles: &[[[f32; 3]; 3]],
        opaque: bool,
    ) -> Result<Self, Error> {
        Self::build(device, triangles, None, &[], opaque)
    }

    pub(crate) fn with_opacity(
        device: &Device,
        triangles: &[[[f32; 3]; 3]],
        micromap: (&MicromapData, OpacityMicromap),
        indices: &[u32],
    ) -> Result<Self, Error> {
        Self::build(device, triangles, Some(micromap), indices, true)
    }

    fn build(
        device: &Device,
        triangles: &[[[f32; 3]; 3]],
        micromap: Option<(&MicromapData, OpacityMicromap)>,
        indices: &[u32],
        opaque: bool,
    ) -> Result<Self, Error> {
        let properties = properties(device)?;
        let count =
            u32::try_from(triangles.len()).map_err(|_| Error::InvalidArgument("三角形数量过多"))?;
        let vertices = count
            .checked_mul(3)
            .ok_or(Error::InvalidArgument("顶点数量过多"))?;
        if count == 0
            || u64::from(count) > properties.max_primitive_count
            || triangles.iter().flatten().flatten().any(|v| !v.is_finite())
        {
            return Err(Error::InvalidArgument("BLAS 为空、坐标非有限或数量超限"));
        }
        if !device
            .format_properties(vk::Format::R32G32B32_SFLOAT)
            .buffer_features
            .contains(vk::FormatFeatureFlags::ACCELERATION_STRUCTURE_VERTEX_BUFFER_KHR)
        {
            return Err(Error::Unsupported("设备不支持 float3 AS 顶点"));
        }
        if let Some((map, _)) = micromap {
            map.validate_indices(indices, triangles.len())?;
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(
            triangles
                .len()
                .checked_mul(36)
                .ok_or(Error::InvalidArgument("顶点字节数溢出"))?,
        )?;
        for value in triangles.iter().flatten().flatten() {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        let usage = vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
        let (_input, address) = Buffer::upload(device, &bytes, usage, 4)?;
        let _input = Guard::new(device, _input);
        let opacity_input = if micromap.is_some() {
            let mut index_bytes = Vec::new();
            index_bytes.try_reserve_exact(
                indices
                    .len()
                    .checked_mul(4)
                    .ok_or(Error::InvalidArgument("OMM index 大小溢出"))?,
            )?;
            for index in indices {
                index_bytes.extend_from_slice(&index.to_ne_bytes());
            }
            let (buffer, address) = Buffer::upload(device, &index_bytes, usage, 4)?;
            Some((Guard::new(device, buffer), address))
        } else {
            None
        };
        let mut opacity = vk::AccelerationStructureTrianglesOpacityMicromapKHR::default();
        let mut data = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
            .vertex_format(vk::Format::R32G32B32_SFLOAT)
            .vertex_data(vk::DeviceOrHostAddressConstKHR {
                device_address: address,
            })
            .vertex_stride(12)
            .max_vertex(vertices - 1)
            .index_type(vk::IndexType::NONE_KHR);
        if let (Some(micromap), Some((_, address))) = (&micromap, &opacity_input) {
            opacity = opacity
                .index_type(vk::IndexType::UINT32)
                .index_buffer(*address)
                .index_stride(4)
                .micromap(micromap.0.raw());
            data = data.push(&mut opacity);
        }
        let geometry = vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
            .geometry(vk::AccelerationStructureGeometryDataKHR { triangles: data })
            .flags(if opaque {
                vk::GeometryFlagsKHR::OPAQUE
            } else {
                vk::GeometryFlagsKHR::empty()
            });
        let bottom = Structure::build(
            device,
            vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL,
            geometry,
            count,
            properties.min_acceleration_structure_scratch_offset_alignment,
        )?;
        Ok(Self::new(micromap.map(|(_, id)| id), bottom))
    }
}

fn validate_instance(m: &[[f32; 4]; 3], custom_index: u32) -> Result<(), Error> {
    let m = m.map(|row| row.map(f64::from));
    let terms = [
        m[0][0] * m[1][1] * m[2][2],
        m[0][0] * m[1][2] * m[2][1],
        m[0][1] * m[1][0] * m[2][2],
        m[0][1] * m[1][2] * m[2][0],
        m[0][2] * m[1][0] * m[2][1],
        m[0][2] * m[1][1] * m[2][0],
    ];
    let determinant = terms[0] - terms[1] - terms[2] + terms[3] + terms[4] - terms[5];
    // f32 输入乘三次再消去可能在 f64 中留下假非零；保守覆盖乘加舍入误差。
    let error = 16.0 * f64::EPSILON * terms.into_iter().map(f64::abs).sum::<f64>();
    if custom_index > 0x00ff_ffff
        || m.iter().flatten().any(|v| !v.is_finite())
        || !determinant.is_finite()
        || determinant.abs() <= error
    {
        return Err(Error::InvalidArgument("TLAS 实例编号或 affine 变换无效"));
    }
    Ok(())
}

fn instance_bytes(instance: &ResolvedInstance<'_>) -> Result<[u8; 64], Error> {
    let m = &instance.transform;
    validate_instance(m, instance.custom_index)?;
    let mut bytes = [0; 64];
    for (word, value) in bytes[..48]
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(m.iter().flatten())
    {
        word.copy_from_slice(&value.to_ne_bytes());
    }
    bytes[48..52].copy_from_slice(&(instance.custom_index | 0xff00_0000).to_ne_bytes());
    bytes[52..56].copy_from_slice(
        &(vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE.as_raw() << 24).to_ne_bytes(),
    );
    bytes[56..].copy_from_slice(&instance.geometry.raw().address().to_ne_bytes());
    Ok(bytes)
}

impl AccelerationData {
    pub(crate) fn recycle(self, device: &Device, pool: &mut BufferPool) {
        self.into_raw().recycle(device, pool);
    }

    /// 同步构建；primitive index 保持输入顺序，空输入生成零实例 TLAS。
    pub(crate) fn native_instances<'a>(
        device: &Device,
        instances: &[TriangleInstance],
        resolve: impl Fn(TriangleBlas) -> Result<&'a BlasData, Error>,
    ) -> Result<Self, Error> {
        Self::instances_target(device, instances, &resolve)
    }

    fn instances_target<'a>(
        device: &Device,
        instances: &[TriangleInstance],
        resolve: &impl Fn(TriangleBlas) -> Result<&'a BlasData, Error>,
    ) -> Result<Self, Error> {
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
        for instance in instances {
            bytes.extend_from_slice(&instance_bytes(&ResolvedInstance {
                geometry: resolve(instance.geometry)?,
                transform: instance.transform,
                custom_index: instance.custom_index,
            })?);
            bottoms.push(instance.geometry);
        }
        // 空 TLAS 仍使用有效地址；primitiveCount=0 不读取占位字节。
        if bytes.is_empty() {
            bytes.resize(64, 0);
        }
        let usage = vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
        let (_input, address) = Buffer::upload(device, &bytes, usage, 16)?;
        let _input = Guard::new(device, _input);
        let data = vk::AccelerationStructureGeometryInstancesDataKHR::default().data(
            vk::DeviceOrHostAddressConstKHR {
                device_address: address,
            },
        );
        let geometry = vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::INSTANCES)
            .geometry(vk::AccelerationStructureGeometryDataKHR { instances: data });
        let top = Structure::build(
            device,
            vk::AccelerationStructureTypeKHR::TOP_LEVEL,
            geometry,
            count,
            properties.min_acceleration_structure_scratch_offset_alignment,
        )?;
        Ok(Self::new(bottoms, top))
    }

    pub(crate) const fn handle(&self) -> vk::AccelerationStructureKHR {
        self.raw().raw
    }
}

/// AS 与 KHR OMM 共用的独占 backing；instance 引用创建时的实际设备地址。
/// <https://docs.vulkan.org/refpages/latest/refpages/source/vkGetAccelerationStructureDeviceAddressKHR.html>
pub(crate) struct Structure {
    raw: vk::AccelerationStructureKHR,
    buffer: Buffer,
    loader: khr::acceleration_structure::Device,
    address: vk::DeviceAddress,
}

impl Structure {
    fn backing_len(size: u64) -> Result<usize, Error> {
        size.checked_add(255)
            .and_then(|size| usize::try_from(size).ok())
            .ok_or(Error::InvalidArgument("AS backing 对齐大小溢出"))
    }

    fn create(
        device: &Device,
        buffer: &Buffer,
        kind: vk::AccelerationStructureTypeKHR,
        size: u64,
    ) -> Result<(vk::AccelerationStructureKHR, vk::DeviceAddress), Error> {
        let address = buffer.address(device);
        let padding = (256 - address % 256) % 256;
        let address = address
            .checked_add(padding)
            .ok_or(Error::InvalidArgument("AS backing 地址溢出"))?;
        let loader =
            khr::device_address_commands::Device::load(&device.physical.instance.raw, &device.raw);
        // backing 独占、完整绑定且不含 STORAGE_BUFFER/TRANSFORM_FEEDBACK usage。
        let raw = unsafe {
            loader.create_acceleration_structure2(
                &vk::AccelerationStructureCreateInfo2KHR::default()
                    .address_range(
                        vk::DeviceAddressRangeKHR::default()
                            .address(address)
                            .size(size),
                    )
                    .address_flags(vk::AddressCommandFlagsKHR::FULLY_BOUND)
                    .ty(kind),
                None,
            )?
        };
        Ok((raw, address))
    }

    pub(crate) fn allocate(
        device: &Device,
        kind: vk::AccelerationStructureTypeKHR,
        size: u64,
    ) -> Result<Self, Error> {
        let buffer = Guard::new(
            device,
            Buffer::new(
                device,
                Self::backing_len(size)?,
                vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                    | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )?,
        );
        let (raw, address) = Self::create(device, &buffer, kind, size)?;
        Ok(Self {
            raw,
            buffer: buffer.finish(),
            loader: khr::acceleration_structure::Device::load(
                &device.physical.instance.raw,
                &device.raw,
            ),
            address,
        })
    }

    pub(crate) const fn raw(&self) -> vk::AccelerationStructureKHR {
        self.raw
    }

    fn recycle(self, device: &Device, pool: &mut BufferPool) {
        unsafe {
            self.loader.destroy_acceleration_structure(self.raw, None);
        }
        pool.give(device, Class::Backing, self.buffer);
    }

    fn allocate_pooled(
        device: &Device,
        pool: &mut BufferPool,
        kind: vk::AccelerationStructureTypeKHR,
        size: u64,
    ) -> Result<Self, Error> {
        let loader =
            khr::acceleration_structure::Device::load(&device.physical.instance.raw, &device.raw);
        let buffer = pool.take(device, Class::Backing, Self::backing_len(size)?)?;
        let (raw, address) = match Self::create(device, &buffer, kind, size) {
            Ok(result) => result,
            Err(error) => {
                pool.give(device, Class::Backing, buffer);
                return Err(error);
            }
        };
        Ok(Self {
            raw,
            buffer,
            loader,
            address,
        })
    }

    fn build(
        device: &Device,
        kind: vk::AccelerationStructureTypeKHR,
        geometry: vk::AccelerationStructureGeometryKHR<'_>,
        count: u32,
        alignment: u32,
    ) -> Result<Self, Error> {
        let loader =
            khr::acceleration_structure::Device::load(&device.physical.instance.raw, &device.raw);
        let geometries = [geometry];
        let mut flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        if kind == vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL && device.position_fetch {
            // 尺寸查询和实际构建须使用相同的 position fetch flags。
            flags |= vk::BuildAccelerationStructureFlagsKHR::ALLOW_DATA_ACCESS;
        }
        let mut build = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(kind)
            .flags(flags)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&geometries);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            loader.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &build,
                Some(&[count]),
                &mut sizes,
            )
        };
        let structure = Guard::new(
            device,
            Self::allocate(device, kind, sizes.acceleration_structure_size)?,
        );
        let padding = u64::from(alignment) - 1;
        let scratch_len = usize::try_from(
            sizes
                .build_scratch_size
                .max(1)
                .checked_add(padding)
                .ok_or(Error::InvalidArgument("AS scratch 大小溢出"))?,
        )
        .map_err(|_| Error::InvalidArgument("AS scratch 超出地址范围"))?;
        let scratch = Guard::new(
            device,
            Buffer::new(
                device,
                scratch_len,
                vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )?,
        );
        let address = (scratch.address(device) + padding) & !padding;
        build = build
            .dst_acceleration_structure(structure.raw)
            .scratch_data(vk::DeviceOrHostAddressKHR {
                device_address: address,
            });
        let ranges = [vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(count)];
        crate::native::immediate(device, |command| unsafe {
            let before = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::HOST_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)];
            device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::HOST,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::DependencyFlags::empty(),
                &before,
                &[],
                &[],
            );
            structure
                .loader
                .cmd_build_acceleration_structures(command, &[build], &[Some(&ranges)]);
            let after = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR)];
            device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR
                    | vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                vk::DependencyFlags::empty(),
                &after,
                &[],
                &[],
            );
            Ok(())
        })?;
        Ok(structure.finish())
    }

    const fn address(&self) -> vk::DeviceAddress {
        self.address
    }
}

impl Destroy for Structure {
    fn destroy(&mut self, device: &Device) {
        unsafe { self.loader.destroy_acceleration_structure(self.raw, None) };
        self.buffer.destroy(device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependent_affine_rows_are_rejected_despite_roundoff() {
        let matrix = [
            [2822420., 2952455., 3643872., 0.],
            [3416325., 1560073., 2245442., 0.],
            [6238745., 4512528., 5889314., 0.],
        ];
        assert!(validate_instance(&matrix, 0).is_err());
    }
}
