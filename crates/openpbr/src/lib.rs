// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
//! OpenPBR 能力子集：opaque／metallic、solid／thin dielectric 和薄层 SSS。
//!
//! 这不是完整 [OpenPBR 标准](https://academysoftwarefoundation.github.io/OpenPBR/)实现；
//! 没有 coat、fuzz、thin-film、dispersion、衍射或厚体 SSS。
//! 数学内核位于引擎 Slang `bsdf.openpbr.api`，使用 GGX、Fresnel、完整边缘 PDF
//! 与多次散射能量补偿，不将其替换为近似 BRDF。来源及许可见 `assets/NOTICE.md`。
//!
//! 本 crate 校验材料与均匀介质，并提供原作者能量表。RGB 属于调用方选择的一致线性工作空间；
//! extinction 单位为该场景长度单位的倒数，薄片 thickness 使用同一长度单位。
//! 调用方保证光学深度、eta 运算和响应的 f32 中间量可表示，不支持任意有限极端值。
//! 不解析应用贴图、不决定介质初值、边界拓扑或路径调度。
//! Slang `may_evaluate` 提供无能量表读取的保守连续支持域快判；false 保证 response/PDF 为零。
//! 调用方可先排除支持域、再查询可见性，最后 evaluate，避免跨光追查询保存闭包状态。
//! `bsdf.openpbr.specialized` 提供八类编译期闭包入口；调用方在发布材料时保证类别匹配。
//! `shade_specialized` 在一次调用内共享两个连续方向及可选散射的状态；纯 delta 不准备连续闭包。
//! `sample_delta_specialized` 仅接受纯 delta 类别，无需 Energy，标量随机数对应普通采样的 `u.z`。
//! `evaluate_solid_continuous`／`sample_solid_continuous` 在调用方保证 roughness²≥1e-4 且实际
//! 界面 IOR 比不等于1时裁掉离散分支，保留原连续 PDF／TIR／能量补偿；通用入口仍支持光滑及匹配 IOR。
//! `evaluate_opaque_continuous`／`sample_opaque_continuous` 对类别0..2要求 roughness²≥1e-4，
//! 保留 diffuse／SSS／glossy 标签和完整 marginal PDF；匹配 IOR 仍合法，通用入口保留光滑边界。
//! 分类不绑定应用编号；通用入口仍支持 fractional metalness，类别不会隐式修复不匹配的材料。
//! [`MaterialRanges`] 在资源重载时分析解码后的参数范围，生成可组合的 [`FlowKey`]；
//! [`FlowPlan`] 按真实可达控制流去重 callable，不以材质名称或纹理编号划分程序。
//! 常量端点、光滑／粗糙支持域和显式介质范围可裁掉不可达路径；分析范围须覆盖全部纹理
//! mip、过滤和动画帧。运行时只读取参数并调用该绑定已发布的程序，不重新扫描资源。
//!
//! `gpu` feature 只增加可复用的 wgpu 3D texture 所有者；调用方每设备创建一份并显式共享，
//! 不隐藏进程级 cache。上传由同一 queue 排序，不额外等待 GPU。native 管线须保活克隆的
//! texture／sampler，不能在在途命令结束前显式销毁它们。

#![forbid(unsafe_code)]

mod recipe;
pub use recipe::{Base, Recipe, Weight};

mod flow;
pub use flow::{FlowError, FlowKey, FlowPlan, InterfaceIor, MaterialRanges, Range, RangeScan};

/// 真实采样资源的颜色范围与材质绑定分析；不持有应用资产或设备资源。
pub mod resource;

/// 资源控制流对应的受控 Slang callable 集合；不创建设备或提交 GPU 命令。
#[cfg(feature = "program")]
pub mod program;

/// 原始 HALF4 能量表的宽、高、深；三个坐标轴是 view、roughness、分支化 IOR。
pub const ENERGY_EXTENT: [u32; 3] = [44, 32, 159];
/// 能量数据的精确字节数；每 texel 四个 little-endian IEEE binary16。
pub const ENERGY_BYTES: usize = 1_790_976;
/// 原作者 2026-07-24 hotfix 能量数据的 SHA-256，未经过重新生成或有损打包。
pub const ENERGY_SHA256: &str = "605c9160fb9348a1d033321c40cf9930226ce74c03f2624033f5b73aacfa67df";
static ENERGY: &[u8; ENERGY_BYTES] = include_bytes!("../assets/trans_ggx.bytes");

/// 借用随包保存的不可变原始能量表，不分配、解压或持有设备资源。
pub const fn energy_bytes() -> &'static [u8] {
    ENERGY
}

/// 材料／介质的输入边界错误，不裁剪无效输入。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// 基色不是有限的 `0..=1` RGB。
    #[error("OpenPBR 基色须为有限的 0..=1 RGB")]
    BaseColor,
    /// 线性粗糙度不在有限 `0..=1`。
    #[error("OpenPBR 粗糙度须在 0..=1")]
    Roughness,
    /// 绝对 IOR 不在支持的有限 `1..=4`。
    #[error("OpenPBR IOR 须在 1..=4")]
    Ior,
    /// 金属度不在有限 `0..=1`。
    #[error("OpenPBR 金属度须在 0..=1")]
    Metalness,
    /// 薄层 subsurface 权重不在有限 `0..=1`。
    #[error("OpenPBR 薄层 SSS 权重须在 0..=1")]
    Subsurface,
    /// 薄片厚度不是有限正数。
    #[error("OpenPBR 薄片厚度须为有限正数")]
    Thickness,
    /// RGB extinction 不是有限非负向量。
    #[error("OpenPBR extinction 须为有限非负 RGB")]
    Extinction,
}

