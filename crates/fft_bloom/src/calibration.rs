//! 镜头的光谱采样。每个波长先算夫琅禾费复振幅的模平方，不同波长的强度按本模块给出的权重相加。
//! 引擎卷积需要三个非负、各自单位通量的传感器通道。观察者预览使用 CIE XYZ，共用一个亮度标尺以保留光源色度。

use color::spectrum::{self, SpectralSample, blackbody, cie_sample, weighted_bins};

/// 标定时照亮光瞳的光源，每单位波长按辐射能量加权。
#[derive(Debug, Clone, Copy)]
pub enum Source {
    /// 等能光谱，复现引擎内置的矩形波段标定。
    Equal,
    /// CIE 标准日光 D65。
    D65,
    /// 普朗克黑体的绝对温度，单位 K。
    Blackbody(f64),
    /// 单色谱线，单位 nm。
    Line(f64),
}

impl Source {
    /// 宽带光源每单位波长的相对功率，域外为零。单色谱线是离散能量，返回 `None`。
    pub fn relative_power(self, nm: f64) -> Option<f64> {
        match self {
            Self::Line(_) => None,
            _ if !(360.0..=830.0).contains(&nm) => Some(0.),
            Self::Equal => Some(1.),
            Self::D65 => Some(cie_sample(nm)[3]),
            Self::Blackbody(k) => Some(blackbody(nm, k)),
        }
    }
}

/// 光斑的三分量积分空间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// R 590–690、G 500–590、B 420–500 nm 矩形响应，逐通道单位积分。
    EngineRgb,
    /// CIE 1931 2° XYZ，光源总 Y 为 1，保留 X/Y 与 Z/Y。
    Observer,
}

/// 校准错误，包含求积和内存错误的原始来源。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 输入温度、谱线波长或采样数无效，或所选通道没有可表示的能量。
    #[error("光源参数无效，或所选波段内没有可表示的能量")]
    Source,
    /// 光谱区间求积失败。
    #[error(transparent)]
    Spectrum(#[from] spectrum::Error),
    /// 汇总波段时内存不足。
    #[error(transparent)]
    Allocation(#[from] std::collections::TryReserveError),
}

/// 求积表与光源自身的色度，均为场景线性量。
pub struct Plan {
    /// 每个波长须用于缩放像面，单位 nm。权重已经包含探测器响应、照明和积分步长。
    pub samples: Vec<SpectralSample>,
    /// 光源的 XYZ，Y 为 1。三带卷积的通道归一化不会改变这里记录的光源色度。
    pub source_xyz: [f64; 3],
}

impl Plan {
    /// `count` 为每个引擎波段的中点数，或覆盖 360–830 nm 的观察者区间数。单色模式共享同一光学 PSF。
    /// # Errors
    /// 零采样、非法温度、可见域外谱线、零能量、求积或内存失败时返回 [`Error`]。
    pub fn new(source: Source, response: Response, count: u16) -> Result<Self, Error> {
        if count == 0 {
            return Err(Error::Source);
        }
        if let Source::Line(nm) = source {
            let [x, y, z, _] = cie_sample(nm);
            if y <= 0. {
                return Err(Error::Source);
            }
            let source_xyz = [x / y, 1., z / y];
            let weight = if response == Response::Observer {
                source_xyz
            } else {
                [1.; 3]
            };
            let mut samples = Vec::new();
            samples.try_reserve_exact(1)?;
            samples.push(SpectralSample { nm, weight });
            return Ok(Self {
                samples,
                source_xyz,
            });
        }
        let power = |nm: f64| source.relative_power(nm).expect("单色谱线已单独积分");
        let mut observer = weighted_bins([360., 830.], count, |nm| {
            let [x, y, z, _] = cie_sample(nm);
            [x, y, z].map(|v| v * power(nm))
        })?;
        let mut total = [0.; 3];
        for node in &observer {
            for (sum, v) in total.iter_mut().zip(node.weight) {
                *sum += v;
            }
        }
        if total[1] <= 0. {
            return Err(Error::Source);
        }
        let source_xyz = total.map(|v| v / total[1]);
        let samples = match response {
            Response::Observer => {
                for node in &mut observer {
                    node.weight = node.weight.map(|v| v / total[1]);
                }
                observer
            }
            Response::EngineRgb => {
                let mut samples = Vec::new();
                samples.try_reserve_exact(usize::from(count) * 3)?;
                for (channel, range) in spectrum::ENGINE_RGB_BANDS_NM.into_iter().enumerate() {
                    let band = weighted_bins(range.map(f64::from), count, |nm| {
                        let mut w = [0.; 3];
                        w[channel] = power(nm);
                        w
                    })?;
                    let mass = band.iter().map(|s| s.weight[channel]).sum::<f64>();
                    if mass <= 0. {
                        return Err(Error::Source);
                    }
                    samples.extend(band.into_iter().map(|mut s| {
                        s.weight[channel] /= mass;
                        s
                    }));
                }
                samples
            }
        };
        Ok(Self {
            samples,
            source_xyz,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_lamp_preserves_flux_in_each_engine_channel() -> Result<(), Error> {
        let plan = Plan::new(Source::Blackbody(2856.), Response::EngineRgb, 24)?;
        for c in 0..3 {
            let mass = plan.samples.iter().map(|s| s.weight[c]).sum::<f64>();
            assert!((mass - 1.).abs() < 1e-12);
        }
        Ok(())
    }

    #[test]
    fn spectral_line_preserves_its_chromaticity() -> Result<(), Error> {
        let plan = Plan::new(Source::Line(532.), Response::Observer, 96)?;
        assert_eq!(plan.samples.len(), 1);
        let [x, y, z, _] = cie_sample(532.);
        assert_eq!(plan.samples[0].weight, [x / y, 1., z / y]);
        Ok(())
    }
}
