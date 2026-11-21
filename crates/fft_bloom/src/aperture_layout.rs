//! 随包标定的像面边长和 FFT 中心核的尺寸，由构建端与运行端共享。
pub(crate) const APERTURE_SIDE: usize = 1825;
#[cfg(feature = "vulkan")]
pub(crate) const CORE_RADIUS: u32 = 32;
#[cfg(feature = "vulkan")]
pub(crate) const CORE_SIDE: usize = 2 * CORE_RADIUS as usize + 1;
