//! Vulkan路径录制器；完整输运可显式省略前缀阶段，场景与受控shader由应用适配。

use vulkan::{
    interop::RayShader,
    native::{
        Bindings, Commands, ComputePipeline, Context, RayTracingPipeline, Recorded, Resource,
        TimestampQueries,
    },
};

/// shader、设备或尺寸不满足公共阶段录制约定。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// native 资源或命令失败。
    #[error(transparent)]
    Vulkan(#[from] vulkan::Error),
    /// 样本预算无效。
    #[error(transparent)]
    Input(#[from] crate::Error),
    /// 视图尺寸不满足采样域支持范围。
    #[error("PT 视图尺寸须属于 1..=65536")]
    Extent,
}

/// 静态场景适配器实例化同一 PT 模块后的受控产物；只在创建 pipeline 时借用。
pub struct Programs<'a> {
    /// 实时执行图；只链接实时首面 callable，离线图不隐式继承它们。
    pub realtime: Option<RealtimePrograms<'a>>,
    /// 可选诊断；只链接几何命中与覆盖率过滤，不链接照明材质。
    pub diagnostic: Option<RayShader<'a>>,
    /// 可选K1 delta前缀；None表示transport负责首查询、完整输运及全部post输入。
    /// 是否可省略由受控shader契约决定，不检查空shader或根据运行输入隐式推断。
    pub prefix: Option<RayShader<'a>>,
    /// K2：完整输运 raygen。
    pub transport: RayShader<'a>,
    /// 场景 miss。
    pub miss: RayShader<'a>,
    /// 场景几何命中重建。
    pub closest_hit: RayShader<'a>,
    /// 可选材质 closest-hit；追加在几何命中记录之后，仅链接到K2。
    /// 非空时K2须使用同一资源计划的相对索引加一，不执行嵌套追踪。
    pub material_hits: &'a [RayShader<'a>],
    /// 可选 alpha 覆盖率过滤；callable 内的 RayQuery 仍由适配器自行确认候选。
    pub any_hit: Option<RayShader<'a>>,
    /// 离线／兼容输运实际需要的callable；原生Hit Group离线图为空。
    /// 实时首面callable由 [`RealtimePrograms`] 单独提供。
    pub callables: &'a [RayShader<'a>],
    /// 有限后处理族；每项为8×8×1 compute，录制时按宿主阶段计划选择。
    pub posts: &'a [RayShader<'a>],
}

/// 首面缓存输运的独立链接集合；所有入口在资源准备边界构造。
pub struct RealtimePrograms<'a> {
    /// 有限相机采样策略；索引由宿主帧计划维持。
    pub primary: &'a [RayShader<'a>],
    /// 固定1 spp输运。
    pub transport: RayShader<'a>,
    /// 固定2 spp双pass；第二项读取第一项的未归一化辐射。
    pub transport_pair: Option<[RayShader<'a>; 2]>,
    /// 固定2 spp合并参考；生产可不准备。
    pub merged: Option<RayShader<'a>>,
    /// 与材质Hit Group同序的首面callable，仅链接到实时输运。
    pub callables: &'a [RayShader<'a>],
}

/// 已准备的实时样本调度；不得与绑定中的样本数不一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealtimeSchedule {
    /// 单条实时路径。
    One,
    /// 两条顺序执行的实时路径。
    SplitTwo,
    /// 显式合并参考；须准备对应入口。
    MergedTwo,
}

/// 一帧实时执行计划；只选择已准备的程序，不创建管线或SBT。
pub struct RealtimeFrame {
    /// 与GPU参数相同的样本调度。
    pub schedule: RealtimeSchedule,
    /// 相机策略在 [`RealtimePrograms::primary`] 中的索引。
    pub primary: usize,
    /// 后处理策略在 [`Programs::posts`] 中的索引。
    pub post: usize,
    /// 若提供须为六槽，依次包围首视线、输运和后处理。
    pub queries: Option<TimestampQueries>,
    /// 若提供须为两槽，包围独立Guide阶段。
    pub guide_queries: Option<TimestampQueries>,
}

