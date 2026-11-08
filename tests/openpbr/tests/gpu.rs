//! 真实能量纹理、特化等价与光学性质。
#![forbid(unsafe_code)]

use simple_engine::{
    gpu_test::{Capability, Gpu, Shader},
    openpbr::{self, GpuEnergy, Material, Medium},
    wgpu,
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Debug)]
struct Case {
    material: Material,
    view: [f32; 4],
    u: [f32; 4],
    incident: Medium,
    transmitted: Medium,
}

impl Case {
    fn words(&self) -> Vec<u32> {
        let mut words = self.material.to_words().to_vec();
        for values in [
            self.view,
            self.u,
            self.incident.to_array(),
            self.transmitted.to_array(),
        ] {
            words.extend(values.map(f32::to_bits));
        }
        words
    }
}

fn run(cases: &[Case]) -> Result<Vec<[[f32; 4]; 5]>> {
    run_shader(
        cases,
        include_bytes!(concat!(env!("OUT_DIR"), "/production.spv")),
        true,
    )
}

fn run_shader(cases: &[Case], spirv: &[u8], bind_energy: bool) -> Result<Vec<[[f32; 4]; 5]>> {
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let result = (|| -> Result<_> {
        let context = gpu.context();
        let device = context.device();
        let energy = bind_energy
            .then(|| GpuEnergy::new(device, context.queue()))
            .transpose()?;
        let bytes: Vec<u8> = cases
            .iter()
            .flat_map(Case::words)
            .flat_map(u32::to_le_bytes)
            .collect();
        let input = context.buffer("OpenPBR cases", &bytes, wgpu::BufferUsages::STORAGE)?;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenPBR results"),
            size: (cases.len() * 80) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenPBR fixture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ][..if bind_energy { 4 } else { 2 }],
        });
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ];
        if let Some(energy) = &energy {
            entries.extend([
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(energy.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(energy.sampler()),
                },
            ]);
        }
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenPBR fixture"),
            layout: &layout,
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenPBR fixture"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = context.shader(spirv, "main", [64, 1, 1])?;
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("OpenPBR fixture"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups((cases.len() as u32).div_ceil(64), 1, 1);
        }
        context.queue().submit([encoder.finish()]);
        let bytes = gpu.readback(&output)?;
        let rows = bytes
            .as_chunks::<80>()
            .0
            .iter()
            .map(|row| {
                std::array::from_fn(|a| {
                    std::array::from_fn(|b| {
                        f32::from_le_bytes(
                            row[a * 16 + b * 4..a * 16 + b * 4 + 4].try_into().unwrap(),
                        )
                    })
                })
            })
            .collect::<Vec<[[f32; 4]; 5]>>();
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row[2][2].to_bits(), index as u32 + 1, "shader执行身份");
        }
        Ok(rows)
    })();
    let cleanup = gpu.finish();
    let rows = result?;
    cleanup?;
    Ok(rows)
}

fn fixture() -> Result<Vec<Case>> {
    let mut materials = Vec::new();
    for roughness in [0.0, 0.03, 0.3, 0.9] {
        for (metal, sss) in [
            (0.0, 0.0),
            (1.0, 0.0),
            (128.0 / 255.0, 0.0),
            (0.0, 128.0 / 255.0),
        ] {
            materials.push(Material::opaque(
                [0.3, 0.6, 0.8],
                roughness,
                1.5,
                metal,
                sss,
            )?);
        }
        materials.push(Material::dielectric(roughness, None)?);
        materials.push(Material::dielectric(roughness, Some(0.0625))?);
    }
    let mut cases = Vec::new();
    for material in materials {
        for (ior_a, ior_b, cosine) in [(1.0, 1.5, 1.0_f32), (1.0, 1.5, 0.2), (1.5, 1.0, 0.3)] {
            for index in 0..64 {
                cases.push(Case {
                    material,
                    view: [(1.0 - cosine * cosine).sqrt(), 0.0, cosine, 0.0],
                    u: [
                        (index as f32 + 0.5) / 64.0,
                        (((index * 19) % 64) as f32 + 0.5) / 64.0,
                        (((index * 37) % 64) as f32 + 0.5) / 64.0,
                        0.0,
                    ],
                    incident: Medium::new(ior_a, [0.0; 3])?,
                    transmitted: Medium::new(ior_b, [0.16, 0.32, 0.48])?,
                });
            }
        }
    }
    Ok(cases)
}

fn close(actual: f32, expected: f32) {
    assert!(
        actual.is_finite()
            && expected.is_finite()
            && (actual - expected).abs() <= 3e-5 * expected.abs().max(1.0),
        "{actual} != {expected}"
    );
}

const SPECIALIZED: [&[u8]; 8] = [
    include_bytes!(concat!(env!("OUT_DIR"), "/specialized-opaque.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/specialized-subsurface.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/specialized-conductor.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/specialized-solid.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/specialized-thin.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/specialized-delta-conductor.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/specialized-delta-solid.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/specialized-delta-thin.spv")),
];

