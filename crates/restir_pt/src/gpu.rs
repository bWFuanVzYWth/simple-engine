//! 原生阶段程序与共享算法资源；唯一 Context 管理在途版本，录制不提交或等待。

use crate::{
    Dispatch, FrameParams, FrameState, FrameToken, Fusion, Layout, Settings, SpatialFusion, Stage,
};
use vulkan::{
    interop::{RayShader, SharedBuffer, WgpuContext},
    native::{
        Bindings, Buffer, Commands, ComputePipeline, Context, RayStackSizes, RayTracingPipeline,
        Recorded, Resource, SubgroupProperties, TimestampQueries,
    },
};

/// 资源、程序或帧状态准备失败；底层错误保留来源。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 布局、设置或帧事务无效。
    #[error(transparent)]
    Input(#[from] crate::Error),
    /// Vulkan 或 wgpu 资源和录制失败。
    #[error(transparent)]
    Vulkan(#[from] vulkan::Error),
    /// 任一物理算法 buffer 超过设备的 storage 或 buffer 上限。
    #[error("ReSTIR PT 算法 buffer 超过设备容量")]
    Capacity,
    /// 融合工作集缺少对应受控 shader。
    #[error("ReSTIR PT 融合调度缺少程序")]
    MissingFusionPrograms,
    /// resize 的工作集融合边界与已创建管线不同。
    #[error("ReSTIR PT resize须保持融合调度")]
    FusionMismatch,
    /// resize改变了程序使用的独立颜色存储边界。
    #[error("ReSTIR PT resize须保持颜色估计存储")]
    ColorEstimateMismatch,
    /// 重绑定来自另一 renderer，或其基础版本已经被替换。
    #[error("ReSTIR PT 重绑定事务已失效")]
    StaleRebinding,
    /// workload要求32-lane subgroup；设备规模控制或compute操作不满足内核前提。
    #[error(
        "ReSTIR PT workload不支持该subgroup：{0:?}；需要可请求size32及compute BASIC/BALLOT/ARITHMETIC"
    )]
    UnsupportedSubgroup(SubgroupProperties),
}

/// 算法工作集；默认使用 wgpu SharedBuffer，原生入口使用 Context 拥有的 Buffer 标识。
/// 两类存储复用相同布局与阶段；不得显式 destroy 借出的 wgpu buffer。
/// 大小按 padded N，两个 primary buffer 也各为16N；发布绑定前保留原生 [`Self::roots`]。
pub struct Buffers<B = SharedBuffer> {
    layout: Layout,
    reservoirs: [B; 2],
    primary_hits: [B; 2],
    color_estimates: Option<[B; 2]>,
    replay: Option<ReplayBuffers<B>>,
    neighbor_mask: B,
    precomputed: B,
    native_roots: Vec<Buffer>,
}

struct ReplayBuffers<B> {
    reconnection: B,
    offsets: B,
    items: B,
    counter: Buffer,
}

impl Buffers<SharedBuffer> {
    /// 创建完整工作集；SharedBuffer 初始化由同队列清零，不产生等长CPU零数组或host wait。
    /// 分配初始化可以提交清零，逐帧 Renderer::record 不提交。native counter 在每次生成队列前 GPU 清零。
    /// 分配失败时已创建共享 handle 自动释放；native 资源仍按 Context collect 契约管理。
    pub fn new(context: &mut WgpuContext, layout: Layout) -> Result<Self, Error> {
        let memory = layout.memory();
        let limits = context.device().limits();
        for bytes in [
            memory.reservoir,
            memory.primary_hit,
            memory.color_estimate,
            memory.reconnection,
            memory.reconnection_offsets,
            memory.neighbor_mask,
            memory.work_items,
            memory.precomputed,
        ] {
            if bytes > limits.max_buffer_size || bytes > limits.max_storage_buffer_binding_size {
                return Err(Error::Capacity);
            }
        }
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let reservoirs = [
            context.zeroed_buffer("ReSTIR initial/temporal A", memory.reservoir, usage)?,
            context.zeroed_buffer("ReSTIR spatial/history B", memory.reservoir, usage)?,
        ];
        let primary_hits = [
            context.zeroed_buffer("ReSTIR current primary", memory.primary_hit, usage)?,
            context.zeroed_buffer("ReSTIR history primary", memory.primary_hit, usage)?,
        ];
        let color_estimates = if layout.has_color_estimates() {
            Some([
                context.zeroed_buffer(
                    "ReSTIR color initial/temporal A",
                    memory.color_estimate,
                    usage,
                )?,
                context.zeroed_buffer(
                    "ReSTIR color spatial/history B",
                    memory.color_estimate,
                    usage,
                )?,
            ])
        } else {
            None
        };
        let replay = if layout.fusion().needs_replay_storage() {
            Some(ReplayBuffers {
                reconnection: context.zeroed_buffer(
                    "ReSTIR reconnection",
                    memory.reconnection,
                    usage,
                )?,
                offsets: context.zeroed_buffer(
                    "ReSTIR reconnection offsets",
                    memory.reconnection_offsets,
                    usage,
                )?,
                items: context.zeroed_buffer("ReSTIR compact queue", memory.work_items, usage)?,
                counter: Buffer::new(context.native(), memory.counter, false)?,
            })
        } else {
            None
        };
        let neighbor_mask =
            context.zeroed_buffer("ReSTIR neighbor masks", memory.neighbor_mask, usage)?;
        let precomputed =
            context.zeroed_buffer("ReSTIR paired shifts", memory.precomputed, usage)?;
        Ok(Self {
            layout,
            reservoirs,
            primary_hits,
            color_estimates,
            replay,
            neighbor_mask,
            precomputed,
            native_roots: Vec::new(),
        })
    }
}

