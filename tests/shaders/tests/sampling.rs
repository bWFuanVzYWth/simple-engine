// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
//! Z Sobol 与 RR 的真实 GPU 行为；每项独占环境，回读并 finish 后才执行 CPU 断言。
//! 固定整数向量由独立 u64/top-down/scalar-direction 参考生成，
//! 来源 src/shader_tests/reference.rs，未用 GPU 输出回填。FastOwen 许可见 LICENSES。

use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
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

const BITS: Shader<'static> = shader!("z_sobol_bits_main");
const FLOATS: Shader<'static> = shader!("z_sobol_float_main");
const S8: Shader<'static> = shader!("z_sobol_s8_main");
const PROBABILITY: Shader<'static> = shader!("roulette_probability_main");
const MEAN: Shader<'static> = shader!("roulette_mean_main");
const ETA: Shader<'static> = shader!("roulette_eta_main");

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
            assert_eq!(row[1], index as u32 + 1, "GPU 执行标识缺失");
        }
    }
    Ok(output)
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn variable_s_matches_independent_frozen_integer_vectors() -> Result {
    let cases: &[(Input, [u32; 2])] = &[
        ([0, 0, 0, 0, 0, 0, 0, 0], [0x04eb_c721, 0xb8fe_aac8]),
        (
            [0, 0, 1, 0, 1, 0x1234_5678, 7, 0],
            [0xd482_76a3, 0xab3a_0e6a],
        ),
        (
            [4095, 2057, 255, 12, 8, 0x89ab_cdef, 17, 0],
            [0xefef_5988, 0x6b9e_e4aa],
        ),
        (
            [8191, 8123, 127, 13, 7, 17, 511, 0],
            [0x8425_188e, 0x493a_1fe7],
        ),
        (
            [8191, 8123, 255, 13, 8, 17, 511, 0],
            [0x3591_579d, 0x3ffd_7f73],
        ),
        (
            [65535, 32769, 0, 16, 0, u32::MAX, 63, 0],
            [0x2c21_700f, 0xc2a3_bc54],
        ),
        (
            [65535, 32769, 1, 16, 1, u32::MAX, 64, 0],
            [0x925b_0826, 0x0102_b984],
        ),
        (
            [65535, 65535, 524287, 16, 19, 31, 1, 0],
            [0xc277_07b7, 0x1027_8449],
        ),
        (
            [65535, 65535, 1048575, 16, 20, 31, 1, 0],
            [0xaa5d_b502, 0x97f2_f4b7],
        ),
        (
            [0, 0, 1048575, 0, 20, 0x1234_5678, u32::MAX, 0],
            [0x17ff_1f6e, 0xe082_90b5],
        ),
        (
            [1, 0, 1048574, 1, 20, 0, 511, 0],
            [0x0d8b_782d, 0x97bd_f9e3],
        ),
        (
            [65535, 65535, 255, 16, 8, 123, 5, 0],
            [0x4033_e095, 0xa82f_c567],
        ),
    ];
    let input: Vec<_> = cases.iter().map(|(input, _)| *input).collect();
    let output = execute(&[(BITS, &input)])?;
    for (row, (_, expected)) in output[0].iter().zip(cases) {
        assert_eq!([row[0], row[2]], *expected);
        assert_eq!(row[3], expected[0], "1D 是同域 2D 的 X 分量");
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn complete_nets_fill_each_dyadic_cell_once() -> Result {
    let configurations = [(8, 8), (12, 9), (16, 12)];
    let mut input = Vec::new();
    for (r, s) in configurations {
        input.extend((0..1 << s).map(|sample| [129, 17, sample, r, s, 123, 7, 0]));
    }
    let output = execute(&[(BITS, &input)])?;
    let mut offset = 0;
    for (_, s) in configurations {
        let count = 1usize << s;
        let rows = &output[0][offset..offset + count];
        // (0,S,2)-net 的独立定义：任意 k+(S-k) 位矩形划分都应每格一个样本。
        for xbits in 0..=s {
            let ybits = s - xbits;
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
                    row[2] >> (32 - ybits)
                };
                bins[((x << ybits) | y) as usize] += 1;
            }
            assert!(bins.iter().all(|&count| count == 1), "S={s} xbits={xbits}");
        }
        offset += count;
    }
    Ok(())
}

