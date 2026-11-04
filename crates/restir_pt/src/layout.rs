use crate::Error;

/// 空间replay/shift的调度粒度；所有方式保留paired跨像素缓存。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SpatialFusion {
    /// 原 Compact 重放和独立 shift。
    #[default]
    None,
    /// 同一 invocation 顺序完成一个像素的三个方向。
    Pixel,
    /// 单独生成几何 mask，每个方向独立完成 replay 和 shift。
    Directions,
}

/// 相同采样算法的阶段融合；各开关仅改变调度和中间状态的保存边界。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Fusion {
    /// primary 命中直接交给初采。
    pub primary_initial: bool,
    /// 同一像素内完成双向 temporal replay、shift 和 MIS。
    pub temporal: bool,
    /// 两方向独立完成replay/shift，再由compute合并；与temporal互斥。
    pub temporal_split: bool,
    /// 空间replay/shift的调度粒度；保留paired缓存。
    pub spatial: SpatialFusion,
    /// 最终空间 reservoir 直接参与输出和历史保存。
    pub merge_resolve: bool,
}

impl Fusion {
    /// NV 分阶段参考调度。
    pub const NONE: Self = Self {
        primary_initial: false,
        temporal: false,
        temporal_split: false,
        spatial: SpatialFusion::None,
        merge_resolve: false,
    };
    /// 只融合首尾，保留参考调度的Compact重放与独立shift。
    pub const ENDS: Self = Self {
        primary_initial: true,
        merge_resolve: true,
        ..Self::NONE
    };
    /// 首尾融合，temporal双向独立重放/shift后合并，空间保留Compact调度。
    pub const SPLIT: Self = Self {
        temporal_split: true,
        ..Self::ENDS
    };
    /// temporal与空间均按方向独立重放/shift，仅保留跨像素paired缓存。
    pub const DIRECTIONS: Self = Self {
        spatial: SpatialFusion::Directions,
        ..Self::SPLIT
    };
    /// 初采、重放/shift和输出均融合；空间方向独立并行，保留paired跨像素同步。
    pub const ALL: Self = Self {
        primary_initial: true,
        temporal: true,
        temporal_split: false,
        spatial: SpatialFusion::Directions,
        merge_resolve: true,
    };

    pub(crate) const fn needs_replay_storage(self) -> bool {
        !(self.temporal || self.temporal_split) || matches!(self.spatial, SpatialFusion::None)
    }
}

/// 16B 场景交点身份；全零无效，其余编码和重建由 scene 维护。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(transparent)]
pub struct HitInfo {
    /// 四个原位标量，不包含宿主指针。
    pub data: [u32; 4],
}

/// NV 未压缩 point reservoir 的 80B scalar ABI；不包含 area film/lens 样本。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct PathReservoir {
    /// 候选或历史有效量。
    pub m: f32,
    /// 构建时为权重和，finalize 后为 W。
    pub weight: f32,
    /// PSS 测度下的完整 integrand。
    pub integrand: [f32; 3],
    /// 原位 ReSTIRPathFlags bits。
    pub path_flags: u32,
    /// 确定性路径重放的起始随机种子。
    pub init_random_seed: u32,
    /// reconnection 后一 bounce 的随机种子。
    pub rc_next_vertex_bounce_seed: u32,
    /// 被选 NEE 光源的密度。
    pub light_pdf: f32,
    /// reconnection 顶点身份。
    pub reconnection_hit: HitInfo,
    /// 被选路径的 reconnection Jacobian。
    pub reconnection_jacobian: f32,
    /// reconnection 入射辐亮度。
    pub reconnection_incident_radiance: [f32; 3],
    /// reconnection 入射方向；不压缩。
    pub reconnection_wi: [f32; 3],
}

/// 未压缩重放结果的 44B scalar ABI。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Reconnection {
    /// reconnection 前一个顶点。
    pub previous_hit: HitInfo,
    /// 重放前缀的 throughput。
    pub path_throughput: [f32; 3],
    /// 前一个顶点的出射方向。
    pub previous_wo: [f32; 3],
    /// Hybrid 可逆性检验的 footprint 因子。
    pub footprint_factor: f32,
}

/// paired 方向复用的 20B scalar ABI。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct PrecomputedShift {
    /// shift 后 integrand。
    pub path_throughput: [f32; 3],
    /// shift Jacobian。
    pub jacobian: f32,
    /// 源 reservoir 的 reconnection Jacobian。
    pub reconnection_jacobian: f32,
}