struct RealtimeStages {
    primary: Vec<RayTracingPipeline>,
    transport: RayTracingPipeline,
    transport_pair: Option<[RayTracingPipeline; 2]>,
    merged: Option<RayTracingPipeline>,
}

/// 一个已准备RT阶段的驱动统计；驱动可能合并着色器或不提供统计字段。
#[derive(Debug)]
pub struct StageStatistics {
    /// 阶段及策略索引；对应实际独立管线，不能据名称推算寄存器数。
    pub name: String,
    /// 驱动划分的可执行单元及原始统计。
    pub executables: Vec<vulkan::native::ExecutableStatistics>,
}

/// 两个宿主共用的阶段配置；GPU 程序和绑定唯一归 Context 所有。
///
/// 同一视图的工作缓冲按样本顺序复用；调用方维护跨批次参数更新与提交顺序。
/// 返回的 [`Recorded`] 在 Context 内登记使用，宿主须显式提交或取消；外部 Flight 真实完成后再关闭。
pub struct Renderer {
    realtime: Option<RealtimeStages>,
    diagnostic: Option<RayTracingPipeline>,
    prefix: Option<RayTracingPipeline>,
    transport: RayTracingPipeline,
    posts: Vec<ComputePipeline>,
    extent: [u32; 2],
}

