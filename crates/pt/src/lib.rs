//! 场景无关的路径追踪：固定随机身份、样本事务与 Vulkan 阶段录制。
//!
//! 场景适配器负责求交、覆盖率、纹理到 OpenPBR 的翻译、光源和环境输入。
//! 共享 Slang 模块位于引擎 `shaders/pt/`，不读取应用材质编号或世界结构。
//! GPU 功能只录制命令；设备、提交、完成证明及呈现由宿主接入层维护。
//!
//! 实时普通PT依次执行共享首视线、独立确定性delta导引、随机输运和后处理。
//! 首命中缓存由宿主定义，导引和每条输运路径从同一首表面重启，不共用delta后缀。
//! `pt.hit_transport` 按准备好的材质程序选择Hit Group，经可选SER执行closest-hit。
//! 缓存首表面通过同键callable执行；`pt.material_vertex`交接为88B，首面只输入样本号、
//! 通道号与已完成路径的辐射和。材质解码、选灯、阴影、BSDF、MIS和roulette均在特化叶子中。
//! 首面启发式按界面反射／进入内部平衡提案；漫反射与薄层SSS属于后者。
//! 1 spp随机选组，2 spp对选择维度分层，单一组退化为两份样本。响应不改，MIS消费新密度。
//! 后续反弹、离线参考及ReSTIR保留材质概率。离线入口不生成导引，逐样本追踪完整路径，
//! 后处理仅负责累计。未启用HitObject能力的设备保留原callable兼容入口。
//! 共享输运包含Beer、发光、NEE/MIS、OpenPBR和eta-aware RR；末顶点保留发光与NEE，
//! 达到反弹上限后不再散射；dielectric事件耗尽显式失败。交接缓冲不按SPP倍增。
//!
//! `pt.denoise` 提供首表面 OpenPBR 方向 diffuse/specular 能量、世界法线和线性
//! roughness，以及前向线性深度与无 jitter 的 previous-current 像素运动计算。
//! 兼容入口的 `IPrefixGuides` 在 alpha 接受后、delta 前缀消费前观察实际首命中；默认观察者
//! 在编译期消除，不增加材质翻译、资源或存储。该首表面接口保持原语义，不包含光学 PSR。
//! `pt.reconstruction` 另提供独立规范 guide、PSR、刚体前态和反射运动；guide 使用自身
//! 查询预算，不随照明 beta、吸收或 roulette 终止。第 N 次查询命中非 delta 或真实逃逸
//! 仍完成，需要第 N+1 次时才保留已观察接口并标记预算代理。平面反射精确，折射仅为
//! 终点切平面代理；未知对象/移动光学接口使用显式运动代理。所有辅助量不含曝光、照明
//! 或路径吞吐。`pt.spatial_denoise` 提供无时域历史的 5x5 几何/材质边界保护空间核。
//! 前后相机使用同一显式锚点，坐标和运动统一左上原点；上层维护对象对应和输出格式。
//! 可选 `vulkan::streamline` 提供 SDK 适配，应用负责设备接入、历史与提交/完成证明。
//!
//! `pt.scene` 的 `SurfaceMaterial` 是当前命中的临时结果：OpenPBR Material、另一侧实际
//! IOR／RGB extinction、发光和界面透射滤色。紧凑 `Hit` 只缓存场景身份及重建属性。
//! 覆盖率、初始介质、光源分布、大气、显示与 shader 特化条件均由场景适配器提供；
//! RayQuery 不执行 any-hit，适配器须自行确认 alpha 候选或使用 OMM。
//!
//! `pt.path_sampler` 固定随机消费域，256 个绝对样本组成 epoch，分批不改变身份。
//! 选灯／事件以二样本分层退化换取组内四样本半区投影；不宣称通用高维 net、跨 epoch
//! 或重投影质量。单轴最多 65536 像素，路径最多 64 次表面反弹与一个末顶点；计数不允许 u32 回绕，
//! radiance 使用未补偿 f32 求和。完整数值前提及来源保留在对应 Slang 模块。

#![forbid(unsafe_code)]

pub mod history;

/// 实时首表面的重建采样策略；只调整界面反射与进入内部的提案选择。
/// 后续反弹仍按材质概率采样；离线累计的任意批大小不使用此枚举。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PrimarySampling {
    /// 单条路径；两类有效贡献各以1/2概率选取，单一有效类别以概率1选取。
    #[default]
    Random,
    /// 共用首视线的两条路径；选择维度各取一个半区，单一类别得到两个独立样本。
    Separated,
}

impl PrimarySampling {
    /// 每像素本帧的实际输运路径数。
    pub const fn samples(self) -> u32 {
        match self {
            Self::Random => 1,
            Self::Separated => 2,
        }
    }
    /// 切换实时策略；不隐式修改路径预算、ReSTIR或重建参数。
    pub const fn next(self) -> Self {
        match self {
            Self::Random => Self::Separated,
            Self::Separated => Self::Random,
        }
    }
}

#[cfg(feature = "vulkan")]
pub mod gpu;

