//! 正交 pinhole 相机、锚点相对坐标、无 jitter 矩阵与 Halton 像素偏移。
//!
//! 世界轴由宿主定义；`right/up/forward` 分别指画面右、上、前，正 view-Z 在相机前方。
//! 像素原点在左上，边界为整数，射线经过 `pixel + 0.5 + jitter`；jitter 以输入像素计。
//! 视锥 aspect 属于最终输出，采样 extent 属于实际输入，两者可以不同。相机不持有图像、
//! SDK 或历史策略；调用方决定何时接受前帧与何时重置历史。
//! 所有计算串行且无分配，批次和调度由调用方组织。
//!
//! [`Frame`] 保留 f64 世界位置；先减显式锚点再由 [`Relative`] 保存 f32 坐标。
//! 前后相机必须转换到相同锚点后投影。所有 [`Matrix`] 为 row-major、行向量变换，
//! 投影为非反转的 `[0,1]` 深度，不含 jitter；不能直接当作 Slang 几何矩阵字节。

#![forbid(unsafe_code)]

mod matrix;
pub use matrix::{Matrix, Projection};

/// 输入不符合相机数值或几何契约。
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    /// 世界位置、锚点或相对差值不可由有限数表示。
    #[error("相机位置或锚点无效")]
    Position,
    /// 基向量非有限、零长度或不正交。
    #[error("相机基须有限、非零且正交")]
    Basis,
    /// 视锥切线、aspect 或投影系数不可由正有限 f32 表示。
    #[error("相机视锥无效")]
    Frustum,
    /// near/far 不满足有限的 `0 < near < far` 或逆投影无法表示。
    #[error("相机近远面无效")]
    Depth,
    /// 输入/输出尺寸为零。
    #[error("相机图像尺寸须非零")]
    Extent,
    /// 像素越界、UV 非有限或 jitter 不在 `[-0.5,0.5]`。
    #[error("相机采样位置无效")]
    Sample,
}

/// 非零输入尺寸；不决定视锥 aspect。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Extent([u32; 2]);

impl Extent {
    /// 校验输入尺寸。
    /// # Errors
    /// 任一轴为零时返回 [`Error::Extent`]。
    pub const fn new(width: u32, height: u32) -> Result<Self, Error> {
        if width == 0 || height == 0 {
            Err(Error::Extent)
        } else {
            Ok(Self([width, height]))
        }
    }

    /// 实际输入宽高。
    pub const fn to_array(self) -> [u32; 2] {
        self.0
    }

    /// 尺寸比；仅在本尺寸也是最终输出时用作视锥 aspect。
    pub const fn aspect(self) -> f32 {
        self.0[0] as f32 / self.0[1] as f32
    }

    /// 左上原点的实际采样 UV；中心偏移以输入像素计。
    /// # Errors
    /// 像素越界、jitter 非有限或超出 `[-0.5,0.5]` 时返回 [`Error::Sample`]。
    pub fn sample_uv(self, pixel: [u32; 2], jitter: [f32; 2]) -> Result<[f32; 2], Error> {
        if (0..2).any(|i| pixel[i] >= self.0[i])
            || jitter.iter().any(|v| !v.is_finite() || v.abs() > 0.5)
        {
            return Err(Error::Sample);
        }
        Ok(std::array::from_fn(|i| {
            (pixel[i] as f32 + 0.5 + jitter[i]) / self.0[i] as f32
        }))
    }
}

/// 归一化视锥与 f64 绝对位置；不拥有可变的帧历史。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    origin: [f64; 3],
    camera: Relative,
}

impl Frame {
    /// 从追踪用的半视锥基构造；三个原长度分别为 `f/r/u`，得到 `tangent=u/f`、`aspect=r/u`。
    ///
    /// 各向量独立归一化；单位基点积容限为 `1e-4`，不会纠正倾斜相机。
    /// 全部计算无分配、无调度。right/up 必须对应实际输出视锥，而非取整后的内部尺寸。
    /// # Errors
    /// 非有限位置、退化/倾斜基或不可表示视锥分别返回对应 [`Error`]。
    pub fn from_scaled(
        origin: [f64; 3],
        forward: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
    ) -> Result<Self, Error> {
        if origin.iter().any(|v| !v.is_finite()) {
            return Err(Error::Position);
        }
        let mut axes = [forward, right, up];
        let mut lengths = [0.0; 3];
        for (axis, length) in axes.iter_mut().zip(&mut lengths) {
            if axis.iter().any(|v| !v.is_finite()) {
                return Err(Error::Basis);
            }
            *length = axis
                .iter()
                .map(|v| f64::from(*v).powi(2))
                .sum::<f64>()
                .sqrt();
            if *length == 0.0 {
                return Err(Error::Basis);
            }
            *axis = axis.map(|v| (f64::from(v) / *length) as f32);
        }
        let [forward, right, up] = axes;
        if [dot(forward, right), dot(forward, up), dot(right, up)]
            .iter()
            .any(|v| v.abs() > 1e-4)
        {
            return Err(Error::Basis);
        }
        let tangent = (lengths[2] / lengths[0]) as f32;
        let aspect = (lengths[1] / lengths[2]) as f32;
        validate_frustum(tangent, aspect)?;
        Ok(Self {
            origin,
            camera: Relative {
                position: [0.0; 3],
                forward,
                right,
                up,
                tangent,
                aspect,
            },
        })
    }

