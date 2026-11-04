//! 使用 SipHash-1-3 球面梯度的三维 Perlin 噪声。
//!
//! 八角点独立生成梯度，使用五次 fade 与 X→Y→Z 插值，缩放系数为 `2/sqrt(3)`。
//! 所有内核只读共享 seed、使用局部状态，不分配、不缓存，也不调度线程。
//! 批输入和输出由调用者持有；应用可通过 [`PerlinNoise::gradient_x8`] 与
//! [`PerlinNoise::sample_gradients_x8`] 复用自己拥有的梯度，缓存有效性由应用维护。
//!
//! 有效采样坐标为有限闭区间 `[-MAX_COORD, MAX_COORD]`，在该域内格点转换精确。
//! [`PerlinNoise::sample_3d_batch`] 在写输出前检查整个输入；固定八路入口用于已校验的内核。
//! 尾部继续使用相同八路算式，不切换标量 sin/cos。相同平台、seed 和坐标保持 lane 独立，
//! 不承诺不同平台浮点运算的逐位一致性。
//!
//! ```
//! use perlin_noise::PerlinNoise;
//! let noise = PerlinNoise::new((7, 11));
//! let mut output = [0.0; 3];
//! noise.sample_3d_batch(&[0.0, 0.25, -0.5], &[0.0; 3], &[0.0; 3], &mut output)?;
//! assert_eq!(output[0], 0.0);
//! # Ok::<(), perlin_noise::NoiseError>(())
//! ```

#![feature(portable_simd)]
#![forbid(unsafe_code)]

mod batch;
mod simd;

#[cfg(test)]
mod batch_tests;

use sip_hash::SipHash;

/// 采样坐标绝对值的闭区间上界；角点格坐标不受此采样域限制。
pub const MAX_COORD: f32 = 2_097_152.0;

/// 八路梯度的三个分量，依次为 x、y、z，每个分量保存八个 lane。
pub type GradientLanes = [[f32; 8]; 3];

/// 八角点梯度，角序为 000、100、010、110、001、101、011、111。
///
/// 每个 lane 的角点必须对应同一个单元；它与相对坐标的一致性由调用方维护。
pub type CornerGradients = [GradientLanes; 8];

/// 噪声输入错误；返回前输出保持原值，不涉及资源分配。
#[derive(Debug, Clone, Copy, Eq, PartialEq, thiserror::Error)]
pub enum NoiseError {
    /// 三个坐标切片与输出长度不一致。
    #[error("坐标与输出长度不一致")]
    LengthMismatch,
    /// 某个坐标不是 finite，或超出有效采样域。
    #[error("轴 {axis} 的第 {index} 个坐标无效")]
    InvalidCoordinate {
        /// 坐标轴，0、1、2 分别表示 x、y、z。
        axis: usize,
        /// 该轴切片中的元素位置。
        index: usize,
    },
}

/// 可只读共享的 seed 状态；没有输出或梯度缓存所有权。
pub struct PerlinNoise {
    pub(crate) hasher: SipHash<1, 3>,
}

impl PerlinNoise {
    /// 建立可复用 seed 状态；不分配。
    #[must_use]
    pub const fn new(seed: (u64, u64)) -> Self {
        Self {
            hasher: SipHash::new(seed),
        }
    }
}
