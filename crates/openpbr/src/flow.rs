// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
//! 资源发布时的控制流证明；键不包含材质名、纹理 ID 或具体参数值。
//!
//! 范围覆盖传给 shader 的解码后参数；调用方须包含所有 mip、动画帧和参数变换。
//! 线性／三线性过滤是非负权重的凸组合，因此端点的闭区间也覆盖过滤结果；
//! 最近点采样同样成立。带负权重的重建滤波或运行时程序纹理须另给有效上界。
//! 本模块不读取应用资产，不假定原始 texel 的 0/1 值经过过滤后仍只有两个值。
//! 控制构建使用 precise float32；阈值证明按 [Vulkan 运算精度] 向外求界，
//! 仅舍入邻域无法排除时保留 delta／continuous 两域，不承诺不同 GPU 的舍入相同。
//!
//! [Vulkan 运算精度]: https://docs.vulkan.org/spec/latest/appendices/spirvenv.html#spirvenv-precision-operation

use crate::{Error, Material};
use std::collections::BTreeMap;

/// 范围、能力声明或绑定表的输入边界错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FlowError {
    /// 区间端点非有限或上下界颠倒。
    #[error("能力分析范围须有限且 min<=max")]
    Range,
    /// 某个材质参数的整个范围超出支持子集。
    #[error(transparent)]
    Material(#[from] Error),
    /// 介质范围超出有限绝对 IOR `1..=4`。
    #[error("能力分析的介质 IOR 须在 1..=4")]
    Ior,
    /// 位组合违反当前子集；不偷偷放宽为通用入口。
    #[error("无效 OpenPBR FlowKey：{0:#x}")]
    Key(u32),
    /// 唯一能力组合数超过 callable 的 `u32` 索引空间。
    #[error("OpenPBR callable 组合数超过 u32")]
    Bindings,
    /// 没有提供解码后的纹理值。
    #[error("纹理范围没有样本")]
    Empty,
}

/// 已校验的有限闭区间；区间内可达性由资源所有者保证，不能用于排除未覆盖的值。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    min: f32,
    max: f32,
}

impl Range {
    /// 建立范围，不裁剪非有限值或颠倒的端点。
    /// # Errors
    /// 非有限端点或 `min>max` 返回 [`FlowError::Range`]。
    pub const fn new(min: f32, max: f32) -> Result<Self, FlowError> {
        if !min.is_finite() || !max.is_finite() || min > max {
            return Err(FlowError::Range);
        }
        Ok(Self { min, max })
    }

    /// 建立常量范围。
    /// # Errors
    /// 非有限输入返回 [`FlowError::Range`]。
    pub const fn point(value: f32) -> Result<Self, FlowError> {
        Self::new(value, value)
    }

    /// 最小值，包括端点。
    pub const fn min(self) -> f32 {
        self.min
    }

    /// 最大值，包括端点。
    pub const fn max(self) -> f32 {
        self.max
    }

    /// 返回两个已校验范围的最小包围区间；也覆盖端点间的线性过滤结果。
    pub const fn union(self, other: Self) -> Self {
        Self {
            min: if self.min < other.min {
                self.min
            } else {
                other.min
            },
            max: if self.max > other.max {
                self.max
            } else {
                other.max
            },
        }
    }

    /// 合并线性粗糙度与法线分布宽度，包围 `sqrt(sqrt(min(r⁴+d⁴,1)))`。
    ///
    /// 用于资源期分析；两输入须覆盖过滤后的全部 `[0,1]` 值。端点单调，按 f64
    /// 求值后外扩32个相邻f32，覆盖precise乘加及两次sqrt的舍入；接近下溢时下界取零。
    /// 不读取纹理、不修改shader求值，也不证明输入资源的覆盖域。
    /// # Errors
    /// 任一范围超出 `[0,1]` 返回 [`Error::Roughness`]。
    pub fn with_normal_distribution(self, distribution: Self) -> Result<Self, FlowError> {
        if !self.unit() || !distribution.unit() {
            return Err(FlowError::Material(Error::Roughness));
        }
        let endpoint = |r: f32, d: f32| {
            let r = f64::from(r);
            let d = f64::from(d);
            r * r * r * r + d * d * d * d
        };
        let lower = endpoint(self.min, distribution.min);
        let upper = endpoint(self.max, distribution.max);
        Ok(Self {
            min: if lower <= f64::from(f32::MIN_POSITIVE) * 8. {
                0.
            } else {
                rounded(lower.min(1.).sqrt().sqrt() as f32, 32).min
            },
            max: rounded(upper.min(1.).sqrt().sqrt() as f32, 32).max.min(1.),
        })
    }

