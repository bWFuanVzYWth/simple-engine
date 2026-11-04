use std::simd::{num::SimdUint, u32x8, u64x8};

#[cfg(not(verus_only))]
use super::ops;
#[cfg(verus_only)]
use super::scalar::proof as scalar;
use crate::SipHash;
#[cfg(verus_only)]
use sip_hash_std::ops;
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "proof/simd.rs"]
#[cfg(verus_only)]
pub(crate) mod proof;

#[cfg(verus_only)]
verus! { broadcast use sip_hash_std::primitives; }

type State = (u64x8, u64x8, u64x8, u64x8);

/// 对任意八路四字状态，每个 lane 分别返回模块规范的 `R`，不混合 lane。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => ensures
    forall|i: int| 0 <= i < 8 ==> proof::lane(result, i) == scalar::round(proof::lane(v, i)),
    no_unwind
))]
fn sipround(v: State) -> State {
    let v0 = v.0;
    let v1 = v.1;
    let v2 = v.2;
    let v3 = v.3;

    let v0 = ops::add(v0, v1);
    let v1 = rotl(v1, 13);
    let v1 = ops::xor(v1, v0);
    let v0 = rotl(v0, 32);
    let v2 = ops::add(v2, v3);
    let v3 = rotl(v3, 16);
    let v3 = ops::xor(v3, v2);
    let v0 = ops::add(v0, v3);
    let v3 = rotl(v3, 21);
    let v3 = ops::xor(v3, v0);
    let v2 = ops::add(v2, v1);
    let v1 = rotl(v1, 17);
    let v1 = ops::xor(v1, v2);
    let v2 = rotl(v2, 32);

    (v0, v1, v2, v3)
}

/// 内部域为 `1 <= n < 64`；结果逐 lane 等于 `x[i].rotate_left(n)`。
/// 生产调用仅使用 13、16、17、21、32，左右移位量均大于 0 且小于 64。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires 1 <= n < 64,
    ensures sip_hash_std::view64(result) == Seq::new(8, |i: int|
        scalar::rotate(sip_hash_std::view64(x)[i], n)),
    no_unwind
))]
fn rotl(x: u64x8, n: u32) -> u64x8 {
    ops::or(
        ops::shl(x, u64x8::splat(n as u64)),
        ops::shr(x, u64x8::splat(64 - n as u64)),
    )
}

/// 八路状态各应用恰好 `N` 轮，零轮保持原状态。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result => ensures
    proof::iterated(state, result, N as nat), proof::iterated_view(state, result, N as nat),
    no_unwind
))]
fn siprounds<const N: usize>(state: State) -> State {
    let mut v = state;
    if N > 0 {
        let mut done = 0;
        #[cfg_attr(verus_only, verus_spec(
            invariant done <= N, proof::iterated(state, v, done as nat),
            invariant_except_break done < N,
            ensures done == N,
            decreases N - done,
        ))]
        loop {
            #[cfg(verus_only)]
            proof! { proof::rounds_step(state, done as nat); }
            v = sipround(v);
            done += 1;
            if done == N {
                break;
            }
        }
    }
    #[cfg(verus_only)]
    proof! { proof::rounds_view(state, v, N as nat); }
    v
}

#[cfg_attr(verus_only, verus_verify)]
impl<const C: usize, const D: usize> SipHash<C, D> {
    /// 逐 lane 计算八个独立的三坐标消息，顺序与输入 lane 一致。
    ///
    /// 每条消息的编码与 [`Self::hash_u32x3`] 相同；不分配或调度。
    /// 对任意 `0 <= i < 8`，`result[i] = self.hash_u32x3(a[i], b[i], c[i])`；
    /// 全部输入位型及全部 `C, D` 均合法，实例状态不变，不返回输入错误。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures
        forall|i: int| 0 <= i < 8 ==> result@[i] == scalar::coords(
            self.state(), a@[i], b@[i], c@[i], C as nat, D as nat),
        no_unwind
    ))]
    pub fn hash_u32x3_x8(&self, a: [u32; 8], b: [u32; 8], c: [u32; 8]) -> [u64; 8] {
        let lanes = self.hash_u32x3_simd(
            u32x8::from_array(a),
            u32x8::from_array(b),
            u32x8::from_array(c),
        );
        let result = lanes.to_array();
        #[cfg(verus_only)]
        proof! {
            assert forall|i: int| 0 <= i < 8 implies result@[i]
                == scalar::coords(self.state(), a@[i], b@[i], c@[i], C as nat, D as nat) by {
                    assert(sip_hash_std::view64(lanes)[i]
                        == scalar::coords(self.state(), a@[i], b@[i], c@[i], C as nat, D as nat));
                }
        }
        result
    }

    /// [`Self::hash_u32x3_x8`] 的 SIMD 入口，供批内核保持向量状态。
    ///
    /// 对任意 `0 <= i < 8`，返回 lane `i` 等于
    /// `self.hash_u32x3(a[i], b[i], c[i])`，不依赖其它 lane。
    /// 所有 `u32` 位型及全部 `C, D` 均合法，实例状态不变，不分配、调度或返回输入错误。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures
        forall|i: int| 0 <= i < 8 ==> sip_hash_std::view64(result)[i] == scalar::coords(
            self.state(), sip_hash_std::view32(a)[i], sip_hash_std::view32(b)[i], sip_hash_std::view32(c)[i], C as nat, D as nat),
        no_unwind
    ))]
    pub fn hash_u32x3_simd(&self, a: u32x8, b: u32x8, c: u32x8) -> u64x8 {
        const LEN: u64 = 12;

        #[cfg(verus_only)]
        proof! { proof::coordinates(self.state(), a, b, c, C as nat, D as nat); }

        let a = a.cast();
        let b = b.cast();
        let c = c.cast();

        let m0 = ops::or(a, ops::shl(b, u64x8::splat(32)));
        let m1 = ops::or(u64x8::splat(LEN << 56), c);

        let v0 = u64x8::splat(self.v0);
        let v1 = u64x8::splat(self.v1);
        let v2 = u64x8::splat(self.v2);
        let v3 = u64x8::splat(self.v3);

        let v3 = ops::xor(v3, m0);
        let (v0, v1, v2, v3) = siprounds::<C>((v0, v1, v2, v3));
        let v0 = ops::xor(v0, m0);

        let v3 = ops::xor(v3, m1);
        let (v0, v1, v2, v3) = siprounds::<C>((v0, v1, v2, v3));
        let v0 = ops::xor(v0, m1);

        let v2 = ops::xor(v2, u64x8::splat(0xFF));
        let (v0, v1, v2, v3) = siprounds::<D>((v0, v1, v2, v3));

        ops::xor(ops::xor(ops::xor(v0, v1), v2), v3)
    }
}

