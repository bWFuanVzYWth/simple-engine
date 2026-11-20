//! 既有镜头种子的 PCG XSL RR 128/64 序列。SeedSequence 的 32 位输入兼容性见随附许可。
const MULT: u128 = 47026247687942121848144207491837523525;
pub(crate) struct Pcg {
    state: u128,
    increment: u128,
}
impl Pcg {
    pub(crate) fn new(seed: u32) -> Self {
        let mut key = 0x43b0d7e5_u32;
        let mut mix_word = |v: u32| {
            let x = v ^ key;
            key = key.wrapping_mul(0x931e8875);
            let x = x.wrapping_mul(key);
            x ^ (x >> 16)
        };
        let mut pool = [seed, 0, 0, 0].map(&mut mix_word);
        for src in 0..4 {
            for dst in 0..4 {
                if src != dst {
                    let x = pool[dst]
                        .wrapping_mul(0xca01f9dd)
                        .wrapping_sub(mix_word(pool[src]).wrapping_mul(0x4973f715));
                    pool[dst] = x ^ (x >> 16);
                }
            }
        }
        let mut key = 0x8b51f9dd_u32;
        let words: [u32; 8] = std::array::from_fn(|i| {
            let x = pool[i % 4] ^ key;
            key = key.wrapping_mul(0x58f38ded);
            let x = x.wrapping_mul(key);
            x ^ (x >> 16)
        });
        let wide = |i: usize| {
            u128::from(words[i]) << 64
                | u128::from(words[i + 1]) << 96
                | u128::from(words[i + 2])
                | u128::from(words[i + 3]) << 32
        };
        let increment = (wide(4) << 1) | 1;
        Self {
            state: increment
                .wrapping_add(wide(0))
                .wrapping_mul(MULT)
                .wrapping_add(increment),
            increment,
        }
    }
    pub(crate) fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        self.state = self.state.wrapping_mul(MULT).wrapping_add(self.increment);
        let high = (self.state >> 64) as u64;
        let bits = (high ^ self.state as u64).rotate_right((high >> 58) as u32);
        lo + (hi - lo) * ((bits >> 11) as f64 * (1.0 / (1_u64 << 53) as f64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archived_seed_reproduces_first_uniform_samples() {
        let mut rng = Pcg::new(20261120);
        let sequence = std::array::from_fn(|_| rng.uniform(0., 1.));
        assert_eq!(
            sequence,
            [0.7370349296243083, 0.1498352523107278, 0.8334288840215224]
        );
    }
}
