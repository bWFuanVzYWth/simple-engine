use crate::{Error, validate_frustum};

/// row-major 的 4×4 行向量矩阵；`point * (a * b)` 表示依次应用 a、b。
/// 投影和姿态分开保存；本类型不隐式加入 jitter 或改变图像原点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix(pub(crate) [f32; 16]);

impl Matrix {
    /// 恒等矩阵。
    pub const IDENTITY: Self = Self([
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ]);

    /// row-major 的 16 个值。
    pub const fn to_array(self) -> [f32; 16] {
        self.0
    }

    /// 应用行向量变换；保留齐次 w，不执行透视除法或范围校验。
    pub fn transform(self, point: [f32; 4]) -> [f32; 4] {
        std::array::from_fn(|i| (0..4).map(|j| point[j] * self.0[j * 4 + i]).sum())
    }
}

impl std::ops::Mul for Matrix {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self(std::array::from_fn(|i| {
            (0..4)
                .map(|k| self.0[(i / 4) * 4 + k] * rhs.0[k * 4 + i % 4])
                .sum()
        }))
    }
}

/// 已校验的非反转 `[0,1]` 透视投影；near/far 为正 view-Z，clip 的 +Y 向上。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    near: f32,
    far: f32,
    view_to_clip: Matrix,
    clip_to_view: Matrix,
}

impl Projection {
    /// 构造无 jitter 的有限远投影；tangent 为垂直半视野切线，aspect 属于最终输出。
    /// # Errors
    /// 视锥无效返回 [`Error::Frustum`]；near/far 不满足 `0 < near < far`、
    /// 深度系数或逆矩阵不能由有限 f32 表示时返回 [`Error::Depth`]。
    pub const fn new(tangent: f32, aspect: f32, near: f32, far: f32) -> Result<Self, Error> {
        if let Err(error) = validate_frustum(tangent, aspect) {
            return Err(error);
        }
        if !near.is_finite() || !far.is_finite() || near <= 0.0 || far <= near {
            return Err(Error::Depth);
        }
        let x = 1.0 / (tangent * aspect);
        let y = 1.0 / tangent;
        let a = far / (far - near);
        let b = -near * a;
        let coefficients = [a, b, 1.0 / b, -a / b];
        let mut index = 0;
        while index < coefficients.len() {
            if !coefficients[index].is_finite() {
                return Err(Error::Depth);
            }
            index += 1;
        }
        Ok(Self {
            near,
            far,
            view_to_clip: Matrix([x, 0., 0., 0., 0., y, 0., 0., 0., 0., a, 1., 0., 0., b, 0.]),
            clip_to_view: Matrix([
                1. / x,
                0.,
                0.,
                0.,
                0.,
                1. / y,
                0.,
                0.,
                0.,
                0.,
                0.,
                1. / b,
                0.,
                0.,
                1.,
                -a / b,
            ]),
        })
    }

    /// 正向近裁面。
    pub const fn near(self) -> f32 {
        self.near
    }
    /// 正向远裁面。
    pub const fn far(self) -> f32 {
        self.far
    }
    /// 无 jitter 的 view 到 clip 行向量矩阵。
    pub const fn view_to_clip(self) -> Matrix {
        self.view_to_clip
    }
    /// 无 jitter 的 clip 到 view 行向量矩阵。
    pub const fn clip_to_view(self) -> Matrix {
        self.clip_to_view
    }
}
