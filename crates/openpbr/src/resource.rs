//! 发布材质时扫描实际可采纹素；应用提供拓扑、采样覆盖域和物理参数。
//!
//! 调用方须依次纳入 shader 能读取的全部 mip、动画帧及区域。最近点采样可排除
//! 未通过 alpha 的纹素；凸组合过滤必须保留这些纹素的 RGB，因为混合结果仍可能
//! 通过 alpha。运行期程序纹理或带负权重的滤波不能据此排除未覆盖值。
//! 扫描不保存纹理副本、不创建线程或 GPU 资源。sRGB8 只证明零／非零，避免将 CPU
//! transfer 的数值误当成 GPU sRGB 解码的逐位结果；非零通道保守保留线性 `[0,1]`。

use crate::{FlowError, FlowKey, InterfaceIor, MaterialRanges, Range, RangeScan};

/// 资源形状、覆盖规则、颜色或 tint 不满足分析约定。
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
pub enum Error {
    /// 已解码范围或材料参数无效，保留原错误。
    #[error(transparent)]
    Flow(#[from] FlowError),
    /// RGBA8 字节数不是完整的四通道记录。
    #[error("颜色资源须由完整 RGBA8 记录组成")]
    Pixels,
    /// 没有纳入任何纹素；全透明但非空的资源不属于此错误。
    #[error("颜色资源没有纹素")]
    Empty,
    /// alpha 或阈值不是有限的 `[0,1]`。
    #[error("覆盖 alpha 与阈值须为有限的 0..=1")]
    Alpha,
    /// tint 范围含负值，或乘积超出支持的基色域。
    #[error("线性 tint 须非负且乘积保持在 0..=1")]
    Tint,
}

/// 当前资源的实际过滤方式；调用方保证没有遗漏采样可能读取的纹素。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filtering {
    /// 每次只读取一个纹素，包括显式 LOD 的最近点采样。
    Nearest,
    /// 线性、三线性或其他非负且权重和为一的过滤。
    Convex,
}

/// 与主查询和阴影候选共同使用的覆盖规则；alpha 不表示光学透射。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Coverage {
    /// alpha 不影响是否接受表面。
    Opaque,
    /// 接受 `alpha >= threshold`；阈值由 [`ColorScan::new`] 校验。
    AlphaAtLeast(f32),
}

/// 仅在一次扫描调用内借用的真实上传数据；不要求分配解码副本。
#[derive(Clone, Copy, Debug)]
pub enum Texels<'a> {
    /// 紧密 RGBA8；RGB 通过 sRGB texture 解码，alpha 仅除以255。
    Srgb8Rgba(&'a [u8]),
    /// 已解码、未预乘的 RGBA；RGB 与 tint 使用同一线性工作空间，所有值在 `[0,1]`。
    LinearRgba(&'a [[f32; 4]]),
}

/// 传给 OpenPBR 的有效线性基色范围；不表示纹理或 tint 的所有权。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorRanges {
    rgb: [Range; 3],
}

impl ColorRanges {
    /// 常量基色；不按 sRGB 解码，也不隐含 alpha 或 tint。
    /// # Errors
    /// 非有限值或颜色超出 `[0,1]` 时保留材料字段错误。
    pub const fn constant(rgb: [f32; 3]) -> Result<Self, Error> {
        let zero = match Range::point(0.0) {
            Ok(value) => value,
            Err(error) => return Err(Error::Flow(error)),
        };
        let mut ranges = [zero; 3];
        let mut axis = 0;
        while axis < 3 {
            ranges[axis] = match Range::point(rgb[axis]) {
                Ok(value) => value,
                Err(error) => return Err(Error::Flow(error)),
            };
            if ranges[axis].min() < 0.0 || ranges[axis].max() > 1.0 {
                return Err(Error::Flow(FlowError::Material(crate::Error::BaseColor)));
            }
            axis += 1;
        }
        Ok(Self { rgb: ranges })
    }

    /// 三个线性通道的闭区间，已包含覆盖与 tint；sRGB8 的非零域是保守范围。
    pub const fn rgb(self) -> [Range; 3] {
        self.rgb
    }

    /// 合并同一材质绑定可读取的颜色来源；可组合不同面、palette 与独立覆盖规则。
    pub const fn union(self, other: Self) -> Self {
        Self {
            rgb: [
                self.rgb[0].union(other.rgb[0]),
                self.rgb[1].union(other.rgb[1]),
                self.rgb[2].union(other.rgb[2]),
            ],
        }
    }
}

/// 一代资源的流式扫描器；失败的 `extend` 不发布该批的部分范围。
#[derive(Clone, Copy, Debug)]
pub struct ColorScan {
    channels: [RangeScan; 3],
    coverage: Coverage,
    filtering: Filtering,
    seen: bool,
}

impl ColorScan {
    /// 建立真实采样／覆盖约定；所有 mip、帧和区域须使用同一约定。
    /// # Errors
    /// alpha 阈值非有限或超出 `[0,1]` 时返回 [`Error::Alpha`]。
    pub const fn new(coverage: Coverage, filtering: Filtering) -> Result<Self, Error> {
        if let Coverage::AlphaAtLeast(threshold) = coverage
            && (!threshold.is_finite() || threshold < 0.0 || threshold > 1.0)
        {
            return Err(Error::Alpha);
        }
        Ok(Self {
            channels: [RangeScan::new(); 3],
            coverage,
            filtering,
            seen: false,
        })
    }