    pub(crate) const fn reflection_measure(self) -> Result<u32, FlowError> {
        if !self.unit() {
            return Err(FlowError::Material(Error::Roughness));
        }
        let alpha = product(self, self);
        Ok((if alpha.min < 1.0e-4 {
            FlowKey::DELTA_REFLECTION
        } else {
            0
        }) | (if alpha.max >= 1.0e-4 {
            FlowKey::CONTINUOUS_REFLECTION
        } else {
            0
        }))
    }

    const fn unit(self) -> bool {
        self.min >= 0.0 && self.max <= 1.0
    }
}

/// 流式扫描已解码通道；可逐块复用，不保存纹理副本或隐藏并行调度。
#[derive(Clone, Copy, Debug, Default)]
pub struct RangeScan {
    range: Option<Range>,
}

impl RangeScan {
    /// 空扫描器；同一资源的所有 mip／帧可依次加入。
    pub const fn new() -> Self {
        Self { range: None }
    }

    /// 加入一批解码、变换后的参数值；失败时保留本批之前的范围。
    /// # Errors
    /// 任一值非有限时返回 [`FlowError::Range`]，不发布部分批次。
    pub fn extend(&mut self, values: &[f32]) -> Result<(), FlowError> {
        let mut range = self.range;
        for &value in values {
            let value = Range::point(value)?;
            range = Some(match range {
                Some(previous) => previous.union(value),
                None => value,
            });
        }
        self.range = range;
        Ok(())
    }

    /// 取得整个输入集合的凸包；不把原始端点类别当作过滤后的类别。
    /// # Errors
    /// 未加入任何值时返回 [`FlowError::Empty`]。
    pub const fn finish(self) -> Result<Range, FlowError> {
        match self.range {
            Some(range) => Ok(range),
            None => Err(FlowError::Empty),
        }
    }
}

/// shader 界面 IOR 的证明；solid 使用较大绝对 IOR 除以较小者。
///
/// 范围只描述路径可能遇到的两侧介质，不表示介质初始化或边界拓扑。
/// 可证明不含匹配 IOR 时，粗糙 solid 的 delta 分支能被删除。
/// thin 另保存 `transmitted/incident` 后规范化的范围，包含倒数和 GPU 舍入误差。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InterfaceIor {
    canonical: Range,
    solid_min: f32,
    thin: Range,
}

impl InterfaceIor {
    /// 无介质可达性证明，保留 `1..=4` 的全部界面比。
    pub const UNKNOWN: Self = Self {
        canonical: Range { min: 1.0, max: 4.0 },
        solid_min: 1.0,
        thin: thin_ior(Range { min: 1.0, max: 4.0 }, Range { min: 1.0, max: 4.0 }),
    };

    /// 根据两侧可能的绝对 IOR 计算范围，包含交换进入／离开方向。
    /// # Errors
    /// 任一绝对 IOR 超出 `1..=4` 时返回 [`FlowError::Ior`]。
    pub const fn between(incident: Range, transmitted: Range) -> Result<Self, FlowError> {
        if incident.min < 1.0
            || incident.max > 4.0
            || transmitted.min < 1.0
            || transmitted.max > 4.0
        {
            return Err(FlowError::Ior);
        }
        let min = if incident.max < transmitted.min {
            transmitted.min / incident.max
        } else if transmitted.max < incident.min {
            incident.min / transmitted.max
        } else {
            1.0
        };
        let first = incident.max / transmitted.min;
        let second = transmitted.max / incident.min;
        let solid_min = rounded(min, 6).min;
        Ok(Self {
            canonical: Range {
                min,
                max: if first > second { first } else { second },
            },
            solid_min: if solid_min < 1.0 { 1.0 } else { solid_min },
            thin: thin_ior(incident, transmitted),
        })
    }

    /// 返回 CPU 规范化的名义比值范围；solid 匹配可达性与 thin 的 GPU 舍入界在内部保留。
    pub const fn range(self) -> Range {
        self.canonical
    }
}

