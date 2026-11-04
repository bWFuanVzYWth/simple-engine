use crate::owner::{Destroy, Guard};
use crate::{Device, Error, acceleration::Structure, buffer::Buffer, khr, vk};
use ash::vk::TaggedStructure as _;

/// primitive 索引中的 fully opaque special index，不引用 micromap 目录。
pub const OPACITY_FULLY_OPAQUE: u32 =
    vk::OpacityMicromapSpecialIndexKHR::FULLY_OPAQUE.as_raw() as u32;

/// 一个三角形的精确 2-state opacity 数据；本类型不推断纹理、UV 或 alpha 阈值。
/// 细分与 bit 顺序须对应实际 BLAS 三角形的同一有序顶点。
#[derive(Clone, Copy)]
pub struct OpacityMap<'a> {
    /// 等分细化级别；包含 `4^subdivision_level` 个 microtriangle，须在设备限制内。
    pub subdivision_level: u16,
    /// Vulkan bird 顺序、每 microtriangle 一 bit、每字节从 LSB 到 MSB。
    /// 0 表示透明、1 表示不透明，长度必须恰为 `ceil(4^level/8)`；尾部未用 bits 忽略。
    pub data: &'a [u8],
}

#[doc(inline)]
pub use crate::handles::OpacityMicromap;
impl OpacityMicromap {
    /// 构建输入中的 map 数量；标识本身不保活资源。
    pub const fn map_count(&self) -> usize {
        self.count
    }
}

pub(crate) struct MicromapData {
    structure: Structure,
    count: usize,
}

impl MicromapData {
    pub(crate) fn new(
        device: &Device,
        wgpu_device: &wgpu::Device,
        maps: &[OpacityMap<'_>],
    ) -> Result<Self, Error> {
        device.ensure_active()?;
        if !device.opacity_micromap {
            return Err(Error::Unsupported("设备未启用 opacity micromap"));
        }
        let mut limits = vk::PhysicalDeviceOpacityMicromapPropertiesKHR::default();
        let mut acceleration = vk::PhysicalDeviceAccelerationStructurePropertiesKHR::default();
        let mut properties = vk::PhysicalDeviceProperties2::default()
            .push(&mut limits)
            .push(&mut acceleration);
        unsafe {
            device
                .physical
                .instance
                .raw
                .get_physical_device_properties2(device.physical.raw, &mut properties);
        }
        if maps.len() as u64 > limits.max_micromap_triangles {
            return Err(Error::InvalidArgument("OMM map 数量超过设备限制"));
        }
        let encoded = Encoded::new(maps, limits.max_opacity2_state_subdivision_level)?;
        let usage = usages(&encoded.levels)?;
        let input_usage = vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
        let (_data, data) = Buffer::upload(device, &encoded.data, input_usage, 128)?;
        let _data = Guard::new(device, _data);
        let (_triangles, triangles) = Buffer::upload(device, &encoded.triangles, input_usage, 128)?;
        let _triangles = Guard::new(device, _triangles);
        let loader =
            khr::acceleration_structure::Device::load(&device.physical.instance.raw, &device.raw);
        let mut micromap = vk::AccelerationStructureGeometryMicromapDataKHR::default()
            .usage_counts(&usage)
            .data(data)
            .triangle_array(triangles)
            .triangle_array_stride(8);
        let geometries = [vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::MICROMAP)
            .push(&mut micromap)];
        let mut build = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::OPACITY_MICROMAP)
            .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&geometries);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        // OMM 没有 primitive range；usage counts 决定完整拓扑，查询传入空指针。
        unsafe {
            loader.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &build,
                None,
                &mut sizes,
            );
        }
        let result = Guard::new(
            device,
            Self {
                structure: Structure::allocate(
                    device,
                    vk::AccelerationStructureTypeKHR::OPACITY_MICROMAP,
                    sizes.acceleration_structure_size,
                )?,
                count: maps.len(),
            },
        );
        let padding =
            u64::from(acceleration.min_acceleration_structure_scratch_offset_alignment) - 1;
        let scratch_len = usize::try_from(
            sizes
                .build_scratch_size
                .max(1)
                .checked_add(padding)
                .ok_or(Error::InvalidArgument("OMM scratch 大小溢出"))?,
        )
        .map_err(|_| Error::InvalidArgument("OMM scratch 超出地址范围"))?;
        let scratch = Guard::new(
            device,
            Buffer::new(
                device,
                scratch_len,
                vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )?,
        );
        build = build.dst_acceleration_structure(result.raw()).scratch_data(
            vk::DeviceOrHostAddressKHR {
                device_address: (scratch.address(device) + padding) & !padding,
            },
        );
        let command = crate::interop::record(device, "OMM build", |command| unsafe {
            barrier(
                device,
                command,
                vk::MemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::HOST)
                    .src_access_mask(vk::AccessFlags2::HOST_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
                    .dst_access_mask(vk::AccessFlags2::SHADER_READ),
            );
            loader.cmd_build_acceleration_structures(command, &[build], &[None]);
            barrier(
                device,
                command,
                vk::MemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
                    .src_access_mask(vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR)
                    .dst_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
                    .dst_access_mask(vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR),
            );
            Ok(())
        })?;
        // wait 失败也先确认全队列完成，否则终止；不释放在途 input/scratch/OMM。
        let submitted = device.submit([command])?;
        crate::interop::wait(wgpu_device, Some(submitted))?;
        Ok(result.finish())
    }

    /// 可供 BLAS 引用的 map 数量；索引与构建输入顺序相同。
    pub const fn map_count(&self) -> usize {
        self.count
    }

    pub(crate) const fn raw(&self) -> vk::AccelerationStructureKHR {
        self.structure.raw()
    }

    pub(crate) fn validate_indices(&self, indices: &[u32], primitives: usize) -> Result<(), Error> {
        if indices.len() != primitives {
            return Err(Error::InvalidArgument("OMM index 数量须与三角形数量相同"));
        }
        validate_indices(self.count, indices)
    }
}