impl<B> Buffers<B> {
    /// 唯一已校验布局。
    pub const fn layout(&self) -> Layout {
        self.layout
    }
    /// A为initial/temporal结果，B为空间输出与下一帧历史。
    pub const fn reservoirs(&self) -> &[B; 2] {
        &self.reservoirs
    }
    /// current/history primary HitInfo，各按 padded N 分配。
    pub const fn primary_hits(&self) -> &[B; 2] {
        &self.primary_hits
    }
    /// 可选独立RGB估计，各为12N字节；A为initial/temporal，B为空间输出与历史。
    /// 清零只初始化资源，不代表颜色历史有效；宿主须独立维护其有效性。
    pub const fn color_estimates(&self) -> Option<&[B; 2]> {
        self.color_estimates.as_ref()
    }
    /// 分阶段重放的44B记录；两类重放均融合时不存在。
    pub fn reconnection(&self) -> Option<&B> {
        self.replay.as_ref().map(|replay| &replay.reconnection)
    }
    /// Q个u32重放结果索引。
    pub fn reconnection_offsets(&self) -> Option<&B> {
        self.replay.as_ref().map(|replay| &replay.offsets)
    }
    /// N个8B neighbor mask/cSum。
    pub const fn neighbor_mask(&self) -> &B {
        &self.neighbor_mask
    }
    /// Q个12+12+8编码工作项。
    pub fn work_items(&self) -> Option<&B> {
        self.replay.as_ref().map(|replay| &replay.items)
    }
    /// 3N个20B paired shift结果。
    pub const fn precomputed(&self) -> &B {
        &self.precomputed
    }
    /// Compact 工作集的原生4B counter，需用 Binding::StorageBuffer 绑定。
    pub fn counter(&self) -> Option<Buffer> {
        self.replay.as_ref().map(|replay| replay.counter)
    }
    /// 保留全部原生工作集；默认共享存储只需额外保留 counter。
    /// SharedBuffer 自身的有效期由其所有者维护，不由此迭代器延长。
    pub fn roots(&self) -> impl Iterator<Item = Resource> {
        self.native_roots
            .iter()
            .copied()
            .map(Resource::from)
            .chain(self.counter().map(Resource::from))
    }
    /// 本对象请求的全部算法字节，不含分配对齐、LUT、场景或旧在途资源。
    pub const fn requested_bytes(&self) -> u64 {
        self.layout.memory().total
    }
}

impl Buffers<Buffer> {
    /// 为外部原生设备分配同一算法工作集；全部资源归 Context，按已有完成证明回收。
    /// 在准备边界用一次 GPU 清零提交初始化；不创建等长 CPU 快照，调用期间独占队列。
    pub fn new_native(context: &mut Context, layout: Layout) -> Result<Self, Error> {
        let memory = layout.memory();
        let reservoirs = [
            Buffer::new(context, memory.reservoir, false)?,
            Buffer::new(context, memory.reservoir, false)?,
        ];
        let primary_hits = [
            Buffer::new(context, memory.primary_hit, false)?,
            Buffer::new(context, memory.primary_hit, false)?,
        ];
        let color_estimates = if layout.has_color_estimates() {
            Some([
                Buffer::new(context, memory.color_estimate, false)?,
                Buffer::new(context, memory.color_estimate, false)?,
            ])
        } else {
            None
        };
        let replay = if layout.fusion().needs_replay_storage() {
            Some(ReplayBuffers {
                reconnection: Buffer::new(context, memory.reconnection, false)?,
                offsets: Buffer::new(context, memory.reconnection_offsets, false)?,
                items: Buffer::new(context, memory.work_items, false)?,
                counter: Buffer::new(context, memory.counter, false)?,
            })
        } else {
            None
        };
        let mut result = Self {
            layout,
            reservoirs,
            primary_hits,
            color_estimates,
            replay,
            neighbor_mask: Buffer::new(context, memory.neighbor_mask, false)?,
            precomputed: Buffer::new(context, memory.precomputed, false)?,
            native_roots: Vec::new(),
        };
        result.native_roots.extend(
            result
                .reservoirs
                .iter()
                .chain(&result.primary_hits)
                .chain(result.color_estimates.iter().flatten())
                .chain([&result.neighbor_mask, &result.precomputed])
                .copied(),
        );
        if let Some(replay) = &result.replay {
            result
                .native_roots
                .extend([replay.reconnection, replay.offsets, replay.items]);
        }
        let mut initialize = Commands::new(context)?;
        for buffer in result
            .reservoirs
            .iter()
            .chain(&result.primary_hits)
            .chain(result.color_estimates.iter().flatten())
            .chain([&result.neighbor_mask, &result.precomputed])
        {
            initialize.clear_buffer(*buffer)?;
        }
        if let Some(replay) = &result.replay {
            for buffer in [
                replay.reconnection,
                replay.offsets,
                replay.items,
                replay.counter,
            ] {
                initialize.clear_buffer(buffer)?;
            }
        }
        initialize.barrier();
        initialize.finish(context)?.submit_and_wait(context)?;
        Ok(result)
    }
}

/// RT launch形状；shader必须采用对应的像素和方向解码契约。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RayLaunch {
    /// 使用Layout给定的二维域。
    #[default]
    Raster,
    /// 将每个RT的`[x,y,z]`改为`[x*y,1,z]`；全部raygen须恢复线性像素编号。
    Linear,
    /// 仅空间逐方向RT改为`[width,height,3]`；slot由LaunchId.z给出。
    SpatialDirectionsZ,
}