impl Renderer {
    /// 原子构造完整阶段；trace绑定供首视线、兼容前缀和输运，post绑定供后处理，均须属于context。
    /// shader 的布局、payload 和无环调用图由受控构建产物维护。
    pub fn new(
        context: &mut Context,
        programs: Programs<'_>,
        trace: Bindings,
        post: Bindings,
        extent: [u32; 2],
    ) -> Result<Self, Error> {
        if extent.into_iter().any(|v| v == 0 || v > 65536) {
            return Err(Error::Extent);
        }
        let prefix = programs
            .prefix
            .map(|prefix| {
                RayTracingPipeline::new(
                    context,
                    trace,
                    [prefix, programs.miss, programs.closest_hit],
                    programs.any_hit,
                    &[],
                )
            })
            .transpose()?;
        let diagnostic = programs
            .diagnostic
            .map(|raygen| {
                RayTracingPipeline::new(
                    context,
                    trace,
                    [raygen, programs.miss, programs.closest_hit],
                    programs.any_hit,
                    &[],
                )
            })
            .transpose()?;
        let hits: Vec<_> = std::iter::once(programs.closest_hit)
            .chain(programs.material_hits.iter().copied())
            .collect();
        let transport = RayTracingPipeline::with_hit_groups(
            context,
            trace,
            programs.transport,
            programs.miss,
            &hits,
            programs.any_hit,
            programs.callables,
        )?;
        let realtime = programs
            .realtime
            .map(|realtime| {
                if realtime.primary.is_empty() {
                    return Err(vulkan::Error::InvalidArgument("实时首视线程序族为空"));
                }
                let primary = realtime
                    .primary
                    .iter()
                    .map(|&raygen| {
                        RayTracingPipeline::new(
                            context,
                            trace,
                            [raygen, programs.miss, programs.closest_hit],
                            programs.any_hit,
                            &[],
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let mut create = |raygen| {
                    RayTracingPipeline::with_hit_groups(
                        context,
                        trace,
                        raygen,
                        programs.miss,
                        &hits,
                        programs.any_hit,
                        realtime.callables,
                    )
                };
                Ok::<_, vulkan::Error>(RealtimeStages {
                    primary,
                    transport: create(realtime.transport)?,
                    transport_pair: realtime
                        .transport_pair
                        .map(|pair| Ok::<_, vulkan::Error>([create(pair[0])?, create(pair[1])?]))
                        .transpose()?,
                    merged: realtime.merged.map(create).transpose()?,
                })
            })
            .transpose()?;
        if programs.posts.is_empty() {
            return Err(vulkan::Error::InvalidArgument("PT后处理程序族为空").into());
        }
        let posts = programs
            .posts
            .iter()
            .map(|&shader| ComputePipeline::new(context, post, shader))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            realtime,
            diagnostic,
            prefix,
            transport,
            posts,
            extent,
        })
    }

    /// 为新场景或尺寸复用全部阶段程序与 SBT；不创建 pipeline 或读取 shader。
    ///
    /// trace 与 post 分别保持原 descriptor 布局和资源内容 ABI；尺寸验证与
    /// [`Self::new`] 相同。旧命令对应版本仍留在 Context，可与返回的新版本同时在途。
    /// callable 集合或 shader 契约变化时须使用 [`Self::new`] 创建新程序世代。
    ///
    /// # Errors
    /// 尺寸越界、绑定设备或 descriptor 布局不兼容时保留原录制器并返回错误。
    pub fn rebind(
        &self,
        context: &mut Context,
        trace: Bindings,
        post: Bindings,
        extent: [u32; 2],
    ) -> Result<Self, Error> {
        if extent.into_iter().any(|v| v == 0 || v > 65536) {
            return Err(Error::Extent);
        }
        Ok(Self {
            realtime: self
                .realtime
                .as_ref()
                .map(|stages| {
                    Ok::<_, vulkan::Error>(RealtimeStages {
                        primary: stages
                            .primary
                            .iter()
                            .map(|p| p.rebind(context, trace))
                            .collect::<Result<_, _>>()?,
                        transport: stages.transport.rebind(context, trace)?,
                        transport_pair: stages
                            .transport_pair
                            .map(|pair| {
                                Ok::<_, vulkan::Error>([
                                    pair[0].rebind(context, trace)?,
                                    pair[1].rebind(context, trace)?,
                                ])
                            })
                            .transpose()?,
                        merged: stages
                            .merged
                            .map(|p| p.rebind(context, trace))
                            .transpose()?,
                    })
                })
                .transpose()?,
            diagnostic: self
                .diagnostic
                .map(|p| p.rebind(context, trace))
                .transpose()?,
            prefix: self
                .prefix
                .map(|prefix| prefix.rebind(context, trace))
                .transpose()?,
            transport: self.transport.rebind(context, trace)?,
            posts: self
                .posts
                .iter()
                .map(|p| p.rebind(context, post))
                .collect::<Result<_, _>>()?,
            extent,
        })
    }

    /// 实际发布的阶段版本；只返回句柄副本，不分配或借用工作存储，供唯一owner回收。
    pub fn roots(&self) -> impl Iterator<Item = Resource> + '_ {
        self.prefix
            .map(Resource::from)
            .into_iter()
            .chain(self.diagnostic.map(Resource::from))
            .chain([self.transport.into()])
            .chain(self.posts.iter().copied().map(Resource::from))
            .chain(self.realtime.iter().flat_map(|stages| {
                stages
                    .primary
                    .iter()
                    .copied()
                    .chain([stages.transport])
                    .chain(stages.transport_pair.into_iter().flatten())
                    .chain(stages.merged)
                    .map(Resource::from)
            }))
    }

    /// 返回全部已准备RT阶段的具名驱动统计，含离线、实时、相机与诊断。
    /// 须显式启用native统计能力；不执行GPU，不含宿主独立Guide或compute。
    /// 统计捕获用于独立分析，不能与普通管线混合计时；查询失败保留底层错误。
    pub fn executable_statistics(&self, context: &Context) -> Result<Vec<StageStatistics>, Error> {
        let mut results = Vec::new();
        let mut capture = |name: String, pipeline: RayTracingPipeline| -> Result<(), Error> {
            results.push(StageStatistics {
                name,
                executables: pipeline.executable_statistics(context)?,
            });
            Ok(())
        };
        capture("offline".into(), self.transport)?;
        if let Some(prefix) = self.prefix {
            capture("prefix".into(), prefix)?;
        }
        if let Some(diagnostic) = self.diagnostic {
            capture("diagnostic".into(), diagnostic)?;
        }
        if let Some(stages) = &self.realtime {
            for (index, &primary) in stages.primary.iter().enumerate() {
                capture(format!("primary-{index}"), primary)?;
            }
            capture("realtime-one".into(), stages.transport)?;
            if let Some(pair) = stages.transport_pair {
                for (lane, pipeline) in pair.into_iter().enumerate() {
                    capture(format!("realtime-two-{lane}"), pipeline)?;
                }
            }
            if let Some(merged) = stages.merged {
                capture("realtime-two-reference".into(), merged)?;
            }
        }
        Ok(results)
    }

