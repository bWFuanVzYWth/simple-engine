use crate::SipHash;
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "proof/scalar.rs"]
#[cfg(verus_only)]
pub(crate) mod proof;

type State = (u64, u64, u64, u64);

#[cfg_attr(verus_only, verus_verify)]
impl<const C: usize, const D: usize> SipHash<C, D> {
    /// 计算任意长度消息；以 little-endian 读块，尾块含消息长度的低八位。
    ///
    /// 不分配，空消息合法；实例的初始化状态保持不变。
    /// 结果等于模块规范的 `H_C,D(seed.0, seed.1, msg)`，其中 seed 是构造实例时的密钥字。
    /// 任意有效切片及全部 `C, D` 均在输入域内；整块消息仍压缩长度尾块，不返回输入错误。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == proof::hash(self.state(), msg@, C as nat, D as nat),
        no_unwind
    ))]
    pub fn hash(&self, msg: &[u8]) -> u64 {
        let (chunks, remainder) = msg.as_chunks::<8>();
        let mut tail = 0;
        let mut last = (msg.len() as u64) << 56;
        #[cfg(verus_only)]
        proof! { proof::tail_start(msg@); }
        #[cfg_attr(verus_only, verus_spec(
            invariant proof::partitioned(msg@, chunks@, remainder@),
                tail <= remainder@.len(), last == proof::partial_tail(msg@, tail as nat),
            decreases remainder@.len() - tail,
        ))]
        while tail < remainder.len() {
            last |= (remainder[tail] as u64) << (tail * 8);
            #[cfg(verus_only)]
            proof! { proof::tail_step(msg@, chunks@, remainder@, tail as nat); }
            tail += 1;
        }
        let mut state = (self.v0, self.v1, self.v2, self.v3);
        let mut pending = chunks;
        #[cfg(verus_only)]
        proof! { proof::pending_start(chunks@); }
        #[cfg_attr(verus_only, verus_spec(
            invariant proof::partitioned(msg@, chunks@, remainder@),
                last == proof::tail(msg@), proof::unprocessed(chunks@, pending@),
                state == proof::absorbed(self.state(), msg@, (chunks@.len() - pending@.len()) as nat, C as nat),
            ensures pending@.len() == 0,
            decreases pending@.len(),
        ))]
        while let Some((word, rest)) = pending.split_first() {
            let m = read_word(*word);
            #[cfg(verus_only)]
            proof! { proof::pending_word(msg@, chunks@, remainder@, pending@, *word, rest@); }
            let (v0, v1, v2, v3) = state;
            let (v0, v1, v2, v3) = siprounds::<C>((v0, v1, v2, v3 ^ m));
            state = (v0 ^ m, v1, v2, v3);
            #[cfg(verus_only)]
            proof! { proof::absorbed_step(self.state(), msg@, (chunks@.len() - pending@.len()) as nat, C as nat); }
            pending = rest;
        }
        let (v0, v1, v2, v3) = state;
        let (v0, v1, v2, v3) = siprounds::<C>((v0, v1, v2, v3 ^ last));
        let (v0, v1, v2, v3) = siprounds::<D>((v0 ^ last, v1, v2 ^ 0xFF, v3));

        v0 ^ v1 ^ v2 ^ v3
    }

    /// 计算三个 `u32` 的 12 字节 little-endian 消息，不构造中间字节缓冲。
    ///
    /// 对全部 `u32` 位型及全部 `C, D`，结果等于
    /// `self.hash(a.to_le_bytes() || b.to_le_bytes() || c.to_le_bytes())`。
    /// 编码的两个压缩字分别为 `u64(a) | (u64(b) << 32)` 与 `(12 << 56) | u64(c)`；
    /// 此处 `u64` 转换为零扩展，实例状态不变，不返回输入错误。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == proof::coords(self.state(), a, b, c, C as nat, D as nat),
        no_unwind
    ))]
    pub fn hash_u32x3(&self, a: u32, b: u32, c: u32) -> u64 {
        let m0 = (a as u64) | ((b as u64) << 32);
        let m1 = (12u64 << 56) | (c as u64);

        let (v0, v1, v2, v3) = siprounds::<C>((self.v0, self.v1, self.v2, self.v3 ^ m0));
        let (v0, v1, v2, v3) = siprounds::<C>((v0 ^ m0, v1, v2, v3 ^ m1));
        let (v0, v1, v2, v3) = siprounds::<D>((v0 ^ m1, v1, v2 ^ 0xFF, v3));

        #[cfg(verus_only)]
        proof! { proof::coords_words(self.state(), a, b, c, C as nat, D as nat); }
        v0 ^ v1 ^ v2 ^ v3
    }
}