/// 参考调度的十阶段程序及可选融合入口；只创建实际启用管线，材质callable保持独立叶子。
/// raygen重放按 x + DispatchRaysDimensions.x*y 读取compact queue，compute组尺寸见Stage。
#[derive(Clone, Copy)]
pub struct Programs<'a> {
    /// 仅改变RT域形状，invocation数量与compute不变；须匹配全部入口的解码方式。
    pub ray_launch: RayLaunch,
    /// 可选融合入口；启用任一融合时必须提供，各阶段链接自身的控制流叶子。
    pub fused: Option<FusedPrograms<'a>>,
    /// 仅追踪primary并写current HitInfo的raygen，不使用SER或callable。
    pub primary: RayShader<'a>,
    /// 读取current primary并追踪完整初始候选，写A与L0。
    pub initial: RayShader<'a>,
    /// 256×1×1 temporal workload compute，固定32-lane subgroup。
    pub temporal_work: RayShader<'a>,
    /// 两方向 temporal replay raygen。
    pub temporal_replay: RayShader<'a>,
    /// 双向shift/MIS temporal merge raygen。
    pub temporal_merge: RayShader<'a>,
    /// 256×1×1 spatial workload compute，固定32-lane subgroup。
    pub spatial_work: RayShader<'a>,
    /// paired spatial replay raygen。
    pub spatial_replay: RayShader<'a>,
    /// paired shift预计算raygen。
    pub spatial_precompute: RayShader<'a>,
    /// 16×16×1 spatial merge compute，A读/B写。
    pub spatial_merge: RayShader<'a>,
    /// 16×16×1 resolve compute，同时更新primary history。
    pub resolve: RayShader<'a>,
    /// 场景miss。
    pub miss: RayShader<'a>,
    /// 场景紧凑命中重建。
    pub closest_hit: RayShader<'a>,
    /// 初采新命中的材质closest-hit；追加在几何命中记录之后。
    /// 已知表面的重放和求值继续使用独立callable表，不链接这些命中程序。
    pub initial_hits: &'a [RayShader<'a>],
    /// 可选主查询alpha过滤；RayQuery覆盖规则仍由场景维护。
    pub any_hit: Option<RayShader<'a>>,
    /// 初采控制流叶子；数量可独立于重放/求值，索引及类型由对应raygen的构建契约保证。
    /// 使用非空 initial_hits 时允许为空，避免链接不会调用的初采 callable。
    pub initial_callables: &'a [RayShader<'a>],
    /// 两个replay阶段共用的Flow叶子。
    pub replay_callables: &'a [RayShader<'a>],
    /// temporal merge与spatial precompute共用的evaluate叶子。
    pub evaluate_callables: &'a [RayShader<'a>],
}

/// initial独立链接初采叶子；重用SBT以replay开头，随后为evaluate或完整重连叶子。
/// 独立重放/求值Flow数相同；初采与完整重连可扩展控制流特化，索引及payload由着色器契约保证。
#[derive(Clone, Copy)]
pub struct FusedPrograms<'a> {
    /// 可选完整重连叶子；空切片沿用独立evaluate。
    /// 非空须按单侧F、双侧F×F排列；融合SBT在它们之前保留F个replay。
    /// 对应raygen须使用同一索引和payload契约，不与独立evaluate入口混用。
    pub reconnection_callables: &'a [RayShader<'a>],
    /// primary 与初采。
    pub initial: RayShader<'a>,
    /// 双向 temporal replay、shift 和 MIS。
    pub temporal: RayShader<'a>,
    /// 两方向独立replay/shift，按`[width,height,2]`录制，写现有paired缓存的前2N槽。
    pub temporal_direction: RayShader<'a>,
    /// 读取双向shift后按原RNG顺序合并temporal；16×16×1 compute。
    pub temporal_combine: RayShader<'a>,
    /// 空间几何准入、三方向 replay 和 shift。
    pub spatial: RayShader<'a>,
    /// 方向并行融合之前的几何mask/cSum；固定32-lane subgroup，不生成Compact队列。
    pub spatial_work: RayShader<'a>,
    /// 按(pixel,slot)独立完成replay和shift。
    pub spatial_direction: RayShader<'a>,
    /// 空间合并、输出和 primary 历史保存。
    pub merge: RayShader<'a>,
}

struct Pipelines {
    ray_launch: RayLaunch,
    primary: Option<RayTracingPipeline>,
    initial: RayTracingPipeline,
    temporal_work: Option<ComputePipeline>,
    temporal_replay: Option<RayTracingPipeline>,
    temporal_merge: TemporalPipeline,
    spatial_work: Option<ComputePipeline>,
    spatial_replay: Option<RayTracingPipeline>,
    spatial_precompute: RayTracingPipeline,
    spatial_merge: ComputePipeline,
    resolve: Option<ComputePipeline>,
}

enum TemporalPipeline {
    Ray(RayTracingPipeline),
    Compute(ComputePipeline),
}

impl TemporalPipeline {
    fn program_root(&self, context: &Context) -> Result<Resource, vulkan::Error> {
        match self {
            Self::Ray(pipeline) => pipeline.program_root(context),
            Self::Compute(pipeline) => pipeline.program_root(context),
        }
    }

    fn root(&self) -> Resource {
        match self {
            Self::Ray(pipeline) => (*pipeline).into(),
            Self::Compute(pipeline) => (*pipeline).into(),
        }
    }

    fn rebind(&self, context: &mut Context, bindings: Bindings) -> Result<Self, vulkan::Error> {
        match self {
            Self::Ray(pipeline) => pipeline.rebind(context, bindings).map(Self::Ray),
            Self::Compute(pipeline) => pipeline.rebind(context, bindings).map(Self::Compute),
        }
    }