/// OpenPBR 支持子域的控制流键；相同键共享一个 callable，具体参数仍在命中时读取。
///
/// `CONTINUOUS_*`／`DELTA_*` 描述 specular 支持域；diffuse／SSS 自身始终连续。
/// opaque 的 dielectric／conductor 可以共存，emission 与所有散射独立。
/// solid／thin 沿用当前子集的互斥拓扑，不扩展 OpenPBR 的 transmission 混合域。
/// 键0是没有非零散射分量的纯吸收面；仅 `EMISSION` 是无散射的发光面。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowKey(u32);

impl FlowKey {
    /// 可能存在 Lambert diffuse；与 SSS 同在时保留运行时混合权重。
    pub const DIFFUSE: u32 = 1 << 0;
    /// 可能存在 dielectric reflection，包括 opaque 上层和透射界面的反射。
    pub const DIELECTRIC_REFLECTION: u32 = 1 << 1;
    /// 可能存在 conductor reflection；与 dielectric 同在时保留 fractional metalness。
    pub const CONDUCTOR_REFLECTION: u32 = 1 << 2;
    /// 可能存在当前子集的薄层 SSS。
    pub const SUBSURFACE: u32 = 1 << 3;
    /// solid dielectric transmission，透射可能更新介质。
    pub const SOLID_TRANSMISSION: u32 = 1 << 4;
    /// thin dielectric transmission，不更新路径介质。
    pub const THIN_TRANSMISSION: u32 = 1 << 5;
    /// specular reflection 可能离散，不包含 diffuse／SSS。
    pub const DELTA_REFLECTION: u32 = 1 << 6;
    /// specular reflection 可能连续，不包含 diffuse／SSS。
    pub const CONTINUOUS_REFLECTION: u32 = 1 << 7;
    /// specular transmission 可能离散，包括匹配 IOR 的直通事件。
    pub const DELTA_TRANSMISSION: u32 = 1 << 8;
    /// specular transmission 可能连续。
    pub const CONTINUOUS_TRANSMISSION: u32 = 1 << 9;
    /// emission 可能非零；未设置时 callable 不读取发光资源。
    pub const EMISSION: u32 = 1 << 10;
    /// conductor 以运行时 metalness 加权；非金属分量可完全吸收，仍须保留选择概率。
    pub const MIXED_METALNESS: u32 = 1 << 11;

    /// 校验外部保存的键；不接受未知位或当前子集不存在的组合。
    /// # Errors
    /// 位组合不满足本类型约束时返回 [`FlowError::Key`]。
    pub const fn from_bits(bits: u32) -> Result<Self, FlowError> {
        let transmission = bits & (Self::SOLID_TRANSMISSION | Self::THIN_TRANSMISSION);
        let opaque = bits & (Self::DIFFUSE | Self::CONDUCTOR_REFLECTION | Self::SUBSURFACE);
        let reflection = bits & (Self::DIELECTRIC_REFLECTION | Self::CONDUCTOR_REFLECTION);
        let reflect_domain = bits & (Self::DELTA_REFLECTION | Self::CONTINUOUS_REFLECTION);
        let transmit_domain = bits & (Self::DELTA_TRANSMISSION | Self::CONTINUOUS_TRANSMISSION);
        let dielectric = bits & Self::DIELECTRIC_REFLECTION != 0;
        let conductor = bits & Self::CONDUCTOR_REFLECTION != 0;
        let mixed = bits & Self::MIXED_METALNESS != 0;
        let layer = bits & (Self::DIELECTRIC_REFLECTION | Self::DIFFUSE | Self::SUBSURFACE) != 0;
        let delta_r = bits & Self::DELTA_REFLECTION != 0;
        let delta_t = bits & Self::DELTA_TRANSMISSION != 0;
        let continuous_r = bits & Self::CONTINUOUS_REFLECTION != 0;
        let continuous_t = bits & Self::CONTINUOUS_TRANSMISSION != 0;
        if bits & !0xfff != 0
            || transmission == Self::SOLID_TRANSMISSION | Self::THIN_TRANSMISSION
            || (transmission != 0 && opaque != 0)
            || ((reflection == 0) != (reflect_domain == 0))
            || ((transmission == 0) != (transmit_domain == 0))
            || (mixed && !conductor)
            || (conductor && layer && !mixed)
            || (transmission != 0 && !dielectric && (!delta_t || continuous_t))
            || (transmission == Self::SOLID_TRANSMISSION
                && ((delta_r && !delta_t) || continuous_r != continuous_t))
            || (transmission == Self::THIN_TRANSMISSION
                && dielectric
                && (delta_r != delta_t || continuous_r != continuous_t))
        {
            return Err(FlowError::Key(bits));
        }
        Ok(Self(bits))
    }

