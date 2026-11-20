//! 场景线性 HDR 泛光，放在曝光与色调映射之前。光斑核描述一个亮点向周围像素分配的光量。
//!
//! 取 RGB 正分量的最大值作为峰值，保留超过阈值的部分，按原通道比例提取高光。
//! 各颜色通道使用自己的单位和实核，画外为黑色。加法泛光将高光卷积乘以强度加回原图。
//! [`Settings::OPTICAL`] 对完整有符号线性 RGB 做能量守恒的卷积，保留 alpha。
//! 负 alpha 或含非有限分量的像素原样输出，对邻域贡献为零。
//! 输入、频谱中间值及合成亮度须在 f32 可表示范围内。

#[cfg(feature = "calibration")]
pub mod calibration;
#[cfg(all(feature = "calibration_gpu", not(target_arch = "wasm32")))]
pub mod diffraction;
#[cfg(feature = "calibration")]
mod optical_rng;
#[cfg(feature = "calibration")]
pub mod pupil;
#[cfg(feature = "calibration")]
mod pupil_outline;

/// native Vulkan 的核频谱准备与逐帧录制。
#[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
pub mod gpu;

#[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
mod rect;

/// 光斑核每轴最大半径，单位为原图像素。
pub const MAX_RADIUS: u32 = 1024;

const APERTURE_SIDE: usize = 1825;

/// 随包镜头的完整 RGB 点扩散函数。每张核覆盖 1825×1825 个原图像素。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Aperture {
    /// 衍射核工作台导出的相机光圈，对应 `camara.rgb32le`。
    #[default]
    Camera,
    /// 衍射核工作台导出的瞳孔，对应 `pupil.rgb32le`。
    Pupil,
}

impl Aperture {
    /// 内置的完整、逐行交错 RGB 小端 f32 标定。其边长为 1825，中心格为零位移。
    pub const fn rgb32le(self) -> &'static [u8; APERTURE_SIDE * APERTURE_SIDE * 12] {
        match self {
            Self::Camera => include_bytes!("../data/camara.rgb32le"),
            Self::Pupil => include_bytes!("../data/pupil.rgb32le"),
        }
    }

    /// 解码随包标定的完整像面。文件已按完整支持域逐通道归一化，CPU 和 GPU 消费同一组系数。
    /// # Errors
    /// 无法为通道保留空间时返回 [`Error::Allocation`]。
    pub fn kernel(self) -> Result<Kernel, Error> {
        Ok(Kernel {
            extent: [APERTURE_SIDE as u32; 2],
            weights: Weights::Rgb(Kernel::decode(
                APERTURE_SIDE * APERTURE_SIDE,
                self.rgb32le().as_chunks::<12>().0.iter(),
            )?),
        })
    }
}

/// 构造光斑或高光提取参数失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 宽高须为非零奇数，每轴半径不超过 [`MAX_RADIUS`]，权重数须等于面积。
    #[error("泛光核宽高须为 1 到 2049 的奇数，权重数须等于宽乘高")]
    KernelShape,
    /// 每个权重须有限且非负，总和须大于零。
    #[error("泛光核需要非负有限权重且总和为正")]
    KernelWeight,
    /// 高斯标准差须为有限正数，单位为像素。
    #[error("泛光高斯标准差须为有限正数")]
    Sigma,
    /// 场景线性阈值须为有限非负数。
    #[error("泛光阈值须为有限非负数")]
    Threshold,
    /// 高光叠加强度须为有限非负数。
    #[error("泛光强度须为有限非负数")]
    Intensity,
    /// CPU 无法为光斑核申请容量。
    #[error("分配泛光核失败")]
    Allocation(#[from] std::collections::TryReserveError),
}

/// 已归一化的二维光斑。中心格对应零位移，中心右侧的权重将光分到亮点右侧。
#[derive(Debug)]
pub struct Kernel {
    extent: [u32; 2],
    weights: Weights,
}

#[derive(Debug)]
enum Weights {
    Shared(Vec<f32>),
    Rgb([Vec<f32>; 3]),
}

