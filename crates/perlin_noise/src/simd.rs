use std::simd::{Select, StdFloat, f32x8, i32x8, prelude::*, u32x8, u64x8};

use crate::{CornerGradients, GradientLanes, PerlinNoise};

impl PerlinNoise {
    /// 采样八个独立坐标；有效域内逐 lane 使用相同算式。
    ///
    /// 调用方须在输入边界保证有限坐标位于 `[-MAX_COORD, MAX_COORD]`。
    /// 本热核不重复校验；超域时不保证噪声语义，需检查时用 [`Self::sample_3d_batch`]。
    /// 不分配，不修改实例，也不解释坐标的世界单位。
    #[must_use]
    pub fn noise_3d_x8(&self, x: [f32; 8], y: [f32; 8], z: [f32; 8]) -> [f32; 8] {
        self.noise_3d_simd(
            f32x8::from_array(x),
            f32x8::from_array(y),
            f32x8::from_array(z),
        )
        .to_array()
    }

    pub(crate) fn noise_3d_simd(&self, x_coord: f32x8, y_coord: f32x8, z_coord: f32x8) -> f32x8 {
        let x0f = x_coord.floor();
        let y0f = y_coord.floor();
        let z0f = z_coord.floor();

        let xf = x_coord - x0f;
        let yf = y_coord - y0f;
        let zf = z_coord - z0f;

        let ix0 = vf32_to_vi32(x0f).cast();
        let iy0 = vf32_to_vi32(y0f).cast();
        let iz0 = vf32_to_vi32(z0f).cast();

        let ix1 = ix0 + u32x8::splat(1);
        let iy1 = iy0 + u32x8::splat(1);
        let iz1 = iz0 + u32x8::splat(1);

        let weights = InterpWeights {
            x: fade(xf),
            y: fade(yf),
            z: fade(zf),
        };

        let xf1 = xf - f32x8::splat(1.0);
        let yf1 = yf - f32x8::splat(1.0);
        let zf1 = zf - f32x8::splat(1.0);

        let dots = CornerDots {
            d000: self.grad_dot_x8(ix0, iy0, iz0, xf, yf, zf),
            d100: self.grad_dot_x8(ix1, iy0, iz0, xf1, yf, zf),
            d010: self.grad_dot_x8(ix0, iy1, iz0, xf, yf1, zf),
            d110: self.grad_dot_x8(ix1, iy1, iz0, xf1, yf1, zf),
            d001: self.grad_dot_x8(ix0, iy0, iz1, xf, yf, zf1),
            d101: self.grad_dot_x8(ix1, iy0, iz1, xf1, yf, zf1),
            d011: self.grad_dot_x8(ix0, iy1, iz1, xf, yf1, zf1),
            d111: self.grad_dot_x8(ix1, iy1, iz1, xf1, yf1, zf1),
        };

        interpolate_gradient_dots(&weights, &dots)
    }

    /// 为八个整数格点生成球面梯度，不采样、不存储缓存。
    ///
    /// 所有 `i32` 格点均合法，按整数补码转 `u32` 后使用原 little-endian hash 编码。
    /// 输出依次为 x/y/z 分量，每个分量按输入 lane 顺序排列。
    #[must_use]
    pub fn gradient_x8(&self, x: [i32; 8], y: [i32; 8], z: [i32; 8]) -> GradientLanes {
        let hashes = self.hasher.hash_u32x3_simd(
            i32x8::from_array(x).cast(),
            i32x8::from_array(y).cast(),
            i32x8::from_array(z).cast(),
        );
        let (gx, gy, gz) = select_gradient(hashes);
        [gx.to_array(), gy.to_array(), gz.to_array()]
    }

