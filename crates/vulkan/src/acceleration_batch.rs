//! 独立 BLAS 在同一命令中构建；输入与 scratch 各用一个共享分配。

use super::*;
use std::time::Instant;

/// 一批 BLAS 的一个输入；primitive 顺序保持不变，覆盖语义由调用方显式指定。
#[derive(Clone, Copy, Debug)]
pub struct TriangleBlasInput<'a> {
    /// 非空、有限的局部三角形；只借用到批构建返回。
    pub triangles: &'a [[[f32; 3]; 3]],
    /// 为真时普通三角形直接接受；为假时由 any-hit 或 RayQuery candidate 确认覆盖。
    /// 已绑定 OMM 的透明度仍遵循其 microtriangle 状态。
    pub opaque: bool,
    /// 可选同设备 OMM 和逐 primitive 索引；长度须等于三角形数量。
    pub opacity: Option<(OpacityMicromap, &'a [u32])>,
}

/// 从已校验的紧凑 quad 位置流构建的一个 BLAS。每个 quad 产生 012、023 两个 primitive。
#[derive(Clone, Copy, Debug)]
pub struct QuadBlasInput<'a> {
    /// 非空局部坐标流；宿主只需保留到录制返回。
    pub quads: &'a scene_cache::ResidentQuads,
    /// 该区发布的设备列；全部输出在同一计算调用中生成。
    pub output: QuadOutput,
    /// 与普通三角形输入相同的 opaque 标志。
    pub opaque: bool,
    /// 同设备 OMM 和每个半面的索引，顺序与输出 primitive 一致。
    pub opacity: Option<(OpacityMicromap, &'a [u32])>,
}

/// 原生设备输出的区域切片，每列起点以 quad 为单位。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuadOutput {
    /// 统一的 quad 槽起点。
    pub first: u32,
    /// 区域坐标，供已知几何重建或体素来源查询。
    pub region: [i32; 3],
    /// 每 quad 三个 uint4 的表面属性。
    pub attributes: Option<crate::native::Buffer>,
    /// 每 quad 一个非零 uint2 跨帧身份。
    pub identities: Option<crate::native::Buffer>,
    /// 连续顶点，每 quad 四条 16B 记录，xyz 用于 BLAS，前三角的 w 保存区域坐标。
    pub vertices: Option<crate::native::Buffer>,
    /// 每 quad 一个 int4 的体素来源（世界格坐标与表面标记）。
    pub cells: Option<crate::native::Buffer>,
}
impl QuadOutput {
    pub(crate) fn columns(self) -> [(Option<crate::native::Buffer>, u64, u64); 4] {
        [
            (self.attributes, 48, 0),
            (self.identities, 8, 0),
            (self.vertices, 64, 0),
            (self.cells, 16, 0),
        ]
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Sources<'a> {
    Triangles(&'a [TriangleBlasInput<'a>]),
    Quads(&'a [QuadBlasInput<'a>]),
    Uploaded(
        &'a [QuadBlasInput<'a>],
        &'a [crate::terrain_upload::Uploaded],
    ),
}
impl Sources<'_> {
    pub(crate) const fn len(self) -> usize {
        match self {
            Self::Triangles(v) => v.len(),
            Self::Quads(v) | Self::Uploaded(v, _) => v.len(),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Positions<'a> {
    Triangles(&'a [[[f32; 3]; 3]]),
    Quads(&'a scene_cache::ResidentQuads),
}

/// 批构建的主机阶段；录制型入口不提交或等待，`total_ns` 另含标识管理成本。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BuildTiming {
    /// 入口至返回的主机墙钟；录制型入口不包含 GPU 执行。
    pub total_ns: u64,
    /// 归属、数量、坐标、格式与 OMM 索引校验。
    pub validation_ns: u64,
    /// 一次 map/copy/unmap，包含顶点及 OMM 索引，不生成中间字节数组。
    pub upload_ns: u64,
    /// 输入、独立 AS backing、共享 scratch 及可选 query pool 的布局查询和分配。
    pub alloc_ns: u64,
    /// 录制统一 barrier、全部 BLAS 和可选 timestamp。
    pub record_ns: u64,
    /// 同步入口的一次提交及完成等待；录制型入口为零，不是纯 GPU 构建时间。
    pub submit_wait_ns: u64,
    /// AS 阶段 timestamp 的设备跨度；禁用或队列不支持时为 `None`，仍受并发 GPU 工作影响。
    pub gpu_build_ns: Option<u64>,
    /// 本批 BLAS 数；空批不提交、不等待。
    pub blas_count: usize,
    /// 实际复制的顶点和索引字节数。
    pub input_bytes: u64,
    /// 共享 scratch 的请求容量，包含地址对齐余量。
    pub scratch_bytes: u64,
    /// 独立 AS backing 的请求容量之和。
    pub backing_bytes: u64,
    /// 本批借出的 input、scratch、backing 实际 allocation 字节，命中时可大于请求容量。
    pub allocation_bytes: u64,
    /// 本批从 Context 池取得闲置分配的次数。
    pub pool_hits: u64,
    /// 本批新分配 buffer/memory 的次数。
    pub pool_misses: u64,
    /// 批返回时的闲置 allocation 字节；不包含随后场景 collect 归还的旧 AS。
    pub pool_idle_bytes: u64,
}

impl BuildTiming {
    pub(crate) fn merge(&mut self, other: Self) {
        self.total_ns += other.total_ns;
        self.validation_ns += other.validation_ns;
        self.upload_ns += other.upload_ns;
        self.alloc_ns += other.alloc_ns;
        self.record_ns += other.record_ns;
        self.submit_wait_ns += other.submit_wait_ns;
        self.blas_count += other.blas_count;
        self.input_bytes += other.input_bytes;
        self.backing_bytes += other.backing_bytes;
        self.scratch_bytes = self.scratch_bytes.max(other.scratch_bytes);
        self.allocation_bytes = self.allocation_bytes.max(other.allocation_bytes);
        self.pool_hits += other.pool_hits;
        self.pool_misses += other.pool_misses;
        self.pool_idle_bytes = other.pool_idle_bytes;
    }
}

/// 全部构建完成的 BLAS，输出次序与输入相同；标识由唯一 Context 保活。
#[derive(Debug)]
pub struct TriangleBlasBatch {
    /// 同步完成后才能取得的正式标识。
    pub blases: Vec<super::TriangleBlas>,
    /// 本批主机及可选设备计时。
    pub timing: BuildTiming,
}

/// 依赖有序且已同步完成的 BLAS/TLAS 场景批。
pub struct TriangleSceneBatch {
    /// 新构建的 BLAS，与输入一一对应。提交等待含整个场景批。
    pub blas: TriangleBlasBatch,
    /// 以输入实例顺序构建的 TLAS。
    pub top: AccelerationStructure,
    /// TLAS 校验、上传、分配和录制的主机墙钟，排除共同等待。
    pub tlas_prepare_ns: u64,
}

pub(crate) struct ResolvedBlas<'a> {
    pub(crate) positions: Positions<'a>,
    pub(crate) destinations: [u64; 4],
    pub(crate) uploaded: Option<u64>,
    pub(crate) region: [i32; 3],
    pub(crate) opaque: bool,
    pub(crate) opacity: Option<(&'a MicromapData, OpacityMicromap, &'a [u32])>,
}

struct Plan {
    count: u32,
    vertices: u32,
    vertex_offset: usize,
    expanded_offset: Option<usize>,
    identity_offset: Option<usize>,
    sorted_opacity: Option<usize>,
    index_offset: Option<usize>,
}

struct BatchResources<'a> {
    device: &'a Device,
    pool: &'a mut BufferPool,
    input: Option<Buffer>,
    expanded: Option<Buffer>,
    scratch: Option<Buffer>,
    blases: Vec<BlasData>,
}

