//! GPU fixture只负责输入字节、固定入口与能量绑定的同一份声明，以及设备完成边界。
use super::Result;
use simple_engine::{
    gpu_test::{Capability, Gpu},
    openpbr::{GpuEnergy, Material, Medium},
    wgpu,
};

pub(super) type Output = [[f32; 4]; 5];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EnergyBinding {
    Absent,
    Required,
}

impl EnergyBinding {
    pub(super) const fn from_required(required: bool) -> Self {
        if required {
            Self::Required
        } else {
            Self::Absent
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Fixture<'a> {
    spirv: &'a [u8],
    entry: &'a str,
    energy: EnergyBinding,
}

impl<'a> Fixture<'a> {
    pub(super) const fn new(spirv: &'a [u8], entry: &'a str, energy: EnergyBinding) -> Self {
        Self {
            spirv,
            entry,
            energy,
        }
    }

    pub(super) const fn main(spirv: &'a [u8], energy: EnergyBinding) -> Self {
        Self::new(spirv, "main", energy)
    }

    pub(super) fn run(self, cases: &[Case]) -> Result<Vec<Output>> {
        let Self {
            spirv,
            entry,
            energy,
        } = self;
        let bind_energy = energy == EnergyBinding::Required;

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
            let shader = context.shader(spirv, entry, [64, 1, 1])?;
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("OpenPBR fixture"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
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
}

#[derive(Clone, Debug)]
pub(super) struct Case {
    pub(super) material: Material,
    pub(super) view: [f32; 4],
    pub(super) u: [f32; 4],
    pub(super) incident: Medium,
    pub(super) transmitted: Medium,
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