#[cfg(test)]
mod tests {
    use std::simd::{u32x8, u64x8};

    use proptest::{collection::vec, prelude::*};

    use super::*;

    #[test]
    fn array_entry_preserves_lane_messages() {
        let hash = SipHash::<1, 3>::new((7, 11));
        let a = [0, 1, 7, 8, 9, 123, 1 << 31, u32::MAX];
        let b = [u32::MAX, 27, 3, 9, 7, 0, 1 << 17, 35];
        let c = [1, 0, 3, 7, 29, 16, u32::MAX, 1];
        assert_eq!(
            hash.hash_u32x3_x8(a, b, c),
            std::array::from_fn(|lane| hash.hash_u32x3(a[lane], b[lane], c[lane]))
        );
    }

    fn make_12byte_msg(a: u32, b: u32, c: u32) -> [u8; 12] {
        let [a0, a1, a2, a3] = a.to_le_bytes();
        let [b0, b1, b2, b3] = b.to_le_bytes();
        let [c0, c1, c2, c3] = c.to_le_bytes();
        [a0, a1, a2, a3, b0, b1, b2, b3, c0, c1, c2, c3]
    }

    fn assert_hashes_match_scalar<const C: usize, const D: usize>(
        seed: (u64, u64),
        lanes: &[(u32, u32, u32)],
    ) {
        assert!(lanes.len().is_multiple_of(8));

        let hasher = SipHash::<C, D>::new(seed);

        let (lane_chunks, remainder) = lanes.as_chunks::<8>();
        assert!(remainder.is_empty());

        let actual: Vec<u64> = lane_chunks
            .iter()
            .flat_map(|chunk| {
                let a = (*chunk).map(|(a, _, _)| a);
                let b = (*chunk).map(|(_, b, _)| b);
                let c = (*chunk).map(|(_, _, c)| c);
                hasher
                    .hash_u32x3_simd(
                        u32x8::from_array(a),
                        u32x8::from_array(b),
                        u32x8::from_array(c),
                    )
                    .to_array()
            })
            .collect();

        let expected: Vec<u64> = lanes
            .iter()
            .map(|&(a, b, c)| {
                let msg = make_12byte_msg(a, b, c);
                SipHash::<C, D>::new(seed).hash(&msg)
            })
            .collect();

        assert_eq!(actual, expected);
    }

    #[test]
    fn rotl_correctness() {
        let x = u64x8::from_array([
            0,
            1,
            0x8000_0000_0000_0000,
            u64::MAX,
            0x1234_5678_9ABC_DEF0,
            0,
            0,
            0,
        ]);
        let r = rotl(x, 1);
        let a = r.to_array();
        assert_eq!(a[0], 0);
        assert_eq!(a[1], 2);
        assert_eq!(a[2], 1);
        assert_eq!(a[3], u64::MAX);
        assert_eq!(a[4], 0x2468_ACF1_3579_BDE0);
    }

    #[test]
    fn make_12byte_msg_roundtrip() {
        let (a, b, c) = (0x0123_4567u32, 0x89AB_CDEFu32, 0x7654_3210u32);
        let msg = make_12byte_msg(a, b, c);
        let expected: Vec<u8> = a
            .to_le_bytes()
            .into_iter()
            .chain(b.to_le_bytes())
            .chain(c.to_le_bytes())
            .collect();
        assert_eq!(msg.to_vec(), expected);

        let seed = (0, 0);
        let direct = SipHash::<2, 4>::new(seed).hash(&expected);
        let via_helper = SipHash::<2, 4>::new(seed).hash(&msg);
        assert_eq!(direct, via_helper);
    }

    proptest! {
        #[test]
        fn fuzz_simd_matches_scalar_2_4(
            seed0 in any::<u64>(),
            seed1 in any::<u64>(),
            lanes in vec((any::<u32>(), any::<u32>(), any::<u32>()), 32),
        ) {
            assert_hashes_match_scalar::<2, 4>((seed0, seed1), &lanes);
        }

        #[test]
        fn fuzz_simd_matches_scalar_1_3(
            seed0 in any::<u64>(),
            seed1 in any::<u64>(),
            lanes in vec((any::<u32>(), any::<u32>(), any::<u32>()), 24),
        ) {
            assert_hashes_match_scalar::<1, 3>((seed0, seed1), &lanes);
        }
    }
}
