#[cfg_attr(verus_only, vstd::prelude::verus_verify)]
pub(crate) const MASK_5: u64 = 0xffff_0000_0000_ffff;
#[cfg_attr(verus_only, vstd::prelude::verus_verify)]
pub(crate) const MASK_4: u64 = 0x00ff_0000_ff00_00ff;
#[cfg_attr(verus_only, vstd::prelude::verus_verify)]
pub(crate) const MASK_3: u64 = 0xf00f_00f0_0f00_f00f;
#[cfg_attr(verus_only, vstd::prelude::verus_verify)]
pub(crate) const MASK_2: u64 = 0x30c3_0c30_c30c_30c3;
#[cfg_attr(verus_only, vstd::prelude::verus_verify)]
pub(crate) const MASK_1: u64 = 0x9249_2492_4924_9249;
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
#[path = "proof/morton.rs"]
pub(crate) mod proof;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
/// 三轴各 20 bit 的 Morton 路径；低位按 x、y、z 交错，编码占低 60 bit。
///
/// 构造时检查坐标或编码范围，之后无需重复检查位宽；不包含世界原点或有符号映射。
pub struct ZOrder20x3(pub(crate) u64);

#[cfg_attr(verus_only, verus_verify)]
impl ZOrder20x3 {
    /// 每轴的坐标位数。
    pub const BIT_DEPTH: u8 = 20;
    /// 每轴坐标范围的右端，不包含该值。
    pub const RANGE_END: u32 = 1_048_576;

    /// 三轴均为零的路径。
    #[cfg_attr(verus_only, verus_spec(ensures proof::value(Self::ZERO) == 0))]
    pub const ZERO: Self = Self(0);

    /// 判断三个坐标是否均位于 `0..RANGE_END`。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures result <==> x < 1_048_576 && y < 1_048_576 && z < 1_048_576, no_unwind))]
    pub const fn all_in_range(x: u32, y: u32, z: u32) -> bool {
        x < Self::RANGE_END && y < Self::RANGE_END && z < Self::RANGE_END
    }

    /// 编码坐标，任一轴越界返回 `None`。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is Some <==> x < 1_048_576 && y < 1_048_576 && z < 1_048_576,
            result is Some ==> proof::value(result->Some_0) == proof::interleave(x, y, z)
                && proof::deinterleave(proof::value(result->Some_0)) == (x, y, z)
                && proof::bit_encoding(proof::value(result->Some_0), x, y, z), no_unwind
    ))]
    pub const fn encode(x: u32, y: u32, z: u32) -> Option<Self> {
        if !Self::all_in_range(x, y, z) {
            return None;
        }
        #[cfg(verus_only)]
        proof! { proof::encode_decode(x, y, z); proof::interleave_bits(x, y, z); proof::input_mask(x, y, z); }
        Some(Self::encode_unchecked(x, y, z))
    }

    /// 截取每轴低 20 位编码；需要拒绝越界输入时使用 encode。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures proof::value(result) == proof::interleave(x, y, z), no_unwind))]
    pub(crate) const fn encode_unchecked(x: u32, y: u32, z: u32) -> Self {
        #[cfg(verus_only)]
        proof_decl! { let ghost original = (x, y, z); }
        let x = (x & (Self::RANGE_END - 1)) as u64;
        let y = (y & (Self::RANGE_END - 1)) as u64;
        let z = (z & (Self::RANGE_END - 1)) as u64;

        let x = (x | x << 32) & MASK_5;
        let y = (y | y << 32) & MASK_5;
        let z = (z | z << 32) & MASK_5;

        let x = (x | x << 16) & MASK_4;
        let y = (y | y << 16) & MASK_4;
        let z = (z | z << 16) & MASK_4;

        let x = (x | x << 8) & MASK_3;
        let y = (y | y << 8) & MASK_3;
        let z = (z | z << 8) & MASK_3;

        let x = (x | x << 4) & MASK_2;
        let y = (y | y << 4) & MASK_2;
        let z = (z | z << 4) & MASK_2;

        let x = (x | x << 2) & MASK_1;
        let y = (y | y << 2) & MASK_1;
        let z = (z | z << 2) & MASK_1;

        #[cfg(verus_only)]
        proof! { proof::interleave_range(original.0, original.1, original.2); }
        Self(x | (y << 1) | (z << 2))
    }

    /// 解码为 x、y、z，返回值均在有效坐标范围内。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == proof::deinterleave(proof::value(self)),
            result.0 < 1_048_576, result.1 < 1_048_576, result.2 < 1_048_576,
            proof::interleave(result.0, result.1, result.2) == proof::value(self), no_unwind
    ))]
    pub const fn decode(self) -> (u32, u32, u32) {
        #[cfg(verus_only)]
        proof! { use_type_invariant(&self); proof::deinterleave_range(self.0); proof::decode_encode(self.0); }
        let x = self.0 & MASK_1;
        let y = (self.0 >> 1) & MASK_1;
        let z = (self.0 >> 2) & MASK_1;

        let x = (x | (x >> 2)) & MASK_2;
        let y = (y | (y >> 2)) & MASK_2;
        let z = (z | (z >> 2)) & MASK_2;

        let x = (x | (x >> 4)) & MASK_3;
        let y = (y | (y >> 4)) & MASK_3;
        let z = (z | (z >> 4)) & MASK_3;

        let x = (x | (x >> 8)) & MASK_4;
        let y = (y | (y >> 8)) & MASK_4;
        let z = (z | (z >> 8)) & MASK_4;

        let x = (x | (x >> 16)) & MASK_5;
        let y = (y | (y >> 16)) & MASK_5;
        let z = (z | (z >> 16)) & MASK_5;

        let x = low_u32(x | (x >> 32));
        let y = low_u32(y | (y >> 32));
        let z = low_u32(z | (z >> 32));

        (x, y, z)
    }

    /// 从有效的 60 位 Morton 编码构造。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is Some <==> value < 0x1000_0000_0000_0000u64,
            result is Some ==> proof::value(result->Some_0) == value, no_unwind
    ))]
    pub const fn from_value(value: u64) -> Option<Self> {
        #[cfg(verus_only)]
        proof! { proof::range_constant(); }
        if value < (1_u64 << 60) {
            Some(Self(value))
        } else {
            None
        }
    }

    /// 返回低 60 bit 的 Morton 编码。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures result == proof::value(self), result < 0x1000_0000_0000_0000u64, no_unwind))]
    pub const fn value(self) -> u64 {
        #[cfg(verus_only)]
        proof! { use_type_invariant(&self); }
        self.0
    }
}

