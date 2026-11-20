//! 可见辐射的光谱积分。波长单位为 nm，各波长的强度在线性空间相加。
//! [CIE 1931 2° 观察者](https://doi.org/10.25039/CIE.DS.xvudnb9b) 和
//! [标准照明体 D65](https://doi.org/10.25039/CIE.DS.hjfjmt59) 使用官方 1 nm 数据。
//! XYZ 表示实际入射光。相对于 D65 显示白点观察自发光体时直接转换原色；
//! 将某个照明体指定为适应白点属于调用方的观看条件。

/// 引擎镜头标定的等能矩形传感器波段，依次为红、绿、蓝，两端以 nm 表示。
pub const ENGINE_RGB_BANDS_NM: [[u16; 2]; 3] = [[590, 690], [500, 590], [420, 500]];

const DATA: &[u8; 471 * 32] = include_bytes!("../data/cie1931_d65.f64le");

/// 在 360–830 nm 线性插值，返回 x̄、ȳ、z̄ 与 D65 相对辐射功率（560 nm 为 100）。域外为零。
pub fn cie_sample(nm: f64) -> [f64; 4] {
    if !(360.0..=830.0).contains(&nm) {
        return [0.; 4];
    }
    let x = nm - 360.;
    let left = x as usize;
    let right = (left + 1).min(470);
    let read = |row: usize, col: usize| {
        let words = DATA.as_chunks::<8>().0;
        f64::from_le_bytes(words[row * 4 + col])
    };
    std::array::from_fn(|c| read(left, c) + x.fract() * (read(right, c) - read(left, c)))
}

/// 普朗克定律的每单位波长辐射功率，以该温度的峰值归一。输入须为有限正数，否则返回零。
pub fn blackbody(nm: f64, kelvin: f64) -> f64 {
    if !(nm.is_finite() && kelvin.is_finite() && nm > 0. && kelvin > 0.) {
        return 0.;
    }
    let x = 14_387_768.775_039_337 / nm / kelvin;
    const PEAK: f64 = 4.965_114_231_744_276;
    if x == 0. || x > 700. {
        return 0.;
    }
    (x / PEAK).powi(5) * PEAK.exp_m1() / x.exp_m1()
}

/// 将物理 XYZ 转成 D65 显示器的线性 sRGB，保留负值与 HDR，不做白平衡、裁剪或传递函数编码。
/// 矩阵来自 [W3C CSS Color 4](https://www.w3.org/TR/css-color-4/#color-conversion-code)。
pub const fn xyz_to_linear_srgb([x, y, z]: [f64; 3]) -> [f64; 3] {
    [
        3.2409699419045226 * x - 1.537383177570094 * y - 0.4986107602930034 * z,
        -0.9692436362808796 * x + 1.8759675015077202 * y + 0.04155505740717559 * z,
        0.05563007969699366 * x - 0.20397695888897652 * y + 1.0569715142428786 * z,
    ]
}

/// 一个波段内的响应积分。用中点波长求值波动光学模型，再乘三分量的积分权重。
#[derive(Debug, Clone, Copy)]
pub struct SpectralSample {
    /// 区间中点，单位 nm。
    pub nm: f64,
    /// 已乘波长步长的响应，按调用方选取的 XYZ 或探测器通道排列。
    pub weight: [f64; 3],
}

/// 光谱求积的输入或分配失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 区间须位于可见数据域内，严格递增且至少分成一个区间。
    #[error("波段须在 360–830 nm 内递增，取样数须大于零")]
    Domain,
    /// 所给辐射功率与响应的乘积须有限非负。
    #[error("光谱响应须为有限非负数")]
    Response,
    /// 无法保留求积表。
    #[error("分配光谱求积表失败")]
    Allocation(#[from] std::collections::TryReserveError),
}

/// 按等宽区间返回未归一化的求积表。每个区间按其端点和内部的整 nm 节点作梯形积分。
/// `response` 给出辐射功率乘探测器响应，每分量独立；总量不受波段划分粒度显著影响。
/// # Errors
/// 无效区间、非有限或负响应、积分溢出和内存申请失败分别返回 [`Error`]。
pub fn weighted_bins(
    range: [f64; 2],
    count: u16,
    response: impl Fn(f64) -> [f64; 3],
) -> Result<Vec<SpectralSample>, Error> {
    let [lo, hi] = range;
    if !(360. <= lo && lo < hi && hi <= 830.) || count == 0 {
        return Err(Error::Domain);
    }
    let at = |nm| {
        let v = response(nm);
        if v.iter().all(|x| x.is_finite() && *x >= 0.) {
            Ok(v)
        } else {
            Err(Error::Response)
        }
    };
    let mut bins = Vec::new();
    bins.try_reserve_exact(usize::from(count))?;
    let width = (hi - lo) / f64::from(count);
    let mut left = lo;
    let mut prev = at(lo)?;
    for i in 1..=count {
        let right = if i == count {
            hi
        } else {
            lo + f64::from(i) * width
        };
        let nm = (left + right) * 0.5;
        let mut weight = [0.; 3];
        while left < right {
            let next = (left.floor() + 1.).min(right);
            let v = at(next)?;
            for c in 0..3 {
                weight[c] += (0.5 * prev[c] + 0.5 * v[c]) * (next - left);
            }
            prev = v;
            left = next;
        }
        if !weight.iter().all(|x| x.is_finite()) {
            return Err(Error::Response);
        }
        bins.push(SpectralSample { nm, weight });
    }
    Ok(bins)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_d65_is_neutral_on_an_srgb_display() -> Result<(), Error> {
        for n in [1, 37, 96] {
            let bins = weighted_bins([360., 830.], n, |nm| {
                let [x, y, z, d] = cie_sample(nm);
                [x * d, y * d, z * d]
            })?;
            let mut xyz = [0.; 3];
            for sample in bins {
                for (a, b) in xyz.iter_mut().zip(sample.weight) {
                    *a += b;
                }
            }
            let rgb = xyz_to_linear_srgb(xyz.map(|v| v / xyz[1]));
            assert!(rgb.iter().all(|v| (v - 1.).abs() < 0.0005), "{rgb:?}");
        }
        Ok(())
    }
    #[test]
    fn spectral_powers_add_before_display_clipping() {
        let blue = xyz_to_linear_srgb(cie_sample(450.)[..3].try_into().unwrap());
        assert!(blue[1] < 0. && blue[2] > 0.);
        assert!(xyz_to_linear_srgb([1.; 3])[0] > 1.15);
    }
}