fn specialized_cases(capability: usize) -> Result<Vec<Case>> {
    let color = [0.3, 0.6, 0.8];
    let roughnesses = if capability >= 5 {
        &[0.0, 0.003, 0.0099][..]
    } else {
        &[0.0, 0.003, 0.0099, 0.01, 0.03, 0.3, 0.9][..]
    };
    let mut materials = Vec::new();
    for &roughness in roughnesses {
        match capability {
            0 => materials.push((Material::opaque(color, roughness, 1.5, 0.0, 0.0)?, false)),
            1 => {
                for sss in [0.0, 128.0 / 255.0, 1.0] {
                    materials.push((Material::opaque(color, roughness, 1.5, 0.0, sss)?, false));
                }
            }
            2 | 5 => materials.push((Material::opaque(color, roughness, 1.5, 1.0, 0.0)?, false)),
            3 | 6 => materials.push((Material::dielectric(roughness, None)?, false)),
            4 | 7 => materials.push((Material::dielectric(roughness, Some(0.0625))?, false)),
            _ => unreachable!("本测试只枚举公开的八个类别"),
        }
    }
    if capability == 6 {
        materials.push((Material::dielectric(0.9, None)?, true));
    }
    let mut cases = Vec::new();
    for (material, index_matched_only) in materials {
        for (ior_a, ior_b, cosine) in [
            (1.0, 1.5, 1.0_f32),
            (1.0, 1.5, 0.2),
            (1.5, 1.0, 0.3),
            (1.5, 1.5, 0.6),
            (1.0, 1.5, -0.2),
        ] {
            if index_matched_only && ior_a != ior_b {
                continue;
            }
            for index in 0..16 {
                let index = index * 4;
                cases.push(Case {
                    material,
                    view: [(1.0 - cosine * cosine).sqrt(), 0.0, cosine, 0.0],
                    u: [
                        (index as f32 + 0.5) / 64.0,
                        (((index * 19) % 64) as f32 + 0.5) / 64.0,
                        (((index * 37) % 64) as f32 + 0.5) / 64.0,
                        0.0,
                    ],
                    incident: Medium::new(ior_a, [0.0; 3])?,
                    transmitted: Medium::new(ior_b, [0.16, 0.32, 0.48])?,
                });
            }
        }
    }
    if capability == 4 {
        // 反向 IOR 的除法再倒数会落在 thin alpha 阈值下侧。
        for choice in [0.001, 0.5] {
            cases.push(Case {
                material: Material::dielectric(f32::from_bits(0x3c72_a950), Some(0.0625))?,
                view: [0.6, 0.0, 0.8, 0.0],
                u: [0.3, 0.7, choice, 0.0],
                incident: Medium::new(1.41688, [0.0; 3])?,
                transmitted: Medium::new(1.0, [0.16, 0.32, 0.48])?,
            });
        }
    }
    Ok(cases)
}

const FLOWS: [&[u8]; 30] = [
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-opaque.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-subsurface.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-conductor.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-solid.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-thin.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-delta-conductor.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-delta-solid.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-delta-thin.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-opaque-continuous.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-subsurface-continuous.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-conductor-continuous.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-solid-continuous.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-thin-continuous.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-opaque-smooth.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-mixed-continuous.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-mixed-smooth.spv")),
    include_bytes!(concat!(
        env!("OUT_DIR"),
        "/flow-mixed-subsurface-continuous.spv"
    )),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-mixed-subsurface.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-subsurface-only.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-absorber.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-lambert.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-subsurface-matched.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-solid-matched.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-thin-matched.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-black-smooth.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-solid-rough-matched.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-metal-null-continuous.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-metal-null-smooth.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-metal-lambert.spv")),
    include_bytes!(concat!(env!("OUT_DIR"), "/flow-binary-metal-sss.spv")),
];

fn flow_cases(profile: usize) -> Result<Vec<Case>> {
    let binary = profile == 29;
    let profile = if binary { 16 } else { profile };
    if profile < 8 {
        return specialized_cases(profile);
    }
    if profile >= 25 {
        let mut cases = specialized_cases(if profile == 25 { 3 } else { 0 })?;
        cases.retain(|case| {
            if profile == 27 {
                f32::from_bits(case.material.to_words()[3]) == 0.0
            } else {
                f32::from_bits(case.material.to_words()[3]) >= 0.03
            }
        });
        if profile >= 26 {
            for case in &mut cases {
                case.material = Material::opaque(
                    if profile == 28 {
                        [0.3, 0.6, 0.8]
                    } else {
                        [0.0; 3]
                    },
                    f32::from_bits(case.material.to_words()[3]),
                    case.incident.to_array()[0],
                    0.5,
                    0.0,
                )?;
            }
        }
        return Ok(cases);
    }
    if profile >= 19 {
        let capability = match profile {
            21 => 1,
            22 => 3,
            23 => 4,
            _ => 0,
        };
        let mut cases = specialized_cases(capability)?;
        for case in &mut cases {
            let words = case.material.to_words();
            if matches!(profile, 19 | 20 | 21 | 24) {
                case.material = Material::opaque(
                    if matches!(profile, 19 | 24) {
                        [0.0; 3]
                    } else {
                        [0.3, 0.6, 0.8]
                    },
                    if profile == 24 {
                        0.0
                    } else {
                        f32::from_bits(words[3])
                    },
                    if profile == 24 {
                        1.5
                    } else {
                        case.incident.to_array()[0]
                    },
                    0.0,
                    if profile == 21 { 1.0 } else { 0.0 },
                )?;
            } else {
                case.transmitted = Medium::new(case.incident.to_array()[0], [0.16, 0.32, 0.48])?;
            }
        }
        if profile == 22 {
            for cosine in [0.0_f32, 1e-5, 0.0003] {
                let mut case = cases[0].clone();
                case.view = [(1.0 - cosine * cosine).sqrt(), 0.0, cosine, 0.0];
                cases.push(case);
            }
        }
        return Ok(cases);
    }
    let capability = match profile {
        8 | 13 | 14 | 15 => 0,
        9 | 16..=18 => 1,
        10 => 2,
        11 => 3,
        12 => 4,
        _ => unreachable!(),
    };
    let mut cases = specialized_cases(capability)?;
    cases.retain(|case| {
        let words = case.material.to_words();
        if binary && !matches!(f32::from_bits(words[6]), 0.0 | 1.0) {
            return false;
        }
        let roughness = f32::from_bits(words[3]);
        let rough = roughness * roughness >= 1e-4;
        if matches!(profile, 13 | 15) {
            return !rough;
        }
        if profile == 18 && f32::from_bits(words[6]) != 1.0 {
            return false;
        }
        // 连续固体排除匹配 IOR；薄片排除缩放后 smooth 的 IOR／roughness。
        if profile == 11 && case.incident.to_array()[0] == case.transmitted.to_array()[0] {
            return false;
        }
        if profile == 12 {
            let a = case.incident.to_array()[0];
            let b = case.transmitted.to_array()[0];
            let interface = openpbr::InterfaceIor::between(
                openpbr::Range::point(a).expect("有效 fixture IOR"),
                openpbr::Range::point(b).expect("有效 fixture IOR"),
            )
            .expect("有效 fixture 界面");
            return openpbr::MaterialRanges::constant(case.material)
                .flow(interface, false)
                .bits()
                == 674;
        }
        profile == 17 || rough
    });
    if matches!(profile, 14..=17) {
        let mut mixed = Vec::new();
        for case in cases {
            let weights: &[f32] = if binary {
                &[0., 1.]
            } else {
                &[0., 0.19, 0.73, 1.]
            };
            for &metalness in weights {
                let mut case = case.clone();
                let words = case.material.to_words();
                case.material = Material::opaque(
                    [
                        f32::from_bits(words[0]),
                        f32::from_bits(words[1]),
                        f32::from_bits(words[2]),
                    ],
                    f32::from_bits(words[3]),
                    f32::from_bits(words[4]),
                    metalness,
                    f32::from_bits(words[6]),
                )?;
                mixed.push(case);
            }
        }
        return Ok(mixed);
    }
    Ok(cases)
}

