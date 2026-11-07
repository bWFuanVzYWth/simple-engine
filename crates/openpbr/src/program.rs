//! 由资源能力生成排序的材质程序集合；支持 callable 和 closest-hit，不持有纹理或设备。
//!
//! 模板通过 `OPENPBR_FLOW` 消费控制流键，不得在源文件中重定义该宏。所有场景适配、
//! payload／descriptor ABI 和设备变体参数须进入模板源码或 args；编译器将它们纳入
//! 内容身份。准备失败不修改旧 [`ProgramSet`]，资源所有者须在整代资源与管线成功后发布。
//! 本模块沿用 shader_build 的有效性验证，不代替应用维护的物理特化／ABI 审计。

use crate::{FlowError, FlowKey, FlowPlan};
use shader_build::{
    Artifact, CompiledBatch, Recipe, RuntimeBatch, RuntimeCompiler, RuntimeTicket, Target,
};

/// 能力计划或 Slang 编译失败；保留精确底层来源。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// callable 数量或能力计划不满足公共约定。
    #[error(transparent)]
    Flow(#[from] FlowError),
    /// 源码、缓存、工具链或编译器诊断失败。
    #[error(transparent)]
    Shader(#[from] shader_build::Error),
}

/// 同一个场景／算法的受控材质模板；阶段由编译入口指定，只在准备程序时借用。
pub struct Template<'a> {
    /// 受控 Slang 主文件路径；导入闭包由 RuntimeCompiler 的源码根声明覆盖。
    pub source: &'a str,
    /// 与所选编译阶段对应的导出入口名。
    pub entry: &'a str,
    /// 内容缓存使用的逻辑输出名；不作为宿主目的目录。
    pub output: &'a str,
    /// 原生 SPIR-V profile 或其他由受控编译器支持的目标。
    pub target: Target<'a>,
    /// precise 模式、设备能力和其他实际变体参数；全部参与缓存身份。
    pub args: &'a [&'a str],
}

/// 一代绑定的能力计划与自有编译产物；键和 artifacts 保持相同排序。
pub struct ProgramSet<Key = FlowKey> {
    plan: FlowPlan<Key>,
    artifacts: Vec<Artifact>,
}

/// 已去重并登记到资源编译批次的程序集；不持有部分编译结果。
pub struct PendingProgramSet<Key = FlowKey> {
    plan: FlowPlan<Key>,
    tickets: Vec<RuntimeTicket>,
}

impl<Key> PendingProgramSet<Key> {
    /// 稳定的键及绑定映射，可在编译前继续规划其他消费者。
    pub const fn plan(&self) -> &FlowPlan<Key> {
        &self.plan
    }

    /// 从完整成功的同批次移动产物，维持键与shader的相同排序。
    /// # Errors
    /// 错误批次或已消费凭证返回编译配置错误，不返回部分程序集。
    pub fn finish(self, compiled: &mut CompiledBatch) -> Result<ProgramSet<Key>, Error> {
        let artifacts = self
            .tickets
            .into_iter()
            .map(|ticket| compiled.take(ticket))
            .collect::<Result<_, _>>()?;
        Ok(ProgramSet {
            plan: self.plan,
            artifacts,
        })
    }
}

impl ProgramSet<FlowKey> {
    /// 从真实绑定顺序去重并编译每个控制流；不更改旧程序或创建 GPU 资源。
    ///
    /// 每个唯一键追加 `-DOPENPBR_FLOW=bits`，其余模板参数按原序交给编译器。
    /// PT／ReSTIR 可用各自的受控入口调用同一方法；本模块不猜测应用 shader ABI。
    /// # Errors
    /// 计划构造失败或任一入口无法编译时保留 [`Error`] 来源，不返回部分程序集。
    pub fn compile(
        compiler: &RuntimeCompiler,
        bindings: impl IntoIterator<Item = FlowKey>,
        template: Template<'_>,
    ) -> Result<Self, Error> {
        Self::compile_inner(compiler, bindings, template, "callable", |flow| {
            (flow, None)
        })
    }
}

impl ProgramSet<(FlowKey, u32)> {
    /// 将去重后的callable入口登记到调用方批次；并行度和执行时机由资源准备层决定。
    /// # Errors
    /// 能力计划、路径或参数无效时返回[`Error`]；不创建GPU资源。
    pub fn queue_variants(
        batch: &mut RuntimeBatch<'_>,
        bindings: impl IntoIterator<Item = (FlowKey, u32)>,
        template: Template<'_>,
    ) -> Result<PendingProgramSet<(FlowKey, u32)>, Error> {
        Self::queue_inner(batch, bindings, template, "callable", |(flow, variant)| {
            (flow, Some(variant))
        })
    }