/// 三邻居有效位与 pairwise MIS 总 M，8B scalar ABI。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct NeighborMask {
    /// bit i 对应邻居 i。
    pub valid: u32,
    /// canonical 与有效邻居 M 之和。
    pub c_sum: f32,
}

/// NV compact queue 的 12bit x、12bit y、8bit retraceIndex 编码。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct WorkItem(u32);

impl WorkItem {
    /// 编码本 profile 的六个有向候选之一，不接受截断坐标。
    pub const fn new(pixel: [u32; 2], direction: u8) -> Result<Self, Error> {
        if pixel[0] >= 4096 || pixel[1] >= 4096 || direction >= 6 {
            return Err(Error::WorkItem);
        }
        Ok(Self(pixel[0] | pixel[1] << 12 | (direction as u32) << 24))
    }
    /// 原位 queue 标量。
    pub const fn bits(self) -> u32 {
        self.0
    }
    /// 原始屏幕像素坐标。
    pub const fn pixel(self) -> [u32; 2] {
        [self.0 & 0xfff, self.0 >> 12 & 0xfff]
    }
    /// retraceIndex；0/1 是 temporal，0..5 是 spatial 的方向槽。
    pub const fn direction(self) -> u8 {
        (self.0 >> 24) as u8
    }
}

/// 每个物理 buffer 的请求字节；同类双缓冲单独计数，total 已包含两份。
/// N为padded Morton槽数；Compact的Q=3N、416N+4字节，两类重放均融合时Q=0、260N字节。
/// 可选独立颜色估计另加24N字节，不改变reservoir ABI。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Memory {
    /// 单份 N 个 reservoir。
    pub reservoir: u64,
    /// 单份 N 个 HitInfo；本实现按 padded N，NV 原图像通常按实际像素 P。
    pub primary_hit: u64,
    /// 单份N个RGB颜色估计，12N字节；未启用独立颜色存储时为0。
    pub color_estimate: u64,
    /// Q 个重放结果；两类重放均融合时为0。
    pub reconnection: u64,
    /// Q 个结果偏移。
    pub reconnection_offsets: u64,
    /// N 个有效 mask 和 cSum。
    pub neighbor_mask: u64,
    /// Q 个原位工作项。
    pub work_items: u64,
    /// 3N 个 paired shift。
    pub precomputed: u64,
    /// 分阶段queue的单个u32 counter；两类重放均融合时为0。
    pub counter: u64,
    /// 算法资源总请求量；不含 scene、LUT、输出、驱动分配对齐和在途旧尺寸资源。
    pub total: u64,
}

/// 已校验的屏幕布局：16×16 tile 内 Morton，tile 本身按行排列。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Layout {
    extent: [u32; 2],
    tiles: [u32; 2],
    pixels: u32,
    elements: u32,
    queue_capacity: u32,
    memory: Memory,
    fusion: Fusion,
}

impl Layout {
    /// 校验NV工作项坐标域、数量及raw buffer的u32字节寻址；设备分配上限另在GPU边界检查。
    pub const fn new(extent: [u32; 2]) -> Result<Self, Error> {
        Self::with_fusion(extent, Fusion::NONE)
    }

    /// 按融合边界计算工作集；temporal 与 spatial 均融合时不分配重放缓存或队列。
    /// 不分配独立颜色估计；尺寸受12bit坐标域和各raw buffer的u32字节寻址约束。
    pub const fn with_fusion(extent: [u32; 2], fusion: Fusion) -> Result<Self, Error> {
        Self::with_resources(extent, fusion, false)
    }

    /// 额外分配独立于赢家路径的RGB估计，A为initial/temporal，B为空间输出与历史。
    /// 两份均按padded N分配；颜色历史有效性和是否计算由宿主维护，不随路径选择复制。
    pub const fn with_color_estimates(extent: [u32; 2], fusion: Fusion) -> Result<Self, Error> {
        Self::with_resources(extent, fusion, true)
    }