    fn accepted(self, alpha: f32) -> bool {
        self.filtering == Filtering::Convex
            || match self.coverage {
                Coverage::Opaque => true,
                Coverage::AlphaAtLeast(threshold) => alpha >= threshold,
            }
    }

    /// 纳入一个实际可采纹素块；可依次提交 mip／帧，不分配纹理副本。
    /// # Errors
    /// RGBA8 尾字节、线性 RGB 或 alpha 无效时返回错误，本次加入完全撤销。
    pub fn extend(&mut self, texels: Texels<'_>) -> Result<(), Error> {
        let mut next = *self;
        match texels {
            Texels::Srgb8Rgba(bytes) => {
                let (pixels, tail) = bytes.as_chunks::<4>();
                if !tail.is_empty() {
                    return Err(Error::Pixels);
                }
                let mut accepted = false;
                let mut nonzero = [false; 3];
                for &[r, g, b, a] in pixels {
                    next.seen = true;
                    if !next.accepted(f32::from(a) / 255.0) {
                        continue;
                    }
                    accepted = true;
                    for (channel, value) in nonzero.iter_mut().zip([r, g, b]) {
                        *channel |= value != 0;
                    }
                }
                if accepted {
                    // 这里只消费零／非零；整块合并一次，保守域覆盖GPU sRGB解码精度。
                    for (channel, value) in next.channels.iter_mut().zip(nonzero) {
                        channel.extend(if value { &[0.0, 1.0] } else { &[0.0] })?;
                    }
                }
            }
            Texels::LinearRgba(pixels) => {
                for &[r, g, b, a] in pixels {
                    if !a.is_finite() || !(0.0..=1.0).contains(&a) {
                        return Err(Error::Alpha);
                    }
                    if [r, g, b]
                        .into_iter()
                        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
                    {
                        return Err(FlowError::Material(crate::Error::BaseColor).into());
                    }
                    next.seen = true;
                    if next.accepted(a) {
                        for (channel, value) in next.channels.iter_mut().zip([r, g, b]) {
                            channel.extend(&[value])?;
                        }
                    }
                }
            }
        }
        *self = next;
        Ok(())
    }

    /// 完成扫描并乘入解码后的线性 tint 范围；未知非负 tint 可保守给 `[0,1]`。
    ///
    /// tint 须覆盖 shader 实际变换后的值；例如先 saturate 再 sRGB 解码的 tint
    /// 可以给 `[0,1]`。最近点下全部被拒绝的资源返回恒黑基色，其他闭包仍由物理
    /// 参数决定；凸组合过滤保守保留全部邻居，不额外证明整幅 alpha 永不可接受。
    /// # Errors
    /// 未加入纹素、tint 为负或颜色乘积超过 `[0,1]` 时返回错误。
    pub fn finish(self, tint: [Range; 3]) -> Result<ColorRanges, Error> {
        if !self.seen {
            return Err(Error::Empty);
        }
        let mut rgb = [Range::point(0.0)?; 3];
        for axis in 0..3 {
            if tint[axis].min() < 0.0 {
                return Err(Error::Tint);
            }
            let color = match self.channels[axis].finish() {
                Ok(value) => value,
                Err(FlowError::Empty) => continue,
                Err(error) => return Err(error.into()),
            };
            let min = f64::from(color.min()) * f64::from(tint[axis].min());
            let max = f64::from(color.max()) * f64::from(tint[axis].max());
            if max > 1.0 {
                return Err(Error::Tint);
            }
            // f64精确容纳两个f32尾数的乘积，向外转换覆盖f32乘法舍入。
            let lower = min as f32;
            let upper = max as f32;
            let lower = if f64::from(lower) > min {
                lower.next_down()
            } else {
                lower
            };
            let upper = if f64::from(upper) < max {
                upper.next_up()
            } else {
                upper
            };
            rgb[axis] = Range::new(lower, upper)?;
        }
        Ok(ColorRanges { rgb })
    }
}

/// 一条真实材质绑定的参数、颜色和界面范围；不保存应用编号或纹理句柄。
#[derive(Clone, Copy, Debug)]
pub struct MaterialBinding {
    ranges: MaterialRanges,
    interface: InterfaceIor,
    emission: bool,
}

impl MaterialBinding {
    /// 合并实际资源颜色与物理参数；介质可达性和非零发光由资源所有者提供。
    /// # Errors
    /// 颜色不满足 OpenPBR 基色域时保留 [`FlowError`]；不改写参数或缩小输入域。
    pub const fn new(
        ranges: MaterialRanges,
        color: ColorRanges,
        interface: InterfaceIor,
        emission: bool,
    ) -> Result<Self, FlowError> {
        let ranges = match ranges.with_base_color(color.rgb) {
            Ok(value) => value,
            Err(error) => return Err(error),
        };
        Ok(Self {
            ranges,
            interface,
            emission,
        })
    }

    /// 真实绑定的全部可达控制流；可交给 [`crate::FlowPlan::new`] 排序去重。
    pub const fn flow(self) -> FlowKey {
        self.ranges.flow(self.interface, self.emission)
    }
}

#[cfg(test)]
#[path = "resource_tests.rs"]
mod tests;