    /// 与 Slang `OPENPBR_FLOW` 完全一致的固定编码。
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// 是否可能有非零 emission。
    pub const fn emits(self) -> bool {
        self.0 & Self::EMISSION != 0
    }

    /// 去掉与散射独立的发光能力，供不消费emission的sample/evaluate程序去重。
    /// 不改变任何BSDF分量、测度或混合权重支持域。
    pub const fn without_emission(self) -> Self {
        Self(self.0 & !Self::EMISSION)
    }

    /// 是否可能评估连续方向；光滑塑料的 diffuse 仍使其为 true。
    pub const fn continuous(self) -> bool {
        self.0
            & (Self::DIFFUSE
                | Self::SUBSURFACE
                | Self::CONTINUOUS_REFLECTION
                | Self::CONTINUOUS_TRANSMISSION)
            != 0
    }

    /// 整个散射闭包是否只有离散事件；发光本身不改变散射支持域。
    pub const fn pure_delta(self) -> bool {
        !self.continuous() && self.0 & (Self::DELTA_REFLECTION | Self::DELTA_TRANSMISSION) != 0
    }
}

#[derive(Clone, Copy, Debug)]
enum Model {
    Opaque { metalness: Range, subsurface: Range },
    Solid,
    Thin,
}

/// 一个绑定的全部可达参数范围；输入校验只在构造时进行，分析时不重复验证。
#[derive(Clone, Copy, Debug)]
pub struct MaterialRanges {
    roughness: Range,
    model: Model,
    base_nonzero: bool,
}

impl MaterialRanges {
    /// 已校验常量材料的精确参数范围；恒黑 RGB 不产生 diffuse／SSS 响应。
    pub const fn constant(material: Material) -> Self {
        let roughness = Range {
            min: material.base_roughness[3],
            max: material.base_roughness[3],
        };
        let model = if material.flags & 1 == 0 {
            Model::Opaque {
                metalness: Range {
                    min: material.surface[1],
                    max: material.surface[1],
                },
                subsurface: Range {
                    min: material.surface[2],
                    max: material.surface[2],
                },
            }
        } else if material.flags & 2 == 0 {
            Model::Solid
        } else {
            Model::Thin
        };
        Self {
            roughness,
            model,
            base_nonzero: material.base_roughness[0] > 0.0
                || material.base_roughness[1] > 0.0
                || material.base_roughness[2] > 0.0,
        }
    }

    /// 建立 opaque 材质的解码后参数范围；保留 fractional metalness 和 SSS。
    /// # Errors
    /// 任一完整范围超出 `0..=1` 时返回对应 [`Error`]。
    pub const fn opaque(
        roughness: Range,
        metalness: Range,
        subsurface: Range,
    ) -> Result<Self, FlowError> {
        if !roughness.unit() {
            return Err(FlowError::Material(Error::Roughness));
        }
        if !metalness.unit() {
            return Err(FlowError::Material(Error::Metalness));
        }
        if !subsurface.unit() {
            return Err(FlowError::Material(Error::Subsurface));
        }
        Ok(Self {
            roughness,
            model: Model::Opaque {
                metalness,
                subsurface,
            },
            base_nonzero: true,
        })
    }

    /// 建立 solid／thin dielectric 的解码后粗糙度范围；拓扑由资源绑定固定。
    /// # Errors
    /// 整个 roughness 范围超出 `0..=1` 时返回 [`Error::Roughness`]。
    pub const fn dielectric(roughness: Range, thin: bool) -> Result<Self, FlowError> {
        if !roughness.unit() {
            return Err(FlowError::Material(Error::Roughness));
        }
        Ok(Self {
            roughness,
            model: if thin { Model::Thin } else { Model::Solid },
            base_nonzero: true,
        })
    }