    /// 使用给定八角梯度与单元内相对坐标采样，不读取 seed 或建立缓存。
    ///
    /// `x/y/z` 通常位于 `[0, 1]`，角序见 [`CornerGradients`]。本核按浮点规则接受任意
    /// 相对坐标和梯度，不裁剪或校验；超出单元表示多项式外推，非有限输入可产生非有限输出。
    /// 调用方维护格点、相对坐标、seed 与缓存生命周期的一致性；所有数据只借用或按值输入。
    #[must_use]
    pub fn sample_gradients_x8(
        x: [f32; 8],
        y: [f32; 8],
        z: [f32; 8],
        gradients: &CornerGradients,
    ) -> [f32; 8] {
        let x = f32x8::from_array(x);
        let y = f32x8::from_array(y);
        let z = f32x8::from_array(z);
        let weights = InterpWeights {
            x: fade(x),
            y: fade(y),
            z: fade(z),
        };
        let dots: [f32x8; 8] = std::array::from_fn(|corner| {
            let dx = x - f32x8::splat((corner & 1) as f32);
            let dy = y - f32x8::splat(((corner >> 1) & 1) as f32);
            let dz = z - f32x8::splat(((corner >> 2) & 1) as f32);
            let [gx, gy, gz] = gradients[corner].map(f32x8::from_array);
            gx.mul_add(dx, gy.mul_add(dy, gz * dz))
        });
        let [d000, d100, d010, d110, d001, d101, d011, d111] = dots;
        interpolate_gradient_dots(
            &weights,
            &CornerDots {
                d000,
                d100,
                d010,
                d110,
                d001,
                d101,
                d011,
                d111,
            },
        )
        .to_array()
    }

    fn grad_dot_x8(
        &self,
        cx: u32x8,
        cy: u32x8,
        cz: u32x8,
        dx: f32x8,
        dy: f32x8,
        dz: f32x8,
    ) -> f32x8 {
        let h = self.hasher.hash_u32x3_simd(cx, cy, cz);
        let (gx, gy, gz) = select_gradient(h);
        gx.mul_add(dx, gy.mul_add(dy, gz * dz))
    }
}

struct InterpWeights {
    x: f32x8,
    y: f32x8,
    z: f32x8,
}

struct CornerDots {
    d000: f32x8,
    d100: f32x8,
    d010: f32x8,
    d110: f32x8,
    d001: f32x8,
    d101: f32x8,
    d011: f32x8,
    d111: f32x8,
}

// Perlin fade 曲线：6t⁵ − 15t⁴ + 10t³

fn fade(t: f32x8) -> f32x8 {
    let inner = f32x8::splat(6.0)
        .mul_add(t, f32x8::splat(-15.0))
        .mul_add(t, f32x8::splat(10.0));
    t * t * t * inner
}

// 梯度插值

fn lerp(t: f32x8, a: f32x8, b: f32x8) -> f32x8 {
    t.mul_add(b - a, a)
}

fn interpolate_gradient_dots(weights: &InterpWeights, dots: &CornerDots) -> f32x8 {
    const FRAC_2_SQRT_3: f32 = 1.154_700_5;

    let x00 = lerp(weights.x, dots.d000, dots.d100);
    let x10 = lerp(weights.x, dots.d010, dots.d110);
    let x01 = lerp(weights.x, dots.d001, dots.d101);
    let x11 = lerp(weights.x, dots.d011, dots.d111);

    let y0 = lerp(weights.y, x00, x10);
    let y1 = lerp(weights.y, x01, x11);

    lerp(weights.z, y0, y1) * f32x8::splat(FRAC_2_SQRT_3)
}

// 单位球面上的随机梯度

fn u32_to_f32_01(u: u32x8) -> f32x8 {
    f32x8::from_bits(u32x8::splat(0x3F80_0000) | (u >> u32x8::splat(9))) - f32x8::splat(1.0)
}

fn vf32_to_vi32(v: f32x8) -> i32x8 {
    const MAGIC: f32 = 12_582_912.0;
    const MAGIC_BITS: u32 = 0x4B40_0000;
    ((v + f32x8::splat(MAGIC)).to_bits() - u32x8::splat(MAGIC_BITS)).cast()
}