    fn record(&self, commands: &mut Commands, dispatch: Dispatch) -> Result<(), vulkan::Error> {
        match (self, dispatch) {
            (Self::Ray(pipeline), Dispatch::Rays(size)) => commands.trace_3d(*pipeline, size),
            (Self::Compute(pipeline), Dispatch::Compute(groups)) => {
                commands.dispatch(*pipeline, groups)
            }
            _ => unreachable!("管线与dispatch由相同Layout构造"),
        }
    }
}

impl Pipelines {
    fn ray_programs(&self) -> [(Stage, Option<RayTracingPipeline>); 6] {
        let temporal_merge = match &self.temporal_merge {
            TemporalPipeline::Ray(pipeline) => Some(*pipeline),
            TemporalPipeline::Compute(_) => None,
        };
        [
            (Stage::Primary, self.primary),
            (Stage::Initial, Some(self.initial)),
            (Stage::TemporalReplay, self.temporal_replay),
            (Stage::TemporalMerge, temporal_merge),
            (Stage::SpatialReplay, self.spatial_replay),
            (Stage::SpatialPrecompute, Some(self.spatial_precompute)),
        ]
    }

    fn program_stack_sizes(
        &self,
        context: &Context,
    ) -> Result<Vec<(Stage, RayStackSizes)>, vulkan::Error> {
        let mut sizes = Vec::with_capacity(6);
        for (stage, pipeline) in self.ray_programs() {
            if let Some(pipeline) = pipeline {
                sizes.push((stage, pipeline.stack_sizes(context)?));
            }
        }
        Ok(sizes)
    }

    fn program_roots(&self, context: &Context) -> Result<Vec<Resource>, vulkan::Error> {
        let mut roots = Vec::with_capacity(10);
        if let Some(pipeline) = self.primary {
            roots.push(pipeline.program_root(context)?);
        }
        roots.push(self.initial.program_root(context)?);
        if let Some(pipeline) = self.temporal_work {
            roots.push(pipeline.program_root(context)?);
        }
        if let Some(pipeline) = self.temporal_replay {
            roots.push(pipeline.program_root(context)?);
        }
        roots.push(self.temporal_merge.program_root(context)?);
        if let Some(pipeline) = self.spatial_work {
            roots.push(pipeline.program_root(context)?);
        }
        if let Some(pipeline) = self.spatial_replay {
            roots.push(pipeline.program_root(context)?);
        }
        roots.push(self.spatial_precompute.program_root(context)?);
        roots.push(self.spatial_merge.program_root(context)?);
        if let Some(pipeline) = self.resolve {
            roots.push(pipeline.program_root(context)?);
        }
        Ok(roots)
    }