    /// 补充解码、tint 和参数变换后的 RGB 范围；恒黑时删除无响应的 diffuse／SSS。
    ///
    /// [`Self::opaque`] 默认颜色可能非零；RGB 贴图不会改变 dielectric／conductor 的判定。
    /// # Errors
    /// 任一通道范围超出 `0..=1` 时返回 [`Error::BaseColor`]。
    pub const fn with_base_color(mut self, color: [Range; 3]) -> Result<Self, FlowError> {
        let mut axis = 0;
        let mut nonzero = false;
        while axis < 3 {
            if !color[axis].unit() {
                return Err(FlowError::Material(Error::BaseColor));
            }
            nonzero |= color[axis].max > 0.0;
            axis += 1;
        }
        self.base_nonzero = nonzero;
        Ok(self)
    }

    /// 推导所有可达控制流；`emission` 须覆盖资源各帧、各 mip 和参数变换后的非零发光。
    /// 未提供严格介质证明时传 [`InterfaceIor::UNKNOWN`]，不会错误裁掉匹配 IOR。
    /// opaque 的 `interface` 必须覆盖其材料 IOR 与全部 incident medium 的实际比；
    /// 只有整个比值域等于1时才删除零响应的 dielectric reflection。
    pub const fn flow(self, interface: InterfaceIor, emission: bool) -> FlowKey {
        let alpha = product(self.roughness, self.roughness);
        let min_alpha = alpha.min;
        let max_alpha = alpha.max;
        let mut bits = if emission { FlowKey::EMISSION } else { 0 };
        match self.model {
            Model::Opaque {
                metalness,
                subsurface,
            } => {
                if metalness.min < 1.0 {
                    if interface.canonical.max > 1.0 {
                        bits |= FlowKey::DIELECTRIC_REFLECTION;
                    }
                    if self.base_nonzero && subsurface.min < 1.0 {
                        bits |= FlowKey::DIFFUSE;
                    }
                    if self.base_nonzero && subsurface.max > 0.0 {
                        bits |= FlowKey::SUBSURFACE;
                    }
                }
                if metalness.max > 0.0 {
                    bits |= FlowKey::CONDUCTOR_REFLECTION;
                    if metalness.min < 1.0 {
                        bits |= FlowKey::MIXED_METALNESS;
                    }
                }
                let reflection =
                    bits & (FlowKey::DIELECTRIC_REFLECTION | FlowKey::CONDUCTOR_REFLECTION) != 0;
                if reflection && min_alpha < 1.0e-4 {
                    bits |= FlowKey::DELTA_REFLECTION;
                }
                if reflection && max_alpha >= 1.0e-4 {
                    bits |= FlowKey::CONTINUOUS_REFLECTION;
                }
            }
            Model::Solid => {
                bits |= FlowKey::SOLID_TRANSMISSION;
                if interface.canonical.max > 1.0 {
                    bits |= FlowKey::DIELECTRIC_REFLECTION;
                    if min_alpha < 1.0e-4 {
                        bits |= FlowKey::DELTA_REFLECTION;
                    }
                }
                if min_alpha < 1.0e-4 || interface.solid_min == 1.0 {
                    bits |= FlowKey::DELTA_TRANSMISSION;
                }
                if max_alpha >= 1.0e-4 && interface.canonical.max > 1.0 {
                    bits |= FlowKey::CONTINUOUS_REFLECTION | FlowKey::CONTINUOUS_TRANSMISSION;
                }
            }
            Model::Thin => {
                bits |= FlowKey::THIN_TRANSMISSION;
                if interface.canonical.max > 1.0 {
                    bits |= FlowKey::DIELECTRIC_REFLECTION;
                }
                // scaler 在 eta>=1 单调递增；仅逐端点向外求界，避免区间依赖扩大正常域。
                let scaled = product(
                    alpha,
                    Range {
                        min: thin_scaler(interface.thin.min).min,
                        max: thin_scaler(interface.thin.max).max,
                    },
                );
                let min = scaled.min;
                let max = scaled.max;
                if min < 1.0e-4 {
                    bits |= FlowKey::DELTA_TRANSMISSION;
                    if interface.canonical.max > 1.0 {
                        bits |= FlowKey::DELTA_REFLECTION;
                    }
                }
                if max >= 1.0e-4 {
                    bits |= FlowKey::CONTINUOUS_REFLECTION | FlowKey::CONTINUOUS_TRANSMISSION;
                }
            }
        }
        FlowKey(bits)
    }
}