const fn unit(value: f32) -> bool {
    value.is_finite() && value >= 0.0 && value <= 1.0
}

/// 已校验的均匀介质；不代表体积散射或介质栈。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Medium {
    values: [f32; 4],
}

impl Medium {
    /// IOR=1、无吸收；相机介质须由调用方确定，不能默认假定所有相机均在真空。
    pub const VACUUM: Self = Self {
        values: [1.0, 0.0, 0.0, 0.0],
    };

    /// 保存绝对 IOR 与 RGB extinction；由边界选择 incident／transmitted 两侧。
    /// # Errors
    /// IOR 超出有限 `1..=4` 或 extinction 非有限／为负时返回对应错误。
    pub const fn new(ior: f32, extinction: [f32; 3]) -> Result<Self, Error> {
        if !ior.is_finite() || ior < 1.0 || ior > 4.0 {
            return Err(Error::Ior);
        }
        let mut axis = 0;
        while axis < 3 {
            if !extinction[axis].is_finite() || extinction[axis] < 0.0 {
                return Err(Error::Extinction);
            }
            axis += 1;
        }
        Ok(Self {
            values: [ior, extinction[0], extinction[1], extinction[2]],
        })
    }

    /// GPU float4 顺序为绝对 IOR、RGB extinction；不作工作空间转换。
    pub const fn to_array(self) -> [f32; 4] {
        self.values
    }
}

/// 有效的支持子域材料；GPU 记录内容由 [`Self::to_words`] 明确给出。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    base_roughness: [f32; 4],
    surface: [f32; 4],
    flags: u32,
}

impl Material {
    /// 创建有色 opaque／metallic 混合，含可选的薄层 SSS 分量。
    ///
    /// 所有 RGB／权重和 roughness 须在 `0..=1`，IOR 在 `1..=4`；不量化 metalness／SSS。
    /// `subsurface>0` 只适用于调用方已确认的薄表面，不表示厚体散射。
    /// # Errors
    /// 返回首个无效字段对应的错误，不创建部分有效记录。
    pub const fn opaque(
        base_color: [f32; 3],
        roughness: f32,
        ior: f32,
        metalness: f32,
        subsurface: f32,
    ) -> Result<Self, Error> {
        let mut axis = 0;
        while axis < 3 {
            if !unit(base_color[axis]) {
                return Err(Error::BaseColor);
            }
            axis += 1;
        }
        if !unit(roughness) {
            return Err(Error::Roughness);
        }
        if !ior.is_finite() || ior < 1.0 || ior > 4.0 {
            return Err(Error::Ior);
        }
        if !unit(metalness) {
            return Err(Error::Metalness);
        }
        if !unit(subsurface) {
            return Err(Error::Subsurface);
        }
        Ok(Self {
            base_roughness: [base_color[0], base_color[1], base_color[2], roughness],
            surface: [ior, metalness, subsurface, 0.0],
            flags: 0,
        })
    }

    /// 创建 solid 或 thin dielectric；两侧 IOR／extinction 通过显式 [`Medium`] 提供。
    ///
    /// `None` 为实体边界，透射后进入 transmitted medium；`Some(thickness)` 为两界面薄片，
    /// 透射方向遵循薄片模型，离开后仍保留 incident medium。厚度由调用方选择，不隐含 1/16 单位。
    /// # Errors
    /// roughness 不在有限 `0..=1` 或薄片厚度不是有限正数时返回对应错误。
    pub const fn dielectric(roughness: f32, thin_thickness: Option<f32>) -> Result<Self, Error> {
        if !unit(roughness) {
            return Err(Error::Roughness);
        }
        let (thickness, flags) = match thin_thickness {
            None => (0.0, 1),
            Some(value) if value.is_finite() && value > 0.0 => (value, 3),
            Some(_) => return Err(Error::Thickness),
        };
        Ok(Self {
            base_roughness: [1.0, 1.0, 1.0, roughness],
            surface: [1.0, 0.0, 0.0, thickness],
            flags,
        })
    }

    /// 返回 Slang `openpbr::Material` 的 48B 内容；不是 Rust 类型布局承诺。
    ///
    /// 前两组 float4 是 `baseRGB,roughness` 与 `opaqueIOR,metalness,subsurface,thinThickness`；
    /// 最后一组 uint4 为 flags、0、0、0，bit0=dielectric、bit1=thin。薄介电的 IOR 取传入 medium。
    /// RGB 可在 shader 命中后由同一工作空间的贴图值替换，其余不变量由调用方维持。
    pub const fn to_words(self) -> [u32; 12] {
        [
            self.base_roughness[0].to_bits(),
            self.base_roughness[1].to_bits(),
            self.base_roughness[2].to_bits(),
            self.base_roughness[3].to_bits(),
            self.surface[0].to_bits(),
            self.surface[1].to_bits(),
            self.surface[2].to_bits(),
            self.surface[3].to_bits(),
            self.flags,
            0,
            0,
            0,
        ]
    }
}

#[cfg(feature = "gpu")]
mod gpu;
#[cfg(feature = "gpu")]
pub use gpu::{GpuEnergy, GpuError};

#[cfg(test)]
mod tests;
