// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only
//! primeDRT 的真实 GPU 回归与独立 f64 灰阶约束。

use simple_engine::{
    display::{PrimeDrtParameters, PrimeDrtSettings},
    gpu_test::{Capability, Gpu, Shader},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const PRODUCTION: Shader<'static> = Shader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/display_main.spv")),
    entry: "display_main",
    workgroup: [64, 1, 1],
};
struct Case {
    color: [f32; 3],
    parameters: PrimeDrtParameters,
    headroom: f32,
    // 0=encoded BT709，1=encoded Rec2020，2=linear BT709。
    mode: u32,
}

fn run(cases: &[Case], shaders: &[Shader<'static>]) -> Result<Vec<Vec<[u32; 4]>>> {
    let mut words = Vec::with_capacity(cases.len() * 16);
    for case in cases {
        words.extend(case.color.into_iter().chain([0.0]).map(f32::to_bits));
        words.extend(case.parameters.to_array().map(f32::to_bits));
        words.extend([case.headroom.to_bits(), case.mode, 0, 0]);
    }
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let result: Result<Vec<Vec<[u32; 4]>>> = shaders
        .iter()
        .map(|shader| {
            let bytes = gpu.compute(
                *shader,
                &bytes,
                cases.len() * 16,
                [(cases.len() as u32).div_ceil(64), 1, 1],
            )?;
            Ok(bytes
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
    for rows in &output {
        assert_eq!(rows.len(), cases.len());
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row[3], index as u32 + 1, "GPU 执行标识缺失");
        }
    }
    Ok(output)
}

fn close(actual: f32, expected: f64) {
    assert!(
        actual.is_finite()
            && (f64::from(actual) - expected).abs() <= 8e-6 * expected.abs().max(1.0),
        "GPU={actual} reference={expected}"
    );
}

fn encode(value: f64) -> f64 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[test]
#[ignore = "需要 Vulkan compute；编码颜色有限且受校准峰值约束"]
fn encoded_color_stays_within_calibrated_headroom() -> Result {
    let mut colors = vec![
        [0.0; 3],
        [-1.0, 0.1, 0.3],
        [1e30, 1e-10, 1e10],
        [f32::NAN, 1.0, 0.0],
        [f32::INFINITY; 3],
    ];
    // 饱和原色、近中性及 toe／压缩起点两侧，覆盖负输入和 outset 补偿路径。
    for level in [
        1e-9,
        0.0031308,
        0.18 - 1e-6,
        0.18,
        0.18 + 1e-6,
        1.0,
        16.0,
        46.08,
        1e10,
    ] {
        for primary in [
            [1.0; 3],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.00001, 1.0],
            [1.0, 0.01, 0.9],
        ] {
            colors.push(primary.map(|value| value * level));
        }
    }
    let mut cases = Vec::new();
    for headroom in [1.0, 2.0, 16.0, 10000.0] {
        for exposure in [1.0 / 256.0, 1.0, 256.0] {
            for hue in [0.0, 0.75, 1.0] {
                for saturation in [0.0, 0.08, 0.5] {
                    let parameters = PrimeDrtSettings::new(exposure, hue, saturation, 0.18)?
                        .prepare(headroom)?;
                    for mode in [0, 1] {
                        cases.extend(colors.iter().map(|&color| Case {
                            color,
                            parameters,
                            headroom,
                            mode,
                        }));
                    }
                }
            }
        }
    }
    let output = run(&cases, &[PRODUCTION])?;
    for (actual, case) in output[0].iter().zip(&cases) {
        for &channel in &actual[..3] {
            let actual = f32::from_bits(channel);
            assert!((0.0..=case.parameters.to_array()[2]).contains(&actual));
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute；灰阶保持中性并匹配独立 f64 曲线"]
fn gray_axis_preserves_neutrality_monotonicity_and_calibrated_headroom() -> Result {
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    let mut groups = Vec::new();
    for headroom in [1.0f32, 16.0] {
        for exposure in [1.0f32 / 16.0, 1.0, 16.0] {
            for start in [0.0f32, 0.18, 0.5, 0.99] {
                let parameters =
                    PrimeDrtSettings::new(exposure, 0.75, 0.04, start)?.prepare(headroom)?;
                let mut levels = vec![
                    0.0,
                    1e-9,
                    0.0031308,
                    0.18,
                    0.5,
                    1.0,
                    4.0,
                    16.0,
                    46.08 * headroom / exposure,
                    1e8,
                ];
                levels.sort_by(f32::total_cmp);
                let begin = cases.len();
                let h = f64::from(headroom);
                let s = f64::from(start);
                let curve_peak = s + (h - s) * (46.08 * h - s) / (46.08 * h - h);
                for value in levels {
                    cases.push(Case {
                        color: [value; 3],
                        parameters,
                        headroom,
                        mode: 0,
                    });
                    let exposed = f64::from(value) * f64::from(exposure);
                    let mapped = if exposed <= s {
                        exposed
                    } else {
                        s + (exposed - s) * (curve_peak - s) / (exposed - s + curve_peak - s)
                    };
                    expected.push(encode(mapped).min(encode(h)));
                }
                groups.push(begin..cases.len());
            }
        }
    }
    let output = run(&cases, &[PRODUCTION])?;
    for (row, expected) in output[0].iter().zip(expected) {
        for &channel in &row[..3] {
            close(f32::from_bits(channel), expected);
        }
    }
    for group in groups {
        for pair in output[0][group].windows(2) {
            assert!(f32::from_bits(pair[0][0]) <= f32::from_bits(pair[1][0]) + 2e-6);
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute；linear出口只在显示边界编码一次"]
fn linear_bt709_reaches_the_encoded_output_through_one_srgb_boundary() -> Result {
    let mut cases = Vec::new();
    for headroom in [1.0, 16.0] {
        let parameters = PrimeDrtSettings::default().prepare(headroom)?;
        for color in [
            [0.0; 3],
            [0.0031308; 3],
            [0.18; 3],
            [1.0, 0.0, 0.0],
            [10000.0, 0.01, 7.0],
            [-1.0, 0.4, 0.9],
            [1e30; 3],
        ] {
            for mode in [0, 2] {
                cases.push(Case {
                    color,
                    parameters,
                    headroom,
                    mode,
                });
            }
        }
    }
    let output = run(&cases, &[PRODUCTION])?;
    for pair in output[0].as_chunks::<2>().0 {
        for channel in 0..3 {
            close(
                f32::from_bits(pair[0][channel]),
                encode(f64::from(f32::from_bits(pair[1][channel]))),
            );
        }
    }
    Ok(())
}