impl Kernel {
    /// 读取按行交错的 RGB 小端 f32 权重，各通道按完整支持域归一化。左上是首像素，中心格为零位移。
    /// # Errors
    /// 宽高须为 1–2049 范围的奇数且每像素恰有 12 字节。权重或分配错误同 [`Self::from_rgb`]。
    pub fn from_rgb32le(extent: [u32; 2], bytes: &[u8]) -> Result<Self, Error> {
        if extent
            .iter()
            .any(|n| n.is_multiple_of(2) || *n > 2 * MAX_RADIUS + 1)
        {
            return Err(Error::KernelShape);
        }
        let pixels = (extent[0] * extent[1]) as usize;
        if bytes.len() != 12 * pixels {
            return Err(Error::KernelShape);
        }
        Self::from_rgb(
            extent,
            Self::decode(pixels, bytes.as_chunks::<12>().0.iter())?,
        )
    }

    /// 接管逐行权重，用 f64 求和并就地归一化。宽高可不同，每轴半径至多 [`MAX_RADIUS`]。
    ///
    /// # Errors
    /// 尺寸或元素数不匹配返回 [`Error::KernelShape`]，无效权重返回 [`Error::KernelWeight`]。
    pub fn new(extent: [u32; 2], mut weights: Vec<f32>) -> Result<Self, Error> {
        Self::normalize(extent, &mut weights)?;
        Ok(Self {
            extent,
            weights: Weights::Shared(weights),
        })
    }

    /// 接管按 R、G、B 排列的三个同尺寸核，各自归一化，各通道独立卷积。
    /// # Errors
    /// 每个通道的输入边界与 [`Self::new`] 相同。
    pub fn from_rgb(extent: [u32; 2], mut channels: [Vec<f32>; 3]) -> Result<Self, Error> {
        for weights in &mut channels {
            Self::normalize(extent, weights)?;
        }
        Ok(Self {
            extent,
            weights: Weights::Rgb(channels),
        })
    }

    fn normalize(extent: [u32; 2], weights: &mut [f32]) -> Result<(), Error> {
        if extent
            .iter()
            .any(|&n| n.is_multiple_of(2) || n > 2 * MAX_RADIUS + 1)
            || weights.len() != (extent[0] * extent[1]) as usize
        {
            return Err(Error::KernelShape);
        }
        if weights.iter().any(|&w| !w.is_finite() || w < 0.0) {
            return Err(Error::KernelWeight);
        }
        let mass: f64 = weights.iter().map(|&w| f64::from(w)).sum();
        if mass == 0.0 {
            return Err(Error::KernelWeight);
        }
        for weight in weights {
            *weight = (f64::from(*weight) / mass) as f32;
        }
        Ok(())
    }

    /// 生成半径内的方形离散高斯核。标准差和截断半径均以原图像素计，半径零给出单位脉冲。
    ///
    /// # Errors
    /// 半径超出 [`MAX_RADIUS`]、标准差非正或非有限、分配失败时返回对应错误。
    pub fn gaussian(radius: u32, sigma: f32) -> Result<Self, Error> {
        if radius > MAX_RADIUS {
            return Err(Error::KernelShape);
        }
        if !sigma.is_finite() || sigma <= 0.0 {
            return Err(Error::Sigma);
        }
        let size = 2 * radius + 1;
        let exponent = -0.5 / f64::from(sigma).powi(2);
        let mut weights = Vec::new();
        weights.try_reserve_exact((size * size) as usize)?;
        for y in 0..size {
            for x in 0..size {
                let r2 = x.abs_diff(radius).pow(2) + y.abs_diff(radius).pow(2);
                weights.push((f64::from(r2) * exponent).exp() as f32);
            }
        }
        Self::new([size; 2], weights)
    }

    /// 权重图的宽和高，单位为像素。
    pub const fn extent(&self) -> [u32; 2] {
        self.extent
    }

    /// 三个通道从左上到右下的单位和权重。消色差核的三份借用指向相同数据。
    pub const fn channels(&self) -> [&[f32]; 3] {
        match &self.weights {
            Weights::Shared(weights) => [weights.as_slice(); 3],
            Weights::Rgb([r, g, b]) => [r.as_slice(), g.as_slice(), b.as_slice()],
        }
    }