    fn new(
        context: &mut Context,
        programs: Programs<'_>,
        bindings: Bindings,
        fusion: Fusion,
    ) -> Result<Self, Error> {
        if programs.ray_launch == RayLaunch::SpatialDirectionsZ
            && !matches!(fusion.spatial, SpatialFusion::Directions)
        {
            return Err(vulkan::Error::InvalidArgument("空间z域launch只适用于独立方向融合").into());
        }
        let ray = |context: &mut Context, shader, callables: &[RayShader<'_>]| {
            RayTracingPipeline::new(
                context,
                bindings,
                [shader, programs.miss, programs.closest_hit],
                programs.any_hit,
                callables,
            )
        };
        let fused = if fusion != Fusion::NONE {
            let count = programs.replay_callables.len();
            if (programs.initial_callables.is_empty() && programs.initial_hits.is_empty())
                || count == 0
                || count > u32::MAX as usize / 2
                || programs.evaluate_callables.len() != count
            {
                return Err(vulkan::Error::InvalidArgument(
                    "融合SBT须含初采命中或callable叶子，以及两组非空等长、合计可由u32索引的重放/求值叶子",
                )
                .into());
            }
            let fused = programs.fused.ok_or(Error::MissingFusionPrograms)?;
            if !fused.reconnection_callables.is_empty()
                && count
                    .checked_add(1)
                    .and_then(|next| count.checked_mul(next))
                    != Some(fused.reconnection_callables.len())
            {
                return Err(vulkan::Error::InvalidArgument(
                    "完整重连SBT须按F个单侧、F×F个双侧叶子排列",
                )
                .into());
            }
            Some(fused)
        } else {
            None
        };
        let shift_callables = fused.map_or(programs.evaluate_callables, |fused| {
            if fused.reconnection_callables.is_empty() {
                programs.evaluate_callables
            } else {
                fused.reconnection_callables
            }
        });
        let mixed: Vec<_> = programs
            .replay_callables
            .iter()
            .chain(shift_callables)
            .copied()
            .collect();
        let initial_hits: Vec<_> = std::iter::once(programs.closest_hit)
            .chain(programs.initial_hits.iter().copied())
            .collect();
        Ok(Self {
            ray_launch: programs.ray_launch,
            primary: if fusion.primary_initial {
                None
            } else {
                Some(ray(context, programs.primary, &[])?)
            },
            initial: RayTracingPipeline::with_hit_groups(
                context,
                bindings,
                if fusion.primary_initial {
                    fused.expect("融合程序已验证").initial
                } else {
                    programs.initial
                },
                programs.miss,
                &initial_hits,
                programs.any_hit,
                programs.initial_callables,
            )?,
            temporal_work: if fusion.temporal || fusion.temporal_split {
                None
            } else {
                Some(ComputePipeline::with_subgroup_size(
                    context,
                    bindings,
                    programs.temporal_work,
                    32,
                )?)
            },
            temporal_replay: if fusion.temporal_split {
                Some(ray(
                    context,
                    fused.expect("融合程序已验证").temporal_direction,
                    &mixed,
                )?)
            } else if fusion.temporal {
                None
            } else {
                Some(ray(
                    context,
                    programs.temporal_replay,
                    programs.replay_callables,
                )?)
            },
            temporal_merge: if fusion.temporal_split {
                TemporalPipeline::Compute(ComputePipeline::new(
                    context,
                    bindings,
                    fused.expect("融合程序已验证").temporal_combine,
                )?)
            } else {
                TemporalPipeline::Ray(ray(
                    context,
                    if fusion.temporal {
                        fused.expect("融合程序已验证").temporal
                    } else {
                        programs.temporal_merge
                    },
                    if fusion.temporal {
                        &mixed
                    } else {
                        programs.evaluate_callables
                    },
                )?)
            },
            spatial_work: match fusion.spatial {
                SpatialFusion::Pixel => None,
                mode => Some(ComputePipeline::with_subgroup_size(
                    context,
                    bindings,
                    if matches!(mode, SpatialFusion::Directions) {
                        fused.expect("融合程序已验证").spatial_work
                    } else {
                        programs.spatial_work
                    },
                    32,
                )?),
            },
            spatial_replay: if matches!(
                fusion.spatial,
                SpatialFusion::Pixel | SpatialFusion::Directions
            ) {
                None
            } else {
                Some(ray(
                    context,
                    programs.spatial_replay,
                    programs.replay_callables,
                )?)
            },
            spatial_precompute: ray(
                context,
                match fusion.spatial {
                    SpatialFusion::Pixel => fused.expect("融合程序已验证").spatial,
                    SpatialFusion::Directions => fused.expect("融合程序已验证").spatial_direction,
                    SpatialFusion::None => programs.spatial_precompute,
                },
                if matches!(
                    fusion.spatial,
                    SpatialFusion::Pixel | SpatialFusion::Directions
                ) {
                    &mixed
                } else {
                    programs.evaluate_callables
                },
            )?,
            spatial_merge: ComputePipeline::new(
                context,
                bindings,
                if fusion.merge_resolve {
                    fused.expect("融合程序已验证").merge
                } else {
                    programs.spatial_merge
                },
            )?,
            resolve: if fusion.merge_resolve {
                None
            } else {
                Some(ComputePipeline::new(context, bindings, programs.resolve)?)
            },
        })
    }
    fn rebind(&self, context: &mut Context, bindings: Bindings) -> Result<Self, Error> {
        Ok(Self {
            ray_launch: self.ray_launch,
            primary: self
                .primary
                .map(|pipeline| pipeline.rebind(context, bindings))
                .transpose()?,
            initial: self.initial.rebind(context, bindings)?,
            temporal_work: self
                .temporal_work
                .map(|pipeline| pipeline.rebind(context, bindings))
                .transpose()?,
            temporal_replay: self
                .temporal_replay
                .map(|pipeline| pipeline.rebind(context, bindings))
                .transpose()?,
            temporal_merge: self.temporal_merge.rebind(context, bindings)?,
            spatial_work: self
                .spatial_work
                .map(|pipeline| pipeline.rebind(context, bindings))
                .transpose()?,
            spatial_replay: self
                .spatial_replay
                .map(|pipeline| pipeline.rebind(context, bindings))
                .transpose()?,
            spatial_precompute: self.spatial_precompute.rebind(context, bindings)?,
            spatial_merge: self.spatial_merge.rebind(context, bindings)?,
            resolve: self
                .resolve
                .map(|pipeline| pipeline.rebind(context, bindings))
                .transpose()?,
        })
    }
    fn roots(&self) -> [Option<Resource>; 10] {
        [
            self.primary.map(Resource::from),
            Some(self.initial.into()),
            self.temporal_work.map(Resource::from),
            self.temporal_replay.map(Resource::from),
            Some(self.temporal_merge.root()),
            self.spatial_work.map(Resource::from),
            self.spatial_replay.map(Resource::from),
            Some(self.spatial_precompute.into()),
            Some(self.spatial_merge.into()),
            self.resolve.map(Resource::from),
        ]
    }
    fn record(
        &self,
        commands: &mut Commands,
        stage: Stage,
        dispatch: Dispatch,
    ) -> Result<(), Error> {
        let dispatch = match (self.ray_launch, stage, dispatch) {
            (RayLaunch::Linear, _, Dispatch::Rays([x, y, z])) => {
                // Layout已校验屏幕域和三方向容量，乘积不会溢出。
                Dispatch::Rays([x * y, 1, z])
            }
            (
                RayLaunch::SpatialDirectionsZ,
                Stage::SpatialPrecompute,
                Dispatch::Rays([x, y, _]),
            ) => Dispatch::Rays([x / 3, y, 3]),
            (_, _, value) => value,
        };
        match (stage, dispatch) {
            (Stage::Primary, Dispatch::Rays(size)) => {
                commands.trace_3d(self.primary.expect("阶段已启用"), size)?
            }
            (Stage::Initial, Dispatch::Rays(size)) => commands.trace_3d(self.initial, size)?,
            (Stage::TemporalReplay, Dispatch::Rays(size)) => {
                commands.trace_3d(self.temporal_replay.expect("阶段已启用"), size)?
            }
            (Stage::TemporalMerge, dispatch) => self.temporal_merge.record(commands, dispatch)?,
            (Stage::SpatialReplay, Dispatch::Rays(size)) => {
                commands.trace_3d(self.spatial_replay.expect("阶段已启用"), size)?
            }
            (Stage::SpatialPrecompute, Dispatch::Rays(size)) => {
                commands.trace_3d(self.spatial_precompute, size)?
            }
            (Stage::TemporalWork, Dispatch::Compute(groups)) => {
                commands.dispatch(self.temporal_work.expect("阶段已启用"), groups)?
            }
            (Stage::SpatialWork, Dispatch::Compute(groups)) => {
                commands.dispatch(self.spatial_work.expect("阶段已启用"), groups)?
            }
            (Stage::SpatialMerge, Dispatch::Compute(groups)) => {
                commands.dispatch(self.spatial_merge, groups)?
            }
            (Stage::Resolve, Dispatch::Compute(groups)) => {
                commands.dispatch(self.resolve.expect("阶段已启用"), groups)?
            }
            _ => unreachable!("dispatch来自已校验Layout"),
        }
        Ok(())
    }
}

