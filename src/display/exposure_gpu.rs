//! GPU 常驻测光；输入、参数上传与提交顺序由宿主维护，不读回 CPU。

use vulkan::{
    interop::RayShader,
    native::{Binding, Bindings, Buffer, Commands, ComputePipeline, Context, Recorded, Resource},
};

/// 从 `display/exposure_compute.slang` 编译的受控程序，资源布局见该模块。
pub struct Programs<'a> {
    /// `histogram_main`；buffer 输入或 `EXPOSURE_TEXTURE=1` 的 texture 输入变体。
    pub histogram: RayShader<'a>,
    /// `update_main`；单线程裁尾统计与时间适应。
    pub update: RayShader<'a>,
}

/// 同一视图的曝光状态与直方图；复制标识不延长资源生命周期。
///
/// 输入为未曝光的场景线性 float4，负 alpha 表示无效像素。参数为
/// [`super::Frame::to_words`] 的 32 字节 storage buffer；宽高必须与构造值一致。
/// input 必须与 buffer／texture 程序变体匹配且覆盖整个 extent；宿主以受控产物维护
/// shader ABI，任意 SPIR-V 与绑定之间的语义不兼容不保证可通过返回错误发现。
/// 宿主保证 producer→本录制→显示的同队列顺序，并维护参数更新、取消和完成证明。
/// 普通相机移动或 PT 累积清零不要求曝光重置。
pub struct Exposure {
    histogram: Buffer,
    state: Buffer,
    measure: ComputePipeline,
    update: ComputePipeline,
    extent: [u32; 2],
}

impl Exposure {
    /// 创建曝光状态；input 和 parameters 分别绑定为 set0 的 1、0。
    ///
    /// # Errors
    /// 返回尺寸校验、资源分配与 pipeline 创建错误。
    pub fn new(
        context: &mut Context,
        programs: Programs<'_>,
        input: Binding,
        parameters: Binding,
        extent: [u32; 2],
    ) -> Result<Self, vulkan::Error> {
        validate_extent(extent)?;
        let histogram = Buffer::new(context, 257 * 4, false)?;
        let state = Buffer::upload(context, &[0; 16])?;
        let bindings = Bindings::new(
            context,
            &[
                parameters,
                input,
                Binding::StorageBuffer(histogram),
                Binding::StorageBuffer(state),
            ],
        )?;
        Ok(Self {
            histogram,
            state,
            measure: ComputePipeline::new(context, bindings, programs.histogram)?,
            update: ComputePipeline::new(context, bindings, programs.update)?,
            extent,
        })
    }

    /// 更换输入与尺寸，保留曝光历史；旧版本仍由 Context 的待执行／在途命令保护。
    /// # Errors
    /// 无效尺寸或绑定返回错误，不修改当前实例。
    pub fn rebind(
        &self,
        context: &mut Context,
        input: Binding,
        parameters: Binding,
        extent: [u32; 2],
    ) -> Result<Self, vulkan::Error> {
        validate_extent(extent)?;
        let bindings = Bindings::new(
            context,
            &[
                parameters,
                input,
                Binding::StorageBuffer(self.histogram),
                Binding::StorageBuffer(self.state),
            ],
        )?;
        Ok(Self {
            histogram: self.histogram,
            state: self.state,
            measure: self.measure.rebind(context, bindings)?,
            update: self.update.rebind(context, bindings)?,
            extent,
        })
    }

    /// 为另一种输入程序预建绑定，共用同一曝光与直方图；各变体必须串行录制执行。
    ///
    /// 可用于原始 buffer 与重建 texture 的切换，避免每帧创建 pipeline 或丢失适应历史。
    /// # Errors
    /// 返回与 [`Self::new`] 相同的尺寸、设备和程序错误。
    pub fn variant(
        &self,
        context: &mut Context,
        programs: Programs<'_>,
        input: Binding,
        parameters: Binding,
        extent: [u32; 2],
    ) -> Result<Self, vulkan::Error> {
        validate_extent(extent)?;
        let bindings = Bindings::new(
            context,
            &[
                parameters,
                input,
                Binding::StorageBuffer(self.histogram),
                Binding::StorageBuffer(self.state),
            ],
        )?;
        Ok(Self {
            histogram: self.histogram,
            state: self.state,
            measure: ComputePipeline::new(context, bindings, programs.histogram)?,
            update: ComputePipeline::new(context, bindings, programs.update)?,
            extent,
        })
    }

    /// 显示 shader 读取的 16 字节 `display::ExposureState`，自动倍率为 `exp2(exposure_ev)`。
    pub const fn state(&self) -> Buffer {
        self.state
    }

    /// 当前发布的 pipeline 根，同时保护参数、输入、直方图与状态依赖。
    pub fn roots(&self) -> [Resource; 2] {
        [self.measure.into(), self.update.into()]
    }

    /// 清空直方图、测光、更新状态并使结果对显示可见；不提交或等待。
    /// # Errors
    /// 返回设备资源与录制错误；失败或取消不发布 CPU 历史标记。
    pub fn record(&self, context: &mut Context) -> Result<Recorded, vulkan::Error> {
        let mut commands = Commands::new(context)?;
        self.record_into(&mut commands)?;
        commands.finish(context)
    }

    /// 追加同一设备的清零、测光与更新，可与前后的渲染阶段固化为单个录制。
    /// # Errors
    /// 命令构建器属于其他设备或工作组范围超限时返回错误；失败后应丢弃已追加前缀的构建器。
    pub fn record_into(&self, commands: &mut Commands) -> Result<(), vulkan::Error> {
        commands.barrier();
        commands.clear_buffer(self.histogram)?;
        commands.barrier();
        commands.dispatch(
            self.measure,
            [self.extent[0].div_ceil(64), self.extent[1].div_ceil(64), 1],
        )?;
        commands.barrier();
        commands.dispatch(self.update, [1; 3])?;
        commands.barrier();
        Ok(())
    }
}

fn validate_extent(extent: [u32; 2]) -> Result<(), vulkan::Error> {
    if extent.contains(&0) || u64::from(extent[0]) * u64::from(extent[1]) > u64::from(u32::MAX) {
        return Err(vulkan::Error::InvalidArgument(
            "自动曝光像素数须属于 1..=u32::MAX",
        ));
    }
    Ok(())
}