fn s8_inputs() -> Vec<Input> {
    [0, 12, 13, 16]
        .into_iter()
        .flat_map(|r| {
            (0..256).map(move |sample| [(1 << r) - 1, 0, sample, r, 8, 0x2468_1357, 71, 0])
        })
        .collect()
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn s8_specialization_preserves_the_generic_sequence() -> Result {
    let input = s8_inputs();
    let output = execute(&[(BITS, &input), (S8, &input)])?;
    assert_eq!(output[0], output[1]);
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn float_samples_are_quantized_to_the_half_open_unit_interval() -> Result {
    let input = s8_inputs();
    let output = execute(&[(BITS, &input), (FLOATS, &input)])?;
    for (bits, floats) in output[0].iter().zip(&output[1]) {
        for channel in [0, 2, 3] {
            let expected = (bits[channel] >> 8) as f32 * (1.0 / 16_777_216.0);
            let actual = f32::from_bits(floats[channel]);
            assert_eq!(actual.to_bits(), expected.to_bits());
            assert!((0.0..1.0).contains(&actual));
            assert_eq!((actual * 16_777_216.0).fract(), 0.0);
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn separate_domains_have_distinct_joint_strata() -> Result {
    let inputs = [0, 1].map(|domain| {
        (0..64)
            .flat_map(|x| (0..256).map(move |sample| [x, 17, sample, 6, 8, 0x1234_5678, domain, 0]))
            .collect::<Vec<_>>()
    });
    let output = execute(&[(BITS, &inputs[0]), (BITS, &inputs[1])])?;
    let mut bins = [0u32; 16];
    for (a, b) in output[0].iter().zip(&output[1]) {
        bins[((a[0] >> 30) * 4 + (b[0] >> 30)) as usize] += 1;
    }
    // 固定代表域的联合分布检查；并非所有 domain 数学独立性的证明。
    assert!(
        bins.into_iter().all(|count| (768..=1280).contains(&count)),
        "{bins:?}"
    );
    Ok(())
}

fn rr_input(beta: [f32; 3], eta: f32, extra: [u32; 4]) -> Input {
    [
        beta[0].to_bits(),
        beta[1].to_bits(),
        beta[2].to_bits(),
        eta.to_bits(),
        extra[0],
        extra[1],
        extra[2],
        extra[3],
    ]
}

const RR_CASES: [([f32; 3], f32); 8] = [
    ([0.0; 3], 1.0),
    ([0.015625, 0.0078125, 0.0], 1.0),
    ([0.25, 0.125, 0.0625], 1.0),
    ([0.25, 0.125, 0.0625], 0.5),
    ([0.25, 0.125, 0.0625], 4.0),
    ([2.0, 1.0, 0.5], 1.0),
    ([0.0, 0.5, 0.0], 1.0),
    ([0.0, 0.0, 0.5], 0.5),
];

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn roulette_probability_has_no_survival_floor_or_upper_cap() -> Result {
    let input: Vec<_> = RR_CASES
        .iter()
        .map(|&(beta, eta)| rr_input(beta, eta, [1.0f32.to_bits(), 0, 2, 2]))
        .collect();
    let output = execute(&[(PROBABILITY, &input)])?;
    for (row, (beta, eta)) in output[0].iter().zip(RR_CASES) {
        let expected =
            (beta.into_iter().map(f64::from).fold(0.0, f64::max) * f64::from(eta)).clamp(0.0, 1.0);
        assert_eq!(f64::from(f32::from_bits(row[0])), expected);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn roulette_preserves_the_finite_uniform_integral() -> Result {
    let input: Vec<_> = RR_CASES
        .iter()
        .map(|&(beta, eta)| rr_input(beta, eta, [4096, 0, 0, 0]))
        .collect();
    let output = execute(&[(MEAN, &input)])?;
    for (row, (expected, _)) in output[0].iter().zip(RR_CASES) {
        let actual = [row[0], row[2], row[3]].map(f32::from_bits);
        assert_eq!(actual, expected, "Bernoulli 存活重权的有限积分应等于原贡献");
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn dielectric_eta_compensation_preserves_the_roulette_metric() -> Result {
    let cases: Vec<_> = [0.5, 1.0, 1.33, 1.5, 2.0]
        .into_iter()
        .flat_map(|eta| [[1.0; 3], [0.125, 0.25, 0.5]].map(move |beta| (beta, eta)))
        .collect();
    let input: Vec<_> = cases
        .iter()
        .map(|&(beta, eta)| rr_input(beta, eta, [0; 4]))
        .collect();
    let output = execute(&[(ETA, &input)])?;
    for (row, (beta, _)) in output[0].iter().zip(cases) {
        let expected = beta.into_iter().fold(0.0f32, f32::max);
        for channel in [0, 2] {
            assert!((f32::from_bits(row[channel]) - expected).abs() <= 2e-6);
        }
        assert_eq!(f32::from_bits(row[3]), 1.0, "反射事件不改变介质 eta scale");
    }
    Ok(())
}