/// 已完成准备、尚未发布的descriptor版本；丢弃不改变renderer，Context后续collect回收未发布版本。
#[must_use = "须commit_rebind发布或丢弃后通过Context collect回收"]
pub struct Rebinding {
    owner: u64,
    base: u64,
    pipelines: Pipelines,
}

impl Rebinding {
    /// 若准备与发布之间执行collect，将这些待发布版本连同现有场景根一起保留。
    pub fn roots(&self) -> impl Iterator<Item = Resource> {
        self.pipelines.roots().into_iter().flatten()
    }
}

/// 独立待确认帧；commands的取消/提交由宿主执行，token只确认CPU历史。
#[must_use = "须提交并commit，或取消commands并reject"]
pub struct PreparedFrame {
    /// 已在Context登记的命令票据，不含提交或等待。
    pub commands: Recorded,
    /// 成功提交后commit；取消后reject。
    pub token: FrameToken,
    /// 与录制前next_frame_params相同，供诊断核对。
    pub params: FrameParams,
}

/// 唯一算法工作集和帧状态；GPU真实版本仍由Context独占。
pub struct Renderer<B = SharedBuffer> {
    buffers: Buffers<B>,
    pipelines: Pipelines,
    settings: Settings,
    frames: FrameState,
    binding_generation: u64,
}

impl Renderer<SharedBuffer> {
    /// 使用 wgpu 共享工作集；资源和绑定须属于该 Context，具体契约同原生构造入口。
    pub fn new(
        context: &mut WgpuContext,
        programs: Programs<'_>,
        bindings: Bindings,
        buffers: Buffers,
        settings: Settings,
    ) -> Result<Self, Error> {
        Self::new_native(context.native(), programs, bindings, buffers, settings)
    }
}

impl<B> Renderer<B> {
    /// 按逻辑 RT 阶段查询链接后的驱动统计；须显式启用 native 统计能力。
    /// compute 阶段不出现；不执行 GPU，查询或资源错误保留底层来源。
    pub fn executable_statistics(
        &self,
        context: &Context,
    ) -> Result<Vec<(Stage, Vec<vulkan::native::ExecutableStatistics>)>, Error> {
        let mut result = Vec::with_capacity(6);
        for (stage, pipeline) in self.pipelines.ray_programs() {
            if let Some(pipeline) = pipeline {
                result.push((stage, pipeline.executable_statistics(context)?));
            }
        }
        Ok(result)
    }

    /// 按逻辑阶段顺序返回实际RT program缓存的驱动栈需求；compute阶段不出现在结果中。
    /// 每次调用分配至多六项的Vec，不查询驱动或录制命令；配置的temporal RT在冷帧仍可查询。
    /// 标识已回收或Context不匹配时保留底层错误；返回值不是寄存器数或实际每ray分配。
    pub fn program_stack_sizes(
        &self,
        context: &Context,
    ) -> Result<Vec<(Stage, RayStackSizes)>, Error> {
        Ok(self.pipelines.program_stack_sizes(context)?)
    }

    /// 按逻辑槽位返回已启用程序身份；RT同时拥有SBT，resize/rebind应保持相同。
    /// 此快照仅含Context标识，不延长资源寿命；原生标识失效时返回来源。
    pub fn program_roots(&self, context: &Context) -> Result<Vec<Resource>, Error> {
        Ok(self.pipelines.program_roots(context)?)
    }

