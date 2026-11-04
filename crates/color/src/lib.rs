//! sRGB 传递函数遵循 [IEC 61966-2-1:1999][srgb]，负值采用 [W3C][code] 的保留符号扩展。
//! Rec. 2020 原色遵循 [ITU-R BT.2020-2 表 3][bt2020]；线性转换矩阵由 W3C 的
//! XYZ-D65 矩阵合成，两种色彩空间均使用 D65 白点。
//!
//! RGB 未预乘 alpha；alpha 为线性覆盖率，在浮点转换中原样保留。
//! 浮点通道的标称范围为 `0..=1`，构造和转换均不裁剪超出范围的值。
//! 转为字节时，四通道均裁剪到 `0..=1`，乘 `255` 后舍入到最近整数，半值向上；
//! NaN、负无穷映射为 `0`，正无穷映射为 `255`。
//!
//! ```
//! let encoded = color::Srgb8Rgba::new(128, 64, 255, 128);
//! let linear = encoded.to_linear_rec2020();
//! assert_eq!(linear.to_srgb8(), encoded);
//! ```
//!
//! [srgb]: https://registry.color.org/rgb-registry/srgb
//! [bt2020]: https://www.itu.int/rec/R-REC-BT.2020-2-201510-I/en
//! [code]: https://www.w3.org/TR/2024/CRD-css-color-4-20240213/#color-conversion-code

