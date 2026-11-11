//! 运行时求解的四波长球层大气、SkyView 与 Aerial 资源。
//!
//! 固定 Earth 物理模型与 balanced 网格移植自 sky_tracer 的 sky-realtime；携带的是
//! 消光、散射、相函数及坐标拟合参数，不含预烘焙散射场。`gpu` 提供 wgpu owner，
//! `native` 提供直接使用受控 Slang SPIR-V 的 Vulkan owner；共享相同求解与缓存语义。
//! 调用者拥有 encoder、提交、等待及场景遮挡。
//! 距离使用 km，方向使用右手系、+Y 向上；颜色为线性 Rec.2020。
//! SkyView 不含太阳盘，太阳直射和场景遮挡由上层积分器处理。
//!
//! 四波长传输后转换为 RGB；Aerial 的 RGB 透射是 solar 加权谱转换的近似，
//! 不能代表任意入射光谱的精确逐波长衰减。模型不包含云、局部介质或地形遮挡。
//! 来源与复现方式见 `data/README.md`。

use bytemuck::{Pod, Zeroable};

mod solar;
pub use solar::{SolarOrbit, SolarOrbitError};

#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(feature = "native")]
pub mod native;
#[cfg(any(feature = "gpu", feature = "native"))]
mod shaders;
#[cfg(any(feature = "gpu", feature = "native"))]
pub use shaders::{SHADERS, ShaderBinary};
#[cfg(any(feature = "gpu", feature = "native", test))]
mod mapping;

/// SkyView 边长；三个 elevation chart 共用同一张图。
pub const SKY_SIZE: u32 = 256;
/// Aerial 各轴 texel 数；距离轴采用 texel 中心的平方分布。
pub const AERIAL_SIZE: u32 = 32;
/// SkyView 的十二个 chart 系数与每行逆映射。
pub const MAPPING_ROWS: usize = 12 + SKY_SIZE as usize;