    /// 按工作集融合边界创建已启用的原生程序；B 可为 Buffer 或 SharedBuffer。
    /// 资源与绑定须来自同一Context，shader ABI由受控构建验证。
    /// 启用workload时显式请求subgroup32，并要求compute BASIC/BALLOT/ARITHMETIC；不满足时失败。
    /// temporal与空间像素融合均启用时不创建workload，不要求这些能力。
    /// 任一准备失败不返回半个renderer；Context按现有collect契约回收未发布原生版本。
    pub fn new_native(
        context: &mut Context,
        programs: Programs<'_>,
        bindings: Bindings,
        buffers: Buffers<B>,
        settings: Settings,
    ) -> Result<Self, Error> {
        let settings = settings.validate()?;
        validate_subgroup(buffers.layout.fusion(), context.subgroup_properties())?;
        let frames = FrameState::new()?;
        let pipelines = Pipelines::new(context, programs, bindings, buffers.layout.fusion())?;
        Ok(Self {
            buffers,
            pipelines,
            settings,
            frames,
            binding_generation: 0,
        })
    }
    /// 借用唯一工作集，不复制第二套reservoir或场景状态。
    pub const fn buffers(&self) -> &Buffers<B> {
        &self.buffers
    }
    /// 已验证的运行参数；外部参数buffer须使用同值。
    pub const fn settings(&self) -> Settings {
        self.settings
    }
    /// 上传相机、seed与history有效位前读取；pending期间拒绝新帧。
    pub const fn next_frame_params(&self) -> Result<FrameParams, crate::Error> {
        self.frames.next_frame_params()
    }
    /// 录制完整默认profile和必要barrier；不提交、不等待、不写外部相机参数。
    /// 录制前须已上传next_frame_params，且宿主维持同队列帧序；失败撤销CPU事务，
    /// Commands::finish之前的临时操作不登记资源，不留下半个native记录。
    pub fn record(&mut self, context: &mut Context) -> Result<PreparedFrame, Error> {
        self.record_inner(context, None)
    }
    /// 诊断录制；20个槽按完整十阶段顺序分别保存起止tick，普通record不插入query。
    /// 每段包含已有结束barrier及起点排序barrier；无历史或被融合的空槽仅写marker。
    /// pool须空闲且由宿主作为资源根保留；提交完成并收取通知后可非WAIT读取。
    /// 这是同队列设备跨度，仍受其他GPU工作、频率和timestamp本身的开销影响。
    pub fn record_timed(
        &mut self,
        context: &mut Context,
        queries: TimestampQueries,
    ) -> Result<PreparedFrame, Error> {
        if queries.len() != 20 {
            return Err(vulkan::Error::InvalidArgument("ReSTIR诊断需要20个timestamp槽").into());
        }
        self.record_inner(context, Some(queries))
    }
    fn record_inner(
        &mut self,
        context: &mut Context,
        queries: Option<TimestampQueries>,
    ) -> Result<PreparedFrame, Error> {
        let (token, params) = self.frames.begin()?;
        match self.record_commands(context, params, queries) {
            Ok(commands) => Ok(PreparedFrame {
                commands,
                token,
                params,
            }),
            Err(error) => {
                self.frames.reject(token).expect("本次独占待确认事务");
                Err(error)
            }
        }
    }

    /// 从固定帧快照构造命令；不修改 CPU 历史，失败只丢弃未登记的临时操作。
    fn record_commands(
        &self,
        context: &mut Context,
        params: FrameParams,
        queries: Option<TimestampQueries>,
    ) -> Result<Recorded, Error> {
        let mut commands = Commands::new(context)?;
        if let Some(queries) = queries {
            commands.reset_timestamps(queries)?;
        }
        for (index, stage) in Stage::sequence(true).enumerate() {
            let enabled = stage.enabled(params.history_valid, self.buffers.layout.fusion());
            if !enabled && queries.is_none() {
                continue;
            }
            if let Some(queries) = queries {
                commands.write_timestamp(queries, 2 * index as u32)?;
                commands.barrier();
            }
            if enabled {
                if matches!(stage, Stage::TemporalWork | Stage::SpatialWork)
                    && !(stage == Stage::SpatialWork
                        && matches!(
                            self.buffers.layout.fusion().spatial,
                            SpatialFusion::Directions
                        ))
                {
                    commands
                        .clear_buffer(self.buffers.counter().expect("分阶段重放持有counter"))?;
                    commands.barrier();
                }
                self.pipelines
                    .record(&mut commands, stage, self.buffers.layout.dispatch(stage))?;
                commands.barrier();
            }
            if let Some(queries) = queries {
                commands.write_timestamp(queries, 2 * index as u32 + 1)?;
            }
        }
        commands.finish(context).map_err(Error::from)
    }
    /// 完整帧成功提交后推进历史；不表示GPU已完成。
    pub fn commit(&mut self, token: FrameToken) -> Result<(), crate::Error> {
        self.frames.commit(token)
    }
    /// 已取消本帧命令后保留原历史；提交成功的帧不能用reject撤回。
    pub fn reject(&mut self, token: FrameToken) -> Result<(), crate::Error> {
        self.frames.reject(token)
    }
    /// 宿主同步重置外部累积和相机历史；不释放或清空在途资源。
    pub fn reset_history(&mut self) -> Result<(), crate::Error> {
        self.frames.reset_history()
    }
    /// 更新运行参数并保留历史；pending 期间拒绝。
    ///
    /// 随机身份由 reservoir 保存，参数变化的支持检查属于局部重放。
    /// 只有宿主确认整个复用域失效时才显式调用 [`Self::reset_history`]。
    pub fn set_settings(&mut self, settings: Settings) -> Result<bool, crate::Error> {
        let settings = settings.validate()?;
        self.frames.next_frame_params()?;
        if settings == self.settings {
            return Ok(false);
        }
        self.settings = settings;
        Ok(true)
    }
    /// 用新工作集准备独立的无历史renderer；复用全部程序及SBT，只建立新descriptor版本。
    /// 原renderer必须无待确认帧，且在宿主发布前保持完整可用；新buffers和绑定来自同一Context。
    /// 适用于尺寸变化，settings、融合模式和颜色存储边界保持不变，不提交或等待GPU。
    pub fn prepare_resize(
        &self,
        context: &mut Context,
        bindings: Bindings,
        buffers: Buffers<B>,
    ) -> Result<Self, Error> {
        self.frames.next_frame_params()?;
        if self.buffers.layout.fusion() != buffers.layout.fusion() {
            return Err(Error::FusionMismatch);
        }
        if self.buffers.layout.has_color_estimates() != buffers.layout.has_color_estimates() {
            return Err(Error::ColorEstimateMismatch);
        }
        let frames = FrameState::new()?;
        let pipelines = self.pipelines.rebind(context, bindings)?;
        Ok(Self {
            buffers,
            pipelines,
            settings: self.settings,
            frames,
            binding_generation: 0,
        })
    }
    /// 只准备不可变descriptor版本，不发布场景或改帧进度；program/SBT复用。
    pub fn prepare_rebind(
        &self,
        context: &mut Context,
        bindings: Bindings,
    ) -> Result<Rebinding, Error> {
        self.frames.next_frame_params()?;
        self.binding_generation
            .checked_add(1)
            .ok_or(crate::Error::Overflow)?;
        Ok(Rebinding {
            owner: self.frames.owner(),
            base: self.binding_generation,
            pipelines: self.pipelines.rebind(context, bindings)?,
        })
    }
    /// 选择另一已准备renderer的阶段程序，保留本renderer的工作集、历史和设置。
    ///
    /// 不编译程序或重建SBT，只建立绑定版本；两者须属于同一Context且融合/颜色布局相同。
    /// 两个renderer均须无待确认帧，发布仍由本renderer的commit_rebind执行。
    /// # Errors
    /// pending、布局不兼容、版本计数溢出或底层绑定失败时保留来源；不改变现有历史。
    pub fn prepare_variant(
        &self,
        context: &mut Context,
        template: &Self,
        bindings: Bindings,
    ) -> Result<Rebinding, Error> {
        self.frames.next_frame_params()?;
        template.frames.next_frame_params()?;
        if self.buffers.layout.fusion() != template.buffers.layout.fusion() {
            return Err(Error::FusionMismatch);
        }
        if self.buffers.layout.has_color_estimates()
            != template.buffers.layout.has_color_estimates()
        {
            return Err(Error::ColorEstimateMismatch);
        }
        self.binding_generation
            .checked_add(1)
            .ok_or(crate::Error::Overflow)?;
        Ok(Rebinding {
            owner: self.frames.owner(),
            base: self.binding_generation,
            pipelines: template.pipelines.rebind(context, bindings)?,
        })
    }
    /// 准备另一组阶段程序并保留算法工作集和历史；用于切换受控的 SER 变体。
    /// 与prepare_rebind相同，只能通过commit_rebind发布。
    pub fn prepare_programs(
        &self,
        context: &mut Context,
        programs: Programs<'_>,
        bindings: Bindings,
    ) -> Result<Rebinding, Error> {
        self.frames.next_frame_params()?;
        self.binding_generation
            .checked_add(1)
            .ok_or(crate::Error::Overflow)?;
        Ok(Rebinding {
            owner: self.frames.owner(),
            base: self.binding_generation,
            pipelines: Pipelines::new(context, programs, bindings, self.buffers.layout.fusion())?,
        })
    }
    /// 在宿主完成所有可失败准备、AS提交和场景提交后发布；外部身份不匹配时不改当前版本。
    pub fn commit_rebind(&mut self, prepared: Rebinding) -> Result<(), Error> {
        self.frames.next_frame_params()?;
        if prepared.owner != self.frames.owner() || prepared.base != self.binding_generation {
            return Err(Error::StaleRebinding);
        }
        self.pipelines = prepared.pipelines;
        self.binding_generation += 1;
        Ok(())
    }
    /// 立即准备并发布绑定的便利入口；需跨场景原子发布时分别调用prepare/commit。
    pub fn rebind(&mut self, context: &mut Context, bindings: Bindings) -> Result<(), Error> {
        let prepared = self.prepare_rebind(context, bindings)?;
        self.commit_rebind(prepared)
    }
    /// 当前发布程序和counter根；descriptor自动保活所有共享资源和场景依赖。
    pub fn roots(&self) -> impl Iterator<Item = Resource> {
        self.pipelines
            .roots()
            .into_iter()
            .flatten()
            .chain(self.buffers.roots())
    }
}