    /// 将去重后的closest-hit入口登记到调用方批次，沿用相同的键及SBT排序。
    /// # Errors
    /// 与[`Self::queue_variants`]相同。
    pub fn queue_hit_variants(
        batch: &mut RuntimeBatch<'_>,
        bindings: impl IntoIterator<Item = (FlowKey, u32)>,
        template: Template<'_>,
    ) -> Result<PendingProgramSet<(FlowKey, u32)>, Error> {
        Self::queue_inner(
            batch,
            bindings,
            template,
            "closesthit",
            |(flow, variant)| (flow, Some(variant)),
        )
    }

    /// 按 OpenPBR 控制流和调用方的离散材质能力共同去重；模板消费 `OPENPBR_VARIANT`。
    /// 变体位由场景适配定义，不参与引擎物理能力的推导。
    /// # Errors
    /// 键数量超限或任何入口编译失败时返回 [`Error`]，不发布部分程序。
    pub fn compile_variants(
        compiler: &RuntimeCompiler,
        bindings: impl IntoIterator<Item = (FlowKey, u32)>,
        template: Template<'_>,
    ) -> Result<Self, Error> {
        Self::compile_inner(
            compiler,
            bindings,
            template,
            "callable",
            |(flow, variant)| (flow, Some(variant)),
        )
    }

    /// 按相同材质键编译closest-hit叶子；仅阶段不同，不改变排序和绑定映射。
    ///
    /// 模板须提供closest-hit入口及相应payload契约。调用方在资源准备边界建立
    /// SBT并按计划索引分派，不为单个几何元素复制程序。
    /// # Errors
    /// 与[`Self::compile_variants`]相同。
    pub fn compile_hit_variants(
        compiler: &RuntimeCompiler,
        bindings: impl IntoIterator<Item = (FlowKey, u32)>,
        template: Template<'_>,
    ) -> Result<Self, Error> {
        Self::compile_inner(
            compiler,
            bindings,
            template,
            "closesthit",
            |(flow, variant)| (flow, Some(variant)),
        )
    }
}

impl<Key: Copy + Ord> ProgramSet<Key> {
    fn compile_inner(
        compiler: &RuntimeCompiler,
        bindings: impl IntoIterator<Item = Key>,
        template: Template<'_>,
        stage: &str,
        definitions: impl Fn(Key) -> (FlowKey, Option<u32>),
    ) -> Result<Self, Error> {
        let mut batch = compiler.batch();
        let pending = Self::queue_inner(&mut batch, bindings, template, stage, definitions)?;
        pending.finish(&mut batch.compile(std::num::NonZeroUsize::MIN)?)
    }

    fn queue_inner(
        batch: &mut RuntimeBatch<'_>,
        bindings: impl IntoIterator<Item = Key>,
        template: Template<'_>,
        stage: &str,
        definitions: impl Fn(Key) -> (FlowKey, Option<u32>),
    ) -> Result<PendingProgramSet<Key>, Error> {
        let plan = FlowPlan::new(bindings)?;
        let mut tickets = Vec::with_capacity(plan.keys().len());
        for &key in plan.keys() {
            let (flow, variant) = definitions(key);
            let definition = format!("-DOPENPBR_FLOW={}", flow.bits());
            let variant = variant.map(|bits| format!("-DOPENPBR_VARIANT={bits}"));
            let mut args = template.args.to_vec();
            args.push(&definition);
            if let Some(variant) = &variant {
                args.push(variant);
            }
            tickets.push(batch.queue(&Recipe {
                source: template.source,
                entry: template.entry,
                stage,
                output: template.output,
                target: template.target,
                args: &args,
            })?);
        }
        Ok(PendingProgramSet { plan, tickets })
    }

    /// 输入绑定到排序 callable 表的映射；shader 索引和 SBT 须消费同一份计划。
    pub const fn plan(&self) -> &FlowPlan<Key> {
        &self.plan
    }

    /// 按 [`FlowPlan::keys`] 顺序排列的自有产物；创建管线时只需短期借用字节。
    pub fn artifacts(&self) -> &[Artifact] {
        &self.artifacts
    }

    /// 键和全部代码身份是否相同；绑定映射变化仍须由资源所有者重新上传或重绑。
    pub fn same_code(&self, previous: &Self) -> bool {
        self.plan.keys() == previous.plan.keys()
            && self
                .artifacts
                .iter()
                .zip(&previous.artifacts)
                .all(|(a, b)| a.fingerprint() == b.fingerprint())
    }
}

#[cfg(test)]
#[path = "program_tests.rs"]
mod tests;