    /// 两个实时输运入口各自的驱动可执行统计；未准备双pass时返回None。
    /// 须显式启用native统计能力，不执行GPU；查询失败保留底层错误。
    pub fn transport_pair_statistics(
        &self,
        context: &Context,
    ) -> Result<Option<[Vec<vulkan::native::ExecutableStatistics>; 2]>, Error> {
        self.realtime
            .as_ref()
            .and_then(|stages| stages.transport_pair)
            .map(|pair| {
                Ok([
                    pair[0].executable_statistics(context)?,
                    pair[1].executable_statistics(context)?,
                ])
            })
            .transpose()
    }

    /// 目标宽高；公共 HDR 按左上原点解释，宿主呈现转换单独执行。
    pub const fn extent(&self) -> [u32; 2] {
        self.extent
    }

    /// 逐样本录制可选K1→K2→K3和实际阶段间可见性，不提交、不等待。
    /// 交接及累积缓冲由绑定所有者持有，不按 samples 倍增。
    /// post须是 [`Programs::posts`] 中已准备的策略索引，越界返回错误。
    pub fn record(
        &self,
        context: &mut Context,
        samples: u32,
        post: usize,
    ) -> Result<Recorded, Error> {
        self.record_inner(context, samples, None, post)
    }

    /// 录制独立诊断，不执行照明材质、累积或后处理；未准备诊断入口时返回错误。
    pub fn record_diagnostic(&self, context: &mut Context) -> Result<Recorded, Error> {
        let pipeline = self
            .diagnostic
            .ok_or(vulkan::Error::InvalidArgument("未配置PT诊断"))?;
        let mut commands = Commands::new(context)?;
        commands.trace(pipeline, self.extent)?;
        commands.barrier();
        Ok(commands.finish(context)?)
    }

    /// 实时四阶段：首视线、可选确定性导引、输运、后处理。
    /// 帧计划须与绑定参数中的采样、相机及后处理策略一致。
    /// 首命中、导引和后处理每帧各执行至多一次。
    /// guide须与trace绑定相同的首命中ABI；调用者保活其资源。
    /// 查询容量由 [`RealtimeFrame`] 规定。
    ///
    /// # Errors
    /// 未准备所请求的调度、相机／后处理索引越界、查询容量不符或录制失败时返回错误，不提交命令。
    pub fn record_realtime(
        &self,
        context: &mut Context,
        guide: Option<RayTracingPipeline>,
        frame: RealtimeFrame,
    ) -> Result<Recorded, Error> {
        let stages = self
            .realtime
            .as_ref()
            .ok_or(vulkan::Error::InvalidArgument("未配置实时PT"))?;
        let primary = *stages
            .primary
            .get(frame.primary)
            .ok_or(vulkan::Error::InvalidArgument("实时相机策略越界"))?;
        let transport = match frame.schedule {
            RealtimeSchedule::SplitTwo => stages
                .transport_pair
                .as_ref()
                .ok_or(vulkan::Error::InvalidArgument("未配置实时双pass输运"))?
                .as_slice(),
            RealtimeSchedule::One => std::slice::from_ref(&stages.transport),
            RealtimeSchedule::MergedTwo => std::slice::from_ref(
                stages
                    .merged
                    .as_ref()
                    .ok_or(vulkan::Error::InvalidArgument("未配置实时合并参考"))?,
            ),
        };
        let post = self.post(frame.post)?;
        let RealtimeFrame {
            queries,
            guide_queries,
            ..
        } = frame;
        if queries.is_some_and(|q| q.len() != 6) {
            return Err(vulkan::Error::InvalidArgument("实时PT timestamp须为六槽").into());
        }
        if guide_queries.is_some_and(|q| q.len() != 2) {
            return Err(vulkan::Error::InvalidArgument("导引timestamp须为两槽").into());
        }
        let mut commands = Commands::new(context)?;
        if let Some(q) = guide_queries {
            commands.reset_timestamps(q)?;
        }
        if let Some(q) = queries {
            commands.reset_timestamps(q)?;
        }
        if let Some(q) = queries {
            commands.write_timestamp(q, 0)?;
        }
        commands.trace(primary, self.extent)?;
        commands.barrier();
        if let Some(q) = queries {
            commands.write_timestamp(q, 1)?;
        }
        if let Some(guide) = guide {
            if let Some(q) = guide_queries {
                commands.write_timestamp(q, 0)?;
            }
            commands.trace(guide, self.extent)?;
            commands.barrier();
            if let Some(q) = guide_queries {
                commands.write_timestamp(q, 1)?;
            }
        }
        if let Some(q) = queries {
            commands.write_timestamp(q, 2)?;
        }
        for &pipeline in transport {
            commands.trace(pipeline, self.extent)?;
            commands.barrier();
        }
        if let Some(q) = queries {
            commands.write_timestamp(q, 3)?;
            commands.write_timestamp(q, 4)?;
        }
        commands.dispatch(
            post,
            [self.extent[0].div_ceil(8), self.extent[1].div_ceil(8), 1],
        )?;
        commands.barrier();
        if let Some(q) = queries {
            commands.write_timestamp(q, 5)?;
        }
        Ok(commands.finish(context)?)
    }

