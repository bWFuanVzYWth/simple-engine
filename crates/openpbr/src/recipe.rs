//! 资源所有者声明的散射组合。只有测度取决于粗糙度与界面 IOR 的数值界限。
use crate::{FlowError, FlowKey, InterfaceIor, MaterialRanges, Range};

/// 同一个可取样权重的支持集。发现任一内点即允许整个单位区间，包括两个端点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    /// 所有样本均为零。
    Zero,
    /// 所有样本均为一。
    One,
    /// 两个端点都可达，仅适用于最近点取样或显式二值选择。
    Switch,
    /// 允许任意比例的混合。
    Blend,
}

impl Weight {
    /// 分类一个已经解码的单位权重。
    /// # Errors
    /// 超出单位区间或非有限时返回范围错误。
    pub const fn new(value: f32) -> Result<Self, FlowError> {
        match value {
            0.0 => Ok(Self::Zero),
            1.0 => Ok(Self::One),
            v if v > 0.0 && v < 1.0 => Ok(Self::Blend),
            _ => Err(FlowError::Range),
        }
    }

    /// 取互斥层或最近点 texel 的联合。线性过滤可能插值时调用方必须声明 Blend。
    pub const fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::Zero, Self::Zero) => Self::Zero,
            (Self::One, Self::One) => Self::One,
            (Self::Blend, _) | (_, Self::Blend) => Self::Blend,
            _ => Self::Switch,
        }
    }
}

/// 非金属底层的可见贡献。全黑时镜面反射仍由上层决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    /// 所有可达颜色通道均为零，底层没有散射贡献。
    Black,
    /// 底色可能非零，按实际权重组合 Lambert 与薄层 SSS。
    Colored(Weight),
}

/// 白名单中的散射组合。粗糙度、IOR、颜色、消光和混合权重由实际绑定提供。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recipe {
    /// 非金属反射层，底层按权重组合 Lambert 与薄层 SSS。
    DielectricBase(Base),
    /// 纯金属反射。
    Metal,
    /// 金属和非金属二值切换；参数为非金属底层的 SSS 支持集。
    MetalSwitch(Base),
    /// 任意金属与非金属比例；底层混合权重与金属度独立。
    MetalBlend(Base),
    /// 具有介质状态转移的实体界面。
    Solid,
    /// 单次穿过薄片的界面。
    Thin,
}

impl Recipe {
    /// 选择 `OPENPBR_RECIPE_VARIANT=1` 时，绑定 variant 中由散射权重占用的位。
    pub const WEIGHT_BITS: u32 = (1 << 11) | (1 << 12);

    /// 两个端点均存在且互斥时选择对应的二值着色程序，内点始终保留完整混合计算。
    pub const fn weight_bits(self) -> u32 {
        let metal = matches!(self, Self::MetalSwitch(_));
        let sss = matches!(
            self,
            Self::DielectricBase(Base::Colored(Weight::Switch))
                | Self::MetalBlend(Base::Colored(Weight::Switch))
                | Self::MetalSwitch(Base::Colored(Weight::Switch))
        );
        ((metal as u32) << 11) | ((sss as u32) << 12)
    }

    /// 从独立权重选择有物理意义的 opaque 组合。
    pub const fn opaque(metal: Weight, base: Base) -> Self {
        match metal {
            Weight::Zero => Self::DielectricBase(base),
            Weight::One => Self::Metal,
            Weight::Switch => Self::MetalSwitch(base),
            Weight::Blend => Self::MetalBlend(base),
        }
    }

    /// 对已选组合编码其数学分量、测度与命中发光。IOR 范围须覆盖全部可达的前驱介质。
    /// # Errors
    /// 粗糙度范围非法时返回输入错误。
    pub const fn flow(
        self,
        roughness: Range,
        ior: InterfaceIor,
        emission: bool,
    ) -> Result<FlowKey, FlowError> {
        use FlowKey as F;
        if matches!(self, Self::Solid | Self::Thin) {
            return match MaterialRanges::dielectric(roughness, matches!(self, Self::Thin)) {
                Ok(ranges) => Ok(ranges.flow(ior, emission)),
                Err(error) => Err(error),
            };
        }
        let mut bits = if emission { F::EMISSION } else { 0 };
        if !matches!(self, Self::Metal) {
            if ior.range().max() > 1.0 {
                bits |= F::DIELECTRIC_REFLECTION;
            }
            match self {
                Self::DielectricBase(Base::Colored(w))
                | Self::MetalSwitch(Base::Colored(w))
                | Self::MetalBlend(Base::Colored(w)) => {
                    if !matches!(w, Weight::One) {
                        bits |= F::DIFFUSE;
                    }
                    if !matches!(w, Weight::Zero) {
                        bits |= F::SUBSURFACE;
                    }
                }
                _ => {}
            }
        }
        if matches!(
            self,
            Self::Metal | Self::MetalSwitch(_) | Self::MetalBlend(_)
        ) {
            bits |= F::CONDUCTOR_REFLECTION;
        }
        if matches!(self, Self::MetalSwitch(_) | Self::MetalBlend(_)) {
            bits |= F::MIXED_METALNESS;
        }
        let measure = match roughness.reflection_measure() {
            Ok(mask) => mask,
            Err(e) => return Err(e),
        };
        if bits & (F::CONDUCTOR_REFLECTION | F::DIELECTRIC_REFLECTION) != 0 {
            bits |= measure;
        }
        F::from_bits(bits)
    }
}