/// 积分参数或绝对样本范围无效。
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    /// 每批需要至少一个样本。
    #[error("PT 每批样本数必须大于零")]
    Samples,
    /// 随机域支持最多 64 次反弹和末顶点 NEE。
    #[error("PT 最大反弹次数须属于 1..=64")]
    Bounces,
    /// dielectric 事件预算需要非零。
    #[error("PT dielectric 事件预算必须大于零")]
    DielectricEvents,
    /// 样本计数不允许回绕或丢失整数精度。
    #[error("PT 绝对样本数超过 u32")]
    SampleOverflow,
    /// 已预留的批次尚未完成编码、提交或取消。
    #[error("PT 已有待接受批次")]
    PendingBatch,
}

/// 两个场景适配器共用的路径预算；显示、环境和材质策略单独提供。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settings {
    /// 本次增加的样本数；批次划分不改变绝对随机身份。
    pub samples: u32,
    /// 最多执行的表面散射次数，属于 1..=64；末顶点仍计算发光与 NEE。
    pub max_bounces: u32,
    /// 像素与绝对样本的 scramble 种子。
    pub seed: u32,
    /// solid dielectric 散射事件上限；耗尽显式报告失败。
    pub max_dielectric_events: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            samples: 1,
            max_bounces: 12,
            seed: 0,
            max_dielectric_events: 128,
        }
    }
}

impl Settings {
    /// 校验当前随机域和路径计数的支持范围。
    pub const fn validate(self) -> Result<Self, Error> {
        if self.samples == 0 {
            return Err(Error::Samples);
        }
        if self.max_bounces == 0 || self.max_bounces > 64 {
            return Err(Error::Bounces);
        }
        if self.max_dielectric_events == 0 {
            return Err(Error::DielectricEvents);
        }
        Ok(self)
    }
}

/// 静态 PT 累积的样本进度；不等同于 GPU 完成进度或随机采样序号。
///
/// 输入变化须由实际所有者显式调用 [`Self::reset`]，`begin` 不比较身份或隐式归零。
/// 开始批次独占借用进度，成功录制后消费 [`Batch::commit`]；丢弃批次保留原进度。
/// 已录制命令的提交与取消仍由拥有资源租约的宿主接入负责。
#[derive(Default)]
pub struct Progress {
    samples: u32,
}

impl Progress {
    /// 创建没有累积样本的视图。
    pub const fn new() -> Self {
        Self { samples: 0 }
    }

    /// 已发布的累积样本数；不证明 GPU 提交或完成。
    pub const fn samples(&self) -> u32 {
        self.samples
    }

    /// 显式丢弃静态均值；调用者同时使 GPU 累积失效，不得回退独立的随机身份。
    pub fn reset(&mut self) {
        self.samples = 0;
    }

    /// 预留一批连续样本；不解释输入身份，失败不修改进度。
    pub fn begin(&mut self, samples: u32) -> Result<Batch<'_>, Error> {
        if samples == 0 {
            return Err(Error::Samples);
        }
        let first = self.samples;
        let next = first.checked_add(samples).ok_or(Error::SampleOverflow)?;
        Ok(Batch {
            progress: self,
            first,
            next,
        })
    }
}

/// 独占累积事务；未消费 `commit` 时不会发布计数。
pub struct Batch<'a> {
    progress: &'a mut Progress,
    first: u32,
    next: u32,
}

impl Batch<'_> {
    /// 本批在静态累积中的第一个样本。
    pub const fn first(&self) -> u32 {
        self.first
    }
    /// 本批成功后的累积样本总数。
    pub const fn next(&self) -> u32 {
        self.next
    }
    /// 本批样本数。
    pub const fn samples(&self) -> u32 {
        self.next - self.first
    }
    /// 成功录制后发布计数；不表示提交或 GPU 完成。
    pub fn commit(self) {
        self.progress.samples = self.next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batching_preserves_sample_identity() {
        let mut progress = Progress::new();
        progress.begin(256).unwrap().commit();
        let batch = progress.begin(7).unwrap();
        assert_eq!((batch.first(), batch.next()), (256, 263));
    }

    #[test]
    fn discarded_batch_keeps_previous_history() {
        let mut progress = Progress::new();
        progress.begin(12).unwrap().commit();
        {
            let _cancelled = progress.begin(3).unwrap();
        }
        assert_eq!(progress.samples(), 12);
    }

    #[test]
    fn explicit_reset_starts_new_accumulation() {
        let mut progress = Progress::new();
        progress.begin(12).unwrap().commit();
        progress.reset();
        assert_eq!(progress.begin(1).unwrap().first(), 0);
    }

    #[test]
    fn sample_overflow_keeps_previous_count() {
        let mut progress = Progress::new();
        progress.begin(u32::MAX).unwrap().commit();
        assert!(matches!(progress.begin(1), Err(Error::SampleOverflow)));
        assert_eq!(progress.samples(), u32::MAX);
    }

    #[test]
    fn bounce_limit_accepts_sixty_four_scatter_events() {
        let settings = Settings {
            max_bounces: 64,
            ..Settings::default()
        };
        assert!(settings.validate().is_ok());
        assert_eq!(
            Settings {
                max_bounces: 65,
                ..settings
            }
            .validate(),
            Err(Error::Bounces)
        );
    }
}
