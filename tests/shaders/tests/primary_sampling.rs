//! 首面提案改变后，验证实际 GPU 物理响应、密度与估计量；不以多路径场景收敛作正确性依据。
use simple_engine::gpu_test::{Capability, Gpu, Shader};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
type Row = [[f32; 4]; 5];

const CASES: &[(u32, &[u8])] = &[
    (
        131,
        include_bytes!(concat!(env!("OUT_DIR"), "/primary-131.spv")),
    ),
    (
        195,
        include_bytes!(concat!(env!("OUT_DIR"), "/primary-195.spv")),
    ),
    (
        2191,
        include_bytes!(concat!(env!("OUT_DIR"), "/primary-2191.spv")),
    ),
    (
        2181,
        include_bytes!(concat!(env!("OUT_DIR"), "/primary-2181.spv")),
    ),
    (
        132,
        include_bytes!(concat!(env!("OUT_DIR"), "/primary-132.spv")),
    ),
    (
        338,
        include_bytes!(concat!(env!("OUT_DIR"), "/primary-338.spv")),
    ),
    (
        658,
        include_bytes!(concat!(env!("OUT_DIR"), "/primary-658.spv")),
    ),
    (
        354,
        include_bytes!(concat!(env!("OUT_DIR"), "/primary-354.spv")),
    ),
    (
        674,
        include_bytes!(concat!(env!("OUT_DIR"), "/primary-674.spv")),
    ),
];

fn run(key: u32, mode: u32, n: usize) -> Result<Vec<Row>> {
    let spirv = CASES.iter().find(|case| case.0 == key).unwrap().1;
    let solid = key & 16 != 0;
    let transmission = key & 48 != 0;
    let smooth = key & 64 != 0;
    let input = [
        [
            if mode == 3 { 0.0_f32 } else { 0.7 },
            if mode == 3 { 0. } else { 0.4 },
            if mode == 3 { 0. } else { 0.2 },
            if smooth { 0. } else { 0.4 },
        ],
        [
            1.5,
            if key & 2048 != 0 {
                if mode == 4 {
                    0.
                } else if mode == 5 {
                    1.
                } else {
                    0.35
                }
            } else if key & 4 != 0 {
                1.
            } else {
                0.
            },
            if key & 8 != 0 { 0.3 } else { 0. },
            0.1,
        ],
        [if mode == 1 && solid { 1.5 } else { 1. }, 0., 0., 0.],
        [if mode != 0 && solid { 1. } else { 1.5 }, 0.1, 0.4, 0.8],
        [
            n as f32,
            if transmission {
                if solid { 1. } else { 3. }
            } else {
                0.
            },
            if mode == 1 { 0.95 } else { 0. },
            if mode == 1 { 0.3122499 } else { 1. },
        ],
    ];
    let bytes: Vec<_> = input
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let output = gpu.compute(
        Shader {
            spirv,
            entry: "primary_sampling_main",
            workgroup: [64, 1, 1],
        },
        &bytes,
        n * 80,
        [u32::try_from(n.div_ceil(64))?, 1, 1],
    );
    let cleanup = gpu.finish();
    let output = output?;
    cleanup?;
    Ok(output
        .as_chunks::<80>()
        .0
        .iter()
        .map(|bytes| {
            std::array::from_fn(|row| {
                std::array::from_fn(|column| {
                    let at = row * 16 + column * 4;
                    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
                })
            })
        })
        .collect())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn proposal_density_and_response_agree_with_evaluation() -> Result {
    for &(key, _) in CASES {
        for mode in 0..6 {
            for row in run(key, mode, 4096)? {
                assert!(
                    row.iter().flatten().all(|x| x.is_finite()),
                    "key={key}, mode={mode}, {row:?}"
                );
                for error in [row[0][3], row[1][3], row[2][3]] {
                    assert!(
                        error < 2e-4,
                        "响应、PDF或MIS不一致：key={key}, mode={mode}, {row:?}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn one_and_two_sample_estimators_preserve_physical_integrals() -> Result {
    for &(key, _) in CASES {
        for mode in 0..3 {
            let rows = run(key, mode, 262144)?;
            for strategy in 0..2 {
                for channel in 0..3 {
                    let (sum, squared) = rows
                        .iter()
                        .map(|row| {
                            let d = f64::from(row[strategy][channel]) - f64::from(row[2][channel]);
                            (d, d * d)
                        })
                        .fold((0., 0.), |a, b| (a.0 + b.0, a.1 + b.1));
                    let n = rows.len() as f64;
                    let mean = sum / n;
                    let standard_error = ((squared / n - mean * mean).max(0.) / n).sqrt();
                    assert!(
                        mean.abs() <= 6. * standard_error + 3e-4,
                        "key={key}, mode={mode}, strategy={strategy}, channel={channel}, difference={mean}, se={standard_error}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn separated_delta_pair_covers_both_events_and_degenerates_under_tir() -> Result {
    for row in run(338, 0, 256)? {
        assert_eq!((row[3][0] as u32 | row[3][1] as u32) & 3, 3);
        assert_eq!(&row[3][2..], &[0.5, 0.5]);
        for channel in 0..3 {
            assert!((row[1][channel] - (0.04 + 0.96 / 2.25)).abs() < 2e-6);
        }
    }
    for row in run(338, 1, 256)? {
        assert_eq!(row[3], [17., 17., 1., 1.]);
        assert_eq!(&row[1][..3], &[1., 1., 1.]);
    }
    Ok(())
}

#[test]
#[ignore = "需要真实 Vulkan compute GPU"]
fn pure_conductor_keeps_two_glossy_direction_samples() -> Result {
    for row in run(132, 0, 1024)? {
        if row[3][0] != 0. && row[3][1] != 0. {
            assert_eq!(row[3][0], 9.);
            assert_eq!(row[3][1], 9.);
            assert!(row[4][..3].iter().any(|x| x.abs() > 1e-6));
        }
    }
    Ok(())
}