const fn validate_subgroup(fusion: Fusion, subgroup: SubgroupProperties) -> Result<(), Error> {
    if (fusion.temporal || fusion.temporal_split) && matches!(fusion.spatial, SpatialFusion::Pixel)
    {
        return Ok(());
    }
    if !subgroup.compute_size_control
        || subgroup.min_size > 32
        || subgroup.max_size < 32
        || !subgroup.compute
        || !subgroup.basic
        || !subgroup.ballot
        || !subgroup.arithmetic
    {
        return Err(Error::UnsupportedSubgroup(subgroup));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VARIABLE_SUBGROUP: SubgroupProperties = SubgroupProperties {
        size: 64,
        compute_size_control: true,
        min_size: 32,
        max_size: 64,
        compute: true,
        basic: true,
        ballot: true,
        arithmetic: true,
    };

    #[test]
    fn requested_size_is_independent_of_device_default() {
        assert!(validate_subgroup(Fusion::NONE, VARIABLE_SUBGROUP).is_ok());
    }

    #[test]
    fn default_size_does_not_replace_size_control() {
        let subgroup = SubgroupProperties {
            size: 32,
            compute_size_control: false,
            ..VARIABLE_SUBGROUP
        };
        assert!(matches!(
            validate_subgroup(Fusion::NONE, subgroup),
            Err(Error::UnsupportedSubgroup(_))
        ));
    }

    #[test]
    fn workload_rejects_an_unsupported_requested_size() {
        let subgroup = SubgroupProperties {
            min_size: 64,
            ..VARIABLE_SUBGROUP
        };
        assert!(matches!(
            validate_subgroup(Fusion::DIRECTIONS, subgroup),
            Err(Error::UnsupportedSubgroup(_))
        ));
    }

    #[test]
    fn pixel_fusion_does_not_require_workload_capabilities() {
        let fusion = Fusion {
            spatial: SpatialFusion::Pixel,
            ..Fusion::ALL
        };
        let subgroup = SubgroupProperties {
            compute_size_control: false,
            compute: false,
            basic: false,
            ballot: false,
            arithmetic: false,
            ..VARIABLE_SUBGROUP
        };
        assert!(validate_subgroup(fusion, subgroup).is_ok());
    }
}