    const fn with_resources(
        extent: [u32; 2],
        fusion: Fusion,
        color_estimates: bool,
    ) -> Result<Self, Error> {
        if extent[0] == 0 || extent[1] == 0 || extent[0] > 4096 || extent[1] > 4096 {
            return Err(Error::Extent);
        }
        if fusion.temporal && fusion.temporal_split {
            return Err(Error::TemporalFusion);
        }
        let tiles = [extent[0].div_ceil(16), extent[1].div_ceil(16)];
        let Some(pixels) = extent[0].checked_mul(extent[1]) else {
            return Err(Error::Overflow);
        };
        let Some(tile_count) = tiles[0].checked_mul(tiles[1]) else {
            return Err(Error::Overflow);
        };
        let Some(elements) = tile_count.checked_mul(256) else {
            return Err(Error::Overflow);
        };
        let Some(queue_capacity) =
            elements.checked_mul(if fusion.needs_replay_storage() { 3 } else { 0 })
        else {
            return Err(Error::Overflow);
        };
        let n = elements as u64;
        let q = queue_capacity as u64;
        let Some(reservoir) = n.checked_mul(80) else {
            return Err(Error::Overflow);
        };
        let Some(primary_hit) = n.checked_mul(16) else {
            return Err(Error::Overflow);
        };
        let Some(color_estimate) = n.checked_mul(if color_estimates { 12 } else { 0 }) else {
            return Err(Error::Overflow);
        };
        let Some(reconnection) = q.checked_mul(44) else {
            return Err(Error::Overflow);
        };
        let Some(reconnection_offsets) = q.checked_mul(4) else {
            return Err(Error::Overflow);
        };
        let Some(neighbor_mask) = n.checked_mul(8) else {
            return Err(Error::Overflow);
        };
        let Some(work_items) = q.checked_mul(4) else {
            return Err(Error::Overflow);
        };
        let Some(precomputed) = n.checked_mul(60) else {
            return Err(Error::Overflow);
        };
        let sizes = [
            reservoir,
            reservoir,
            primary_hit,
            primary_hit,
            color_estimate,
            color_estimate,
            reconnection,
            reconnection_offsets,
            neighbor_mask,
            work_items,
            precomputed,
            if fusion.needs_replay_storage() { 4 } else { 0 },
        ];
        let mut total = 0_u64;
        let mut index = 0;
        while index < sizes.len() {
            if sizes[index] > u32::MAX as u64 {
                return Err(Error::Overflow);
            }
            let Some(next) = total.checked_add(sizes[index]) else {
                return Err(Error::Overflow);
            };
            total = next;
            index += 1;
        }
        Ok(Self {
            extent,
            tiles,
            pixels,
            elements,
            queue_capacity,
            memory: Memory {
                reservoir,
                primary_hit,
                color_estimate,
                reconnection,
                reconnection_offsets,
                neighbor_mask,
                work_items,
                precomputed,
                counter: if fusion.needs_replay_storage() { 4 } else { 0 },
                total,
            },
            fusion,
        })
    }

    /// 此工作集所支持的实际调度；重放缓存的存在由该值唯一决定。
    pub const fn fusion(self) -> Fusion {
        self.fusion
    }

    /// 是否包含独立RGB估计的双缓冲；resize须保持该资源边界。
    pub const fn has_color_estimates(self) -> bool {
        self.memory.color_estimate != 0
    }

    /// 实际视图尺寸，所有 shader 必须对 padding lane 做边界判断。
    pub const fn extent(self) -> [u32; 2] {
        self.extent
    }
    /// 二维 tile 数，也是 256×1 workload kernel 的 dispatch group 数。
    pub const fn tiles(self) -> [u32; 2] {
        self.tiles
    }
    /// 实际像素数 P。
    pub const fn pixels(self) -> u32 {
        self.pixels
    }
    /// padded reservoir 数 N，不等于实际像素数 P。
    pub const fn elements(self) -> u32 {
        self.elements
    }
    /// 最坏Compact容量Q=3N；每像素至多三个空间方向，两阶段清零并复用队列。
    /// 两类重放均融合时为0，不代表实际任务数。
    pub const fn queue_capacity(self) -> u32 {
        self.queue_capacity
    }
    /// 每 buffer 和总请求字节。
    pub const fn memory(self) -> Memory {
        self.memory
    }
    /// 有效屏幕像素的 Morton reservoir offset；padding 或越界输入返回 None。
    pub const fn reservoir_offset(self, pixel: [u32; 2]) -> Option<u32> {
        if pixel[0] >= self.extent[0] || pixel[1] >= self.extent[1] {
            return None;
        }
        let tile = (pixel[1] >> 4) * self.tiles[0] + (pixel[0] >> 4);
        let mut local = 0;
        let mut bit = 0;
        while bit < 4 {
            local |= ((pixel[0] >> bit) & 1) << (2 * bit);
            local |= ((pixel[1] >> bit) & 1) << (2 * bit + 1);
            bit += 1;
        }
        Some(tile * 256 + local)
    }