    /// 与 [`Self::record`] 共用实际阶段；每样本四槽分别包围主体 TraceRays 和 post。
    /// 可选前缀不计时；起点位于额外排序屏障之后，终点位于原有阶段结束屏障之后。
    /// 这些 ALL_COMMANDS 时间戳是设备调度跨度，不能解释为隔离执行或窗口整帧。
    /// pool 须属于 context，容量恰好为 `samples * 4`，并在复用前收取完成通知；
    /// 本方法 reset 整个 pool，调用方负责在 collect 时保活它。
    ///
    /// # Errors
    /// 样本、pool 容量或 native 命令无效时保留错误；不提交、不等待。
    pub fn record_with_timestamps(
        &self,
        context: &mut Context,
        samples: u32,
        queries: TimestampQueries,
        post: usize,
    ) -> Result<Recorded, Error> {
        if samples
            .checked_mul(4)
            .is_none_or(|count| queries.len() != count)
        {
            return Err(vulkan::Error::InvalidArgument("PT timestamp 容量不足").into());
        }
        self.record_inner(context, samples, Some(queries), post)
    }

    fn record_inner(
        &self,
        context: &mut Context,
        samples: u32,
        queries: Option<TimestampQueries>,
        post: usize,
    ) -> Result<Recorded, Error> {
        if samples == 0 {
            return Err(crate::Error::Samples.into());
        }
        let post = self.post(post)?;
        let mut commands = Commands::new(context)?;
        if let Some(queries) = queries {
            commands.reset_timestamps(queries)?;
        }
        for sample in 0..samples {
            if let Some(prefix) = self.prefix {
                commands.trace(prefix, self.extent)?;
                commands.barrier();
            }
            if let Some(queries) = queries {
                commands.barrier();
                commands.write_timestamp(queries, sample * 4)?;
            }
            commands.trace(self.transport, self.extent)?;
            commands.barrier();
            if let Some(queries) = queries {
                commands.write_timestamp(queries, sample * 4 + 1)?;
                commands.write_timestamp(queries, sample * 4 + 2)?;
            }
            commands.dispatch(
                post,
                [self.extent[0].div_ceil(8), self.extent[1].div_ceil(8), 1],
            )?;
            commands.barrier();
            if let Some(queries) = queries {
                commands.write_timestamp(queries, sample * 4 + 3)?;
            }
        }
        Ok(commands.finish(context)?)
    }

    fn post(&self, index: usize) -> Result<ComputePipeline, Error> {
        self.posts
            .get(index)
            .copied()
            .ok_or_else(|| vulkan::Error::InvalidArgument("PT后处理策略越界").into())
    }
}
