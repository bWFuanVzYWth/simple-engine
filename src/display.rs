// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

//! primeDRT 参数与场景线性 HDR 自动曝光。
//!
//! 参数作用于已完成积分的场景线性 RGB，修改它们不要求清空线性累积。
//! [`exposure`] 提供自动测光帧参数；启用 `vulkan` 时增加 GPU 常驻录制器。
//! [`PrimeDrtSettings`] 保存有效的曝光、色相／饱和补偿和压缩起点；
//! [`PrimeDrtSettings::prepare`] 在输出校准变化时推导曲线及编码峰值，避免逐像素重复计算。
//! headroom 是输出峰值与参考白的比值，SDR 为 1；它不包含窗口或 HDR 协议选择。
//!
//! 对应 Slang 模块为 `display.prime_drt`，可直接接收线性 BT.709／sRGB 或 Rec.2020，
//! 两者均采用 D65 白点。算法在虚拟色域内压缩高光，返回原色域后保留负 outset 通道，
//! 完成 sRGB 编码与 HSV 补偿后才裁剪。编码输出不能直接写入会再次编码的 sRGB attachment；
//! 需要线性输出时使用 `display::prime_drt_linear_bt709`。
//!
//! 采用项目根目录 `LICENSE` 中的 GPL-3.0-only。

pub mod exposure;

/// primeDRT 参数的输入边界错误；不创建部分有效的参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// 手动曝光倍率不是有限正数。
    Exposure,
    /// 色相补偿不在有限范围 `0..=1`。
    HueCompensation,
    /// 饱和补偿不在有限范围 `0..=0.5`。
    SaturationCompensation,
    /// 压缩起点不在有限范围 `0..=0.99`。
    CompressionStart,
    /// 输出峰值／参考白不在有限范围 `1..=10000`。
    Headroom,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Exposure => "primeDRT 曝光须为有限正数",
            Self::HueCompensation => "primeDRT 色相补偿须在 0..=1",
            Self::SaturationCompensation => "primeDRT 饱和补偿须在 0..=0.5",
            Self::CompressionStart => "primeDRT 压缩起点须在 0..=0.99",
            Self::Headroom => "primeDRT 输出峰值与参考白之比须在 1..=10000",
        })
    }
}

impl std::error::Error for Error {}

/// 有效的手动显示调整，不含自动曝光、设备校准或传递函数选择。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimeDrtSettings {
    exposure_multiplier: f32,
    hue_compensation: f32,
    saturation_compensation: f32,
    compression_start: f32,
}

impl PrimeDrtSettings {
    /// 曝光 1、色相补偿 75%、饱和补偿 5%、压缩起点 0.4。
    pub const DEFAULT: Self = Self {
        exposure_multiplier: 1.0,
        hue_compensation: 0.75,
        saturation_compensation: 0.05,
        compression_start: 0.4,
    };

    /// 校验并保存曝光倍率、色相补偿、饱和补偿与压缩起点；不裁剪输入。
    ///
    /// 曝光须有限且为正，其余三项分别限定在 `0..=1`、`0..=0.5`、`0..=0.99`。
    /// # Errors
    /// 返回首个无效参数对应的 [`Error`]。
    pub const fn new(
        exposure_multiplier: f32,
        hue_compensation: f32,
        saturation_compensation: f32,
        compression_start: f32,
    ) -> Result<Self, Error> {
        if !exposure_multiplier.is_finite() || exposure_multiplier <= 0.0 {
            return Err(Error::Exposure);
        }
        if !hue_compensation.is_finite() || hue_compensation < 0.0 || hue_compensation > 1.0 {
            return Err(Error::HueCompensation);
        }
        if !saturation_compensation.is_finite()
            || saturation_compensation < 0.0
            || saturation_compensation > 0.5
        {
            return Err(Error::SaturationCompensation);
        }
        if !compression_start.is_finite() || compression_start < 0.0 || compression_start > 0.99 {
            return Err(Error::CompressionStart);
        }
        Ok(Self {
            exposure_multiplier,
            hue_compensation,
            saturation_compensation,
            compression_start,
        })
    }

    /// 替换曝光倍率并保留其余显示控制；不规定应用的 EV 范围或调节步长。
    ///
    /// # Errors
    /// 倍率不是有限正数时返回 [`Error::Exposure`]。
    pub const fn with_exposure(self, exposure_multiplier: f32) -> Result<Self, Error> {
        if !exposure_multiplier.is_finite() || exposure_multiplier <= 0.0 {
            return Err(Error::Exposure);
        }
        Ok(Self {
            exposure_multiplier,
            ..self
        })
    }

    /// 按曝光、色相补偿、饱和补偿、压缩起点顺序返回有效的用户调整。
    pub const fn to_array(self) -> [f32; 4] {
        [
            self.exposure_multiplier,
            self.hue_compensation,
            self.saturation_compensation,
            self.compression_start,
        ]
    }

    /// 以输出峰值／参考白推导 GPU 参数；SDR 传入 1。
    ///
    /// 压缩曲线在起点与恒等映射相切，并将 `0.18 * 2^8 * headroom` 映射到 headroom。
    /// 推导使用 f64，上传前转换为 f32；手动曝光只在 shader 中乘一次，不改变曲线校准。
    /// # Errors
    /// headroom 非有限或超出 `1..=10000` 时返回 [`Error::Headroom`]。
    pub fn prepare(self, headroom: f32) -> Result<PrimeDrtParameters, Error> {
        if !headroom.is_finite() || !(1.0..=10000.0).contains(&headroom) {
            return Err(Error::Headroom);
        }
        let headroom64 = f64::from(headroom);
        let start = f64::from(self.compression_start);
        let tangent_distance = 0.18 * 256.0 * headroom64 - start;
        let output_distance = headroom64 - start;
        let curve_peak = (start
            + output_distance * tangent_distance / (tangent_distance - output_distance))
            as f32;
        let encoded_peak = if headroom == 1.0 {
            1.0
        } else {
            (1.055 * headroom64.powf(1.0 / 2.4) - 0.055) as f32
        };
        Ok(PrimeDrtParameters {
            values: [
                self.exposure_multiplier,
                curve_peak,
                encoded_peak,
                self.hue_compensation,
                self.saturation_compensation,
                self.compression_start,
                0.0,
                0.0,
            ],
        })
    }
}