#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => ensures result == value as u32, no_unwind))]
const fn low_u32(value: u64) -> u32 {
    value as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn sixty_bit_boundary() {
        let max = (1_u64 << 60) - 1;
        assert_eq!(
            ZOrder20x3::from_value(max).map(ZOrder20x3::decode),
            Some((1_048_575, 1_048_575, 1_048_575))
        );
        for invalid in [1_u64 << 60, 1_u64 << 63, u64::MAX] {
            assert!(ZOrder20x3::from_value(invalid).is_none());
        }
        assert_eq!(
            ZOrder20x3::encode_unchecked(u32::MAX, u32::MAX, u32::MAX).value(),
            max
        );
    }

    /// 逐比特交织参照实现，验证 `encode`。
    fn encode_reference(x: u32, y: u32, z: u32) -> Option<u64> {
        if !ZOrder20x3::all_in_range(x, y, z) {
            return None;
        }
        Some((0..ZOrder20x3::BIT_DEPTH).fold(0u64, |code, bit| {
            let code = code | u64::from((x >> bit) & 1) << (bit * 3);
            let code = code | u64::from((y >> bit) & 1) << (bit * 3 + 1);
            code | u64::from((z >> bit) & 1) << (bit * 3 + 2)
        }))
    }

    /// 逐比特解交织参照实现，验证 `decode`。
    fn decode_reference(code: u64) -> (u32, u32, u32) {
        (0..ZOrder20x3::BIT_DEPTH).fold((0u32, 0u32, 0u32), |(x, y, z), bit| {
            let x = x | (((code >> (bit * 3)) & 1) as u32) << bit;
            let y = y | (((code >> (bit * 3 + 1)) & 1) as u32) << bit;
            let z = z | (((code >> (bit * 3 + 2)) & 1) as u32) << bit;
            (x, y, z)
        })
    }

    proptest! {
        /// `encode` 与参照实现一致。
        #[test]
        fn encode_matches_reference(
            x in 0u32..1 << ZOrder20x3::BIT_DEPTH,
            y in 0u32..1 << ZOrder20x3::BIT_DEPTH,
            z in 0u32..1 << ZOrder20x3::BIT_DEPTH,
        ) {
            let encoded = ZOrder20x3::encode(x, y, z).map(ZOrder20x3::value);
            let reference = encode_reference(x, y, z);
            prop_assert_eq!(encoded, reference);
        }

        /// `decode` 与参照实现一致。
        #[test]
        fn decode_matches_reference(code in 0u64..(1u64 << (u64::from(ZOrder20x3::BIT_DEPTH) * 3))) {
            let decoded = ZOrder20x3(code).decode();
            assert_eq!(decoded, decode_reference(code));
        }

        /// encode → decode 必须往返：decode(encode(x, y, z)) == (x, y, z)。
        #[test]
        fn encode_decode_round_trip(
            x in 0u32..1 << ZOrder20x3::BIT_DEPTH,
            y in 0u32..1 << ZOrder20x3::BIT_DEPTH,
            z in 0u32..1 << ZOrder20x3::BIT_DEPTH,
        ) {
            let decoded = ZOrder20x3::encode(x, y, z).map(ZOrder20x3::decode);
            prop_assert_eq!(decoded, Some((x, y, z)));
        }

        /// `decode` → `encode` 也往返。
        #[test]
        fn decode_encode_round_trip(code in 0u64..(1u64 << (u64::from(ZOrder20x3::BIT_DEPTH) * 3))) {
            let (x, y, z) = ZOrder20x3(code).decode();
            let reencoded = ZOrder20x3::encode(x, y, z).map(ZOrder20x3::value);
            prop_assert_eq!(reencoded, Some(code));
        }

        /// 超出 20-bit 范围的值必须返回 None。
        #[test]
        fn out_of_range_returns_none(
            x in prop_oneof![
                0u32..1 << ZOrder20x3::BIT_DEPTH,
                (1u32 << ZOrder20x3::BIT_DEPTH)..,
            ],
            y in prop_oneof![
                0u32..1 << ZOrder20x3::BIT_DEPTH,
                (1u32 << ZOrder20x3::BIT_DEPTH)..,
            ],
            z in prop_oneof![
                0u32..1 << ZOrder20x3::BIT_DEPTH,
                (1u32 << ZOrder20x3::BIT_DEPTH)..,
            ],
        ) {
            let result = ZOrder20x3::encode(x, y, z);
            if ZOrder20x3::all_in_range(x,y,z) {
                assert!(result.is_some());
            } else {
                assert!(result.is_none());
            }
        }
    }
}