    /// 按融合模式给出默认launch；Programs可选择等数量的RT域变形。
    /// Compact按二维raster线性化，temporal独立方向用z。
    pub const fn dispatch(self, stage: Stage) -> Dispatch {
        match stage {
            Stage::TemporalWork | Stage::SpatialWork | Stage::SpatialMerge | Stage::Resolve => {
                Dispatch::Compute([self.tiles[0], self.tiles[1], 1])
            }
            Stage::TemporalReplay if self.fusion.temporal_split => {
                Dispatch::Rays([self.extent[0], self.extent[1], 2])
            }
            Stage::TemporalReplay => Dispatch::Rays([2 * self.extent[0], self.extent[1], 1]),
            Stage::TemporalMerge if self.fusion.temporal_split => {
                Dispatch::Compute([self.tiles[0], self.tiles[1], 1])
            }
            Stage::SpatialReplay => Dispatch::Rays([3 * self.extent[0], self.extent[1], 1]),
            Stage::SpatialPrecompute
                if matches!(self.fusion.spatial, SpatialFusion::Directions) =>
            {
                Dispatch::Rays([3 * self.extent[0], self.extent[1], 1])
            }
            _ => Dispatch::Rays([self.extent[0], self.extent[1], 1]),
        }
    }
}

/// point profile 的十个逻辑计时槽；融合入口复用对应槽位，按实际读写依赖排列。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    /// 仅追踪primary，写current HitInfo，不调用材质callable。
    Primary,
    /// 生成A与L0；融合时同时追踪并保存primary。
    Initial,
    /// Compact temporal queue，组大小 256×1×1。
    TemporalWork,
    /// 最多2P个有向重放；split时按z=0/1独立完成replay与shift。
    TemporalReplay,
    /// 双向shift/MIS；temporal融合时含两方向replay，split时为16×16纯compute合并。
    TemporalMerge,
    /// Compact paired spatial queue，组大小 256×1×1。
    SpatialWork,
    /// 最多3P个空间重放项，仍用六方向编码；shader按counter early-out。
    SpatialReplay,
    /// 三个paired方向的shift；融合时同时完成replay，可按像素或方向并行。
    SpatialPrecompute,
    /// 从A合并写B，16×16×1；融合时直接完成输出和primary历史保存。
    SpatialMerge,
    /// 读取 B 输出贡献并保存下帧 primary history，组大小 16×16×1。
    Resolve,
}

/// Vulkan launch 数量；Compute 保存 workgroup 数，Rays 保存 invocation 尺寸。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dispatch {
    /// 由 stage 固定的 compute 组大小解释。
    Compute([u32; 3]),
    /// Compact索引为x+DispatchRaysDimensions.x*y；独立方向另用z。
    Rays([u32; 3]),
}

impl Stage {
    /// 当前历史和融合配置下是否实际执行；未执行阶段不需要 dispatch 或 barrier。
    pub const fn enabled(self, history_valid: bool, fusion: Fusion) -> bool {
        match self {
            Self::Primary => !fusion.primary_initial,
            Self::Initial | Self::SpatialPrecompute | Self::SpatialMerge => true,
            Self::TemporalWork => history_valid && !(fusion.temporal || fusion.temporal_split),
            Self::TemporalReplay => history_valid && !fusion.temporal,
            Self::TemporalMerge => history_valid,
            Self::SpatialWork => !matches!(fusion.spatial, SpatialFusion::Pixel),
            Self::SpatialReplay => matches!(fusion.spatial, SpatialFusion::None),
            Self::Resolve => !fusion.merge_resolve,
        }
    }

    /// 分阶段逻辑顺序；无历史时省略temporal，融合调度另用enabled筛选。
    pub fn sequence(history_valid: bool) -> impl Iterator<Item = Self> {
        [
            Self::Primary,
            Self::Initial,
            Self::TemporalWork,
            Self::TemporalReplay,
            Self::TemporalMerge,
            Self::SpatialWork,
            Self::SpatialReplay,
            Self::SpatialPrecompute,
            Self::SpatialMerge,
            Self::Resolve,
        ]
        .into_iter()
        .filter(move |stage| {
            history_valid
                || !matches!(
                    stage,
                    Self::TemporalWork | Self::TemporalReplay | Self::TemporalMerge
                )
        })
    }
}