    /// f64 世界位置。
    pub const fn origin(self) -> [f64; 3] {
        self.origin
    }

    /// 先在 f64 中减锚点，再舍入为 f32；方向和视锥不变。
    /// # Errors
    /// 非有限锚点或差值不能由有限 f32 表示时返回 [`Error::Position`]。
    pub fn relative(self, anchor: [f64; 3]) -> Result<Relative, Error> {
        let position = std::array::from_fn(|i| (self.origin[i] - anchor[i]) as f32);
        if anchor.iter().any(|v| !v.is_finite()) || position.iter().any(|v| !v.is_finite()) {
            return Err(Error::Position);
        }
        Ok(Relative {
            position,
            ..self.camera
        })
    }
}

/// 同一显式锚点下的 f32 相机；字段由已校验的 [`Frame`] 维护。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Relative {
    position: [f32; 3],
    forward: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    tangent: f32,
    aspect: f32,
}

/// 相机射线；direction 为单位世界方向，距离参数单位同场景。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    /// 锚点相对起点。
    pub origin: [f32; 3],
    /// 单位方向。
    pub direction: [f32; 3],
}

/// 无 jitter 投影；pixel 为左上原点的输入像素位置，允许位于画面外。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projected {
    /// 未裁剪的像素位置，像素中心带 `.5`。
    pub pixel: [f32; 2],
    /// 正向线性深度；不是射线距离。
    pub view_z: f32,
}

impl Relative {
    /// 锚点相对位置。
    pub const fn position(self) -> [f32; 3] {
        self.position
    }
    /// 正交单位前向。
    pub const fn forward(self) -> [f32; 3] {
        self.forward
    }
    /// 正交单位右向。
    pub const fn right(self) -> [f32; 3] {
        self.right
    }
    /// 正交单位上向。
    pub const fn up(self) -> [f32; 3] {
        self.up
    }
    /// 垂直半视野切线。
    pub const fn tangent(self) -> f32 {
        self.tangent
    }
    /// 最终输出视锥的宽高比。
    pub const fn aspect(self) -> f32 {
        self.aspect
    }

    /// `position.xyz/tangent, forward.xyz/aspect, right.xyz/0, up.xyz/0`，与 Slang `geometry::Camera` 一致。
    pub const fn words(self) -> [f32; 16] {
        let [p, f, r, u] = [self.position, self.forward, self.right, self.up];
        [
            p[0],
            p[1],
            p[2],
            self.tangent,
            f[0],
            f[1],
            f[2],
            self.aspect,
            r[0],
            r[1],
            r[2],
            0.0,
            u[0],
            u[1],
            u[2],
            0.0,
        ]
    }

    /// 从左上原点的采样 UV 生成单位方向；UV 可在画面外。
    /// # Errors
    /// UV 非有限或生成方向不能表示时返回 [`Error::Sample`]。
    pub fn direction(self, uv: [f32; 2]) -> Result<[f32; 3], Error> {
        scaled_direction(
            self.forward,
            self.right.map(|v| v * self.aspect * self.tangent),
            self.up.map(|v| v * self.tangent),
            uv,
        )
    }

    /// 射线经过输入像素中心加 jitter；extent 不改变已存储的输出 aspect。
    /// # Errors
    /// 采样位置无效时返回 [`Error::Sample`]。
    pub fn ray(self, pixel: [u32; 2], extent: Extent, jitter: [f32; 2]) -> Result<Ray, Error> {
        Ok(Ray {
            origin: self.position,
            direction: self.direction(extent.sample_uv(pixel, jitter)?)?,
        })
    }

    /// 无 jitter 投影；点必须与相机处于同一锚点。
    /// 深度不大于 `1e-6`、非有限输入或不可表示结果返回 `None`。
    pub fn project(self, point: [f32; 3], extent: Extent) -> Option<Projected> {
        self.project_direction(std::array::from_fn(|i| point[i] - self.position[i]), extent)
    }

