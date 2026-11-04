// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-FileCopyrightText: Copyright (c) 1998-2020 Matt Pharr, Wenzel Jakob, and Greg Humphreys
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
//! Binary Sobol S8 的 GPU 契约与已知联合分层代价。
//! CPU 参考直接构造 u64 Morton、前缀双射与 Sobol 方向数，独立于 shader 的双 word
//! 移位和位折叠；FastOwen 来自 PBRT-v4，许可见 LICENSES/pbrt-Apache-2.0.txt。
//! 每项测试独占环境，回读并 finish 后断言；不把有限数学性质解释为渲染质量保证。

use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
// pixel.x/y、sample、R、seed、pair_domain、scalar_domain、保留字。
type Input = [u32; 8];
type Row = [u32; 4];

macro_rules! shader {
    ($entry:literal) => {
        Shader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/", $entry, ".spv")),
            entry: $entry,
            workgroup: [64, 1, 1],
        }
    };
}

const BITS: Shader<'static> = shader!("binary_sobol_bits_main");
const DIRECTED: Shader<'static> = shader!("binary_sobol_3d_main");
const FLOATS: Shader<'static> = shader!("binary_sobol_float_main");

#[test]
fn compiled_sampling_is_loop_free_without_calls_or_wide_arithmetic() {
    for shader in [BITS, DIRECTED, FLOATS] {
        let words: Vec<_> = shader
            .spirv
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| u32::from_le_bytes(*bytes))
            .collect();
        assert_eq!(words[0], 0x0723_0203);
        let mut offset = 5;
        while offset < words.len() {
            let size = (words[offset] >> 16) as usize;
            let opcode = words[offset] & 0xffff;
            assert!(size > 0 && offset + size <= words.len());
            assert!(
                !matches!(opcode, 57 | 246),
                "{} 包含函数调用或循环",
                shader.entry
            );
            if matches!(opcode, 21 | 22) {
                assert_eq!(words[offset + 2], 32, "{} 的算术位宽", shader.entry);
            }
            if opcode == 71 && words[offset + 2] == 33 {
                assert!(words[offset + 3] <= 1, "{} 包含额外采样资源", shader.entry);
            }
            offset += size;
        }
    }
}

