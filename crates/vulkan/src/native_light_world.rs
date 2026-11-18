//! 独立区块子树与世界树。世界版本绑定实际设备地址及同一组生命周期边。
//! 依赖图从世界到区块，从区块到显存页。区块仅在最后一份使用记录完成后归还范围。
use super::{
    light_tree::{BUILD_PUSH_BYTES, Dispatch, light_power, light_source},
    *,
};
use crate::lifecycle::resource::WorldLights;

/// 世界目录中的一个非空区块。输入按网格坐标有序，相同坐标可用于不同 LOD。
#[derive(Clone, Copy)]
pub enum LightRegionInput {
    /// 沿用同一 Context 已提交或已完成的区块，记录类型保持一致。
    Reuse(LightRegion),
    /// 从本批叶表的下一段构建非空区块。顶点相对于其逐顶点区域坐标。
    Build {
        /// 以 64m 为单位的区块原点。
        grid: [i32; 3],
        /// 本区块连续的活动灯数。
        count: u32,
    },
}

/// 从区块发布本次世界版本。只有 `Build` 区块消耗输入叶、排序和写出记录。
pub struct LightWorldInput<'a> {
    /// 与场景 BLAS 共用的四顶点列，记录在之前的队列命令中发布。
    pub vertices: Buffer,
    /// 按 `regions` 中 Build 的顺序拼接的灯行，依次为 quad 槽、正功率 bits、材质或稳定槽、保留字。
    pub leaves: &'a [[u32; 4]],
    /// 本次世界的所有非空区块。移除区块直接从列表省略。
    pub regions: &'a [LightRegionInput],
    /// 每灯写局部四角及面积记录，关闭时仅写稳定灯槽。
    pub records: bool,
    /// 可选按预算传输的驻留输入，与 leaves 逐行一致并在录制和执行期间只读。
    pub source: Option<Buffer>,
}