    fn decode<'a>(
        pixels: usize,
        texels: impl Iterator<Item = &'a [u8; 12]>,
    ) -> Result<[Vec<f32>; 3], std::collections::TryReserveError> {
        let mut channels = [Vec::new(), Vec::new(), Vec::new()];
        for channel in &mut channels {
            channel.try_reserve_exact(pixels)?;
        }
        for texel in texels {
            for (channel, word) in channels.iter_mut().zip(texel.as_chunks::<4>().0) {
                channel.push(f32::from_le_bytes(*word));
            }
        }
        Ok(channels)
    }
}

/// 高光提取的场景线性阈值和卷积后的叠加强度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    threshold: f32,
    intensity: f32,
    optical: bool,
}

impl Settings {
    /// 默认从峰值 1 以上提取高光，按 0.1 倍叠加到输入。
    pub const DEFAULT: Self = Self {
        threshold: 1.0,
        intensity: 0.1,
        optical: false,
    };

    /// 完整镜头点扩散函数，直接重新分布场景光量。中心系数在空间域合成以保留高亮动态范围。
    pub const OPTICAL: Self = Self {
        threshold: 0.0,
        intensity: 1.0,
        optical: true,
    };

    /// 阈值为零时提取全部正 RGB，强度为零时保留原图。
    ///
    /// # Errors
    /// 阈值或强度非有限或为负时分别返回 [`Error::Threshold`] 或 [`Error::Intensity`]。
    pub const fn new(threshold: f32, intensity: f32) -> Result<Self, Error> {
        if !threshold.is_finite() || threshold < 0.0 {
            return Err(Error::Threshold);
        }
        if !intensity.is_finite() || intensity < 0.0 {
            return Err(Error::Intensity);
        }
        Ok(Self {
            threshold,
            intensity,
            optical: false,
        })
    }

    /// 返回线性阈值和叠加强度，顺序与 Slang 参数块中的浮点字段一致。
    pub const fn to_array(self) -> [f32; 2] {
        [self.threshold, self.intensity]
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_rectangular_kernel_preserves_interleaved_channel_order() -> Result<(), Error> {
        let texels = [0_f32, 1., 0., 0., 0., 2., 4., 0., 0.];
        let bytes = texels
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let kernel = Kernel::from_rgb32le([1, 3], &bytes)?;
        assert_eq!(
            kernel.channels(),
            [&[0., 0., 1.][..], &[1., 0., 0.], &[0., 1., 0.]]
        );
        Ok(())
    }

    #[test]
    fn normalizes_hdr_weights_without_overflow() -> Result<(), Error> {
        let kernel = Kernel::new([3, 1], vec![f32::MAX, 0.0, f32::MAX])?;
        assert_eq!(kernel.channels()[0], &[0.5, 0.0, 0.5]);
        Ok(())
    }

    #[test]
    fn calibrated_apertures_preserve_rgb_flux() -> Result<(), Error> {
        for shape in [Aperture::Camera, Aperture::Pupil] {
            let kernel = shape.kernel()?;
            for weights in kernel.channels() {
                assert!(weights.iter().all(|&w| w.is_finite() && w >= 0.));
                assert!((weights.iter().copied().map(f64::from).sum::<f64>() - 1.).abs() < 1e-7);
            }
        }
        Ok(())
    }

    #[test]
    fn rejects_mismatched_kernel_shape() {
        for shape in [[2, 1], [1, u32::MAX], [3, 3]] {
            assert!(matches!(
                Kernel::new(shape, vec![1.0]),
                Err(Error::KernelShape)
            ));
        }
    }

    #[test]
    fn rejects_undefined_kernel_mass() {
        for weight in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(matches!(
                Kernel::new([1; 2], vec![weight]),
                Err(Error::KernelWeight)
            ));
        }
    }

    #[test]
    fn rejects_invalid_highlight_controls() {
        for value in [-1.0, f32::NAN, f32::INFINITY] {
            assert!(matches!(Settings::new(value, 1.0), Err(Error::Threshold)));
            assert!(matches!(Settings::new(1.0, value), Err(Error::Intensity)));
        }
    }
}
