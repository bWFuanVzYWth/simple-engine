//! 场景无关的三阶段路径追踪：固定随机身份、样本事务与 Vulkan 阶段录制。
//!
//! 场景适配器负责求交、覆盖率、纹理到 OpenPBR 的翻译、光源和环境输入。
//! 共享 Slang 模块位于引擎 `shaders/pt/`，不读取应用材质编号或世界结构。
//! GPU 功能只录制命令；设备、提交、完成证明及呈现由宿主接入层维护。
//!
//! 每个绝对样本依次执行 K1（solid delta 前缀）、K2（剩余路径）与 K3（合成和累积）。
//! 主查询只在 raygen 中执行，材质 callable 为叶子；阴影通过场景的 RayQuery 接口查询。
//! 共享输运包含 Beer、发光、NEE/MIS、OpenPBR 散射和 eta-aware RR。末顶点保留发光与 NEE，
//! 顶点预算硬截断；dielectric 事件耗尽显式失败。单份交接缓冲在批内逐样本复用。
//!
//! `pt.denoise` 提供首表面 OpenPBR 方向 diffuse/specular 能量、世界法线和线性
//! roughness，以及前向线性深度与无 jitter 的 previous-current 像素运动计算。
//! K1 的 `IPrefixGuides` 在 alpha 接受后、delta 前缀消费前观察实际首命中；默认观察者
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
//! 或重投影质量。单轴最多 65536 像素，路径最多 64 个顶点；计数不允许 u32 回绕，
//! radiance 使用未补偿 f32 求和。完整数值前提及来源保留在对应 Slang 模块。

#![forbid(unsafe_code)]

#[cfg(feature = "vulkan")]
pub mod gpu;

/// 积分参数或绝对样本范围无效。
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    /// 每批需要至少一个样本。
    #[error("PT 每批样本数必须大于零")]
    Samples,
    /// 随机域支持最多 64 个路径顶点。
    #[error("PT 路径顶点预算须属于 1..=64")]
    Vertices,
    /// dielectric 事件预算需要非零。
    #[error("PT dielectric 事件预算必须大于零")]
    DielectricEvents,
    /// 样本计数不允许回绕或丢失整数精度。
    #[error("PT 绝对样本数超过 u32")]
    SampleOverflow,
}

/// 两个场景适配器共用的路径预算；显示、环境和材质策略单独提供。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settings {
    /// 本次增加的样本数；批次划分不改变绝对随机身份。
    pub samples: u32,
    /// 包含末次发光与 NEE 的顶点预算，属于 1..=64。
    pub max_vertices: u32,
    /// 像素与绝对样本的 scramble 种子。
    pub seed: u32,
    /// solid dielectric 散射事件上限；耗尽显式报告失败。
    pub max_dielectric_events: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            samples: 1,
            max_vertices: 12,
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
        if self.max_vertices == 0 || self.max_vertices > 64 {
            return Err(Error::Vertices);
        }
        if self.max_dielectric_events == 0 {
            return Err(Error::DielectricEvents);
        }
        Ok(self)
    }
}

/// 某一视图身份的绝对样本进度；不等同于 GPU 完成进度。
///
/// `K` 包含影响积分的输入身份，由场景所有者提供；曝光和批次长度不应进入身份。
/// 开始批次独占借用进度，成功录制后消费 [`Batch::commit`]；丢弃批次保留原进度。
/// 已录制命令的提交与取消仍由拥有资源租约的宿主接入负责。
pub struct Progress<K> {
    key: Option<K>,
    samples: u32,
}

impl<K> Default for Progress<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> Progress<K> {
    /// 创建尚无历史样本的视图。
    pub const fn new() -> Self {
        Self {
            key: None,
            samples: 0,
        }
    }

    /// 已发布的绝对样本数；只表示成功录制，不证明提交或 GPU 完成。
    pub const fn samples(&self) -> u32 {
        self.samples
    }

    /// 令下一批从样本 0 开始；调用者同时使 GPU 累积失效。
    pub fn reset(&mut self) {
        self.key = None;
        self.samples = 0;
    }
}

impl<K: PartialEq> Progress<K> {
    /// 预留一批样本；身份改变时重置，失败不修改进度。
    pub fn begin(&mut self, key: K, samples: u32) -> Result<Batch<'_, K>, Error> {
        if samples == 0 {
            return Err(Error::Samples);
        }
        let first = if self.key.as_ref() == Some(&key) {
            self.samples
        } else {
            0
        };
        let next = first.checked_add(samples).ok_or(Error::SampleOverflow)?;
        Ok(Batch {
            progress: self,
            key,
            first,
            next,
        })
    }
}

/// 独占样本事务；未消费 `commit` 时不会发布新的历史身份或计数。
pub struct Batch<'a, K> {
    progress: &'a mut Progress<K>,
    key: K,
    first: u32,
    next: u32,
}

impl<K> Batch<'_, K> {
    /// 本批第一个绝对样本。
    pub const fn first(&self) -> u32 {
        self.first
    }
    /// 本批成功后的绝对样本总数。
    pub const fn next(&self) -> u32 {
        self.next
    }
    /// 本批样本数。
    pub const fn samples(&self) -> u32 {
        self.next - self.first
    }
    /// 成功录制后发布身份和计数；不表示提交或 GPU 完成。
    pub fn commit(self) {
        self.progress.key = Some(self.key);
        self.progress.samples = self.next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batching_preserves_absolute_sample_identity() {
        let mut progress = Progress::new();
        progress.begin(7, 256).unwrap().commit();
        let batch = progress.begin(7, 7).unwrap();
        assert_eq!((batch.first(), batch.next()), (256, 263));
    }

    #[test]
    fn discarded_batch_keeps_previous_history() {
        let mut progress = Progress::new();
        progress.begin(7, 12).unwrap().commit();
        {
            let _canceled = progress.begin(8, 3).unwrap();
        }
        assert_eq!(progress.begin(7, 1).unwrap().first(), 12);
    }

    #[test]
    fn changed_identity_starts_new_accumulation() {
        let mut progress = Progress::new();
        progress.begin(7, 12).unwrap().commit();
        assert_eq!(progress.begin(8, 1).unwrap().first(), 0);
    }

    #[test]
    fn sample_overflow_keeps_previous_count() {
        let mut progress = Progress::new();
        progress.begin(7, u32::MAX).unwrap().commit();
        assert!(matches!(progress.begin(7, 1), Err(Error::SampleOverflow)));
        assert_eq!(progress.samples(), u32::MAX);
    }
}