/// 8-bit sRGB 编码 RGB 与线性 alpha，均未预乘。
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Srgb8Rgba {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

/// sRGB 传递函数编码的浮点 RGB 与线性 alpha，允许超出标称范围。
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SrgbRgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

/// D65 白点的线性 sRGB 与未预乘 alpha。
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearSrgbRgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

/// D65 白点的线性 Rec. 2020 与未预乘 alpha。
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearRec2020Rgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Srgb8Rgba {
    /// 按 RGBA 顺序保存通道，不作预乘。
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// 返回 RGBA 顺序的原始编码字节。
    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// 四通道除以 255，RGB 仍保留 sRGB 编码。
    pub const fn to_srgb(self) -> SrgbRgba {
        SrgbRgba::new(
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        )
    }

    /// 归一化并解码 sRGB 传递函数；alpha 仅归一化。
    pub fn to_linear_srgb(self) -> LinearSrgbRgba {
        self.to_srgb().to_linear_srgb()
    }

    /// 解码 sRGB 后转换原色至线性 Rec. 2020，不裁剪色域。
    pub fn to_linear_rec2020(self) -> LinearRec2020Rgba {
        self.to_linear_srgb().to_linear_rec2020()
    }
}

impl SrgbRgba {
    /// 原样保存浮点通道，不检查范围或有限性。
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// 返回 RGBA 顺序的编码值，alpha 保持线性。
    pub const fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// 按 crate 级裁剪和舍入规则量化四通道。
    pub const fn to_srgb8(self) -> Srgb8Rgba {
        Srgb8Rgba::new(
            unorm8(self.r),
            unorm8(self.g),
            unorm8(self.b),
            unorm8(self.a),
        )
    }

    /// 解码 sRGB RGB，负值按保留符号的扩展处理；alpha 原样保留。
    pub fn to_linear_srgb(self) -> LinearSrgbRgba {
        LinearSrgbRgba::new(
            srgb_to_linear(self.r),
            srgb_to_linear(self.g),
            srgb_to_linear(self.b),
            self.a,
        )
    }

    /// 解码 RGB 并转换原色，超色域值与 alpha 均不裁剪。
    pub fn to_linear_rec2020(self) -> LinearRec2020Rgba {
        self.to_linear_srgb().to_linear_rec2020()
    }
}

impl LinearSrgbRgba {
    /// 原样保存线性通道，不检查范围或有限性。
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// 返回 RGBA 顺序的线性值。
    pub const fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// 编码 sRGB RGB，负值按保留符号的扩展处理；alpha 原样保留。
    pub fn to_srgb(self) -> SrgbRgba {
        SrgbRgba::new(
            linear_to_srgb(self.r),
            linear_to_srgb(self.g),
            linear_to_srgb(self.b),
            self.a,
        )
    }

    /// 编码 RGB 后按 crate 级规则量化 RGBA。
    pub fn to_srgb8(self) -> Srgb8Rgba {
        self.to_srgb().to_srgb8()
    }

    /// 在同一 D65 白点下转换原色，不裁剪色域或改变 alpha。
    pub const fn to_linear_rec2020(self) -> LinearRec2020Rgba {
        // W3C 矩阵合成：XYZ_to_lin_2020 × lin_sRGB_to_XYZ。
        let r = self.r as f64;
        let g = self.g as f64;
        let b = self.b as f64;
        LinearRec2020Rgba::new(
            (0.627403895934699 * r + 0.3292830383778837 * g + 0.043313065687417225 * b) as f32,
            (0.06909728935823208 * r + 0.9195403950754587 * g + 0.011362315566309178 * b) as f32,
            (0.01639143887515028 * r + 0.08801330787722575 * g + 0.895595253247624 * b) as f32,
            self.a,
        )
    }
}

impl LinearRec2020Rgba {
    /// 原样保存线性通道，不检查范围或有限性。
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// 返回 RGBA 顺序的线性值。
    pub const fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// 在同一 D65 白点下转换原色，可产生负值与大于 1 的 RGB。
    pub const fn to_linear_srgb(self) -> LinearSrgbRgba {
        // W3C 矩阵合成：XYZ_to_lin_sRGB × lin_2020_to_XYZ。
        let r = self.r as f64;
        let g = self.g as f64;
        let b = self.b as f64;
        LinearSrgbRgba::new(
            (1.6604910021084345 * r - 0.5876411387885495 * g - 0.07284986331988488 * b) as f32,
            (-0.12455047452159074 * r + 1.1328998971259603 * g - 0.008349422604369477 * b) as f32,
            (-0.018150763354905303 * r - 0.10057889800800739 * g + 1.1187296613629127 * b) as f32,
            self.a,
        )
    }

    /// 转换至 sRGB 原色再编码 RGB；alpha 原样保留。
    pub fn to_srgb(self) -> SrgbRgba {
        self.to_linear_srgb().to_srgb()
    }

    /// 转换并编码至 sRGB，再按 crate 级规则量化 RGBA。
    pub fn to_srgb8(self) -> Srgb8Rgba {
        self.to_srgb().to_srgb8()
    }
}

impl From<Srgb8Rgba> for SrgbRgba {
    fn from(value: Srgb8Rgba) -> Self {
        value.to_srgb()
    }
}

impl From<Srgb8Rgba> for LinearSrgbRgba {
    fn from(value: Srgb8Rgba) -> Self {
        value.to_linear_srgb()
    }
}

impl From<Srgb8Rgba> for LinearRec2020Rgba {
    fn from(value: Srgb8Rgba) -> Self {
        value.to_linear_rec2020()
    }
}

impl From<SrgbRgba> for Srgb8Rgba {
    fn from(value: SrgbRgba) -> Self {
        value.to_srgb8()
    }
}

impl From<SrgbRgba> for LinearSrgbRgba {
    fn from(value: SrgbRgba) -> Self {
        value.to_linear_srgb()
    }
}

impl From<SrgbRgba> for LinearRec2020Rgba {
    fn from(value: SrgbRgba) -> Self {
        value.to_linear_rec2020()
    }
}

impl From<LinearSrgbRgba> for Srgb8Rgba {
    fn from(value: LinearSrgbRgba) -> Self {
        value.to_srgb8()
    }
}

impl From<LinearSrgbRgba> for SrgbRgba {
    fn from(value: LinearSrgbRgba) -> Self {
        value.to_srgb()
    }
}

impl From<LinearSrgbRgba> for LinearRec2020Rgba {
    fn from(value: LinearSrgbRgba) -> Self {
        value.to_linear_rec2020()
    }
}

impl From<LinearRec2020Rgba> for Srgb8Rgba {
    fn from(value: LinearRec2020Rgba) -> Self {
        value.to_srgb8()
    }
}

impl From<LinearRec2020Rgba> for SrgbRgba {
    fn from(value: LinearRec2020Rgba) -> Self {
        value.to_srgb()
    }
}

impl From<LinearRec2020Rgba> for LinearSrgbRgba {
    fn from(value: LinearRec2020Rgba) -> Self {
        value.to_linear_srgb()
    }
}

// 用 f64 避免乘法舍入将略低于量化半步的值推过边界。
const fn unorm8(value: f32) -> u8 {
    (value as f64 * 255.0 + 0.5) as u8
}

fn srgb_to_linear(value: f32) -> f32 {
    if value.abs() <= 0.04045 {
        value / 12.92
    } else {
        ((value.abs() + 0.055) / 1.055).powf(2.4).copysign(value)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value.abs() <= 0.0031308 {
        value * 12.92
    } else {
        (1.055 * value.abs().powf(1.0 / 2.4) - 0.055).copysign(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_rgb_close(actual: [f32; 4], expected: [f32; 3]) {
        for (actual, expected) in actual[..3].iter().zip(expected) {
            assert!((actual - expected).abs() < 2e-6, "{actual} ≉ {expected}");
        }
    }

    #[test]
    fn srgb_transfer_matches_standard() {
        for (encoded, linear) in [
            (0.04, 0.0030959752),
            (0.04045, 0.003130805),
            (0.5, 0.21404114),
            (-0.5, -0.21404114),
            (1.0, 1.0),
        ] {
            let decoded = SrgbRgba::new(encoded, 0.0, 0.0, 1.0).to_linear_srgb();
            let encoded_again = LinearSrgbRgba::new(linear, 0.0, 0.0, 1.0).to_srgb();
            assert!((decoded.r - linear).abs() < 1e-6);
            assert!((encoded_again.r - encoded).abs() < 1e-6);
        }
    }

    #[test]
    fn rec2020_red_in_srgb_matches_standard() {
        assert_rgb_close(
            LinearRec2020Rgba::new(1.0, 0.0, 0.0, 1.0)
                .to_linear_srgb()
                .to_array(),
            [1.660491, -0.12455048, -0.01815076],
        );
    }

    #[test]
    fn floating_colors_round_trip_without_clipping() {
        let restored = SrgbRgba::new(-0.25, 0.4, 1.25, 1.0)
            .to_linear_rec2020()
            .to_srgb();
        assert_rgb_close(restored.to_array(), [-0.25, 0.4, 1.25]);
    }

    #[test]
    fn gamut_conversion_preserves_neutral_colors() {
        for converted in [
            LinearSrgbRgba::new(0.18, 0.18, 0.18, 1.0)
                .to_linear_rec2020()
                .to_array(),
            LinearRec2020Rgba::new(0.18, 0.18, 0.18, 1.0)
                .to_linear_srgb()
                .to_array(),
        ] {
            assert_rgb_close(converted, [0.18; 3]);
        }
    }

    #[test]
    fn floating_conversions_preserve_alpha() {
        let srgb = SrgbRgba::new(0.2, 0.4, 0.8, 0.25);
        let linear = LinearSrgbRgba::new(0.2, 0.4, 0.8, 0.25);
        let rec2020 = LinearRec2020Rgba::new(0.2, 0.4, 0.8, 0.25);
        for converted in [
            srgb.to_linear_srgb().to_array(),
            srgb.to_linear_rec2020().to_array(),
            linear.to_srgb().to_array(),
            linear.to_linear_rec2020().to_array(),
            rec2020.to_srgb().to_array(),
            rec2020.to_linear_srgb().to_array(),
        ] {
            assert_eq!(converted[3], 0.25);
        }
    }

    #[test]
    fn rgb_does_not_depend_on_alpha() {
        let converted = [0.0, 1.0].map(|alpha| {
            SrgbRgba::new(0.2, 0.4, 0.8, alpha)
                .to_linear_rec2020()
                .to_srgb()
                .to_array()
        });
        assert_eq!(converted[0][..3], converted[1][..3]);
    }

    #[test]
    fn byte_colors_round_trip() {
        for value in 0..=255 {
            let color = Srgb8Rgba::new(value, 127, 255 - value, value);
            assert_eq!(color.to_linear_rec2020().to_srgb8(), color);
        }
    }

    #[test]
    fn byte_quantization_clamps() {
        assert_eq!(
            SrgbRgba::new(-0.25, 1.25, f32::NEG_INFINITY, f32::INFINITY)
                .to_srgb8()
                .to_array(),
            [0, 255, 0, 255],
        );
    }

    #[test]
    fn byte_quantization_rounds_to_nearest_with_ties_up() {
        assert_eq!(
            SrgbRgba::new(0.25, 0.5, 0.75, 0.5).to_srgb8().to_array(),
            [64, 128, 191, 128],
        );
    }

    #[test]
    fn byte_quantization_maps_nan_to_zero() {
        assert_eq!(
            SrgbRgba::new(f32::NAN, f32::NAN, f32::NAN, f32::NAN)
                .to_srgb8()
                .to_array(),
            [0; 4],
        );
    }
}
