//! 同一 Vulkan 三阶段录制器；场景描述符与受控 shader 由应用适配器提供。

use vulkan::{
    interop::RayShader,
    native::{
        Bindings, Commands, ComputePipeline, Context, RayTracingPipeline, Recorded, Resource,
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
    /// K1：delta 前缀 raygen。
    pub prefix: RayShader<'a>,
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
    /// 资源世代去重的控制流程序，顺序与适配器的紧凑 SBT 索引一致。
    pub callables: &'a [RayShader<'a>],
    /// K3：8×8×1 compute，每样本合成、累积与批末输出。
    pub post: RayShader<'a>,
}

/// 两个宿主共用的阶段配置；GPU 程序和绑定唯一归 Context 所有。
///
/// 同一视图的工作缓冲按样本顺序复用；调用方维护跨批次参数更新与提交顺序。
/// 返回的 [`Recorded`] 在 Context 内登记使用，宿主须显式提交或取消；外部 Flight 真实完成后再关闭。
pub struct Renderer {
    prefix: RayTracingPipeline,
    transport: RayTracingPipeline,
    post: ComputePipeline,
    extent: [u32; 2],
}

impl Renderer {
    /// 原子构造完整阶段；trace 绑定供 K1/K2，post 绑定供 K3，均须属于 context。
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
        let prefix = RayTracingPipeline::new(
            context,
            trace,
            [programs.prefix, programs.miss, programs.closest_hit],
            programs.any_hit,
            &[],
        )?;
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
            prefix: self.prefix.rebind(context, trace)?,
            transport: self.transport.rebind(context, trace)?,
            post: self.post.rebind(context, post)?,
            extent,
        })
    }

    /// 当前发布的三阶段版本；供唯一 owner 的资源回收使用。
    pub fn roots(&self) -> [Resource; 3] {
        [self.prefix.into(), self.transport.into(), self.post.into()]
    }

    /// 按 K1、K2 顺序返回链接后驱动可执行统计；须显式启用 native 统计能力。
    /// 不执行 GPU，不含 compute K3；资源或查询错误原样返回。
    pub fn executable_statistics(
        &self,
        context: &Context,
    ) -> Result<[Vec<vulkan::native::ExecutableStatistics>; 2], Error> {
        Ok([
            self.prefix.executable_statistics(context)?,
            self.transport.executable_statistics(context)?,
        ])
    }

    /// 目标宽高；公共 HDR 按左上原点解释，宿主呈现转换单独执行。
    pub const fn extent(&self) -> [u32; 2] {
        self.extent
    }

    /// 逐样本录制 K1→K2→K3 和阶段间可见性，不提交、不等待。
    /// 交接及累积缓冲由绑定所有者持有，不按 samples 倍增。
    pub fn record(&self, context: &mut Context, samples: u32) -> Result<Recorded, Error> {
        if samples == 0 {
            return Err(crate::Error::Samples.into());
        }
        let mut commands = Commands::new(context)?;
        for _ in 0..samples {
            commands.trace(self.prefix, self.extent)?;
            commands.barrier();
            commands.trace(self.transport, self.extent)?;
            commands.barrier();
            commands.dispatch(
                self.post,
                [self.extent[0].div_ceil(8), self.extent[1].div_ceil(8), 1],
            )?;
            commands.barrier();
        }
        Ok(commands.finish(context)?)
    }
}