/// 对任意四字状态应用模块规范的 `R^N`；`N = 0` 时原样返回。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == proof::rounds(state, N as nat),
    no_unwind
))]
fn siprounds<const N: usize>(state: State) -> State {
    let mut result = state;
    let mut count = 0;
    #[cfg_attr(verus_only, verus_spec(
        invariant count <= N, result == proof::rounds(state, count as nat),
        decreases N - count,
    ))]
    while count < N {
        result = sipround_scalar(result);
        #[cfg(verus_only)]
        proof! { proof::rounds_step(state, count as nat); }
        count += 1;
    }
    result
}

/// 对任意四字状态返回模块规范的 `R(v)`；加法均按模 `2^64` 计算。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == proof::round(v),
    no_unwind
))]
const fn sipround_scalar(v: State) -> State {
    let v0 = v.0;
    let v1 = v.1;
    let v2 = v.2;
    let v3 = v.3;

    let v0 = v0.wrapping_add(v1);
    let v1 = v1.rotate_left(13);
    let v1 = v1 ^ v0;
    let v0 = v0.rotate_left(32);
    let v2 = v2.wrapping_add(v3);
    let v3 = v3.rotate_left(16);
    let v3 = v3 ^ v2;
    let v0 = v0.wrapping_add(v3);
    let v3 = v3.rotate_left(21);
    let v3 = v3 ^ v0;
    let v2 = v2.wrapping_add(v1);
    let v1 = v1.rotate_left(17);
    let v1 = v1 ^ v2;
    let v2 = v2.rotate_left(32);

    (v0, v1, v2, v3)
}

