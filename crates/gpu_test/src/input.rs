//! 可按绝对索引重放的属性输入协议。
//!
//! `chacha20-64-v1` 固定 ChaCha20 的 256-bit seed、64-bit stream 与 64-bit block counter。
//! 同一索引的输出不受分块、执行顺序或并行度影响。参考向量和位映射属于协议契约；
//! 更换算法须另起协议名，保留旧 seed 的解释与重放路径。
//!
//! 来源：[rand_chacha 固定版本源码](https://docs.rs/rand_chacha/0.10.0/src/rand_chacha/chacha.rs.html)、
//! [ChaCha20 参考向量](https://www.rfc-editor.org/rfc/rfc8439#appendix-A.1)。

use rand_chacha::ChaCha20Rng;
use rand_core::{Rng, SeedableRng};

/// 未指定输入种子时采用的完整 256-bit seed。
pub const DEFAULT_SEED: [u8; 32] = [0; 32];
/// 记录与重放使用的稳定协议名。
pub const RNG_PROTOCOL: &str = "chacha20-64-v1";

/// seed 文本不满足完整字节编码要求。
#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    /// 长度或字符不符合 64 位十六进制 ASCII 编码。
    #[error("种子须为 64 个十六进制 ASCII 字符")]
    Invalid,
}

/// 按书写顺序将 64 个十六进制字符解释为 32 字节；接受大小写。
pub fn parse_seed(hex: &str) -> Result<[u8; 32], SeedError> {
    if hex.len() != 64 {
        return Err(SeedError::Invalid);
    }
    let nibble = |byte| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(SeedError::Invalid),
    };
    let mut seed = [0; 32];
    for (pair, output) in hex.as_bytes().as_chunks::<2>().0.iter().zip(&mut seed) {
        *output = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(seed)
}

/// 独占 ChaCha20 状态的随机访问输入生成器；每个样本占一个 64-byte block。
pub struct PropertyRng {
    rng: ChaCha20Rng,
}

impl PropertyRng {
    /// 用完整 seed 和属性专用 stream 初始化，不从窄整数扩展种子。
    pub fn new(seed: [u8; 32], stream: u64) -> Self {
        let mut rng = ChaCha20Rng::from_seed(seed);
        rng.set_stream(stream);
        Self { rng }
    }

    /// 定位到 `index * 16` 的 word position，返回该 block 的 16 个 little-endian `u32`。
    pub fn words(&mut self, index: u64) -> [u32; 16] {
        let position = u128::from(index) * 16;
        if self.rng.get_word_pos() != position {
            self.rng.set_word_pos(position);
        }
        std::array::from_fn(|_| self.rng.next_u32())
    }
}

/// 高 24-bit 映射到 [0,1)，不依赖 rand 的分布实现。
pub const fn unit_f32(word: u32) -> f32 {
    (word >> 8) as f32 * (1.0 / 16_777_216.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_hex_preserves_byte_order() {
        assert_eq!(
            parse_seed("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f").unwrap(),
            std::array::from_fn(|i| i as u8)
        );
    }

    // ChaCha20 参考向量，字节按 little-endian 解释为 u32。
    // https://www.rfc-editor.org/rfc/rfc8439#appendix-A.1
    #[test]
    fn first_block_matches_chacha20_reference() {
        assert_eq!(
            PropertyRng::new(DEFAULT_SEED, 0).words(0),
            [
                0xade0b876, 0x903df1a0, 0xe56a5d40, 0x28bd8653, 0xb819d2bd, 0x1aed8da0, 0xccef36a8,
                0xc70d778b, 0x7c5941da, 0x8d485751, 0x3fe02477, 0x374ad8b8, 0xf4b8436a, 0x1ca11815,
                0x69b687c3, 0x8665eeb2,
            ]
        );
    }

    #[test]
    fn sample_index_selects_chacha20_block() {
        assert_eq!(
            PropertyRng::new(DEFAULT_SEED, 0).words(1),
            [
                0xbee7079f, 0x7a385155, 0x7c97ba98, 0x0d082d73, 0xa0290fcb, 0x6965e348, 0x3e53c612,
                0xed7aee32, 0x7621b729, 0x434ee69c, 0xb03371d5, 0xd539d874, 0x281fed31, 0x45fb0a51,
                0x1f0ae1ac, 0x6f4d794b,
            ]
        );
    }

    #[test]
    fn stream_matches_chacha20_nonce() {
        assert_eq!(
            PropertyRng::new(DEFAULT_SEED, 2 << 56).words(0),
            [
                0x374dc6c2, 0x3736d58c, 0xb904e24a, 0xcd3f93ef, 0x88228b1a, 0x96a4dfb3, 0x5b76ab72,
                0xc727ee54, 0x0e0e978a, 0xf3145c95, 0x1b748ea8, 0xf786c297, 0x99c28f5f, 0x628314e8,
                0x398a19fa, 0x6ded1b53,
            ]
        );
    }

    #[test]
    fn partitioning_preserves_samples() {
        let mut rng = PropertyRng::new(DEFAULT_SEED, 7);
        let expected: Vec<_> = (0..37).map(|index| rng.words(index)).collect();
        let mut actual = Vec::new();
        for start in (0..37).step_by(7) {
            let mut rng = PropertyRng::new(DEFAULT_SEED, 7);
            actual.extend((start..(start + 7).min(37)).map(|index| rng.words(index)));
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn reordering_preserves_samples() {
        let indices = [32, 0, u64::MAX, 17, 1 << 40, 1, 32];
        let expected = indices.map(|index| PropertyRng::new(DEFAULT_SEED, 11).words(index));
        let mut rng = PropertyRng::new(DEFAULT_SEED, 11);
        assert_eq!(indices.map(|index| rng.words(index)), expected);
    }

    #[test]
    fn unit_mapping_uses_high_24_bits() {
        assert_eq!(
            [0, 0x80000000, 0x800000ff, u32::MAX].map(|word| unit_f32(word).to_bits()),
            [0, 0x3f000000, 0x3f000000, 0x3f7fffff]
        );
    }
}