    /// 投影世界方向，不引入相机平移，适合方向天空；失败条件同 [`Self::project`]。
    pub fn project_direction(self, direction: [f32; 3], extent: Extent) -> Option<Projected> {
        let view_z = dot(direction, self.forward);
        if view_z <= 1e-6 || !view_z.is_finite() {
            return None;
        }
        let denominator = [view_z * self.tangent * self.aspect, view_z * self.tangent];
        if denominator.iter().any(|v| !v.is_finite() || *v <= 0.) {
            return None;
        }
        let screen = [
            dot(direction, self.right) / denominator[0],
            -dot(direction, self.up) / denominator[1],
        ];
        let pixel = std::array::from_fn(|i| (screen[i] * 0.5 + 0.5) * extent.0[i] as f32);
        pixel
            .iter()
            .all(|v| v.is_finite())
            .then_some(Projected { pixel, view_z })
    }
    /// 世界相对位置到正 view-Z 视图的无 jitter 行向量矩阵。
    pub fn world_to_view(self) -> Matrix {
        let [r, u, f, p] = [self.right, self.up, self.forward, self.position];
        Matrix([
            r[0],
            u[0],
            f[0],
            0.,
            r[1],
            u[1],
            f[1],
            0.,
            r[2],
            u[2],
            f[2],
            0.,
            -dot(p, r),
            -dot(p, u),
            -dot(p, f),
            1.,
        ])
    }

    /// 视图到世界相对坐标的行向量矩阵。
    pub const fn view_to_world(self) -> Matrix {
        let [r, u, f, p] = [self.right, self.up, self.forward, self.position];
        Matrix([
            r[0], r[1], r[2], 0., u[0], u[1], u[2], 0., f[0], f[1], f[2], 0., p[0], p[1], p[2], 1.,
        ])
    }

    /// 当前 clip 到目标 clip；两相机必须处于同一锚点，各自使用无 jitter 投影。
    pub fn clip_to(
        self,
        projection: Projection,
        target: Self,
        target_projection: Projection,
    ) -> Matrix {
        projection.clip_to_view()
            * self.view_to_world()
            * target.world_to_view()
            * target_projection.view_to_clip()
    }
}

/// Halton(2,3) 的有限相位序列；不推进帧编号，也不决定历史有效性。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Jitter {
    phases: u32,
}

impl Jitter {
    /// 相位数为 `ceil(8 × output_width² / input_width²)`，夹在 `8..=256`。
    /// 尺寸仅决定序列长度，偏移始终以输入像素为单位；整数计算使用 u128 避免溢出。
    /// # Errors
    /// 任一宽度为零时返回 [`Error::Extent`]。
    pub const fn new(input_width: u32, output_width: u32) -> Result<Self, Error> {
        if input_width == 0 || output_width == 0 {
            return Err(Error::Extent);
        }
        let count = (8 * output_width as u128 * output_width as u128)
            .div_ceil(input_width as u128 * input_width as u128);
        Ok(Self {
            phases: if count < 8 {
                8
            } else if count > 256 {
                256
            } else {
                count as u32
            },
        })
    }

    /// 完整周期的相位数量。
    pub const fn phases(self) -> u32 {
        self.phases
    }

    /// index 对周期取模，首个样本使用 Halton 索引 1，返回 `[-0.5,0.5)` 中的中心偏移。
    pub const fn sample(self, index: u32) -> [f32; 2] {
        let phase = index % self.phases + 1;
        [radical(phase, 2), radical(phase, 3)]
    }
}

const fn radical(mut index: u32, base: u32) -> f32 {
    let mut weight = 1.0;
    let mut value = 0.0;
    while index != 0 {
        weight /= base as f32;
        value += (index % base) as f32 * weight;
        index /= base;
    }
    value - 0.5
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

const fn validate_frustum(tangent: f32, aspect: f32) -> Result<(), Error> {
    let x = 1.0 / (tangent * aspect);
    let y = 1.0 / tangent;
    let values = [tangent, aspect, x, y, 1.0 / x, 1.0 / y];
    let mut index = 0;
    while index < values.len() {
        if !values[index].is_finite() || values[index] <= 0.0 {
            return Err(Error::Frustum);
        }
        index += 1;
    }
    Ok(())
}

/// 从已准备的半视锥基与左上 UV 生成单位方向，不重新归一化或验证基的正交性。
/// 适合批量射线内核，调用方在批次边界校验相机；UV 可在画面外。
/// # Errors
/// UV/向量非有限、合成方向退化或中间结果不能表示时返回 [`Error::Sample`]。
pub fn scaled_direction(
    forward: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    uv: [f32; 2],
) -> Result<[f32; 3], Error> {
    if uv
        .iter()
        .chain(forward.iter())
        .chain(right.iter())
        .chain(up.iter())
        .any(|v| !v.is_finite())
    {
        return Err(Error::Sample);
    }
    let screen = uv.map(|v| v * 2.0 - 1.0);
    let direction: [f32; 3] =
        std::array::from_fn(|i| forward[i] + right[i] * screen[0] - up[i] * screen[1]);
    let length = dot(direction, direction).sqrt();
    if !length.is_finite() || length == 0.0 {
        return Err(Error::Sample);
    }
    Ok(direction.map(|v| v / length))
}

#[cfg(test)]
mod tests;