// 确定 Slang 物理存储地址在标量布局中的位置。目录 first 是当前世界灯号，job 中是输入偏移。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RegionRow {
    address: u64,
    count: u32,
    first: u32,
    grid: [i32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Header {
    topology: u64,
    regions: u64,
    counts: [u32; 4],
    anchor: [i32; 4],
}

const _: () = {
    assert!(size_of::<Header>() == 48);
    assert!(size_of::<RegionRow>() == 32);
};

struct WorldBuild {
    world: LightWorld,
    groups: u32,
    dirty: u32,
    vertices: Buffer,
    sources: Buffer,
    jobs: Option<Buffer>,
    batches: Vec<(u32, u32, u32)>,
    scratch: Option<Buffer>,
    records: bool,
}

fn region_depth(count: u32) -> u32 {
    count.next_power_of_two().ilog2()
}

const fn region_bytes(count: u32, records: bool) -> u64 {
    let n = count as u64;
    let tree = 1 + (n.saturating_sub(1) * 10).next_multiple_of(8) / 4 + n.div_ceil(2);
    let payload = if records { n * 4 } else { n.div_ceil(4) };
    16 * (tree + payload)
}

impl LightWorld {
    pub(super) fn data(self, context: &Context) -> Result<&WorldLights<buffer::Buffer>, Error> {
        match context.get(self.key)? {
            Data::LightWorld(data) => Ok(data),
            _ => Err(Error::InvalidArgument("世界灯树标识类型无效")),
        }
    }

    /// 当前世界目录及其区块子树的逻辑字节数。不计在途版本及临时构建空间。
    pub const fn bytes(self) -> u64 {
        self.bytes
    }
}

impl LightRegion {
    /// 区块内的活动灯数量。
    pub const fn count(self) -> u32 {
        self.count
    }
}

fn world_buffer(
    context: &mut Context,
    regions: Vec<LightRegion>,
    directories: &[RegionRow],
    count: u32,
) -> Result<LightWorld, Error> {
    if !context.device.ray_pipeline {
        return Err(Error::Unsupported("世界灯树需要设备地址"));
    }
    let groups = directories.len();
    let top = size_of::<Header>() + size_of_val(directories);
    let len = top + 48 * groups.saturating_sub(1);
    if len as u64 > u64::from(context.max_storage_buffer_range()) {
        return Err(Error::InvalidArgument("世界灯树超过设备缓冲限制"));
    }
    let raw = buffer::Buffer::new(
        &context.device,
        len,
        vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        vk::MemoryPropertyFlags::DEVICE_LOCAL
            | vk::MemoryPropertyFlags::HOST_VISIBLE
            | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    let raw = Guard::new(&context.device, raw);
    let address = raw.address(&context.device);
    let header = Header {
        topology: address + top as u64,
        regions: address + size_of::<Header>() as u64,
        counts: [count, groups as u32, 0, 0],
        anchor: directories.first().map_or([0; 4], |row| row.grid),
    };
    raw.write_slices(
        &context.device,
        &[
            (0, bytemuck::bytes_of(&header)),
            (size_of::<Header>(), bytemuck::cast_slice(directories)),
        ],
    )?;
    let bytes = len as u64 + regions.iter().map(|region| region.bytes).sum::<u64>();
    let raw = raw.finish();
    let key = context.insert(Data::LightWorld(WorldLights {
        regions,
        producer: None,
        raw,
    }))?;
    Ok(LightWorld { key, bytes })
}

impl WorldBuild {
    fn new(context: &mut Context, input: LightWorldInput<'_>) -> Result<Self, Error> {
        if input.leaves.len() > 1 << 24 || input.regions.len() > 1 << 24 {
            return Err(Error::InvalidArgument("灯数超出24位采样域"));
        }
        context.active(input.vertices.key)?;
        let mut regions = Vec::new();
        let mut rows = Vec::new();
        let mut jobs = Vec::new();
        regions.try_reserve_exact(input.regions.len())?;
        rows.try_reserve_exact(input.regions.len())?;
        let mut first = 0u32;
        let mut count = 0u32;
        let mut power = 0.0;
        let mut pending_bytes = input
            .regions
            .iter()
            .filter_map(|entry| match *entry {
                LightRegionInput::Build { count, .. } => Some(region_bytes(count, input.records)),
                LightRegionInput::Reuse(_) => None,
            })
            .sum();
        for &entry in input.regions {
            let region = match entry {
                LightRegionInput::Reuse(region) => {
                    context.active(region.key)?;
                    if region.records != input.records {
                        return Err(Error::InvalidArgument("复用灯区块的记录布局不同"));
                    }
                    region
                }
                LightRegionInput::Build { grid, count } => {
                    if count == 0 || count as usize > input.leaves.len() - first as usize {
                        return Err(Error::InvalidArgument("构建区块须有连续非空叶段"));
                    }
                    let end = first + count;
                    let power =
                        light_power(input.vertices, &input.leaves[first as usize..end as usize])?;
                    let bytes = region_bytes(count, input.records);
                    let (key, address) = context.light_slice(bytes, pending_bytes)?;
                    pending_bytes -= bytes;
                    let [x, y, z] = grid;
                    jobs.try_reserve(1)?;
                    jobs.push(RegionRow {
                        address,
                        count,
                        first,
                        grid: [x, y, z, 0],
                    });
                    first = end;
                    LightRegion {
                        key,
                        address,
                        bytes,
                        grid,
                        count,
                        records: input.records,
                        power_bits: power.to_bits(),
                    }
                }
            };
            if regions
                .last()
                .is_some_and(|prev: &LightRegion| prev.grid > region.grid)
            {
                return Err(Error::InvalidArgument("世界灯区块须按网格坐标排列"));
            }
            let [x, y, z] = region.grid;
            rows.push(RegionRow {
                address: region.address,
                count: region.count,
                first: count,
                grid: [x, y, z, 0],
            });
            regions.push(region);
            count = count
                .checked_add(region.count)
                .filter(|&n| n <= 1 << 24)
                .ok_or(Error::InvalidArgument("灯数超出24位采样域"))?;
            power += f64::from_bits(region.power_bits);
        }
        if first as usize != input.leaves.len() || power > f64::from(f32::MAX) {
            return Err(Error::InvalidArgument("活动叶段长度或光源总功率无效"));
        }
        let sources = light_source(context, input.vertices, input.leaves, input.source)?;
        let mut batches = Vec::new();
        let mut scratch = None;
        let mut job_buffer = None;
        if !jobs.is_empty() {
            jobs.sort_unstable_by_key(|job| region_depth(job.count));
            let mut begin = 0;
            batches.try_reserve_exact(25)?;
            for same_depth in jobs.chunk_by(|a, b| region_depth(a.count) == region_depth(b.count)) {
                let n = same_depth.len() as u32;
                batches.push((begin, n, region_depth(same_depth[0].count)));
                begin += n;
            }
            let depth = batches.last().expect("非空任务有批次").2;
            if depth != 0 {
                scratch = Some(Buffer::new(
                    context,
                    u64::from(first) * if depth > 6 { 32 } else { 16 },
                    false,
                )?);
            }
            job_buffer = Some(Buffer::upload(context, bytemuck::cast_slice(&jobs))?);
        }
        Ok(Self {
            world: world_buffer(context, regions, &rows, count)?,
            groups: rows.len() as u32,
            dirty: first,
            vertices: input.vertices,
            sources,
            jobs: job_buffer,
            batches,
            scratch,
            records: input.records,
        })
    }

    fn roots(&self, commands: &mut Commands) -> Result<(), Error> {
        if let Some(jobs) = self.jobs {
            for buffer in [
                Some(self.vertices),
                Some(self.sources),
                Some(jobs),
                self.scratch,
            ]
            .into_iter()
            .flatten()
            {
                commands.access(buffer.into())?;
            }
        }
        Ok(())
    }

    fn record(&self, context: &Context, cmd: vk::CommandBuffer) -> Result<(), Error> {
        if self.jobs.is_none() && self.groups <= 1 {
            return Ok(());
        }
        let dispatch = Dispatch::new(context, cmd)?;
        let mut push = [0u8; BUILD_PUSH_BYTES];
        if let Some(jobs) = self.jobs {
            push[..8].copy_from_slice(&dispatch.address(self.vertices)?.to_ne_bytes());
            push[8..16].copy_from_slice(&dispatch.address(self.sources)?.to_ne_bytes());
            push[44..48].copy_from_slice(&u32::from(self.records).to_ne_bytes());
            push[60..64].copy_from_slice(&((self.vertices.len() / 64) as u32).to_ne_bytes());
            push[88..96].copy_from_slice(&dispatch.address(jobs)?.to_ne_bytes());
            push[116..120].copy_from_slice(&1u32.to_ne_bytes());
            for &(first, groups, depth) in &self.batches {
                push[104..108].copy_from_slice(&first.to_ne_bytes());
                dispatch.local(&mut push, groups, self.dirty, depth, self.scratch)?;
            }
        }
        if self.groups > 1 {
            let address = self.world.data(context)?.raw.address(&context.device);
            push[96..104].copy_from_slice(&address.to_ne_bytes());
            push[80..84].copy_from_slice(&3u32.to_ne_bytes());
            for level in (1..=region_depth(self.groups)).rev() {
                push[36..40].copy_from_slice(&level.to_ne_bytes());
                dispatch.run(&mut push, 1u32 << level);
            }
        }
        Ok(())
    }
}

impl Context {
    /// 创建空的已完成世界树，可用于无灯场景的绑定。
    /// # Errors
    /// 设备能力或分配失败时返回错误。
    pub fn empty_light_world(&mut self) -> Result<LightWorld, Error> {
        world_buffer(self, Vec::new(), &[], 0)
    }

    /// 从已提交或已完成的世界版本借用子树身份。按构建输入的顺序，供后续版本复用。
    /// # Errors
    /// 世界属于其他设备、尚未提交、取消或退役时返回错误。
    pub fn light_regions(&self, world: LightWorld) -> Result<&[LightRegion], Error> {
        self.active(world.key)?;
        self.producer_submitted(world.key)?;
        Ok(&world.data(self)?.regions)
    }

    /// 在原生队列发布世界树，仅重建本批指定的区块。提交保活所有引用的子树。
    /// # Errors
    /// 无效输入、容量、设备或提交失败保留已发布资源。
    pub fn enqueue_light_world(&mut self, input: LightWorldInput<'_>) -> Result<LightWorld, Error> {
        let build = WorldBuild::new(self, input)?;
        let raw = terrain_flight::Recording::record(&self.device, |cmd| build.record(self, cmd))?;
        self.submit_terrain(raw, Default::default(), vec![build.world.key], |op| {
            build.roots(op)
        })?;
        Ok(build.world)
    }

    /// 同步宿主的世界树发布入口。返回时本批区块和世界树均可读取。
    /// # Errors
    /// 无效输入、容量或底层设备失败时返回错误。
    pub fn build_light_world(&mut self, input: LightWorldInput<'_>) -> Result<LightWorld, Error> {
        let build = WorldBuild::new(self, input)?;
        immediate(&self.device, |cmd| build.record(self, cmd))?;
        Ok(build.world)
    }
}

impl WgpuContext {
    /// 录制区块与世界树的同批更新，按顶点生成命令、世界构建、世界消费者的顺序提交。
    /// 绑定和命令的依赖边同时保活旧的世界版本及所引用的独立区块。
    /// # Errors
    /// 无效输入、容量或设备录制失败时返回错误，旧版本仍可用。
    pub fn record_light_world(
        &mut self,
        input: LightWorldInput<'_>,
    ) -> Result<(LightWorld, crate::interop::CommandBuffer), Error> {
        let context = self.native();
        let build = WorldBuild::new(context, input)?;
        let raw = crate::interop::record(&context.device, "build light world", |cmd| {
            build.record(context, cmd)
        })?;
        let mut op = Commands::new(context)?;
        build.roots(&mut op)?;
        let record = op.finish_build(context, vec![build.world.key])?;
        Ok((
            build.world,
            crate::interop::CommandBuffer::recorded(raw, record.key),
        ))
    }
}