fn sincos_turns(u: f32x8) -> (f32x8, f32x8) {
    const SIN_C3: f32 = -1.666_666_7e-1;
    const SIN_C5: f32 = 8.333_333e-3;
    const SIN_C7: f32 = -1.981_07e-4;
    const SIN_C9: f32 = 2.608_316e-6;

    const COS_C2: f32 = -4.999_999_7e-1;
    const COS_C4: f32 = 4.166_664_6e-2;
    const COS_C6: f32 = -1.388_731_6e-3;
    const COS_C8: f32 = 2.479_904_5e-5;

    let four_u = f32x8::splat(4.0) * u;
    let nf = (four_u + f32x8::splat(0.5)).floor();
    let q = vf32_to_vi32(nf);
    let theta = (four_u - nf) * f32x8::splat(std::f32::consts::FRAC_PI_2);

    let x2 = theta * theta;

    let sp = f32x8::splat(SIN_C9)
        .mul_add(x2, f32x8::splat(SIN_C7))
        .mul_add(x2, f32x8::splat(SIN_C5))
        .mul_add(x2, f32x8::splat(SIN_C3));
    let x3 = x2 * theta;
    let sin_r = sp.mul_add(x3, theta);

    let cq = f32x8::splat(COS_C8)
        .mul_add(x2, f32x8::splat(COS_C6))
        .mul_add(x2, f32x8::splat(COS_C4))
        .mul_add(x2, f32x8::splat(COS_C2));
    let cos_r = cq.mul_add(x2, f32x8::splat(1.0));

    let odd = (q & i32x8::splat(1)).simd_ne(i32x8::splat(0));
    let sin_base = odd.select(cos_r, sin_r);
    let cos_base = odd.select(sin_r, cos_r);

    let q_u = q.cast();
    let sin_sign = (q_u & u32x8::splat(2)) << u32x8::splat(30);
    let cos_sign = ((q_u + u32x8::splat(1)) & u32x8::splat(2)) << u32x8::splat(30);

    let sin_val = f32x8::from_bits(sin_base.to_bits() ^ sin_sign);
    let cos_val = f32x8::from_bits(cos_base.to_bits() ^ cos_sign);

    (sin_val, cos_val)
}