/// 构造、输入或设备边界错误；GPU 验证由调用者的 error scope 管理。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 原生 Vulkan 资源、上传或录制失败，保留来源。
    #[cfg(feature = "native")]
    #[error("大气 Vulkan 失败: {0}")]
    Vulkan(#[from] vulkan::Error),
    /// CPU 物理参数存储预留失败，保留分配器来源。
    #[error("大气物理参数分配失败: {0}")]
    Allocation(#[from] std::collections::TryReserveError),
    /// 嵌入坐标拟合参数解析失败。
    #[error("大气坐标参数解析失败: {0}")]
    Calibration(#[from] serde_json::Error),
    /// 内嵌物理数据与当前 shader ABI 不匹配。
    #[error("大气物理数据无效: {0}")]
    Data(&'static str),
    /// 相机、太阳或距离跨字段约束不成立。
    #[error("大气视图无效: {0}")]
    View(&'static str),
    /// 设备不能容纳当前固定网格。
    #[error("大气资源超出设备限制: {0}")]
    Limit(&'static str),
    /// 外部分配的输出纹理不符合固定图表契约。
    #[error("大气输出纹理无效: {0}")]
    Output(&'static str),
    /// 尚未记录运行时物理场初始化。
    #[error("大气场尚未初始化")]
    NotInitialized,
    /// 物理场已记录初始化，不能覆盖供现有消费者使用的场。
    #[error("大气场已经初始化")]
    AlreadyInitialized,
}

/// 观察者与太阳；水平位置由球层介质的旋转对称性消去。
///
/// `forward + x*right + y*up` 是 NDC `(x,y)` 的世界方向，顶端 `y=+1`。
/// `right/up` 已包含 `tan(fov_y/2)` 和 aspect；方向必须有限且基底非退化。
/// `sun_direction` 指向太阳，须为单位向量，容许 0.001 长度误差并规范化。
/// 支持高度 0..=36000 km；不承诺任意有限 f32 极端坐标。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// 相对半径 6360 km 球面的高度，单位 km。
    pub altitude_km: f32,
    /// 视锥中心方向。
    pub forward: [f32; 3],
    /// 视锥水平方向，已含 FOV 和 aspect 缩放。
    pub right: [f32; 3],
    /// 视锥竖直方向，已含 FOV 缩放。
    pub up: [f32; 3],
    /// 观察者局部 +Y 坐标系中指向太阳的单位方向。
    pub sun_direction: [f32; 3],
    /// Aerial 最远射线段长度，单位 km，必须为正。
    pub max_distance_km: f32,
    /// Aerial 辐亮度的非负线性 Rec.2020 倍率；SkyView 本身保持原物理尺度。
    pub radiance_scale: [f32; 3],
}

impl Default for View {
    fn default() -> Self {
        Self {
            altitude_km: 0.002,
            forward: [0.0, 0.0, 1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            sun_direction: [0.0, 1.0, 0.0],
            max_distance_km: 64.0,
            radiance_scale: [1.0; 3],
        }
    }
}

impl View {
    /// 校验输入边界并规范化容差内的太阳方向，不分配。
    /// # Errors
    /// 高度、距离、颜色、方向或基底不符合类型文档时返回错误。
    pub fn validated(mut self) -> Result<Self, Error> {
        if !self.altitude_km.is_finite() || !(0.0..=36000.0).contains(&self.altitude_km) {
            return Err(Error::View("高度不在 0..=36000 km"));
        }
        if !self.max_distance_km.is_finite() || self.max_distance_km <= 0.0 {
            return Err(Error::View("Aerial 距离必须有限且为正"));
        }
        if self
            .forward
            .iter()
            .chain(&self.right)
            .chain(&self.up)
            .chain(&self.sun_direction)
            .chain(&self.radiance_scale)
            .any(|x| !x.is_finite())
            || self.radiance_scale.iter().any(|x| *x < 0.0)
        {
            return Err(Error::View("方向或颜色倍率无效"));
        }
        let determinant = self.forward[0] as f64
            * (self.right[1] as f64 * self.up[2] as f64 - self.right[2] as f64 * self.up[1] as f64)
            - self.forward[1] as f64
                * (self.right[0] as f64 * self.up[2] as f64
                    - self.right[2] as f64 * self.up[0] as f64)
            + self.forward[2] as f64
                * (self.right[0] as f64 * self.up[1] as f64
                    - self.right[1] as f64 * self.up[0] as f64);
        if determinant == 0.0 {
            return Err(Error::View("视锥方向基底退化"));
        }
        let length = self.sun_direction.iter().map(|v| v * v).sum::<f32>().sqrt();
        if (length - 1.0).abs() > 0.001 || !length.is_finite() {
            return Err(Error::View("太阳方向不是单位向量"));
        }
        self.sun_direction = self.sun_direction.map(|v| v / length);
        Ok(self)
    }

    /// 生成 80 B AerialView：四个 column-major 矩阵列与距离／颜色倍率。
    /// 调用前应通过 [`Self::validated`]；不检查输入或分配。
    pub const fn aerial_parameters(self) -> [[f32; 4]; 5] {
        [
            [self.right[0], self.right[1], self.right[2], 0.0],
            [self.up[0], self.up[1], self.up[2], 0.0],
            [0.0; 4],
            [self.forward[0], self.forward[1], self.forward[2], 1.0],
            [
                self.max_distance_km,
                self.radiance_scale[0],
                self.radiance_scale[1],
                self.radiance_scale[2],
            ],
        ]
    }
}

/// 本次录制需要更新的缓存；稳态两项均为 `false`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Update {
    /// 高度或太阳 elevation 改变，重建 SkyView 与坐标映射。
    pub sky: bool,
    /// 视锥、太阳方向、高度、距离或辐亮度倍率改变，重建 Aerial。
    pub aerial: bool,
}

/// 资源 payload 字节估算；不含驱动分配粒度、pipeline、后端 staging 或外部消费者副本。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    /// 初始化结束后的 GPU texture/buffer payload。
    pub resident_bytes: u64,
    /// 初始化额外保留的第二 bank、incident/direction/moment scratch 与参数。
    pub initialization_bytes: u64,
    /// Rust owner 与自有 Vec 容量；不含后端内部、驱动或外部消费者副本。
    pub cpu_bytes: u64,
}

impl Memory {
    /// 当前 resident 与尚未释放的初始化 scratch 总 payload。
    pub const fn gpu_bytes(self) -> u64 {
        self.resident_bytes + self.initialization_bytes
    }
}

/// 与 Slang 同顺序的 400 B 参数；字段私有，由物理模型与 [`View`] 共同维护。
///
/// `words()` 可复制到 native `StructuredBuffer<uint4>`；内容是 IEEE f32 / u32 原始位，
/// 不做数值转换。布局与版本必须与本 crate 同批构建的 shader 匹配。
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Parameters {
    size_steps: [u32; 4],
    view: [f32; 4],
    sun: [f32; 4],
    dims: [u32; 4],
    offsets0: [u32; 4],
    offsets1: [u32; 4],
    medium: [f32; 4],
    solar: [f32; 4],
    rgb: [[f32; 4]; 4],
    solve: [u32; 4],
    extra: [u32; 4],
    mapping: [u32; 4],
    sky: [u32; 4],
    optical_segments: [[f32; 4]; 6],
    angular_mapping: [f32; 4],
    importance: [f32; 4],
    batch: [u32; 4],
}

impl Parameters {
    /// 借用 100 个原始 32-bit words；owner 更新前有效，不分配或复制。
    pub fn words(&self) -> &[u32; 100] {
        bytemuck::cast_ref(self)
    }
    /// 四个采样波长的太阳法向辐照度，单位 W m⁻² nm⁻¹。
    pub const fn solar_irradiance(&self) -> [f32; 4] {
        self.solar
    }
    /// 太阳角半径，单位 rad。
    pub const fn sun_angular_radius(&self) -> f32 {
        self.sun[2]
    }
    /// 四波长到线性 Rec.2020 的转换列，第四分量为零。
    pub const fn spectral_to_rgb(&self) -> [[f32; 4]; 4] {
        self.rgb
    }
    /// 大气外太阳的线性 Rec.2020 法向辐照度，单位 W m⁻²。
    pub const fn solar_irradiance_rgb(&self) -> [f32; 3] {
        let mut rgb = [0.0; 3];
        let mut wavelength = 0;
        while wavelength < 4 {
            let mut channel = 0;
            while channel < 3 {
                rgb[channel] += self.solar[wavelength] * self.rgb[wavelength][channel];
                channel += 1;
            }
            wavelength += 1;
        }
        rgb
    }
    /// 大气外均匀太阳盘的线性 Rec.2020 辐亮度，单位 W m⁻² sr⁻¹。
    /// 法向辐照度除以投影立体角 π sin²(半角)；方向采样 PDF 使用未投影立体角。
    pub fn sun_radiance_rgb(&self) -> [f32; 3] {
        let sine = self.sun[2].sin();
        let solid_angle = std::f32::consts::PI * sine * sine;
        self.solar_irradiance_rgb().map(|v| v / solid_angle)
    }
}

#[cfg(any(feature = "gpu", feature = "native", test))]
struct Model {
    parameters: Parameters,
    data: Vec<[f32; 4]>,
    calibration: mapping::Calibration,
}

#[cfg(any(feature = "gpu", feature = "native", test))]
impl Model {
    fn new() -> Result<Self, Error> {
        let bytes = include_bytes!("../data/params.bin");
        let parameters = bytemuck::try_pod_read_unaligned::<Parameters>(bytes)
            .map_err(|_| Error::Data("Params 必须为 400 B"))?;
        let bytes = include_bytes!("../data/medium.bin");
        if !bytes.len().is_multiple_of(16)
            || parameters.dims != [40, 160, 128, 512]
            || parameters.solve[..3] != [1536, 20, 12]
            || parameters.mapping[0] != 63
            || !(1..=40).contains(&parameters.offsets1[1])
        {
            return Err(Error::Data("balanced 网格或物理数据长度不匹配"));
        }
        let mut data = Vec::new();
        data.try_reserve_exact(bytes.len() / 16)?;
        data.extend(
            bytes
                .as_chunks::<16>()
                .0
                .iter()
                .map(|row| bytemuck::pod_read_unaligned::<[f32; 4]>(row)),
        );
        Ok(Self {
            parameters,
            data,
            calibration: mapping::Calibration::new()?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg(any(feature = "gpu", feature = "native", test))]
struct SkyKey {
    height: f32,
    elevation: f32,
}

#[cfg(any(feature = "gpu", feature = "native", test))]
#[derive(Clone)]
struct FrameState {
    parameters: Parameters,
    mapping: [[f32; 4]; MAPPING_ROWS],
    last_sky: Option<SkyKey>,
    last_aerial: Option<View>,
    view: Option<View>,
    pending: Update,
}

#[cfg(any(feature = "gpu", feature = "native", test))]
impl FrameState {
    #[cfg(any(feature = "gpu", test))]
    fn invalidate_view(&mut self) {
        self.last_sky = None;
        self.last_aerial = None;
        self.pending = Update::default();
    }

    fn new(parameters: Parameters) -> Self {
        Self {
            parameters,
            mapping: [[0.0; 4]; MAPPING_ROWS],
            last_sky: None,
            last_aerial: None,
            view: None,
            pending: Update::default(),
        }
    }
    fn prepare(&mut self, view: View, calibration: &mapping::Calibration) -> Result<Update, Error> {
        let view = view.validated()?;
        let elevation = view.sun_direction[1].clamp(-1.0, 1.0).asin();
        let key = SkyKey {
            height: view.altitude_km,
            elevation,
        };
        let previous_pending = self.pending;
        self.pending = Update {
            sky: self.last_sky != Some(key),
            aerial: self.last_aerial != Some(view),
        };
        self.parameters.view[3] = view.altitude_km;
        self.parameters.sun[0] = view.sun_direction[0].atan2(view.sun_direction[2]);
        self.parameters.sun[1] = elevation;
        self.parameters.size_steps = [SKY_SIZE, SKY_SIZE, 64, 1];
        if self.pending.sky || previous_pending.sky {
            let (mapping, layout) = calibration.cache(view.altitude_km, elevation);
            self.mapping = mapping;
            self.parameters.sky = layout;
        }
        self.view = Some(view);
        Ok(self.pending)
    }
    fn commit(&mut self) -> Update {
        let update = std::mem::take(&mut self.pending);
        if let Some(view) = self.view {
            self.last_sky = Some(SkyKey {
                height: view.altitude_km,
                elevation: self.parameters.sun[1],
            });
            self.last_aerial = Some(view);
        }
        update
    }
}

#[cfg(test)]
mod tests;