impl Destroy for MicromapData {
    fn destroy(&mut self, device: &Device) {
        self.structure.destroy(device);
    }
}

// Device 保证 Vulkan 1.4 和 synchronization2 已启用；command 属于其 compute 队列。
unsafe fn barrier(device: &Device, command: vk::CommandBuffer, memory: vk::MemoryBarrier2<'_>) {
    let memory = [memory];
    let info = vk::DependencyInfo::default().memory_barriers(&memory);
    unsafe {
        device.raw.cmd_pipeline_barrier2(command, &info);
    }
}

struct Encoded {
    data: Vec<u8>,
    triangles: Vec<u8>,
    levels: Vec<u16>,
}

impl Encoded {
    fn new(maps: &[OpacityMap<'_>], max_level: u32) -> Result<Self, Error> {
        if maps.is_empty() || u32::try_from(maps.len()).is_err() {
            return Err(Error::InvalidArgument("OMM map 数量为空或超限"));
        }
        let mut result = Self {
            data: Vec::new(),
            triangles: Vec::new(),
            levels: Vec::new(),
        };
        result.triangles.try_reserve_exact(
            maps.len()
                .checked_mul(8)
                .ok_or(Error::InvalidArgument("OMM 目录大小溢出"))?,
        )?;
        result.levels.try_reserve_exact(maps.len())?;
        for map in maps {
            if u32::from(map.subdivision_level) > max_level {
                return Err(Error::Unsupported(
                    "设备不支持请求的 2-state OMM subdivision level",
                ));
            }
            let bits = 1_usize
                .checked_shl(u32::from(map.subdivision_level) * 2)
                .ok_or(Error::InvalidArgument("OMM subdivision level 超出地址范围"))?;
            if map.data.len() != bits.div_ceil(8) {
                return Err(Error::InvalidArgument(
                    "OMM 2-state 数据长度与 subdivision level 不匹配",
                ));
            }
            let offset = u32::try_from(result.data.len())
                .map_err(|_| Error::InvalidArgument("OMM 数据偏移超出 u32"))?;
            result.data.try_reserve(map.data.len())?;
            result.data.extend_from_slice(map.data);
            result.triangles.extend_from_slice(&offset.to_ne_bytes());
            result
                .triangles
                .extend_from_slice(&map.subdivision_level.to_ne_bytes());
            result.triangles.extend_from_slice(
                &(vk::OpacityMicromapFormatKHR::TYPE_2_STATE.as_raw() as u16).to_ne_bytes(),
            );
            result.levels.push(map.subdivision_level);
        }
        Ok(result)
    }
}

fn validate_indices(count: usize, indices: &[u32]) -> Result<(), Error> {
    if indices
        .iter()
        .any(|&index| index != OPACITY_FULLY_OPAQUE && index as usize >= count)
    {
        return Err(Error::InvalidArgument(
            "OMM index 未引用有效 map 或 fully opaque special",
        ));
    }
    Ok(())
}

fn usages(levels: &[u16]) -> Result<Vec<vk::MicromapUsageKHR>, Error> {
    let mut result: Vec<vk::MicromapUsageKHR> = Vec::new();
    result.try_reserve_exact(levels.len())?;
    for &level in levels {
        let level = u32::from(level);
        if let Some(usage) = result
            .iter_mut()
            .find(|usage| usage.subdivision_level == level)
        {
            usage.count = usage
                .count
                .checked_add(1)
                .ok_or(Error::InvalidArgument("OMM map 数量超出 u32"))?;
        } else {
            result.push(vk::MicromapUsageKHR {
                count: 1,
                subdivision_level: level,
                format: vk::OpacityMicromapFormatKHR::TYPE_2_STATE,
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bird_bits_are_preserved_and_directory_keeps_input_order() -> Result<(), Error> {
        let maps = [
            OpacityMap {
                subdivision_level: 0,
                data: &[1],
            },
            OpacityMap {
                subdivision_level: 1,
                data: &[0b1101],
            },
            OpacityMap {
                subdivision_level: 2,
                data: &[0x12, 0x34],
            },
        ];
        let encoded = Encoded::new(&maps, 8)?;
        assert_eq!(encoded.data, [1, 13, 0x12, 0x34]);
        for (index, offset) in [0_u32, 1, 2].into_iter().enumerate() {
            let record = &encoded.triangles[index * 8..index * 8 + 8];
            assert_eq!(u32::from_ne_bytes(record[..4].try_into().unwrap()), offset);
            assert_eq!(
                u16::from_ne_bytes(record[4..6].try_into().unwrap()),
                index as u16
            );
            assert_eq!(u16::from_ne_bytes(record[6..].try_into().unwrap()), 1);
        }
        Ok(())
    }

    #[test]
    fn build_counts_follow_input_levels() -> Result<(), Error> {
        let usage = usages(&[4, 6, 4])?;
        assert_eq!(usage.len(), 2);
        assert_eq!((usage[0].subdivision_level, usage[0].count), (4, 2));
        assert_eq!((usage[1].subdivision_level, usage[1].count), (6, 1));
        Ok(())
    }

    #[test]
    fn primitive_indices_accept_maps_and_fully_opaque_only() {
        assert!(validate_indices(3, &[2, 0, 2, OPACITY_FULLY_OPAQUE, 1]).is_ok());
        assert!(validate_indices(1, &[1]).is_err());
        assert!(validate_indices(1, &[u32::MAX]).is_err());
    }

    #[test]
    fn invalid_binary_map_boundaries_are_rejected() {
        assert!(Encoded::new(&[], 8).is_err());
        for map in [
            OpacityMap {
                subdivision_level: 1,
                data: &[],
            },
            OpacityMap {
                subdivision_level: 2,
                data: &[0],
            },
            OpacityMap {
                subdivision_level: 1,
                data: &[0, 0],
            },
            OpacityMap {
                subdivision_level: 9,
                data: &[],
            },
            OpacityMap {
                subdivision_level: u16::MAX,
                data: &[],
            },
        ] {
            assert!(Encoded::new(&[map], 8).is_err());
        }
    }
}