impl Default for PrimeDrtSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 已推导且不借用设置的参数；没有图像或后端所有权。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimeDrtParameters {
    values: [f32; 8],
}

impl PrimeDrtParameters {
    /// 替换手动曝光倍率，复用已准备的曲线和输出校准。
    /// # Errors
    /// 倍率不是有限正数时返回 [`Error::Exposure`]。
    pub const fn with_exposure(mut self, exposure_multiplier: f32) -> Result<Self, Error> {
        if !exposure_multiplier.is_finite() || exposure_multiplier <= 0.0 {
            return Err(Error::Exposure);
        }
        self.values[0] = exposure_multiplier;
        Ok(self)
    }

    /// 返回 Slang `display::PrimeDrtParameters` 的 32 字节内容，可按两个 float4 上传。
    ///
    /// 顺序为曝光倍率、曲线渐近峰值、sRGB 编码输出峰值、色相补偿、饱和补偿、压缩起点、
    /// 两个保留零。此方法只规定数组内容；Rust 类型本身不作为 GPU ABI 使用。
    pub const fn to_array(self) -> [f32; 8] {
        self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(x: f64, parameters: PrimeDrtParameters) -> f64 {
        let values = parameters.to_array();
        let start = f64::from(values[5]);
        if x <= start {
            x
        } else {
            let extent = f64::from(values[1]) - start;
            start + extent / (1.0 + extent / (x - start))
        }
    }

    #[test]
    fn curve_maps_eight_ev_to_calibrated_headroom() {
        for start in [0.0, 0.5, 0.99] {
            for headroom in [1.0, 16.0, 10000.0] {
                let parameters = PrimeDrtSettings::new(1.0, 0.75, 0.04, start)
                    .unwrap()
                    .prepare(headroom)
                    .unwrap();
                let peak = f64::from(headroom);
                assert!((curve(0.18 * 256.0 * peak, parameters) - peak).abs() < 1e-6 * peak);
            }
        }
    }

    #[test]
    fn curve_preserves_identity_and_unit_tangent_at_join() {
        for start in [0.0, 0.18, 0.99] {
            let parameters = PrimeDrtSettings::new(1.0, 0.75, 0.04, start)
                .unwrap()
                .prepare(1.0)
                .unwrap();
            let join = f64::from(start);
            assert_eq!(curve(join, parameters), join);
            assert_eq!(curve(join - 0.1, parameters), join - 0.1);
            let step = 1e-8;
            let slope =
                (curve(join + step, parameters) - curve(join - step, parameters)) / (2.0 * step);
            assert!((slope - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn encoded_output_peak_decodes_to_linear_headroom() {
        for headroom in [1.0, 2.0, 16.0, 10000.0] {
            let encoded = PrimeDrtSettings::default()
                .prepare(headroom)
                .unwrap()
                .to_array()[2];
            let decoded = ((f64::from(encoded) + 0.055) / 1.055).powf(2.4);
            assert!((decoded - f64::from(headroom)).abs() < 1e-6 * f64::from(headroom));
        }
    }

    #[test]
    fn exposure_does_not_recalibrate_the_curve() {
        let first = PrimeDrtSettings::new(0.25, 0.75, 0.04, 0.5)
            .unwrap()
            .prepare(2.0)
            .unwrap()
            .to_array();
        let second = PrimeDrtSettings::new(4.0, 0.75, 0.04, 0.5)
            .unwrap()
            .prepare(2.0)
            .unwrap()
            .to_array();
        assert_eq!(first[0], 0.25);
        assert_eq!(second[0], 4.0);
        assert_eq!(first[1..], second[1..]);
    }

    #[test]
    fn replacing_exposure_preserves_the_other_display_controls() {
        let settings = PrimeDrtSettings::new(1.0, 0.3, 0.2, 0.18).unwrap();
        assert_eq!(
            settings.with_exposure(256.0).unwrap().to_array(),
            [256.0, 0.3, 0.2, 0.18]
        );
        for exposure in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(settings.with_exposure(exposure), Err(Error::Exposure));
        }
    }

    #[test]
    fn construction_reports_the_invalid_control() {
        for exposure in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                PrimeDrtSettings::new(exposure, 0.75, 0.04, 0.5),
                Err(Error::Exposure)
            );
        }
        assert_eq!(
            PrimeDrtSettings::new(1.0, 1.01, 0.04, 0.5),
            Err(Error::HueCompensation)
        );
        assert_eq!(
            PrimeDrtSettings::new(1.0, 0.75, -0.01, 0.5),
            Err(Error::SaturationCompensation)
        );
        assert_eq!(
            PrimeDrtSettings::new(1.0, 0.75, 0.04, 1.0),
            Err(Error::CompressionStart)
        );
        assert!(PrimeDrtSettings::new(f32::MIN_POSITIVE, 0.0, 0.0, 0.0).is_ok());
        assert!(PrimeDrtSettings::new(1.0, 1.0, 0.5, 0.99).is_ok());
    }

    #[test]
    fn calibration_rejects_unsupported_headroom() {
        for headroom in [0.0, 0.99, 10001.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                PrimeDrtSettings::default().prepare(headroom),
                Err(Error::Headroom)
            );
        }
    }
}