pub(crate) struct BatchOutput {
    pub(crate) blases: Vec<BlasData>,
    pub(crate) timing: BuildTiming,
    pub(crate) recorded: Option<(wgpu::CommandBuffer, BuildBuffers)>,
    pub(crate) raw_buffers: BuildBuffers,
}

#[derive(Clone, Copy)]
pub(crate) enum BatchMode {
    Sync,
    Wgpu,
    Raw(vk::CommandBuffer),
}

impl<'a> BatchResources<'a> {
    fn new(device: &'a Device, pool: &'a mut BufferPool, count: usize) -> Result<Self, Error> {
        let mut blases = Vec::new();
        blases.try_reserve_exact(count)?;
        Ok(Self {
            device,
            pool,
            input: None,
            expanded: None,
            scratch: None,
            blases,
        })
    }
}

impl Drop for BatchResources<'_> {
    fn drop(&mut self) {
        if self.device.is_quarantined() {
            std::mem::forget(std::mem::take(&mut self.blases));
            // Buffer 的 Vulkan 销毁为显式操作，隔离设备上的句柄不再回池。
            return;
        }
        for blas in self.blases.drain(..) {
            blas.recycle(self.device, self.pool);
        }
        if let Some(buffer) = self.input.take() {
            self.pool.give(self.device, Class::Input, buffer);
        }
        if let Some(buffer) = self.expanded.take() {
            self.pool.give(self.device, Class::Expanded, buffer);
        }
        if let Some(buffer) = self.scratch.take() {
            self.pool.give(self.device, Class::Scratch, buffer);
        }
    }
}

