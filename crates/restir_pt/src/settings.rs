use crate::Error;

/// 完整默认 point profile 的运行参数；材质、相机与光源由上层适配。
/// K=3、一轮 spatial、Hybrid/Compact/paired 是本 profile 的固定 shader 契约。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// NV maxSurfaceBounces 上限，属于 1..=64；不含 camera，primary 命中 vertexIndex=1。
    /// 按 NV bounce 计数传给 shader，不是旧 PT 的含末顶点消费预算。
    pub max_vertices: u32,
    /// temporal reservoir 的 M 上限；默认 20，M 按参考算法使用 f32。
    pub history_length: u32,
    /// 基础随机种子；具体 replay 身份由 shader 和场景适配器保存。
    pub seed: u32,
    /// Hybrid粗糙度阈值r；支持分量索引的材质可按线性粗糙度判断，
    /// NV通用回退使用完整sample PDF <= 1/r²，r=0时准入全部连续样本。
    pub specular_roughness_threshold: f32,
    /// NV UI的Hybrid footprint距离阈值c，须为正；上传shader前除以100。
    pub near_field_distance_threshold: f32,
    /// footprint 距离阈值的随机扰动尺度，须非负。
    pub near_field_distance_sigma: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_vertices: 10,
            history_length: 20,
            seed: 0,
            specular_roughness_threshold: 0.2,
            near_field_distance_threshold: 0.02,
            near_field_distance_sigma: 0.2,
        }
    }
}

impl Settings {
    /// 在输入边界校验路径计数与浮点阈值；不修正或 clamp 参数。
    pub const fn validate(self) -> Result<Self, Error> {
        if self.max_vertices == 0 || self.max_vertices > 64 {
            return Err(Error::Vertices);
        }
        if self.history_length == 0 {
            return Err(Error::HistoryLength);
        }
        if !self.specular_roughness_threshold.is_finite()
            || self.specular_roughness_threshold < 0.0
            || self.specular_roughness_threshold > 1.0
            || !self.near_field_distance_threshold.is_finite()
            || self.near_field_distance_threshold <= 0.0
            || !self.near_field_distance_sigma.is_finite()
            || self.near_field_distance_sigma < 0.0
        {
            return Err(Error::ShiftThreshold);
        }
        Ok(self)
    }
}