#[test]
#[ignore = "需要 Vulkan compute：组合能力裁剪与未裁剪入口的同参数／随机映射一致"]
fn flow_compositions_preserve_the_complete_supported_closure() -> Result {
    for (profile, shader) in FLOWS.into_iter().enumerate() {
        let cases = flow_cases(profile)?;
        assert!(!cases.is_empty());
        let bind_energy = !matches!(profile, 2 | 4..=7 | 10 | 12 | 19..=24 | 26..=28);
        let actual = run_shader(&cases, shader, bind_energy)?;
        let reference = run(&cases)?;
        for (index, (a, b)) in actual.iter().zip(&reference).enumerate() {
            assert_eq!(
                a[2][0].to_bits(),
                b[2][0].to_bits(),
                "profile={profile} case={index} event"
            );
            for row in [0, 1, 3, 4] {
                for channel in 0..4 {
                    let a = a[row][channel];
                    let b = b[row][channel];
                    assert!(
                        a.is_finite() && b.is_finite() && (a - b).abs() <= 3e-5 * b.abs().max(1.0),
                        "profile={profile} case={index} row={row} channel={channel}: {a} != {b}; {:?}",
                        cases[index]
                    );
                }
            }
        }
        let mut queries = Vec::new();
        for case in cases.iter().step_by(16) {
            for direction in [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0]] {
                let mut case = case.clone();
                case.u = [direction[0], direction[1], direction[2], 2.0];
                queries.push(case);
            }
        }
        let actual = run_shader(&queries, shader, bind_energy)?;
        let reference = run(&queries)?;
        for (index, (a, b)) in actual.iter().zip(reference).enumerate() {
            let may_a = a[2][1].to_bits() != 0;
            let may_b = b[2][1].to_bits() != 0;
            assert!(
                !may_a || may_b,
                "profile={profile} query={index} unnecessary visibility query"
            );
            assert!(
                may_a || !b[3][..3].iter().any(|v| *v > 0.0),
                "profile={profile} query={index} rejected physical response"
            );
            for channel in 0..4 {
                close(a[3][channel], b[3][channel]);
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "真实GPU；连续介电在掠射、临界角和低粗糙度下须保持有限吞吐"]
fn continuous_dielectric_extreme_samples_remain_finite() -> Result {
    let mut state = 0x71e4_926du32;
    let mut uniform = || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        (state >> 8) as f32 / 16777216.
    };
    for exiting in [false, true] {
        let mut cases = Vec::new();
        for _ in 0..262_144 {
            let cosine = 10.0f32.powf(-6. * uniform());
            let roughness = 10.0f32.powf(-1.999 * uniform());
            let ior = if uniform() < 0.5 { 1.33 } else { 1.5 };
            cases.push(Case {
                material: Material::dielectric(roughness, None)?,
                view: [(1. - cosine * cosine).sqrt(), 0., cosine, 0.],
                u: [uniform(), uniform(), uniform(), 0.],
                incident: Medium::new(if exiting { ior } else { 1. }, [0.; 3])?,
                transmitted: Medium::new(if exiting { 1. } else { ior }, [0.; 3])?,
            });
        }
        let rows = run_shader(&cases, FLOWS[3], true)?;
        let mut peak = 0.;
        let mut worst = 0;
        for (index, row) in rows.iter().enumerate() {
            assert!(
                row[1].iter().all(|v| v.is_finite()),
                "case={:?} row={row:?}",
                cases[index]
            );
            if row[1][3] > 0. {
                let weight = row[1][0] / row[1][3];
                assert!(weight.is_finite(), "case={:?} row={row:?}", cases[index]);
                if weight > peak {
                    peak = weight;
                    worst = index;
                }
            }
        }
        eprintln!(
            "exiting={exiting} peak={peak} case={:?} row={:?}",
            cases[worst], rows[worst]
        );
    }
    Ok(())
}

#[test]
#[ignore = "真实GPU；所有白名单组合在数值边界的原始内核不得产生非有限响应"]
fn composed_kernels_are_finite_before_sanitization() -> Result {
    let edge = f32::from_bits(1.0f32.to_bits() - 1);
    let randoms = [
        [0., 0., 0.],
        [edge, edge, edge],
        [0.5, 0.5, 0.5],
        [0., edge, 0.5],
        [edge, 0., 0.5],
        [0.25, 0.75, 0.],
    ];
    let cosines = [0., 1e-8_f32, 1e-5, 0.001, 0.1, 0.5, edge, 1.];
    let mut failures = 0;
    let mut tested = 0;
    for (profile, shader) in FLOWS.into_iter().enumerate() {
        let mut unique = std::collections::HashSet::new();
        let mut cases = Vec::new();
        for seed in flow_cases(profile)? {
            let key = (
                seed.material.to_words(),
                seed.incident.to_array().map(f32::to_bits),
                seed.transmitted.to_array().map(f32::to_bits),
            );
            if !unique.insert(key) {
                continue;
            }
            for cosine in cosines {
                let mut c = seed.clone();
                c.view = [(1. - cosine * cosine).sqrt(), 0., cosine, 0.];
                for u in randoms {
                    c.u = [u[0], u[1], u[2], 6.];
                    cases.push(c.clone());
                }
                for outgoing in [-1., -0.5, -1e-5, 0., 1e-5, 0.5, 1.] {
                    c.u = [(1.0f32 - outgoing * outgoing).sqrt(), 0., outgoing, 7.];
                    cases.push(c.clone());
                }
            }
        }
        if matches!(profile, 0..=4 | 14 | 16) {
            let mut state = 0x453e_196b_u32;
            let mut uniform = || {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / 16777216.
            };
            for index in 0..16_384 {
                let ior_a = if index % 4 == 0 {
                    1.
                } else {
                    1. + 3. * uniform()
                };
                let ior_b = match index % 4 {
                    0 => f32::from_bits(1.0f32.to_bits() + 1),
                    1 => ior_a,
                    _ => 1. + 3. * uniform(),
                };
                let cosine = if index % 3 == 0 && ior_a > ior_b {
                    let critical = (1. - (ior_b / ior_a).powi(2)).sqrt();
                    f32::from_bits(
                        critical
                            .to_bits()
                            .saturating_add_signed((index / 3) % 3 - 1),
                    )
                } else {
                    10.0f32.powf(-8. * uniform())
                };
                let roughness = if index % 8 == 0 {
                    0.01
                } else {
                    10.0f32.powf(-4. * uniform())
                };
                let roughness = if profile >= 14 {
                    roughness.max(0.01)
                } else {
                    roughness
                };
                let material = match profile {
                    3 => Material::dielectric(roughness, None)?,
                    4 => Material::dielectric(roughness, Some(10.0f32.powf(-4. * uniform())))?,
                    _ => Material::opaque(
                        [uniform(), uniform(), uniform()],
                        roughness,
                        ior_b,
                        if profile == 2 {
                            1.
                        } else if profile >= 14 {
                            uniform()
                        } else {
                            0.
                        },
                        if matches!(profile, 1 | 16) {
                            uniform()
                        } else {
                            0.
                        },
                    )?,
                };
                let phi = 2. * std::f32::consts::PI * uniform();
                let sine = (1. - cosine * cosine).sqrt();
                cases.push(Case {
                    material,
                    view: [sine * phi.cos(), sine * phi.sin(), cosine, 0.],
                    u: [uniform(), uniform(), uniform(), 6.],
                    incident: Medium::new(ior_a, [0.; 3])?,
                    transmitted: Medium::new(ior_b, [0.2; 3])?,
                });
            }
        }
        let rows = run_shader(&cases, shader, true)?;
        let mut count = 0;
        let mut interior_count = 0;
        for (c, row) in cases.iter().zip(&rows) {
            let valid_sample = c.u[3] == 6. && row[2][0].to_bits() != 0;
            let sample_ok = !valid_sample
                || row[0].iter().chain(&row[1]).all(|x| x.is_finite())
                    && row[1].iter().all(|x| *x >= 0.)
                    && (row[1][3] == 0. || row[1][..3].iter().all(|x| (x / row[1][3]).is_finite()));
            let evaluated_ok =
                (c.u[3] == 6. && !valid_sample) || row[3].iter().all(|x| x.is_finite() && *x >= 0.);
            if !sample_ok || !evaluated_ok {
                if count < 1 || c.view[2] != 0. && interior_count < 2 {
                    eprintln!("profile={profile} case={c:?} row={row:?}");
                }
                interior_count += usize::from(c.view[2] != 0.);
                count += 1;
            }
        }
        tested += cases.len();
        failures += count;
        eprintln!(
            "profile={profile} tested={} nonfinite={count} interior={interior_count}",
            cases.len()
        );
    }
    assert_eq!(failures, 0, "{tested} 个原始内核边界案例中发现非有限响应");
    Ok(())
}

#[test]
#[ignore = "真实GPU；透射遮蔽在 Gamma 溢出区域仍与独立 Beta 递推一致"]
fn transmission_masking_matches_integer_beta_identities() -> Result {
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for roughness in [0.01_f32, 0.2, 1.] {
        let alpha = f64::from(roughness).powi(2);
        for cosine in [1e-8_f32, 1e-4, 0.1, 1.] {
            let sine = (1. - cosine * cosine).sqrt();
            let a =
                (1. + (1. + (alpha * f64::from(sine) / f64::from(cosine)).powi(2)).sqrt()) * 0.5;
            for n in [1_u32, 2, 8, 32] {
                let outgoing =
                    (alpha / (alpha * alpha + 4. * f64::from(n * (n - 1))).sqrt()) as f32;
                cases.push(Case {
                    material: Material::dielectric(roughness, None)?,
                    view: [sine, 0., cosine, 0.],
                    u: [(1. - outgoing * outgoing).sqrt(), 0., -outgoing, 0.],
                    incident: Medium::VACUUM,
                    transmitted: Medium::VACUUM,
                });
                let mut value = 1. / a;
                for k in 1..n {
                    value *= f64::from(k) / (a + f64::from(k));
                }
                expected.push(value);
            }
        }
    }
    let rows = run_shader(
        &cases,
        include_bytes!(concat!(env!("OUT_DIR"), "/numerical.spv")),
        false,
    )?;
    for ((case, row), expected) in cases.iter().zip(rows).zip(expected) {
        let actual = f64::from(row[0][0]);
        assert!(
            actual.is_finite() && (actual - expected).abs() <= 3e-4 * expected + 1e-37,
            "{case:?}: {actual} != {expected}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "真实GPU；反射的掠射极限须保留有限的解析响应"]
fn reflection_masking_preserves_the_grazing_limit() -> Result {
    let mut cases = Vec::new();
    for roughness in [0.01, 0.1, 1.] {
        for cosine in [0., 1e-8_f32, 1e-4] {
            cases.push(Case {
                material: Material::dielectric(roughness, None)?,
                view: [(1. - cosine * cosine).sqrt(), 0., cosine, 0.],
                u: [0.6, 0., 0.8, 1.],
                incident: Medium::VACUUM,
                transmitted: Medium::VACUUM,
            });
        }
    }
    let rows = run_shader(
        &cases,
        include_bytes!(concat!(env!("OUT_DIR"), "/numerical.spv")),
        false,
    )?;
    for (case, row) in cases.iter().zip(rows) {
        let alpha = f64::from(f32::from_bits(case.material.to_words()[3])).powi(2);
        let ci = f64::from(case.view[2]);
        let co = f64::from(case.u[2]);
        let expected = if ci == 0. {
            1. / (2. * alpha)
        } else {
            let li = (1. + (alpha * f64::from(case.view[0]) / ci).powi(2)).sqrt();
            let lo = (1. + (alpha * f64::from(case.u[0]) / co).powi(2)).sqrt();
            1. / (2. * ci * (li + lo))
        };
        assert!(
            (f64::from(row[0][1]) / expected - 1.).abs() < 5e-6,
            "{case:?}: {row:?}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute：RR方向能量组合与独立通用参考一致"]
fn flow_guide_energy_preserves_supported_materials() -> Result {
    for (profile, shader) in FLOWS.into_iter().enumerate() {
        let mut cases: Vec<_> = flow_cases(profile)?.into_iter().step_by(16).collect();
        for case in &mut cases {
            case.u[3] = 5.;
        }
        let actual = run_shader(&cases, shader, true)?;
        let reference = run(&cases)?;
        for (index, (actual, reference)) in actual.iter().zip(&reference).enumerate() {
            for row in 0..2 {
                for channel in 0..3 {
                    let (a, b) = (actual[row][channel], reference[row][channel]);
                    assert!(
                        a.is_finite() && b.is_finite() && (a - b).abs() <= 3e-5 * b.abs().max(1.),
                        "profile={profile} case={index} row={row} channel={channel}: {a} != {b}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute：组合顶点共享准备与独立查询具有相同输出"]
fn flow_batch_shares_preparation_without_changing_the_vertex() -> Result {
    for (profile, shared, separate) in [
        (
            8,
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-shared-opaque.spv")).as_slice(),
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-separate-opaque.spv")).as_slice(),
        ),
        (
            11,
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-shared-solid.spv")).as_slice(),
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-separate-solid.spv")).as_slice(),
        ),
    ] {
        let cases = flow_cases(profile)?;
        let actual = run_shader(&cases, shared, true)?;
        let reference = run_shader(&cases, separate, true)?;
        for (index, (a, b)) in actual.iter().zip(reference).enumerate() {
            assert_eq!(
                a[2][0].to_bits(),
                b[2][0].to_bits(),
                "profile={profile} case={index} event"
            );
            for row in [0, 1, 3, 4] {
                for channel in 0..4 {
                    close(a[row][channel], b[row][channel]);
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute：八类编译期闭包与通用闭包物理输出一致"]
fn specialized_closures_match_generic() -> Result {
    for (capability, shader) in SPECIALIZED.into_iter().enumerate() {
        let cases = specialized_cases(capability)?;
        // conductor、thin 和三个 delta fixture 都不创建或绑定能量资源。
        let actual = run_shader(&cases, shader, matches!(capability, 0 | 1 | 3))?;
        let reference = run(&cases)?;
        for (index, (a, b)) in actual.iter().zip(reference).enumerate() {
            assert_eq!(
                a[2][0].to_bits(),
                b[2][0].to_bits(),
                "cap={capability} case={index} event"
            );
            for row in [0, 1, 3, 4] {
                for channel in 0..4 {
                    let a = a[row][channel];
                    let b = b[row][channel];
                    assert!(
                        a.is_finite() && b.is_finite() && (a - b).abs() <= 3e-5 * b.abs().max(1.0),
                        "cap={capability} case={index}, row={row}, channel={channel}, port={a}, reference={b}, input={:?}",
                        cases[index]
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute：特化连续支持域保留一般SSS/薄片并排除纯delta"]
fn specialized_continuous_support_preserves_its_domain() -> Result {
    let color = [0.3, 0.6, 0.8];
    let materials = [
        Material::opaque(color, 0.3, 1.5, 0.0, 0.0)?,
        Material::opaque(color, 0.3, 1.5, 0.0, 0.37)?,
        Material::opaque(color, 0.3, 1.5, 1.0, 0.0)?,
        Material::dielectric(0.3, None)?,
        Material::dielectric(0.3, Some(0.023))?,
        Material::opaque(color, 0.0, 1.5, 1.0, 0.0)?,
        Material::dielectric(0.0, None)?,
        Material::dielectric(0.0, Some(0.023))?,
    ];
    let medium = Medium::new(1.5, [0.16, 0.32, 0.48])?;
    for (capability, material) in materials.into_iter().enumerate() {
        let cases: Vec<_> = [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0]]
            .into_iter()
            .map(|direction| Case {
                material,
                view: [0.0, 0.0, 1.0, 0.0],
                u: [direction[0], direction[1], direction[2], 2.0],
                incident: Medium::VACUUM,
                transmitted: medium,
            })
            .collect();
        let actual = run_shader(
            &cases,
            SPECIALIZED[capability],
            matches!(capability, 0 | 1 | 3),
        )?;
        let reference = run(&cases)?;
        for (index, (a, b)) in actual.iter().zip(reference).enumerate() {
            for channel in 0..4 {
                close(a[3][channel], b[3][channel]);
            }
            let may_evaluate = a[2][1].to_bits() != 0;
            if capability >= 5 {
                assert!(!may_evaluate, "cap={capability} 纯delta不能进入NEE");
                assert_eq!(a[3], [0.0; 4], "cap={capability} 纯delta连续响应");
            } else if !may_evaluate {
                assert_eq!(
                    b[3], [0.0; 4],
                    "cap={capability} direction={index} 不得漏掉通用散射"
                );
            }
        }
        if capability == 1 {
            assert!(
                actual[1][3][..3].iter().any(|value| *value > 0.0),
                "一般SSS权重保留反侧散射"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute：同命中batch与三个独立specialized入口一致"]
fn specialized_batch_preserves_independent_evaluations_and_sample() -> Result {
    for (capability, shader) in SPECIALIZED.into_iter().enumerate() {
        let cases: Vec<_> = specialized_cases(capability)?
            .into_iter()
            .step_by(16)
            .collect();
        let bind_energy = matches!(capability, 0 | 1 | 3);
        let samples = run_shader(&cases, shader, bind_energy)?;
        let evaluate = |direction: [f32; 3]| {
            let queries: Vec<_> = cases
                .iter()
                .map(|case| {
                    let mut case = case.clone();
                    case.u = [direction[0], direction[1], direction[2], 2.0];
                    case
                })
                .collect();
            run_shader(&queries, shader, bind_energy)
        };
        let evaluations = [evaluate([0.6, 0.0, 0.8])?, evaluate([0.6, 0.0, -0.8])?];
        let mut queries = Vec::new();
        for case in &cases {
            for control in 0..8 {
                for mode in [3.0, 4.0] {
                    let mut query = case.clone();
                    query.view[3] = control as f32;
                    query.u[3] = mode;
                    queries.push(query);
                }
            }
        }
        let actual = run_shader(&queries, shader, bind_energy)?;
        for (index, case) in cases.iter().enumerate() {
            for control in 0..8 {
                for direction in 0..2 {
                    let row = &actual[(index * 8 + control) * 2 + direction];
                    let mut expected = samples[index];
                    if control & 4 == 0 {
                        expected = [[0.0; 4]; 5];
                        expected[0][3] = 1.0;
                        expected[4] = case.incident.to_array();
                    }
                    assert_eq!(
                        row[2][0].to_bits(),
                        expected[2][0].to_bits(),
                        "cap={capability} case={index} control={control} event"
                    );
                    for r in [0, 1, 4] {
                        for channel in 0..4 {
                            let a = row[r][channel];
                            let b = expected[r][channel];
                            assert!(
                                a.is_finite()
                                    && b.is_finite()
                                    && (a - b).abs() <= 3e-5 * b.abs().max(1.0),
                                "cap={capability} case={index} control={control} sample row={r} channel={channel}: {a} != {b}"
                            );
                        }
                    }
                    let expected = if control & (1 << direction) != 0 {
                        evaluations[direction][index][3]
                    } else {
                        [0.0; 4]
                    };
                    for channel in 0..4 {
                        let a = row[3][channel];
                        let b = expected[channel];
                        assert!(
                            a.is_finite()
                                && b.is_finite()
                                && (a - b).abs() <= 3e-5 * b.abs().max(1.0),
                            "cap={capability} case={index} control={control} evaluation={direction} channel={channel}: {a} != {b}"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute：保守连续支持域与通用闭包评估"]
fn continuous_support_preserves_scattering_directions() -> Result {
    let color = [0.3, 0.6, 0.8];
    let diffuse = Material::opaque(color, 0.3, 1.5, 0.0, 0.0)?;
    let metal = Material::opaque(color, 0.3, 1.5, 1.0, 0.0)?;
    let mixed = Material::opaque(color, 0.3, 1.5, 128.0 / 255.0, 0.0)?;
    let subsurface = Material::opaque(color, 0.3, 1.5, 0.0, 128.0 / 255.0)?;
    let mixed_subsurface = Material::opaque(color, 0.3, 1.5, 128.0 / 255.0, 128.0 / 255.0)?;
    let metal_with_subsurface = Material::opaque(color, 0.3, 1.5, 1.0, 128.0 / 255.0)?;
    let solid = Material::dielectric(0.3, None)?;
    let thin = Material::dielectric(0.3, Some(0.0625))?;
    let front = [0.0, 0.0, 1.0];
    let back = [0.0, 0.0, -1.0];
    // 同一焦点：快判不得拒绝闭包已有的连续散射；true 仍允许 tangent 零响应。
    let queries = [
        ("diffuse/back", diffuse, front, back, false, false),
        ("metal/back", metal, front, back, false, false),
        ("mixed/back", mixed, front, back, false, false),
        (
            "metal+SSS/back",
            metal_with_subsurface,
            front,
            back,
            false,
            false,
        ),
        ("diffuse/front", diffuse, front, front, true, true),
        ("SSS/back", subsurface, front, back, true, true),
        ("mixed+SSS/back", mixed_subsurface, front, back, true, true),
        ("solid/back", solid, front, back, true, true),
        ("thin/back", thin, front, back, true, true),
        ("tangent", diffuse, front, [1.0, 0.0, 0.0], true, false),
        ("reversed/back", diffuse, back, front, false, false),
        ("reversed/front", diffuse, back, back, true, true),
    ];
    let transmitted = Medium::new(1.5, [0.16, 0.32, 0.48])?;
    let cases: Vec<_> = queries
        .iter()
        .map(|(_, material, view, direction, _, _)| Case {
            material: *material,
            view: [view[0], view[1], view[2], 0.0],
            u: [direction[0], direction[1], direction[2], 2.0],
            incident: Medium::VACUUM,
            transmitted,
        })
        .collect();
    let actual = run(&cases)?;
    for (query, a) in queries.iter().zip(&actual) {
        let (label, _, _, _, may_evaluate, positive) = query;
        assert_eq!(a[2][1].to_bits(), u32::from(*may_evaluate), "{label}");
        if !*may_evaluate {
            assert_eq!(a[3], [0.0; 4], "{label} 被拒绝的方向");
        }
        if *positive {
            assert!(
                a[3][..3].iter().any(|value| *value > 0.0) && a[3][3] > 0.0,
                "{label} 连续散射不能因快判消失"
            );
        }
    }
    Ok(())
}

// 仅用于本 fixture 的 f32 舍入模型，不是所有 GPU 的精度保证或生产输入修复。
// D(theta)=alpha²/[pi*(alpha²*cos²(theta)+sin²(theta))²]，
// |d log(D)/d theta|<=2*(1-alpha²)/alpha。half-vector 的相消条件数为
// ||abs(wi)+abs(eta*wo)||/||wi+eta*wo||；gamma(8) 为反射／重建链的舍入预算。
// 它只放大窄高光的原始密度比较；response/pdf 仍维持严格阈值。
fn density_tolerance(case: &Case, row: &[[f32; 4]; 5]) -> f32 {
    let words = case.material.to_words();
    let roughness = f32::from_bits(words[3]);
    let mut alpha = f64::from(roughness).powi(2);
    let thin = words[8] & 2 != 0;
    if thin {
        let eta =
            f64::from(case.transmitted.to_array()[0]) / f64::from(case.incident.to_array()[0]);
        let eta = eta.max(1.0 / eta);
        alpha *= 3.7 * (eta - 1.0) * (eta - 0.5).powi(2) / eta.powi(3);
    }
    if alpha == 0.0 || alpha >= 0.01 {
        return 3e-5;
    }
    let transmitted = row[2][0].to_bits() & 2 != 0;
    let eta = if !thin && transmitted {
        f64::from(row[0][3])
    } else {
        1.0
    };
    let mut sum_squared = 0.0;
    let mut absolute_squared = 0.0;
    for axis in 0..3 {
        let wi = f64::from(case.view[axis]);
        let mut wo = f64::from(row[0][axis]);
        if thin && transmitted && axis == 2 {
            wo = -wo;
        }
        sum_squared += (wi + eta * wo).powi(2);
        absolute_squared += (wi.abs() + (eta * wo).abs()).powi(2);
    }
    let u = f64::from(f32::EPSILON) * 0.5;
    let gamma8 = 8.0 * u / (1.0 - 8.0 * u);
    let angular_budget = gamma8 * (absolute_squared / sum_squared).sqrt();
    let log_density_budget = 2.0 * (1.0 - alpha * alpha) / alpha * angular_budget;
    (3e-5 + log_density_budget.exp_m1()) as f32
}

#[test]
#[ignore = "需要 Vulkan compute：连续sample与完整闭包边缘eval/pdf一致"]
fn continuous_samples_match_the_evaluated_marginal() -> Result {
    let cases = fixture()?;
    let actual = run(&cases)?;
    let mut compared = 0;
    let mut deviations = Vec::new();
    let mut maxima = std::collections::BTreeMap::<u32, [f32; 3]>::new();
    for (index, row) in actual.iter().enumerate() {
        let events = row[2][0].to_bits();
        if events == 0 || events & 16 != 0 {
            continue;
        }
        assert!(row[1][3] > 0.0 && row[3][3] > 0.0, "case={index} 连续PDF");
        let tolerance = density_tolerance(&cases[index], row);
        let group = maxima
            .entry(cases[index].material.to_words()[3])
            .or_default();
        for channel in 0..4 {
            let a = row[1][channel];
            let b = row[3][channel];
            let error = (a - b).abs() / b.abs().max(1.0);
            group[usize::from(channel == 3)] = group[usize::from(channel == 3)].max(error);
            assert!(
                a.is_finite() && b.is_finite() && error <= tolerance,
                "case={index}, channel={channel}, error={error}, conditioning budget={tolerance}, input={:?}",
                cases[index]
            );
            if error > 3e-5 {
                deviations.push((error, index, channel, tolerance));
            }
            if channel < 3 {
                let sampled = a / row[1][3];
                let evaluated = b / row[3][3];
                let error = (sampled - evaluated).abs() / evaluated.abs().max(1.0);
                group[2] = group[2].max(error);
                assert!(
                    sampled.is_finite() && evaluated.is_finite() && error <= 3e-5,
                    "case={index}, channel={channel}, response/pdf 误差={error}"
                );
            }
        }
        compared += 1;
    }
    for (roughness, errors) in maxima {
        eprintln!(
            "roughness={} 最大归一化误差 response/pdf/responseOverPdf={errors:?}",
            f32::from_bits(roughness)
        );
    }
    deviations.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    for &(error, index, channel, tolerance) in deviations.iter().take(3) {
        eprintln!(
            "case={index}, channel={channel}, error={error}, conditioning budget={tolerance}, input={:?}",
            cases[index]
        );
    }
    assert!(compared > 1000, "连续闭包应实际执行");
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute：独立法向Fresnel、TIR与solid/thin介质所有权"]
fn ideal_dielectric_follows_fresnel_eta_and_medium_transition() -> Result {
    let air = Medium::VACUUM;
    let glass = Medium::new(1.5, [0.0; 3])?;
    let solid = Material::dielectric(0.0, None)?;
    let thin = Material::dielectric(0.0, Some(0.0625))?;
    let cases = [
        Case {
            material: solid,
            view: [0.0, 0.0, 1.0, 0.0],
            u: [0.5, 0.5, 0.5, 0.0],
            incident: air,
            transmitted: glass,
        },
        Case {
            material: solid,
            view: [0.0, 0.0, 1.0, 0.0],
            u: [0.5, 0.5, 0.9999, 0.0],
            incident: air,
            transmitted: glass,
        },
        Case {
            material: solid,
            view: [0.96_f32.sqrt(), 0.0, 0.2, 0.0],
            u: [0.5, 0.5, 0.5, 0.0],
            incident: glass,
            transmitted: air,
        },
        Case {
            material: thin,
            view: [0.0, 0.0, 1.0, 0.0],
            u: [0.5, 0.5, 0.5, 0.0],
            incident: air,
            transmitted: glass,
        },
    ];
    let rows = run(&cases)?;
    assert_eq!(rows[0][2][0].to_bits(), 2 | 16);
    close(rows[0][0][3], 1.5);
    close(rows[0][0][2], -1.0);
    close(rows[0][1][3], 0.96);
    close(rows[0][1][0] / rows[0][1][3], 1.0 / 2.25);
    assert_eq!(rows[0][4], glass.to_array());
    assert_eq!(rows[1][2][0].to_bits(), 1 | 16);
    close(rows[1][1][3], 0.04);
    assert_eq!(rows[1][4], air.to_array());
    assert_eq!(rows[2][2][0].to_bits(), 1 | 16);
    close(rows[2][1][3], 1.0);
    assert_eq!(rows[2][4], glass.to_array());
    assert_eq!(rows[3][2][0].to_bits(), 2 | 16);
    close(rows[3][0][3], 1.0);
    assert_eq!(rows[3][4], air.to_array());
    Ok(())
}

fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = ((bits >> 10) & 31) as i32;
    let mantissa = f32::from(bits & 1023);
    if exponent == 0 {
        sign * mantissa * 2.0_f32.powi(-24)
    } else {
        sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15)
    }
}

#[test]
#[ignore = "需要 Vulkan compute：原始HALF4表的D3轴序、采样和上传"]
fn real_energy_texture_preserves_selected_half4_texels() -> Result {
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for (x, y, z) in [
        (0, 0, 0),
        (43, 31, 158),
        (21, 15, 48),
        (5, 20, 62),
        (35, 2, 111),
    ] {
        cases.push(Case {
            material: Material::opaque([0.5; 3], 0.5, 1.5, 0.0, 0.0)?,
            view: [
                (x as f32 + 0.5) / 44.0,
                (y as f32 + 0.5) / 32.0,
                (z as f32 + 0.5) / 159.0,
                0.0,
            ],
            u: [0.0, 0.0, 0.0, 1.0],
            incident: Medium::VACUUM,
            transmitted: Medium::VACUUM,
        });
        let offset = ((z * 32 + y) * 44 + x) * 8;
        expected.push(std::array::from_fn::<_, 4, _>(|c| {
            let bytes = &openpbr::energy_bytes()[offset + c * 2..offset + c * 2 + 2];
            half(u16::from_le_bytes([bytes[0], bytes[1]]))
        }));
    }
    for (row, expected) in run(&cases)?.iter().zip(expected) {
        for channel in 0..4 {
            close(row[0][channel], expected[channel]);
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute：可表示贡献不得因中间权重／乘积溢出丢失"]
fn stable_transport_preserves_representable_weighted_contributions() -> Result {
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let values = [1e-25_f32, 1e-4, 1e30, 1e-30];
    let input: Vec<u8> = values.into_iter().flat_map(f32::to_le_bytes).collect();
    let result = gpu.compute(
        Shader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/transport.spv")),
            entry: "main",
            workgroup: [64, 1, 1],
        },
        &input,
        16,
        [1, 1, 1],
    );
    let cleanup = gpu.finish();
    let bytes = result?;
    cleanup?;
    let words: [u32; 4] =
        std::array::from_fn(|i| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()));
    assert_eq!(words[3], 1);
    let reference =
        f64::from(values[0]) / (f64::from(values[0]).powi(2) + f64::from(values[1]).powi(2));
    let actual = f32::from_bits(words[0]);
    assert!(actual > 0.0 && ((f64::from(actual) - reference) / reference).abs() < 1e-5);
    for word in [words[1], words[2]] {
        let actual = f32::from_bits(word);
        assert!(actual.is_finite() && ((actual - 1e30) / 1e30).abs() < 1e-5);
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute：乘除与 MIS 在普通、边界及极端输入下符合实数表达式"]
fn transport_matches_wide_precision_reference() -> Result {
    let lo = 2.0_f32.powi(-40);
    let hi = 2.0_f32.powi(40);
    let below = f32::from_bits(lo.to_bits() - 1);
    let above = f32::from_bits(hi.to_bits() + 1);
    let cases = [
        ([0., 0.25, 3.], [1., 2., 0.5], 0.125, 4., 0.25, 0.5),
        ([lo, hi, 1.], [lo, hi, lo], hi, lo, lo, hi),
        ([hi, lo, 1.], [hi, lo, hi], lo, hi, hi, lo),
        ([below, above, 0.], [hi, lo, 1.], hi, lo, below, above),
        (
            [1e30, 1e-25, 0.],
            [1e30, 1e30, 1.],
            1e30,
            1e-30,
            1e-25,
            1e-4,
        ),
        ([0.7, 0.002, 17.], [0.13, 21., 0.03], 0.91, 0.29, 0.37, 0.37),
        ([1., 2., 3.], [4., 5., 6.], 7., 8., 0., 1.),
        ([1., 2., 3.], [4., 5., 6.], 7., 8., 1., 0.),
    ];
    let mut input = Vec::new();
    let mut expected = Vec::new();
    for (first, second, denominator, third, sampled, other) in cases {
        for row in [
            [first[0], first[1], first[2], denominator],
            [second[0], second[1], second[2], third],
            [sampled, other, 0., 0.],
        ] {
            input.extend(row.into_iter().flat_map(f32::to_le_bytes));
        }
        let product = std::array::from_fn::<_, 3, _>(|i| {
            f64::from(first[i]) * f64::from(second[i]) / f64::from(denominator)
        });
        expected.extend(product);
        expected.push(product[0]);
        expected
            .extend((0..3).map(|i| f64::from(first[i]) * f64::from(second[i]) * f64::from(third)));
        expected.push(if sampled == 0. {
            0.
        } else {
            f64::from(sampled) / (f64::from(sampled).powi(2) + f64::from(other).powi(2))
        });
    }
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let result = gpu.compute(
        Shader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/transport-vectors.spv")),
            entry: "vectors",
            workgroup: [64, 1, 1],
        },
        &input,
        expected.len() * 4,
        [1, 1, 1],
    );
    let cleanup = gpu.finish();
    let bytes = result?;
    cleanup?;
    for (index, (word, reference)) in bytes.as_chunks::<4>().0.iter().zip(expected).enumerate() {
        let actual = f64::from(f32::from_le_bytes(*word));
        assert!(actual.is_finite(), "result {index}: {actual}");
        if reference == 0. {
            assert_eq!(actual, 0., "result {index}");
        } else {
            assert!(
                (actual / reference - 1.).abs() < 2e-6,
                "result {index}: {actual} versus {reference}"
            );
        }
    }
    Ok(())
}