fn nanos(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn triangle_bytes(triangles: &[[[f32; 3]; 3]]) -> &[u8] {
    bytemuck::cast_slice(triangles)
}

fn index_bytes(indices: &[u32]) -> &[u8] {
    bytemuck::cast_slice(indices)
}

struct Queries {
    raw: vk::QueryPool,
    bits: u32,
    period: f64,
}

impl Queries {
    fn new(device: &Device, enabled: bool) -> Result<Option<Self>, Error> {
        if !enabled {
            return Ok(None);
        }
        let bits =
            device.physical.queue_families()[device.queue_family as usize].timestamp_valid_bits;
        if bits == 0 {
            return Ok(None);
        }
        let raw = unsafe {
            device.raw.create_query_pool(
                &vk::QueryPoolCreateInfo::default()
                    .query_type(vk::QueryType::TIMESTAMP)
                    .query_count(2),
                None,
            )?
        };
        Ok(Some(Self {
            raw,
            bits,
            period: f64::from(device.physical.properties().limits.timestamp_period),
        }))
    }

    fn elapsed(&self, device: &Device) -> Result<u64, Error> {
        let mut values = [0_u64; 2];
        // 批次已完成；不追加 WAIT flag 或额外的 host 等待。
        unsafe {
            device.raw.get_query_pool_results(
                self.raw,
                0,
                &mut values,
                vk::QueryResultFlags::TYPE_64,
            )?;
        }
        let mask = if self.bits == 64 {
            u64::MAX
        } else {
            (1_u64 << self.bits) - 1
        };
        let delta = values[1].wrapping_sub(values[0]) & mask;
        Ok((delta as f64 * self.period).round() as u64)
    }
}

impl Destroy for Queries {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_query_pool(self.raw, None);
        }
    }
}

const fn scratch_range(start: u64, size: u64, alignment: u64) -> Result<(u64, u64), Error> {
    if !alignment.is_power_of_two() {
        return Err(Error::InvalidArgument("AS scratch 对齐无效"));
    }
    let Some(offset) = start.checked_add(alignment - 1) else {
        return Err(Error::InvalidArgument("批 scratch 对齐溢出"));
    };
    let offset = offset & !(alignment - 1);
    let size = if size == 0 { 1 } else { size };
    let Some(end) = offset.checked_add(size) else {
        return Err(Error::InvalidArgument("批 scratch 大小溢出"));
    };
    Ok((offset, end))
}