fn execute(batches: &[(Shader<'static>, &[Input])]) -> Result<Vec<Vec<Row>>> {
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let result: Result<Vec<Vec<Row>>> = batches
        .iter()
        .map(|(shader, input)| {
            let bytes: Vec<_> = input
                .iter()
                .flatten()
                .flat_map(|word| word.to_le_bytes())
                .collect();
            let output = gpu.compute(
                *shader,
                &bytes,
                input.len() * 16,
                [(input.len() as u32).div_ceil(64), 1, 1],
            )?;
            Ok(output
                .as_chunks::<16>()
                .0
                .iter()
                .map(|row| {
                    std::array::from_fn(|i| {
                        u32::from_le_bytes(row[i * 4..i * 4 + 4].try_into().unwrap())
                    })
                })
                .collect())
        })
        .collect();
    let cleanup = gpu.finish();
    let output = result?;
    cleanup?;
    for (batch, (_, input)) in output.iter().zip(batches) {
        assert_eq!(batch.len(), input.len());
        for (index, row) in batch.iter().enumerate() {
            assert_eq!(row[3], index as u32 + 1, "GPU 执行标识缺失");
        }
    }
    Ok(output)
}

fn avalanche(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

// H 是低位三角双射；反转前后得到高位前缀双射 P，不执行 GPU 的逐 word 流程。
fn prefix_permutation(value: u32, seed: u32) -> u32 {
    let mut value = value.reverse_bits();
    value ^= value.wrapping_mul(0x3d20_adea);
    value = value.wrapping_add(seed);
    value = value.wrapping_mul((seed >> 16) | 1);
    value ^= value.wrapping_mul(0x0552_6c56);
    value ^= value.wrapping_mul(0x53a2_2864);
    value.reverse_bits()
}

fn index(input: &Input, domain: u32) -> (u64, u32) {
    let [x, y, sample, r, seed, _, _, _] = *input;
    let mut source = u64::from(sample);
    for position in 0..r {
        source |= u64::from((x >> position) & 1) << (8 + position * 2);
        source |= u64::from((y >> position) & 1) << (9 + position * 2);
    }
    let count = 2 * r + 8;
    let key = avalanche(seed ^ domain);
    let mask = (1u64 << count) - 1;
    let permuted = if count <= 32 {
        u64::from(prefix_permutation(source as u32, key))
    } else {
        let original_high = (source >> 32) as u32;
        let high = u64::from(prefix_permutation(original_high, key));
        let low_key = avalanche(original_high ^ avalanche(key ^ 0x632b_e59b));
        high << 32 | u64::from(prefix_permutation(source as u32, low_key))
    };
    (permuted & mask, key)
}

fn pair(input: &Input) -> [u32; 2] {
    let (index, key) = index(input, input[5]);
    let mut second = 0;
    let mut direction = 1u32 << 31;
    for bit in 0..32 {
        if ((index as u32) ^ (index >> 32) as u32) & (1 << bit) != 0 {
            second ^= direction;
        }
        direction ^= direction >> 1;
    }
    [
        prefix_permutation((index as u32).reverse_bits(), avalanche(key ^ 0xa511_e9b3)),
        prefix_permutation(second, avalanche(key ^ 0x63d8_3595)),
    ]
}

fn scalar(input: &Input, deferred: bool) -> u32 {
    let (index, key) = index(input, input[6]);
    // 在整数定义上交换低两位，只改变每个对齐四点块内部的排列。
    let index = if deferred {
        (index & !3) | ((index & 1) << 1) | ((index & 2) >> 1)
    } else {
        index
    };
    prefix_permutation((index as u32).reverse_bits(), avalanche(key ^ 0xa511_e9b3))
}

fn net_inputs() -> Vec<Input> {
    [
        (0, 0, 0, 1),
        (12, u32::MAX, 2, 3),
        (13, 17, 4, 5),
        (16, 0x2468_1357, u32::MAX, u32::MAX - 1),
    ]
    .into_iter()
    .flat_map(|(r, seed, pair, scalar)| {
        (0..256).map(move |sample| {
            let maximum = (1 << r) - 1;
            [maximum, maximum / 2, sample, r, seed, pair, scalar, 0]
        })
    })
    .collect()
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn integer_results_match_independent_wide_index_reference() -> Result {
    let mut inputs = Vec::new();
    for r in [0, 12, 13, 16] {
        let maximum = (1 << r) - 1;
        for [x, y] in [[0, 0], [maximum, maximum], [maximum / 2, maximum]] {
            for sample in [0, 1, 2, 3, 127, 128, 254, 255] {
                for [seed, pair, scalar] in
                    [[0, 0, 1], [u32::MAX, u32::MAX, 0], [0x89ab_cdef, 71, 93]]
                {
                    inputs.push([x, y, sample, r, seed, pair, scalar, 0]);
                }
            }
        }
    }
    let output = execute(&[(BITS, &inputs), (DIRECTED, &inputs)])?;
    for (batch, deferred) in output.iter().zip([false, true]) {
        for (row, input) in batch.iter().zip(&inputs) {
            let [x, y] = pair(input);
            assert_eq!(row[..3], [x, y, scalar(input, deferred)], "{input:?}");
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn ordinary_scalar_is_the_same_domain_pair_x() -> Result {
    let mut inputs = net_inputs();
    for input in &mut inputs {
        input[6] = input[5];
    }
    let output = execute(&[(BITS, &inputs)])?;
    for row in &output[0] {
        assert_eq!(row[0], row[2]);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn float_samples_are_quantized_to_the_half_open_unit_interval() -> Result {
    let inputs = net_inputs();
    let output = execute(&[(DIRECTED, &inputs), (FLOATS, &inputs)])?;
    for (bits, floats) in output[0].iter().zip(&output[1]) {
        for channel in 0..3 {
            let expected = (bits[channel] >> 8) as f32 * (1.0 / 16_777_216.0);
            let actual = f32::from_bits(floats[channel]);
            assert_eq!(actual, expected);
            assert!((0.0..1.0).contains(&actual));
            assert_eq!((actual * 16_777_216.0).fract(), 0.0);
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn every_aligned_power_of_two_pair_block_is_a_dyadic_net() -> Result {
    let inputs = net_inputs();
    let output = execute(&[(DIRECTED, &inputs)])?;
    for (configuration, rows) in output[0].as_chunks::<256>().0.iter().enumerate() {
        for exponent in 1..=8 {
            let count = 1usize << exponent;
            for (block, rows) in rows.chunks_exact(count).enumerate() {
                // (0,m,2)-net 的定义：任意 xbits+ybits=m 的矩形划分每格恰有一点。
                for xbits in 0..=exponent {
                    let ybits = exponent - xbits;
                    let mut bins = vec![0u32; count];
                    for row in rows {
                        let x = if xbits == 0 {
                            0
                        } else {
                            row[0] >> (32 - xbits)
                        };
                        let y = if ybits == 0 {
                            0
                        } else {
                            row[1] >> (32 - ybits)
                        };
                        bins[((x << ybits) | y) as usize] += 1;
                    }
                    assert!(
                        bins.into_iter().all(|count| count == 1),
                        "配置{configuration} m={exponent} block={block} xbits={xbits}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn directed_four_sample_triples_form_a_three_dimensional_dyadic_net() -> Result {
    let inputs = net_inputs();
    let output = execute(&[(DIRECTED, &inputs)])?;
    for rows in output[0].as_chunks::<4>().0 {
        for xbits in 0..=2 {
            for ybits in 0..=2 - xbits {
                let zbits = 2 - xbits - ybits;
                let mut bins = [0u32; 4];
                for row in rows {
                    let cell = [xbits, ybits, zbits].into_iter().enumerate().fold(
                        0,
                        |cell, (axis, bits)| {
                            (cell << bits)
                                | if bits == 0 {
                                    0
                                } else {
                                    row[axis] >> (32 - bits)
                                }
                        },
                    );
                    bins[cell as usize] += 1;
                }
                assert_eq!(bins, [1; 4]);
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn deferred_scalar_explicitly_loses_two_sample_half_stratification() -> Result {
    let inputs = net_inputs();
    let output = execute(&[(DIRECTED, &inputs)])?;
    for rows in output[0].as_chunks::<2>().0 {
        assert_eq!(rows[0][2] >> 31, rows[1][2] >> 31);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn deferred_scalar_preserves_every_aligned_block_set_from_four_samples() -> Result {
    let inputs = net_inputs();
    let output = execute(&[(BITS, &inputs), (DIRECTED, &inputs)])?;
    for exponent in 2..=8 {
        let count = 1usize << exponent;
        for (ordinary, directed) in output[0]
            .chunks_exact(count)
            .zip(output[1].chunks_exact(count))
        {
            let mut seen = vec![false; count];
            for row in directed {
                let index = ordinary
                    .iter()
                    .position(|value| value[2] == row[2])
                    .unwrap();
                assert!(!seen[index], "块内标量样本重复");
                seen[index] = true;
            }
            assert!(seen.into_iter().all(|seen| seen));
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn ordinary_domains_keep_the_known_two_sample_half_xor_defect() -> Result {
    let inputs = net_inputs();
    let output = execute(&[(BITS, &inputs)])?;
    for rows in output[0].as_chunks::<2>().0 {
        assert_eq!(
            (rows[0][0] ^ rows[0][2]) >> 31,
            (rows[1][0] ^ rows[1][2]) >> 31
        );
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn directed_three_dimensional_half_parity_remains_a_known_joint_limitation() -> Result {
    let inputs = net_inputs();
    let output = execute(&[(DIRECTED, &inputs)])?;
    // 四点可同时填满三个二维半区投影，但三个半区位仍有固定 parity；
    // 这与四点 (0,2,3)-net 相容，但该非线性联合函数在四点内仍重复。
    for rows in output[0].as_chunks::<4>().0 {
        let parity = (rows[0][0] ^ rows[0][1] ^ rows[0][2]) >> 31;
        for row in rows {
            assert_eq!((row[0] ^ row[1] ^ row[2]) >> 31, parity);
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn directed_eight_sample_triples_have_a_known_dyadic_net_counterexample() -> Result {
    let inputs: Vec<_> = (0..8).map(|sample| [0, 0, sample, 0, 0, 2, 1, 0]).collect();
    let output = execute(&[(DIRECTED, &inputs)])?;
    let mut cells = [0u32; 8];
    for row in &output[0] {
        cells[((row[1] >> 30) * 2 + (row[2] >> 31)) as usize] += 1;
    }
    assert_eq!(cells.iter().filter(|&&count| count != 0).count(), 4);
    assert!(cells.into_iter().all(|count| matches!(count, 0 | 2)));
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn wide_morton_pixel_bits_remain_observable() -> Result {
    let mut inputs = Vec::new();
    for [r, x, y] in [[13, 4096, 0], [13, 0, 4096], [16, 32768, 32768]] {
        for [x, y] in [[0, 0], [x, y]] {
            inputs.extend((0..256).map(|sample| [x, y, sample, r, 17, 9, 8, 0]));
        }
    }
    let output = execute(&[(BITS, &inputs)])?;
    for rows in output[0].as_chunks::<512>().0 {
        assert!(
            rows[..256]
                .iter()
                .zip(&rows[256..])
                .any(|(a, b)| a[..2] != b[..2])
        );
    }
    Ok(())
}