/// 从固定字节块读取完整 little-endian 字。
#[inline]
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == proof::packed(block@, 0, 8),
    no_unwind
))]
fn read_word(block: [u8; 8]) -> u64 {
    let result = (block[0] as u64)
        | ((block[1] as u64) << 8)
        | ((block[2] as u64) << 16)
        | ((block[3] as u64) << 24)
        | ((block[4] as u64) << 32)
        | ((block[5] as u64) << 40)
        | ((block[6] as u64) << 48)
        | ((block[7] as u64) << 56);
    #[cfg(verus_only)]
    proof! {
        proof::packed_first(block@, 0);
        reveal_with_fuel(proof::packed, 9);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 官方测试
    pub const VECTORS_SIP64: [[u8; 8]; 64] = [
        [0x31, 0x0e, 0x0e, 0xdd, 0x47, 0xdb, 0x6f, 0x72],
        [0xfd, 0x67, 0xdc, 0x93, 0xc5, 0x39, 0xf8, 0x74],
        [0x5a, 0x4f, 0xa9, 0xd9, 0x09, 0x80, 0x6c, 0x0d],
        [0x2d, 0x7e, 0xfb, 0xd7, 0x96, 0x66, 0x67, 0x85],
        [0xb7, 0x87, 0x71, 0x27, 0xe0, 0x94, 0x27, 0xcf],
        [0x8d, 0xa6, 0x99, 0xcd, 0x64, 0x55, 0x76, 0x18],
        [0xce, 0xe3, 0xfe, 0x58, 0x6e, 0x46, 0xc9, 0xcb],
        [0x37, 0xd1, 0x01, 0x8b, 0xf5, 0x00, 0x02, 0xab],
        [0x62, 0x24, 0x93, 0x9a, 0x79, 0xf5, 0xf5, 0x93],
        [0xb0, 0xe4, 0xa9, 0x0b, 0xdf, 0x82, 0x00, 0x9e],
        [0xf3, 0xb9, 0xdd, 0x94, 0xc5, 0xbb, 0x5d, 0x7a],
        [0xa7, 0xad, 0x6b, 0x22, 0x46, 0x2f, 0xb3, 0xf4],
        [0xfb, 0xe5, 0x0e, 0x86, 0xbc, 0x8f, 0x1e, 0x75],
        [0x90, 0x3d, 0x84, 0xc0, 0x27, 0x56, 0xea, 0x14],
        [0xee, 0xf2, 0x7a, 0x8e, 0x90, 0xca, 0x23, 0xf7],
        [0xe5, 0x45, 0xbe, 0x49, 0x61, 0xca, 0x29, 0xa1],
        [0xdb, 0x9b, 0xc2, 0x57, 0x7f, 0xcc, 0x2a, 0x3f],
        [0x94, 0x47, 0xbe, 0x2c, 0xf5, 0xe9, 0x9a, 0x69],
        [0x9c, 0xd3, 0x8d, 0x96, 0xf0, 0xb3, 0xc1, 0x4b],
        [0xbd, 0x61, 0x79, 0xa7, 0x1d, 0xc9, 0x6d, 0xbb],
        [0x98, 0xee, 0xa2, 0x1a, 0xf2, 0x5c, 0xd6, 0xbe],
        [0xc7, 0x67, 0x3b, 0x2e, 0xb0, 0xcb, 0xf2, 0xd0],
        [0x88, 0x3e, 0xa3, 0xe3, 0x95, 0x67, 0x53, 0x93],
        [0xc8, 0xce, 0x5c, 0xcd, 0x8c, 0x03, 0x0c, 0xa8],
        [0x94, 0xaf, 0x49, 0xf6, 0xc6, 0x50, 0xad, 0xb8],
        [0xea, 0xb8, 0x85, 0x8a, 0xde, 0x92, 0xe1, 0xbc],
        [0xf3, 0x15, 0xbb, 0x5b, 0xb8, 0x35, 0xd8, 0x17],
        [0xad, 0xcf, 0x6b, 0x07, 0x63, 0x61, 0x2e, 0x2f],
        [0xa5, 0xc9, 0x1d, 0xa7, 0xac, 0xaa, 0x4d, 0xde],
        [0x71, 0x65, 0x95, 0x87, 0x66, 0x50, 0xa2, 0xa6],
        [0x28, 0xef, 0x49, 0x5c, 0x53, 0xa3, 0x87, 0xad],
        [0x42, 0xc3, 0x41, 0xd8, 0xfa, 0x92, 0xd8, 0x32],
        [0xce, 0x7c, 0xf2, 0x72, 0x2f, 0x51, 0x27, 0x71],
        [0xe3, 0x78, 0x59, 0xf9, 0x46, 0x23, 0xf3, 0xa7],
        [0x38, 0x12, 0x05, 0xbb, 0x1a, 0xb0, 0xe0, 0x12],
        [0xae, 0x97, 0xa1, 0x0f, 0xd4, 0x34, 0xe0, 0x15],
        [0xb4, 0xa3, 0x15, 0x08, 0xbe, 0xff, 0x4d, 0x31],
        [0x81, 0x39, 0x62, 0x29, 0xf0, 0x90, 0x79, 0x02],
        [0x4d, 0x0c, 0xf4, 0x9e, 0xe5, 0xd4, 0xdc, 0xca],
        [0x5c, 0x73, 0x33, 0x6a, 0x76, 0xd8, 0xbf, 0x9a],
        [0xd0, 0xa7, 0x04, 0x53, 0x6b, 0xa9, 0x3e, 0x0e],
        [0x92, 0x59, 0x58, 0xfc, 0xd6, 0x42, 0x0c, 0xad],
        [0xa9, 0x15, 0xc2, 0x9b, 0xc8, 0x06, 0x73, 0x18],
        [0x95, 0x2b, 0x79, 0xf3, 0xbc, 0x0a, 0xa6, 0xd4],
        [0xf2, 0x1d, 0xf2, 0xe4, 0x1d, 0x45, 0x35, 0xf9],
        [0x87, 0x57, 0x75, 0x19, 0x04, 0x8f, 0x53, 0xa9],
        [0x10, 0xa5, 0x6c, 0xf5, 0xdf, 0xcd, 0x9a, 0xdb],
        [0xeb, 0x75, 0x09, 0x5c, 0xcd, 0x98, 0x6c, 0xd0],
        [0x51, 0xa9, 0xcb, 0x9e, 0xcb, 0xa3, 0x12, 0xe6],
        [0x96, 0xaf, 0xad, 0xfc, 0x2c, 0xe6, 0x66, 0xc7],
        [0x72, 0xfe, 0x52, 0x97, 0x5a, 0x43, 0x64, 0xee],
        [0x5a, 0x16, 0x45, 0xb2, 0x76, 0xd5, 0x92, 0xa1],
        [0xb2, 0x74, 0xcb, 0x8e, 0xbf, 0x87, 0x87, 0x0a],
        [0x6f, 0x9b, 0xb4, 0x20, 0x3d, 0xe7, 0xb3, 0x81],
        [0xea, 0xec, 0xb2, 0xa3, 0x0b, 0x22, 0xa8, 0x7f],
        [0x99, 0x24, 0xa4, 0x3c, 0xc1, 0x31, 0x57, 0x24],
        [0xbd, 0x83, 0x8d, 0x3a, 0xaf, 0xbf, 0x8d, 0xb7],
        [0x0b, 0x1a, 0x2a, 0x32, 0x65, 0xd5, 0x1a, 0xea],
        [0x13, 0x50, 0x79, 0xa3, 0x23, 0x1c, 0xe6, 0x60],
        [0x93, 0x2b, 0x28, 0x46, 0xe4, 0xd7, 0x06, 0x66],
        [0xe1, 0x91, 0x5f, 0x5c, 0xb1, 0xec, 0xa4, 0x6c],
        [0xf3, 0x25, 0x96, 0x5c, 0xa1, 0x6d, 0x62, 0x9f],
        [0x57, 0x5f, 0xf2, 0x8e, 0x60, 0x38, 0x1b, 0xe5],
        [0x72, 0x45, 0x06, 0xeb, 0x4c, 0x32, 0x8a, 0x95],
    ];

    const fn make_seed() -> (u64, u64) {
        (
            u64::from_le_bytes([0, 1, 2, 3, 4, 5, 6, 7]),
            u64::from_le_bytes([8, 9, 10, 11, 12, 13, 14, 15]),
        )
    }

    #[test]
    fn spec_conformance_sip64() {
        let hasher = SipHash::<2, 4>::new(make_seed());
        let msg: Vec<u8> = (0u8..=63).collect();
        for (i, expected_bytes) in VECTORS_SIP64.iter().enumerate() {
            let actual = hasher.hash(msg.get(..i).unwrap_or(&[]));
            let expected = u64::from_le_bytes(*expected_bytes);
            assert_eq!(actual, expected, "mismatch at message length {i}");
        }
    }

    #[test]
    fn different_seed_different_result() {
        let h1 = SipHash::<2, 4>::new((0, 0));
        let h2 = SipHash::<2, 4>::new((1, 0));
        assert_ne!(h1.hash(b"hello"), h2.hash(b"hello"));
    }

    #[test]
    fn different_cd_yield_different_results() {
        let msg = b"some message to hash";
        let h1 = SipHash::<1, 3>::new(make_seed());
        let h2 = SipHash::<2, 4>::new(make_seed());
        assert_ne!(h1.hash(msg), h2.hash(msg));
    }

    #[test]
    fn sipround_known_intermediate() {
        let (v0, v1, v2, v3) = sipround_scalar((1, 0, 0, 0));
        assert_eq!(v0, 0x0000_0001_0000_0000);
        assert_eq!(v1, 0x0000_0000_0002_0001);
        assert_eq!(v2, 0x0000_0001_0000_0000);
        assert_eq!(v3, 0x0000_0001_0000_0000);

        let a = sipround_scalar((1, 2, 3, 4));
        let b = sipround_scalar((1, 2, 3, 4));
        assert_eq!(a, b);
        let c = sipround_scalar((4, 3, 2, 1));
        assert_ne!(a, c, "different inputs must produce different outputs");
        assert_ne!(a, (1, 2, 3, 4), "sipround must transform its input");
    }

    #[test]
    fn zero_rounds_semantics() {
        let h00 = SipHash::<0, 0>::new(make_seed());
        let h01 = SipHash::<0, 1>::new(make_seed());
        let h10 = SipHash::<1, 0>::new(make_seed());

        let msg = b"zero-round edge case";
        let v00 = h00.hash(msg);
        let v01 = h01.hash(msg);
        let v10 = h10.hash(msg);

        assert_ne!(v00, v01, "C=0,D=0 should differ from C=0,D=1");
        assert_ne!(v00, v10, "C=0,D=0 should differ from C=1,D=0");
        assert_ne!(v01, v10, "C=0,D=1 should differ from C=1,D=0");

        assert_eq!(h01.hash(msg), v01, "C=0,D=1 should be deterministic");
        assert_eq!(h10.hash(msg), v10, "C=1,D=0 should be deterministic");
    }

    #[test]
    fn hash_u32x3_matches_12byte_slice() {
        let hasher = SipHash::<1, 3>::new(make_seed());
        let a = 0x0123_4567u32;
        let b = 0x89AB_CDEFu32;
        let c = 0x7654_3210u32;
        let msg = [a.to_le_bytes(), b.to_le_bytes(), c.to_le_bytes()].concat();

        assert_eq!(hasher.hash_u32x3(a, b, c), hasher.hash(&msg));
    }
}