impl BlasData {
    pub(crate) fn batch(
        device: &Device,
        inputs: &[ResolvedBlas<'_>],
        timestamps: bool,
        mode: BatchMode,
        pool: &mut BufferPool,
        expansion: Option<&crate::quad_expansion::Expansion>,
    ) -> Result<BatchOutput, Error> {
        let started = Instant::now();
        let mut timing = BuildTiming {
            blas_count: inputs.len(),
            ..Default::default()
        };
        if inputs.is_empty() {
            timing.total_ns = nanos(started);
            return Ok(BatchOutput {
                blases: Vec::new(),
                timing,
                recorded: if matches!(mode, BatchMode::Wgpu) {
                    Some((
                        crate::interop::record(device, "empty BLAS batch", |_| Ok(()))?,
                        BuildBuffers::default(),
                    ))
                } else {
                    None
                },
                raw_buffers: BuildBuffers::default(),
            });
        }
        let stage = Instant::now();
        u32::try_from(inputs.len()).map_err(|_| Error::InvalidArgument("BLAS 批次过大"))?;
        let properties = properties(device)?;
        if !device
            .format_properties(vk::Format::R32G32B32_SFLOAT)
            .buffer_features
            .contains(vk::FormatFeatureFlags::ACCELERATION_STRUCTURE_VERTEX_BUFFER_KHR)
        {
            return Err(Error::Unsupported("设备不支持 float3 AS 顶点"));
        }
        let mut plans = Vec::new();
        plans.try_reserve_exact(inputs.len())?;
        let mut input_len = 0_usize;
        let mut expanded_len = 0usize;
        let mut sorted_bytes = 0usize;
        let mut largest = None;
        for input in inputs {
            let (triangles, bytes, expanded_offset) = match input.positions {
                Positions::Triangles(v) => {
                    if v.iter().flatten().flatten().any(|v| !v.is_finite()) {
                        return Err(Error::InvalidArgument("BLAS 坐标非有限"));
                    }
                    (v.len(), size_of_val(v), None)
                }
                Positions::Quads(v) => {
                    if expansion.is_none() {
                        return Err(Error::InvalidArgument("地形展开管线尚未准备"));
                    }
                    let count = v.positions().headers().len();
                    if count > u32::MAX as usize / 18 {
                        return Err(Error::InvalidArgument("quad展开寻址超过32位"));
                    }
                    let offset = expanded_len;
                    if input.destinations[2] == 0 {
                        expanded_len = count
                            .checked_mul(64)
                            .and_then(|n| offset.checked_add(n))
                            .ok_or(Error::InvalidArgument("quad 展开缓冲溢出"))?;
                    }
                    if largest.is_none_or(|(_, previous)| count > previous) {
                        largest = Some((plans.len(), count));
                    }
                    (count * 2, v.positions().bytes(), Some(offset))
                }
            };
            let count =
                u32::try_from(triangles).map_err(|_| Error::InvalidArgument("三角形数量过多"))?;
            let vertices = count
                .checked_mul(if expanded_offset.is_some() { 2 } else { 3 })
                .ok_or(Error::InvalidArgument("顶点数量过多"))?;
            if count == 0 || u64::from(count) > properties.max_primitive_count {
                return Err(Error::InvalidArgument("BLAS 为空、坐标非有限或数量超限"));
            }
            let previous_len = input_len;
            if input.uploaded.is_some() {
                input_len = 0;
            }
            let vertex_offset = input_len;
            input_len = input_len
                .checked_add(bytes)
                .ok_or(Error::InvalidArgument("批输入大小溢出"))?;
            let identity_offset = if let Positions::Quads(v) = input.positions
                && input.destinations[1] != 0
                && !v.consecutive()
            {
                let offset = input_len;
                input_len = input_len
                    .checked_add(size_of_val(v.explicit_tokens()))
                    .ok_or(Error::InvalidArgument("身份字节数溢出"))?;
                Some(offset)
            } else {
                None
            };
            let index_offset = if let Some((map, _, indices)) = input.opacity {
                map.validate_indices(indices, triangles)?;
                let offset = input_len;
                input_len = input_len
                    .checked_add(size_of_val(indices))
                    .ok_or(Error::InvalidArgument("批输入大小溢出"))?;
                Some(offset)
            } else {
                None
            };
            let sorted_opacity = if expanded_offset.is_some() && index_offset.is_some() {
                let at = sorted_bytes;
                sorted_bytes = sorted_bytes
                    .checked_add(triangles * 4)
                    .ok_or(Error::InvalidArgument("OMM 分组输出溢出"))?;
                Some(at)
            } else {
                None
            };
            plans.push(Plan {
                count,
                vertices,
                vertex_offset,
                expanded_offset,
                identity_offset,
                sorted_opacity,
                index_offset,
            });
            if input.uploaded.is_some() {
                input_len = previous_len;
            }
        }
        timing.input_bytes = input_len as u64;
        timing.validation_ns = nanos(stage);

        let stage = Instant::now();
        let pool_before = pool.stats();
        let mut resources = BatchResources::new(device, pool, inputs.len())?;
        let input_len = input_len
            .checked_add(3)
            .ok_or(Error::InvalidArgument("批输入对齐溢出"))?;
        if timing.input_bytes != 0 {
            resources.input = Some(resources.pool.take(device, Class::Input, input_len)?);
        }
        if let Some((_, count)) = largest {
            let total = count
                .checked_mul(24)
                .and_then(|indices| expanded_len.checked_add(indices))
                .and_then(|bytes| bytes.checked_add(sorted_bytes))
                .and_then(|bytes| bytes.checked_add(15))
                .ok_or(Error::InvalidArgument("quad 设备缓冲溢出"))?;
            resources.expanded = Some(resources.pool.take(device, Class::Expanded, total)?);
        }
        let expanded_address = resources
            .expanded
            .as_ref()
            .map_or(0, |b| b.address(device).next_multiple_of(16));
        let index_address = expanded_address + expanded_len as u64;
        let opacity_address = index_address + largest.map_or(0, |(_, n)| n as u64 * 24);
        let vertex_address = |input: &ResolvedBlas, offset: usize| {
            if input.destinations[2] != 0 {
                input.destinations[2]
            } else {
                expanded_address + offset as u64
            }
        };
        let input = resources.input.as_ref();
        timing.allocation_bytes = input.map_or(0, Buffer::allocation_bytes)
            + resources
                .expanded
                .as_ref()
                .map_or(0, Buffer::allocation_bytes);
        let input_address = input.map_or(0, |b| b.address(device));
        let padding = ((4 - input_address % 4) % 4) as usize;
        let source_address =
            |s: &ResolvedBlas| s.uploaded.unwrap_or(input_address + padding as u64);
        timing.alloc_ns += nanos(stage);
        let stage = Instant::now();
        let mut writes = Vec::new();
        writes.try_reserve_exact(
            inputs
                .len()
                .checked_mul(5)
                .ok_or(Error::InvalidArgument("批输入数溢出"))?,
        )?;
        for (input, plan) in inputs.iter().zip(&plans) {
            if input.uploaded.is_some() {
                continue;
            }
            let offset = plan.vertex_offset + padding;
            match input.positions {
                Positions::Triangles(v) => writes.push((offset, triangle_bytes(v))),
                Positions::Quads(v) => {
                    if let Some(offset) = plan.identity_offset {
                        writes.push((offset + padding, bytemuck::cast_slice(v.explicit_tokens())));
                    }
                    let v = v.positions();
                    let surfaces = offset + size_of_val(v.headers());
                    writes.push((offset, index_bytes(v.headers().as_flattened())));
                    writes.push((surfaces, index_bytes(v.surfaces().as_flattened())));
                    writes.push((
                        surfaces + size_of_val(v.surfaces()),
                        index_bytes(v.payload()),
                    ));
                }
            }
            if let (Some(offset), Some((_, _, indices))) = (plan.index_offset, input.opacity) {
                writes.push((offset + padding, index_bytes(indices)));
            }
        }
        if let Some(input) = input {
            input.write_slices(device, &writes)?;
        }
        timing.upload_ns = nanos(stage);

        let stage = Instant::now();
        let mut opacities = Vec::new();
        opacities.try_reserve_exact(inputs.len())?;
        opacities.resize(
            inputs.len(),
            vk::AccelerationStructureTrianglesOpacityMicromapKHR::default(),
        );
        let mut geometries = Vec::new();
        geometries.try_reserve_exact(inputs.len())?;
        for ((input, plan), opacity) in inputs.iter().zip(&plans).zip(&mut opacities) {
            let mut data = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
                .vertex_format(vk::Format::R32G32B32_SFLOAT)
                .vertex_data(vk::DeviceOrHostAddressConstKHR {
                    device_address: plan.expanded_offset.map_or(
                        source_address(input) + plan.vertex_offset as u64,
                        |offset| vertex_address(input, offset),
                    ),
                })
                .vertex_stride(if plan.expanded_offset.is_some() {
                    16
                } else {
                    12
                })
                .max_vertex(plan.vertices - 1)
                .index_type(vk::IndexType::NONE_KHR);
            if plan.expanded_offset.is_some() {
                data = data.index_type(vk::IndexType::UINT32).index_data(
                    vk::DeviceOrHostAddressConstKHR {
                        device_address: index_address,
                    },
                );
            }
            if let (Some(offset), Some((map, _, _))) = (plan.index_offset, input.opacity) {
                *opacity = vk::AccelerationStructureTrianglesOpacityMicromapKHR::default()
                    .index_type(vk::IndexType::UINT32)
                    .index_buffer(
                        plan.sorted_opacity
                            .map_or(source_address(input) + offset as u64, |at| {
                                opacity_address + at as u64
                            }),
                    )
                    .index_stride(4)
                    .micromap(map.raw());
                data = data.push(opacity);
            }
            geometries.push(
                vk::AccelerationStructureGeometryKHR::default()
                    .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                    .geometry(vk::AccelerationStructureGeometryDataKHR { triangles: data })
                    .flags(if input.opaque {
                        vk::GeometryFlagsKHR::OPAQUE
                    } else {
                        vk::GeometryFlagsKHR::empty()
                    }),
            );
        }
        let loader =
            khr::acceleration_structure::Device::load(&device.physical.instance.raw, &device.raw);
        let mut flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        if device.position_fetch {
            flags |= vk::BuildAccelerationStructureFlagsKHR::ALLOW_DATA_ACCESS;
        }
        let alignment = u64::from(properties.min_acceleration_structure_scratch_offset_alignment);
        let mut scratch_len = 0_u64;
        let mut scratch_offsets = Vec::new();
        let mut builds = Vec::new();
        let mut ranges = Vec::new();
        scratch_offsets.try_reserve_exact(inputs.len())?;
        builds.try_reserve_exact(inputs.len())?;
        ranges.try_reserve_exact(inputs.len())?;
        for ((input, plan), geometry) in inputs.iter().zip(&plans).zip(&geometries) {
            let mut build = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
                .flags(flags)
                .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                .geometries(std::slice::from_ref(geometry));
            let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
            unsafe {
                loader.get_acceleration_structure_build_sizes(
                    vk::AccelerationStructureBuildTypeKHR::DEVICE,
                    &build,
                    Some(&[plan.count]),
                    &mut sizes,
                );
            }
            let structure = Self::new(
                input.opacity.map(|(_, id, _)| id),
                Structure::allocate_pooled(
                    device,
                    resources.pool,
                    vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL,
                    sizes.acceleration_structure_size,
                )?,
            );
            let raw = structure.raw().raw;
            let allocation_bytes = structure.raw().buffer.allocation_bytes();
            resources.blases.push(structure);
            timing.allocation_bytes = timing
                .allocation_bytes
                .checked_add(allocation_bytes)
                .ok_or(Error::InvalidArgument("批 allocation 总量溢出"))?;
            timing.backing_bytes = timing
                .backing_bytes
                .checked_add(sizes.acceleration_structure_size)
                .ok_or(Error::InvalidArgument("AS backing 总量溢出"))?;
            let (offset, end) = scratch_range(scratch_len, sizes.build_scratch_size, alignment)?;
            scratch_offsets.push(offset);
            scratch_len = end;
            build = build.dst_acceleration_structure(raw);
            builds.push(build);
            ranges
                .push([vk::AccelerationStructureBuildRangeInfoKHR::default()
                    .primitive_count(plan.count)]);
        }
        scratch_len = scratch_len
            .checked_add(alignment - 1)
            .ok_or(Error::InvalidArgument("批 scratch 大小溢出"))?;
        resources.scratch = Some(
            resources.pool.take(
                device,
                Class::Scratch,
                usize::try_from(scratch_len)
                    .map_err(|_| Error::InvalidArgument("批 scratch 超出地址范围"))?,
            )?,
        );
        let scratch = resources.scratch.as_ref().expect("已取得批 scratch 分配");
        timing.allocation_bytes = timing
            .allocation_bytes
            .checked_add(scratch.allocation_bytes())
            .ok_or(Error::InvalidArgument("批 allocation 总量溢出"))?;
        let scratch_address = (scratch.address(device) + alignment - 1) & !(alignment - 1);
        for (build, offset) in builds.iter_mut().zip(&scratch_offsets) {
            build.scratch_data = vk::DeviceOrHostAddressKHR {
                device_address: scratch_address + offset,
            };
        }
        timing.scratch_bytes = scratch_len;
        let queries = Queries::new(device, timestamps && matches!(mode, BatchMode::Sync))?
            .map(|queries| Guard::new(device, queries));
        let mut range_refs = Vec::new();
        range_refs.try_reserve_exact(ranges.len())?;
        range_refs.extend(ranges.iter().map(|range| Some(range.as_slice())));
        timing.alloc_ns += nanos(stage);

        let encode = |command| {
            // 独立构建使用互不重叠的 scratch 切片；组内无依赖，也不插入 barrier。
            unsafe {
                if let Some(queries) = &queries {
                    device.raw.cmd_reset_query_pool(command, queries.raw, 0, 2);
                }
                device.raw.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::HOST
                        | vk::PipelineStageFlags::TRANSFER
                        | vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR
                        | vk::PipelineStageFlags::COMPUTE_SHADER
                        | vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                    vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR
                        | vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[vk::MemoryBarrier::default()
                        .src_access_mask(
                            vk::AccessFlags::HOST_WRITE
                                | vk::AccessFlags::TRANSFER_WRITE
                                | vk::AccessFlags::SHADER_READ
                                | vk::AccessFlags::SHADER_WRITE,
                        )
                        .dst_access_mask(
                            vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                        )],
                    &[],
                    &[],
                );
                if let Some((index_writer, _)) = largest {
                    let program = expansion.expect("压缩输入已经校验设备程序");
                    for (number, (plan, source)) in plans.iter().zip(inputs).enumerate() {
                        if let Some(offset) = plan.expanded_offset {
                            let Positions::Quads(resident) = source.positions else {
                                unreachable!("展开范围只能属于 quad 输入")
                            };
                            let stream = resident.positions();
                            let count = plan.count / 2;
                            let header = source_address(source) + plan.vertex_offset as u64;
                            let [attrs, ids, _, cells] = source.destinations;
                            let id_src = plan
                                .identity_offset
                                .map_or(0, |at| source_address(source) + at as u64);
                            let opacity = plan.index_offset.zip(plan.sorted_opacity).map_or(
                                [0; 2],
                                |(src, dst)| {
                                    [
                                        source_address(source) + src as u64,
                                        opacity_address + dst as u64,
                                    ]
                                },
                            );
                            program.record(
                                device,
                                command,
                                crate::quad_expansion::QuadDispatch {
                                    addresses: [
                                        header,
                                        header
                                            + size_of_val(stream.headers()) as u64
                                            + size_of_val(stream.surfaces()) as u64,
                                        vertex_address(source, offset),
                                        attrs,
                                        ids,
                                        if number == index_writer {
                                            index_address
                                        } else {
                                            0
                                        },
                                        cells,
                                        id_src,
                                    ],
                                    count,
                                    region: source.region,
                                    first_token: resident.first_token(),
                                    opacity,
                                },
                            );
                        }
                    }
                    device.raw.cmd_pipeline_barrier(
                        command,
                        vk::PipelineStageFlags::COMPUTE_SHADER,
                        vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR
                            | vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR
                            | vk::PipelineStageFlags::COMPUTE_SHADER
                            | vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[vk::MemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                            .dst_access_mask(
                                vk::AccessFlags::SHADER_READ | vk::AccessFlags::TRANSFER_READ,
                            )],
                        &[],
                        &[],
                    );
                }
                if let Some(queries) = &queries {
                    device.raw.cmd_write_timestamp(
                        command,
                        vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                        queries.raw,
                        0,
                    );
                }
                loader.cmd_build_acceleration_structures(command, &builds, &range_refs);
                if let Some(queries) = &queries {
                    device.raw.cmd_write_timestamp(
                        command,
                        vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                        queries.raw,
                        1,
                    );
                }
                device.raw.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                    vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR
                        | vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::DependencyFlags::empty(),
                    &[vk::MemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                        .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR)],
                    &[],
                    &[],
                );
            }
            Ok(())
        };
        let mut raw_buffers = BuildBuffers::default();
        let recorded = if let BatchMode::Raw(command) = mode {
            let stage = Instant::now();
            encode(command)?;
            timing.record_ns = nanos(stage);
            raw_buffers = BuildBuffers {
                input: resources.input.take(),
                expanded: resources.expanded.take(),
                scratch: resources.scratch.take(),
            };
            None
        } else if matches!(mode, BatchMode::Wgpu) {
            let stage = Instant::now();
            let command = crate::interop::record(device, "BLAS batch", encode)?;
            timing.record_ns = nanos(stage);
            Some((
                command,
                BuildBuffers {
                    input: resources.input.take(),
                    expanded: resources.expanded.take(),
                    scratch: resources.scratch.take(),
                },
            ))
        } else if device._owner.is_some() {
            let stage = Instant::now();
            let command = crate::interop::record(device, "BLAS batch", encode)?;
            timing.record_ns = nanos(stage);
            let stage = Instant::now();
            let submitted = device.submit([command])?;
            crate::interop::wait(device, Some(submitted))?;
            timing.submit_wait_ns = nanos(stage);
            if let Some(queries) = &queries {
                timing.gpu_build_ns = Some(queries.elapsed(device)?);
            }
            None
        } else {
            let stage = Instant::now();
            // 同一 raw 录制用于两种宿主；一次提交/完成后才归还共享 input 与 scratch。
            // immediate 失败时确认队列完成或保留资源，guards 不会释放仍在执行的候选。
            crate::native::immediate(device, |command| {
                let stage = Instant::now();
                let result = encode(command);
                timing.record_ns = nanos(stage);
                result
            })?;
            timing.submit_wait_ns = nanos(stage).saturating_sub(timing.record_ns);
            if let Some(queries) = &queries {
                timing.gpu_build_ns = Some(queries.elapsed(device)?);
            }
            None
        };
        let output = std::mem::take(&mut resources.blases);
        drop(resources);
        let pool_after = pool.stats();
        timing.pool_hits = pool_after.temp_hits.saturating_sub(pool_before.temp_hits)
            + pool_after
                .backing_hits
                .saturating_sub(pool_before.backing_hits);
        timing.pool_misses = pool_after
            .temp_misses
            .saturating_sub(pool_before.temp_misses)
            + pool_after
                .backing_misses
                .saturating_sub(pool_before.backing_misses);
        timing.pool_idle_bytes = pool_after.idle_bytes;
        timing.total_ns = nanos(started);
        Ok(BatchOutput {
            blases: output,
            timing,
            recorded,
            raw_buffers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ash::vk::Handle as _;

    fn raw_owner() -> Result<(crate::interop::WgpuContext, crate::native::RawDevice), Error> {
        let owner = pollster::block_on(crate::interop::WgpuContext::with_capabilities(
            0,
            crate::native::Capabilities {
                ray_tracing: true,
                position_fetch: true,
                ..Default::default()
            },
            wgpu::Features::empty(),
        ))?;
        let device = &owner.native_ref().device;
        let raw = crate::native::RawDevice {
            instance: device.physical.instance.raw.handle().as_raw(),
            physical_device: device.physical.raw.as_raw(),
            device: device.raw.handle().as_raw(),
            queue: device.queue.as_raw(),
            queue_family: device.queue_family,
            api_version: vk::API_VERSION_1_4,
            subgroup_size_control: device.subgroup_size_control,
            capabilities: owner.native_ref().capabilities(),
        };
        Ok((owner, raw))
    }

    #[test]
    #[ignore = "需要真实KHR RT Vulkan；原生借入队列批构建及TLAS消费，统一串行运行"]
    fn native_batch_results_are_ready_for_tlas_and_pool_reuse() -> Result<(), Error> {
        super::super::validation::start();
        let (owner, raw) = raw_owner()?;
        // SAFETY: owner 保活所有宿主句柄；本测试在单线程独占队列，借入 Context 先析构。
        let mut context = unsafe { crate::native::Context::from_raw(raw)? };
        let triangles = [
            [[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]],
            [[[2., 0., 0.], [3., 0., 0.], [2., 1., 0.]]],
        ];
        let inputs = triangles.each_ref().map(|triangles| TriangleBlasInput {
            triangles,
            opaque: false,
            opacity: None,
        });
        let first = context.triangle_blas_batch(&inputs)?;
        assert_eq!(first.blases.len(), 2);
        let instances: Vec<_> = first
            .blases
            .iter()
            .enumerate()
            .map(|(index, &geometry)| crate::TriangleInstance {
                geometry,
                transform: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]],
                custom_index: index as u32,
            })
            .collect();
        let tlas = context.acceleration_instances(&instances)?;
        context.collect(&[tlas.into()])?;
        context.collect(&[])?;
        let second = context.triangle_blas_batch(&inputs)?;
        assert_ne!(first.blases, second.blases);
        assert!(second.timing.pool_hits >= 4);
        assert_eq!(second.timing.pool_misses, 0);
        drop(context);
        drop(owner);
        super::super::validation::finish()?;
        Ok(())
    }

    #[test]
    #[ignore = "需要真实 KHR RT Vulkan；统一串行运行"]
    fn native_scene_batch_rejects_invalid_tlas_without_replacing_old_scene() -> Result<(), Error> {
        super::super::validation::start();
        let (owner, raw) = raw_owner()?;
        // SAFETY: owner 保活所有宿主句柄；此处独占队列，借入 Context 先析构。
        let mut context = unsafe { crate::native::Context::from_raw(raw)? };
        let triangles = [[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]];
        let inputs = [TriangleBlasInput {
            triangles: &triangles,
            opaque: true,
            opacity: None,
        }];
        let original = crate::TriangleInstance {
            geometry: crate::BatchBlas::New(0),
            transform: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]],
            custom_index: 0,
        };
        let old = context.triangle_scene_batch(&inputs, std::slice::from_ref(&original))?;
        let invalid = crate::TriangleInstance {
            transform: [[0.; 4]; 3],
            ..original.clone()
        };
        let live_bytes = |context: &crate::native::Context| {
            context.allocated_bytes() - context.build_pool_idle_bytes()
        };
        let before = live_bytes(&context);
        assert!(matches!(
            context.triangle_scene_batch(&inputs, &[invalid]),
            Err(Error::InvalidArgument(_))
        ));
        assert_eq!(live_bytes(&context), before);
        context.collect(&[old.top.into()])?;
        let _next = context.triangle_scene_batch(
            &inputs,
            &[
                original.clone(),
                crate::TriangleInstance {
                    geometry: crate::BatchBlas::Existing(old.blas.blases[0]),
                    custom_index: 1,
                    ..original
                },
            ],
        )?;
        drop(context);
        drop(owner);
        super::super::validation::finish()?;
        Ok(())
    }

    #[test]
    #[ignore = "需要真实 KHR RT Vulkan；主任务统一串行运行"]
    fn retired_backings_and_batch_temporaries_are_reused() -> Result<(), Error> {
        let mut context = pollster::block_on(crate::interop::WgpuContext::new())?;
        let triangles = [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]];
        let input = [TriangleBlasInput {
            triangles: &triangles,
            opaque: true,
            opacity: None,
        }];
        let first = context.triangle_blas_batch(&input)?;
        context.native().collect(&[])?;
        let idle = context.native_ref().build_pool_idle_bytes();
        assert!(idle > 0);
        assert!(context.allocated_native_bytes() >= idle);
        let second = context.triangle_blas_batch(&input)?;
        assert!(second.timing.pool_hits > 0);
        assert_eq!(second.timing.pool_misses, 0);
        assert_ne!(first.blases, second.blases);
        context.native().collect(&[])?;
        let pool = context.native_ref().as_pool_stats();
        assert!(pool.idle_bytes <= pool.idle_limit);
        assert_eq!(
            context.native_ref().build_pool_idle_bytes(),
            pool.idle_bytes
        );
        Ok(())
    }

    #[test]
    fn parallel_scratch_slices_are_aligned_and_disjoint() {
        let mut end = 0;
        for size in [1, 129, 0, 7, 4096] {
            let (offset, next) = scratch_range(end, size, 256).unwrap();
            assert!(offset >= end);
            assert_eq!(offset % 256, 0);
            assert!(next > offset);
            assert!(next - offset >= size);
            end = next;
        }
    }

    #[test]
    fn scratch_size_overflow_is_rejected() {
        assert!(scratch_range(u64::MAX, 1, 256).is_err());
        assert!(scratch_range(256, u64::MAX, 256).is_err());
    }

    #[test]
    fn borrowed_triangle_bytes_preserve_float_bits_and_order() {
        let triangles = [[
            [1.0, -0.0, f32::from_bits(0x7fc0_0123)],
            [-9.0, f32::INFINITY, 2.0],
            [0.0, f32::MIN_POSITIVE, 42.0],
        ]];
        let expected: Vec<_> = triangles
            .iter()
            .flatten()
            .flatten()
            .flat_map(|value| value.to_ne_bytes())
            .collect();
        assert_eq!(triangle_bytes(&triangles), expected);
    }
}