// Vulkan float32 OpFDiv 允许 2.5 ULP；其余基本运算使用实现定义的正确舍入。
// https://docs.vulkan.org/spec/latest/appendices/spirvenv.html#spirvenv-precision-operation
// 正值域按相邻 float 向外扩展：除法6步覆盖跨 binade 的 2.5 ULP 与 CPU 舍入，
// 乘法2步覆盖正确舍入差异。仅阈值无法严格分离时保留 delta／continuous 两域。
const fn rounded(value: f32, steps: u32) -> Range {
    Range {
        min: f32::from_bits(value.to_bits().saturating_sub(steps)),
        max: f32::from_bits(value.to_bits() + steps),
    }
}

const fn product(first: Range, second: Range) -> Range {
    Range {
        min: rounded(first.min * second.min, 2).min,
        max: rounded(first.max * second.max, 2).max,
    }
}

const fn quotient(first: Range, second: Range) -> Range {
    Range {
        min: rounded(first.min / second.max, 6).min,
        max: rounded(first.max / second.min, 6).max,
    }
}

const fn thin_ior(incident: Range, transmitted: Range) -> Range {
    let ratio = quotient(transmitted, incident);
    let inverse = quotient(Range { min: 1.0, max: 1.0 }, ratio);
    let min = if ratio.max < 1.0 {
        inverse.min
    } else if ratio.min > 1.0 {
        ratio.min
    } else {
        1.0
    };
    Range {
        min: if min < 1.0 { 1.0 } else { min },
        max: if ratio.max > inverse.max {
            ratio.max
        } else {
            inverse.max
        },
    }
}

const fn thin_scaler(eta: f32) -> Range {
    if eta <= 1.0 {
        return Range { min: 0.0, max: 0.0 };
    }
    // 用单调的实数公式取端点，避免分别放大分子／分母造成区间依赖。
    // 复用平方项计入8份基本运算相对误差及2.5 ULP除法后，误差<12*EPSILON；
    // f64参考误差可忽略，转回f32后外扩32相邻值还覆盖 binade 间距变化。
    let eta = eta as f64;
    let difference = eta - 0.5;
    let value = (3.7_f32 as f64) * (eta - 1.0) * (difference * difference) / (eta * eta * eta);
    rounded(value as f32, 32)
}

impl Material {
    /// 无介质可达性证明时推导绑定能力；发光存在性由外部场景翻译后提供。
    pub const fn flow(self, emission: bool) -> FlowKey {
        MaterialRanges::constant(self).flow(InterfaceIor::UNKNOWN, emission)
    }
}

/// 一次资源发布的稳定去重结果；程序、参数表和纹理须由上层同一资源世代共同持有。
#[derive(Clone, Debug)]
pub struct FlowPlan<Key = FlowKey> {
    keys: Vec<Key>,
    bindings: Vec<u32>,
}

impl<Key: Copy + Ord> FlowPlan<Key> {
    /// 将绑定顺序映射到排序去重的 callable 表；同键只产生一个程序入口。
    /// # Errors
    /// 唯一键数超出 `u32` 时返回 [`FlowError::Bindings`]，不产生部分表。
    pub fn new(bindings: impl IntoIterator<Item = Key>) -> Result<Self, FlowError> {
        let binding_keys: Vec<_> = bindings.into_iter().collect();
        let mut indices: BTreeMap<_, _> = binding_keys.iter().map(|&key| (key, 0)).collect();
        for (index, value) in indices.values_mut().enumerate() {
            *value = u32::try_from(index).map_err(|_| FlowError::Bindings)?;
        }
        let bindings = binding_keys.into_iter().map(|key| indices[&key]).collect();
        Ok(Self {
            keys: indices.into_keys().collect(),
            bindings,
        })
    }

    /// 按键升序排列的唯一键，顺序与 pipeline／SBT 的 callable 必须一致。
    pub fn keys(&self) -> &[Key] {
        &self.keys
    }

    /// 输入绑定顺序对应的 callable 索引；不依赖材质名称或纹理 ID。
    pub fn bindings(&self) -> &[u32] {
        &self.bindings
    }
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
