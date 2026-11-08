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
    /// 实时首视线阶段；只写本帧共享首命中，离线 record 不执行它。
    pub primary: Option<RayShader<'a>>,
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
    /// 已准备的叶子callable；实时首面与后续Hit Group使用同一材质计划顺序。
    pub callables: &'a [RayShader<'a>],
    /// 后处理：8×8×1 compute，合成、累积与批末输出。
    pub post: RayShader<'a>,
}

/// 两个宿主共用的阶段配置；GPU 程序和绑定唯一归 Context 所有。
///
/// 同一视图的工作缓冲按样本顺序复用；调用方维护跨批次参数更新与提交顺序。
/// 返回的 [`Recorded`] 在 Context 内登记使用，宿主须显式提交或取消；外部 Flight 真实完成后再关闭。
pub struct Renderer {
    primary: Option<RayTracingPipeline>,
    prefix: Option<RayTracingPipeline>,
    transport: RayTracingPipeline,
    post: ComputePipeline,
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
        let primary = programs
            .primary
            .map(|primary| {
                RayTracingPipeline::new(
                    context,
                    trace,
                    [primary, programs.miss, programs.closest_hit],
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
        let post = ComputePipeline::new(context, post, programs.post)?;
        Ok(Self {
            primary,
            prefix,
            transport,
            post,
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
            primary: self.primary.map(|p| p.rebind(context, trace)).transpose()?,
            prefix: self
                .prefix
                .map(|prefix| prefix.rebind(context, trace))
                .transpose()?,
            transport: self.transport.rebind(context, trace)?,
            post: self.post.rebind(context, post)?,
            extent,
        })
    }

    /// 实际发布的阶段版本；只返回句柄副本，不分配或借用工作存储，供唯一owner回收。
    pub fn roots(&self) -> impl Iterator<Item = Resource> + use<> {
        self.prefix
            .map(Resource::from)
            .into_iter()
            .chain(self.primary.map(Resource::from))
            .chain([self.transport.into(), self.post.into()])
    }

    /// 按兼容前缀、输运顺序返回驱动可执行统计；省略前缀时第一个向量为空，须显式启用native统计能力。
    /// 不执行GPU，不含首视线、独立guide和compute后处理；资源或查询错误原样返回。
    pub fn executable_statistics(
        &self,
        context: &Context,
    ) -> Result<[Vec<vulkan::native::ExecutableStatistics>; 2], Error> {
        Ok([
            self.prefix
                .map(|prefix| prefix.executable_statistics(context))
                .transpose()?
                .unwrap_or_default(),
            self.transport.executable_statistics(context)?,
        ])
    }

    /// 目标宽高；公共 HDR 按左上原点解释，宿主呈现转换单独执行。
    pub const fn extent(&self) -> [u32; 2] {
        self.extent
    }

    /// 逐样本录制可选K1→K2→K3和实际阶段间可见性，不提交、不等待。
    /// 交接及累积缓冲由绑定所有者持有，不按 samples 倍增。
    pub fn record(&self, context: &mut Context, samples: u32) -> Result<Recorded, Error> {
        self.record_inner(context, samples, None)
    }

    /// 实时四阶段：首视线、可选确定性导引、输运、后处理。输运在一次dispatch中
    /// 消费1 spp随机或2 spp分离策略；首命中和导引每帧只生成一次。
    /// guide须与trace绑定相同的首命中ABI；调用者保活其资源。
    /// queries若提供须恰有六槽，依次包围首视线、输运和后处理；guide_queries须有两槽。
    ///
    /// # Errors
    /// 未配置首视线、查询容量不符或录制失败时返回错误，不提交命令。
    pub fn record_realtime(
        &self,
        context: &mut Context,
        guide: Option<RayTracingPipeline>,
        queries: Option<TimestampQueries>,
        guide_queries: Option<TimestampQueries>,
    ) -> Result<Recorded, Error> {
        let primary = self
            .primary
            .ok_or(vulkan::Error::InvalidArgument("未配置实时首视线"))?;
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
        commands.trace(self.transport, self.extent)?;
        commands.barrier();
        if let Some(q) = queries {
            commands.write_timestamp(q, 3)?;
            commands.write_timestamp(q, 4)?;
        }
        commands.dispatch(
            self.post,
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
    ) -> Result<Recorded, Error> {
        if samples
            .checked_mul(4)
            .is_none_or(|count| queries.len() != count)
        {
            return Err(vulkan::Error::InvalidArgument("PT timestamp 容量不足").into());
        }
        self.record_inner(context, samples, Some(queries))
    }

    fn record_inner(
        &self,
        context: &mut Context,
        samples: u32,
        queries: Option<TimestampQueries>,
    ) -> Result<Recorded, Error> {
        if samples == 0 {
            return Err(crate::Error::Samples.into());
        }
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
                self.post,
                [self.extent[0].div_ceil(8), self.extent[1].div_ceil(8), 1],
            )?;
            commands.barrier();
            if let Some(queries) = queries {
                commands.write_timestamp(queries, sample * 4 + 3)?;
            }
        }
        Ok(commands.finish(context)?)
    }
}