fn select_gradient(h: u64x8) -> (f32x8, f32x8, f32x8) {
    let h_lo = h.cast();
    let h_hi = (h >> u64x8::splat(32)).cast();

    let u1 = u32_to_f32_01(h_lo);
    let u2 = u32_to_f32_01(h_hi);

    let z = u2 * f32x8::splat(2.0) - f32x8::splat(1.0);
    let r_sq = (f32x8::splat(1.0) - z * z).simd_max(f32x8::splat(0.0));
    let r = r_sq.sqrt();
    let (sin_phi, cos_phi) = sincos_turns(u1);

    (r * cos_phi, r * sin_phi, z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{array::uniform8, prelude::*};

    const MAX_TEST_COORD: f32 = 2_097_152.0;

    const MAX_ABS_ERROR: f64 = f32::EPSILON as f64 * 8.0;

    fn finite_coord_lanes() -> impl Strategy<Value = [f32; 8]> {
        uniform8(-MAX_TEST_COORD..MAX_TEST_COORD)
    }

    fn unit_interval_lanes() -> impl Strategy<Value = [f32; 8]> {
        uniform8(0.0f32..1.0f32)
    }

    fn integer_lattice_lanes() -> impl Strategy<Value = [f32; 8]> {
        uniform8(-4_000_000.0f32..=4_000_000.0f32).prop_map(|values| values.map(f32::round))
    }

    fn scalar_u32_to_f32_01(u: u32) -> f32 {
        f32::from_bits(0x3F80_0000 | (u >> 9)) - 1.0
    }

    fn scalar_u32_to_f64_01(u: u32) -> f64 {
        f64::from(scalar_u32_to_f32_01(u))
    }

    fn output_scale_f64() -> f64 {
        (0.5 * 3.0_f64.sqrt()).recip()
    }

    fn assert_f32_near_f64(actual: f32, expected: f64, context: &str) {
        let diff = (f64::from(actual) - expected).abs();
        assert!(
            diff <= MAX_ABS_ERROR,
            "{context}: actual={actual:?} expected={expected:?} abs_diff={diff}"
        );
    }

    fn make_12byte_msg(a: u32, b: u32, c: u32) -> [u8; 12] {
        let [a0, a1, a2, a3] = a.to_le_bytes();
        let [b0, b1, b2, b3] = b.to_le_bytes();
        let [c0, c1, c2, c3] = c.to_le_bytes();
        [a0, a1, a2, a3, b0, b1, b2, b3, c0, c1, c2, c3]
    }

    fn scalar_hash_u32x3(hasher: &sip_hash::SipHash<1, 3>, x: u32, y: u32, z: u32) -> u64 {
        let msg = make_12byte_msg(x, y, z);
        hasher.hash(&msg)
    }

    fn scalar_select_gradient(h: u64) -> (f64, f64, f64) {
        let (h_low, h_high) = split_u64(h);
        let u1 = scalar_u32_to_f64_01(h_low);
        let u2 = scalar_u32_to_f64_01(h_high);
        let z = u2.mul_add(2.0, -1.0);
        let r = (1.0 - z * z).max(0.0).sqrt();
        let phi = std::f64::consts::TAU * u1;
        let (sin_phi, cos_phi) = phi.sin_cos();
        (r * cos_phi, r * sin_phi, z)
    }

    const fn split_u64(value: u64) -> (u32, u32) {
        let [
            byte_0,
            byte_1,
            byte_2,
            byte_3,
            byte_4,
            byte_5,
            byte_6,
            byte_7,
        ] = value.to_le_bytes();
        (
            u32::from_le_bytes([byte_0, byte_1, byte_2, byte_3]),
            u32::from_le_bytes([byte_4, byte_5, byte_6, byte_7]),
        )
    }

    fn floor_to_u32_bits(v: f32) -> u32 {
        v.floor() as i32 as u32
    }

    fn fade_scalar(t: f64) -> f64 {
        let t2 = t * t;
        let t3 = t2 * t;
        t3 * 6.0f64.mul_add(t, -15.0).mul_add(t, 10.0)
    }

    fn lerp_scalar(t: f64, a: f64, b: f64) -> f64 {
        t.mul_add(b - a, a)
    }

    fn grad_dot_scalar(
        noise: &PerlinNoise,
        cx: u32,
        cy: u32,
        cz: u32,
        dx: f64,
        dy: f64,
        dz: f64,
    ) -> f64 {
        let hash = scalar_hash_u32x3(&noise.hasher, cx, cy, cz);
        let (gx, gy, gz) = scalar_select_gradient(hash);
        gx.mul_add(dx, gy.mul_add(dy, gz * dz))
    }

    fn noise3_scalar(noise: &PerlinNoise, x_f32: f32, y_f32: f32, z_f32: f32) -> f64 {
        let x_coord = f64::from(x_f32);
        let y_coord = f64::from(y_f32);
        let z_coord = f64::from(z_f32);

        let x0f = x_coord.floor();
        let y0f = y_coord.floor();
        let z0f = z_coord.floor();

        let xf = x_coord - x0f;
        let yf = y_coord - y0f;
        let zf = z_coord - z0f;

        let ix0 = floor_to_u32_bits(x_f32);
        let iy0 = floor_to_u32_bits(y_f32);
        let iz0 = floor_to_u32_bits(z_f32);
        let ix1 = ix0.wrapping_add(1);
        let iy1 = iy0.wrapping_add(1);
        let iz1 = iz0.wrapping_add(1);

        let x_weight = fade_scalar(xf);
        let y_weight = fade_scalar(yf);
        let z_weight = fade_scalar(zf);

        let xf1 = xf - 1.0;
        let yf1 = yf - 1.0;
        let zf1 = zf - 1.0;

        let d000 = grad_dot_scalar(noise, ix0, iy0, iz0, xf, yf, zf);
        let d100 = grad_dot_scalar(noise, ix1, iy0, iz0, xf1, yf, zf);
        let d010 = grad_dot_scalar(noise, ix0, iy1, iz0, xf, yf1, zf);
        let d110 = grad_dot_scalar(noise, ix1, iy1, iz0, xf1, yf1, zf);
        let d001 = grad_dot_scalar(noise, ix0, iy0, iz1, xf, yf, zf1);
        let d101 = grad_dot_scalar(noise, ix1, iy0, iz1, xf1, yf, zf1);
        let d011 = grad_dot_scalar(noise, ix0, iy1, iz1, xf, yf1, zf1);
        let d111 = grad_dot_scalar(noise, ix1, iy1, iz1, xf1, yf1, zf1);

        let x00 = lerp_scalar(x_weight, d000, d100);
        let x10 = lerp_scalar(x_weight, d010, d110);
        let x01 = lerp_scalar(x_weight, d001, d101);
        let x11 = lerp_scalar(x_weight, d011, d111);
        let y0 = lerp_scalar(y_weight, x00, x10);
        let y1 = lerp_scalar(y_weight, x01, x11);

        lerp_scalar(z_weight, y0, y1) * output_scale_f64()
    }

    mod components {
        use super::*;

        #[test]
        fn lattice_conversion_matches_signed_integer_at_domain_edges() {
            let coords = [
                -crate::MAX_COORD,
                (-crate::MAX_COORD).next_up(),
                -17.001,
                -1.0,
                -0.001,
                -0.0,
                crate::MAX_COORD.next_down(),
                crate::MAX_COORD,
            ];
            let floors = f32x8::from_array(coords).floor();
            let actual: [u32; 8] = vf32_to_vi32(floors).cast().to_array();
            assert_eq!(actual, coords.map(floor_to_u32_bits));
        }

        proptest! {
            #[test]
            fn select_gradient_matches_f64_reference(
                seed0 in any::<u64>(),
                seed1 in any::<u64>(),
                xs in uniform8(any::<u32>()),
                ys in uniform8(any::<u32>()),
                zs in uniform8(any::<u32>()),
            ) {
                let noise = PerlinNoise::new((seed0, seed1));
                let hashes: [u64; 8] = std::array::from_fn(|lane| {
                    scalar_hash_u32x3(&noise.hasher, xs[lane], ys[lane], zs[lane])
                });
                let [gx, gy, gz] = noise.gradient_x8(
                    xs.map(|v| v as i32),
                    ys.map(|v| v as i32),
                    zs.map(|v| v as i32),
                );

                for (lane, ((&ax, &ay), (&az, &h))) in gx.iter().zip(&gy).zip(gz.iter().zip(&hashes)).enumerate() {
                    let (expected_x, expected_y, expected_z) = scalar_select_gradient(h);
                    assert_f32_near_f64(ax, expected_x, &format!("gradient x lane {lane}"));
                    assert_f32_near_f64(ay, expected_y, &format!("gradient y lane {lane}"));
                    assert_f32_near_f64(az, expected_z, &format!("gradient z lane {lane}"));
                }
            }

            #[test]
            fn sincos_turns_matches_stdlib(
                us in unit_interval_lanes(),
            ) {
                let (sin_vals, cos_vals) = sincos_turns(f32x8::from_array(us));
                let sin_vals = sin_vals.to_array();
                let cos_vals = cos_vals.to_array();

                for (lane, ((&sv, &cv), &u)) in sin_vals.iter().zip(&cos_vals).zip(&us).enumerate() {
                    let phi = std::f64::consts::TAU * f64::from(u);
                    let (expected_sin, expected_cos) = phi.sin_cos();
                    assert_f32_near_f64(sv, expected_sin, &format!("sin lane {lane}"));
                    assert_f32_near_f64(cv, expected_cos, &format!("cos lane {lane}"));
                }
            }
        }
    }

    mod noise3 {
        use super::*;

        proptest! {
            #[test]
            fn supplied_gradients_preserve_direct_samples(
                seed0 in any::<u64>(),
                seed1 in any::<u64>(),
                xs in finite_coord_lanes(),
                ys in finite_coord_lanes(),
                zs in finite_coord_lanes(),
            ) {
                let noise = PerlinNoise::new((seed0, seed1));
                let bx = xs.map(|v| v.floor() as i32);
                let by = ys.map(|v| v.floor() as i32);
                let bz = zs.map(|v| v.floor() as i32);
                let gradients = std::array::from_fn(|corner| {
                    noise.gradient_x8(
                        bx.map(|v| v.wrapping_add((corner & 1) as i32)),
                        by.map(|v| v.wrapping_add(((corner >> 1) & 1) as i32)),
                        bz.map(|v| v.wrapping_add(((corner >> 2) & 1) as i32)),
                    )
                });
                let actual = PerlinNoise::sample_gradients_x8(
                    xs.map(|v| v - v.floor()),
                    ys.map(|v| v - v.floor()),
                    zs.map(|v| v - v.floor()),
                    &gradients,
                );
                let expected = noise.noise_3d_x8(xs, ys, zs);
                prop_assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
            }

            #[test]
            fn lanes_match_splat_evaluation(
                seed0 in any::<u64>(),
                seed1 in any::<u64>(),
                xs in finite_coord_lanes(),
                ys in finite_coord_lanes(),
                zs in finite_coord_lanes(),
            ) {
                let noise = PerlinNoise::new((seed0, seed1));
                let batch = noise
                    .noise_3d_simd(
                        f32x8::from_array(xs),
                        f32x8::from_array(ys),
                        f32x8::from_array(zs),
                    )
                    .to_array();

                for (lane, ((&b, (&x, &y)), &z)) in batch.iter().zip(xs.iter().zip(&ys)).zip(&zs).enumerate() {
                    let scalar = noise
                        .noise_3d_simd(
                            f32x8::splat(x),
                            f32x8::splat(y),
                            f32x8::splat(z),
                        )
                        .to_array()[0];
                    prop_assert_eq!(b.to_bits(), scalar.to_bits(), "lane {}", lane);
                }
            }

            #[test]
            fn matches_scalar_oracle(
                seed0 in any::<u64>(),
                seed1 in any::<u64>(),
                xs in finite_coord_lanes(),
                ys in finite_coord_lanes(),
                zs in finite_coord_lanes(),
            ) {
                let noise = PerlinNoise::new((seed0, seed1));
                let batch = noise
                    .noise_3d_simd(
                        f32x8::from_array(xs),
                        f32x8::from_array(ys),
                        f32x8::from_array(zs),
                    )
                    .to_array();

                for (lane, ((&b, (&x, &y)), &z)) in batch.iter().zip(xs.iter().zip(&ys)).zip(&zs).enumerate() {
                    let expected = noise3_scalar(&noise, x, y, z);
                    assert_f32_near_f64(b, expected, &format!("noise lane {lane}"));
                }
            }

            #[test]
            fn integer_lattice_points_are_zero(
                seed0 in any::<u64>(),
                seed1 in any::<u64>(),
                xs in integer_lattice_lanes(),
                ys in integer_lattice_lanes(),
                zs in integer_lattice_lanes(),
            ) {
                let noise = PerlinNoise::new((seed0, seed1));
                let output = noise.noise_3d_simd(
                    f32x8::from_array(xs),
                    f32x8::from_array(ys),
                    f32x8::from_array(zs),
                );

                for value in output.to_array() {
                    prop_assert_eq!(value.to_bits() & 0x7FFF_FFFF, 0, "lattice point output={}", value);
                }
            }

            #[test]
            fn outputs_are_finite(
                seed0 in any::<u64>(),
                seed1 in any::<u64>(),
                xs in finite_coord_lanes(),
                ys in finite_coord_lanes(),
                zs in finite_coord_lanes(),
            ) {
                let output = PerlinNoise::new((seed0, seed1))
                    .noise_3d_simd(
                        f32x8::from_array(xs),
                        f32x8::from_array(ys),
                        f32x8::from_array(zs),
                    )
                    .to_array();

                for value in output {
                    prop_assert!(value.is_finite(), "non-finite noise output={value}");
                }
            }

            #[test]
            fn deterministic_for_same_seed(
                seed0 in any::<u64>(),
                seed1 in any::<u64>(),
                xs in finite_coord_lanes(),
                ys in finite_coord_lanes(),
                zs in finite_coord_lanes(),
            ) {
                let noise = PerlinNoise::new((seed0, seed1));
                let a = noise
                    .noise_3d_simd(
                        f32x8::from_array(xs),
                        f32x8::from_array(ys),
                        f32x8::from_array(zs),
                    )
                    .to_array();
                let b = noise
                    .noise_3d_simd(
                        f32x8::from_array(xs),
                        f32x8::from_array(ys),
                        f32x8::from_array(zs),
                    )
                    .to_array();

                prop_assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
            }
        }
    }

    mod interpolation {
        use super::*;

        fn corner_dot([gx, gy, gz]: [f32; 3], dx: f32x8, dy: f32x8, dz: f32x8) -> f32x8 {
            f32x8::splat(gx) * dx + f32x8::splat(gy) * dy + f32x8::splat(gz) * dz
        }

        fn make_extreme_setup() -> (InterpWeights, f32x8, f32x8, f32x8, f32x8, f32x8, f32x8, f32) {
            const INV_SQRT3: f32 = 0.577_350_26;

            let xf = f32x8::splat(0.5);
            let yf = f32x8::splat(0.5);
            let zf = f32x8::splat(0.5);
            let one = f32x8::splat(1.0);
            let xf1 = xf - one;
            let yf1 = yf - one;
            let zf1 = zf - one;
            let weights = InterpWeights {
                x: fade(xf),
                y: fade(yf),
                z: fade(zf),
            };
            (weights, xf, yf, zf, xf1, yf1, zf1, INV_SQRT3)
        }

        #[test]
        fn extreme_positive_gradients_hits_one() {
            let (weights, xf, yf, zf, xf1, yf1, zf1, s) = make_extreme_setup();
            let dots = CornerDots {
                d000: corner_dot([s, s, s], xf, yf, zf),
                d100: corner_dot([-s, s, s], xf1, yf, zf),
                d010: corner_dot([s, -s, s], xf, yf1, zf),
                d110: corner_dot([-s, -s, s], xf1, yf1, zf),
                d001: corner_dot([s, s, -s], xf, yf, zf1),
                d101: corner_dot([-s, s, -s], xf1, yf, zf1),
                d011: corner_dot([s, -s, -s], xf, yf1, zf1),
                d111: corner_dot([-s, -s, -s], xf1, yf1, zf1),
            };

            let output = interpolate_gradient_dots(&weights, &dots).to_array();

            for value in output {
                assert_f32_near_f64(value, 1.0, "positive extreme output");
            }
        }

        #[test]
        fn extreme_negative_gradients_hits_minus_one() {
            let (weights, xf, yf, zf, xf1, yf1, zf1, s) = make_extreme_setup();
            let dots = CornerDots {
                d000: corner_dot([-s, -s, -s], xf, yf, zf),
                d100: corner_dot([s, -s, -s], xf1, yf, zf),
                d010: corner_dot([-s, s, -s], xf, yf1, zf),
                d110: corner_dot([s, s, -s], xf1, yf1, zf),
                d001: corner_dot([-s, -s, s], xf, yf, zf1),
                d101: corner_dot([s, -s, s], xf1, yf, zf1),
                d011: corner_dot([-s, s, s], xf, yf1, zf1),
                d111: corner_dot([s, s, s], xf1, yf1, zf1),
            };

            let output = interpolate_gradient_dots(&weights, &dots).to_array();

            for value in output {
                assert_f32_near_f64(value, -1.0, "negative extreme output");
            }
        }
    }
}
