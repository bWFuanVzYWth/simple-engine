//! 直接使用受控 Slang SPIR-V 的原生 Vulkan 大气 owner。
//!
//! 与 wgpu owner 使用相同物理数据、八个生产入口及 observer cache。initialize/record
//! 只录制；资源创建包含同步布局转换，宿主须独占队列并负责提交、排序与完成。
//! 初始化完成后调用 [`Atmosphere::release_initialization`]。
//! 每次 `prepare` 前须通过 Context 取消或完成所有引用上次参数的 Recorded/Flight；输出原位复用，
//! 初始化命令取消后须重建 owner；视图更新在实际提交后调用 [`Atmosphere::submitted`]，
//! 取消未提交的更新保留 pending。多视图需要独立 owner，不隐藏调度或等待。

use crate::{
    AERIAL_SIZE, Error, FrameState, MAPPING_ROWS, Memory, Model, Parameters, SHADERS, SKY_SIZE,
    Update, View,
};
use std::ffi::CString;
use vulkan::{
    interop::RayShader,
    native::{
        Binding, Bindings as Resources, Buffer, Commands, ComputePipeline, Context, Image,
        ImageFormat, Recorded, Resource, Sampler,
    },
};

#[cfg(all(test, feature = "gpu"))]
mod tests;

struct Bank {
    source: Image,
    mean: Image,
    ground: Image,
    high: Image,
}
impl Bank {
    fn new(context: &mut Context, p: &Parameters) -> Result<Self, Error> {
        let [h, s, _, _] = p.dims;
        let low = p.offsets1[1];
        Ok(Self {
            source: Image::new(
                context,
                [s * p.solve[1], low * p.solve[2], 1],
                ImageFormat::Rgba16Float,
            )?,
            mean: Image::new(context, [s, h, 1], ImageFormat::Rgba32Float)?,
            ground: Image::new(context, [s, 1, 1], ImageFormat::Rgba32Float)?,
            high: Image::new(context, [s * 5, h - low + 1, 1], ImageFormat::Rgba32Float)?,
        })
    }
    fn bytes(p: &Parameters) -> u64 {
        let [h, s, _, _] = p.dims.map(u64::from);
        let low = u64::from(p.offsets1[1]);
        low * s * u64::from(p.solve[1]) * u64::from(p.solve[2]) * 8
            + (h - low + 1) * s * 5 * 16
            + h * s * 16
            + s * 16
    }
}

fn upload(context: &mut Context, bytes: &[u8]) -> Result<Buffer, Error> {
    let result = Buffer::new(context, bytes.len() as u64, true)?;
    result.write(context, 0, bytes)?;
    Ok(result)
}
fn uniform(buffer: &Buffer) -> Binding {
    Binding::UniformBuffer(*buffer)
}
fn storage(buffer: &Buffer) -> Binding {
    Binding::StorageBuffer(*buffer)
}
fn sampled(image: &Image) -> Binding {
    Binding::SampledImage(*image)
}
fn writable(image: &Image) -> Binding {
    Binding::StorageImage(*image)
}

fn program(
    context: &mut Context,
    index: usize,
    resources: &[Binding],
) -> Result<ComputePipeline, Error> {
    let shader = &SHADERS[index];
    let entry = CString::new(shader.entry).expect("固定 shader 入口不含 NUL");
    let bindings = Resources::new(context, resources)?;
    Ok(ComputePipeline::new(
        context,
        bindings,
        RayShader {
            spirv: shader.spirv,
            entry: &entry,
        },
    )?)
}
fn dispatch(
    context: &mut Context,
    commands: &mut Commands,
    programs: &mut [Option<ComputePipeline>; 6],
    index: usize,
    resources: &[Binding],
    groups: [u32; 3],
) -> Result<(), Error> {
    if groups.contains(&0) {
        return Ok(());
    }
    let bindings = Resources::new(context, resources)?;
    let pipeline = if let Some(template) = &programs[index] {
        template.rebind(context, bindings)?
    } else {
        let shader = &SHADERS[index];
        let entry = CString::new(shader.entry).expect("固定 shader 入口不含 NUL");
        let pipeline = ComputePipeline::new(
            context,
            bindings,
            RayShader {
                spirv: shader.spirv,
                entry: &entry,
            },
        )?;
        programs[index] = Some(pipeline);
        pipeline
    };
    commands.dispatch(pipeline, groups)?;
    commands.barrier();
    Ok(())
}

struct Scratch {
    _bank: Bank,
    _buffers: Vec<Buffer>,
    bytes: u64,
}

/// 借用同一设备输出；纹理处于 GENERAL，布局语义与 `sampling.slang` 一致。
pub struct Bindings<'a> {
    /// RGBA16F 512×128 四波长 optical depth。
    pub optical_depth: &'a Image,
    /// RGBA32F 256² 三 chart log Rec.2020 SkyView。
    pub sky: &'a Image,
    /// RGBA16F 32³ 介质散射 RGB，已乘 View::radiance_scale。
    pub aerial_light: &'a Image,
    /// RGBA16F 32³ RGB 透射近似。
    pub aerial_transmittance: &'a Image,
    /// clamp-to-edge 线性采样器；SkyView 采用手动采样。
    pub sampler: &'a Sampler,
    /// 对应 CPU mapping 的只读 storage buffer。
    pub sky_mapping: &'a Buffer,
    /// 最新 prepare 的 400 B uniform；消费方也可按 raw words 作为 storage 绑定。
    pub parameters: &'a Buffer,
}

/// 固定 Earth/balanced 场与一份 observer cache；GPU 资源唯一位于调用方 Context。
pub struct Atmosphere {
    data: Buffer,
    base: Parameters,
    calibration: crate::mapping::Calibration,
    frame: FrameState,
    field: Bank,
    tau: Image,
    sky: Image,
    aerial_light: Image,
    aerial_t: Image,
    sampler: Sampler,
    sky_uniform: Buffer,
    frame_uniform: Buffer,
    aerial_uniform: Buffer,
    mapping: Buffer,
    sky_program: ComputePipeline,
    aerial_program: ComputePipeline,
    initialized: bool,
    scratch: Option<Scratch>,
    resident_bytes: u64,
}
impl Atmosphere {
    /// 创建输出、物理数据和两个视图程序；图像创建包含初始化布局转换，尚未求解物理场。
    /// # Errors
    /// 物理数据、设备容量、资源创建、上传或 Slang pipeline 失败时返回来源。
    pub fn new(context: &mut Context) -> Result<Self, Error> {
        let model = Model::new()?;
        let p = model.parameters;
        let data = upload(context, bytemuck::cast_slice(&model.data))?;
        let field = Bank::new(context, &p)?;
        let tau = Image::new(context, [p.dims[3], p.dims[2], 1], ImageFormat::Rgba16Float)?;
        let sky = Image::new(context, [SKY_SIZE, SKY_SIZE, 1], ImageFormat::Rgba32Float)?;
        let aerial_light = Image::new(context, [AERIAL_SIZE; 3], ImageFormat::Rgba16Float)?;
        let aerial_t = Image::new(context, [AERIAL_SIZE; 3], ImageFormat::Rgba16Float)?;
        let sampler = Sampler::new(context, true)?;
        let sky_uniform = upload(context, bytemuck::bytes_of(&p))?;
        let frame_uniform = upload(context, bytemuck::bytes_of(&p))?;
        let aerial_uniform = upload(
            context,
            bytemuck::cast_slice(&View::default().aerial_parameters()),
        )?;
        let mapping = upload(context, bytemuck::cast_slice(&[[0.0f32; 4]; MAPPING_ROWS]))?;
        let sky_program = program(
            context,
            6,
            &[
                uniform(&sky_uniform),
                storage(&data),
                sampled(&tau),
                sampled(&field.source),
                sampled(&field.mean),
                sampled(&field.ground),
                Binding::Sampler(sampler),
                sampled(&field.high),
                storage(&mapping),
                writable(&sky),
            ],
        )?;
        let aerial_program = program(
            context,
            7,
            &[
                uniform(&frame_uniform),
                storage(&data),
                sampled(&tau),
                sampled(&field.source),
                sampled(&field.mean),
                sampled(&field.ground),
                Binding::Sampler(sampler),
                sampled(&field.high),
                uniform(&aerial_uniform),
                writable(&aerial_light),
                writable(&aerial_t),
            ],
        )?;
        let resident_bytes = Bank::bytes(&p)
            + model.data.len() as u64 * 16
            + u64::from(p.dims[2]) * u64::from(p.dims[3]) * 8
            + u64::from(SKY_SIZE).pow(2) * 16
            + u64::from(AERIAL_SIZE).pow(3) * 16
            + (MAPPING_ROWS * 16) as u64
            + 880;
        Ok(Self {
            data,
            base: p,
            calibration: model.calibration,
            frame: FrameState::new(p),
            field,
            tau,
            sky,
            aerial_light,
            aerial_t,
            sampler,
            sky_uniform,
            frame_uniform,
            aerial_uniform,
            mapping,
            sky_program,
            aerial_program,
            initialized: false,
            scratch: None,
            resident_bytes,
        })
    }

    /// 记录 optical depth 和八轮多次散射，不提交队列；每批四个高度，scratch 显式复用。
    /// 各轮参数独立；仅编译六个初始化程序，后续批次复用程序并绑定独立 descriptor。
    /// # Errors
    /// 重复初始化、资源分配、上传、pipeline 或录制失败时返回错误。
    pub fn initialize(&mut self, context: &mut Context) -> Result<Recorded, Error> {
        if self.initialized {
            return Err(Error::AlreadyInitialized);
        }
        let p = self.base;
        let extra = Bank::new(context, &p)?;
        let count = 4 * u64::from(p.dims[1]) * u64::from(p.solve[0]);
        let incoming = Buffer::new(context, count * 16, false)?;
        let directions = Buffer::new(context, count * 16, false)?;
        let moments = Buffer::new(context, 4 * u64::from(p.dims[1]) * 7 * 16, false)?;
        let initial = upload(context, bytemuck::bytes_of(&p))?;
        let mut commands = Commands::new(context)?;
        let mut programs = std::array::from_fn(|_| None);
        dispatch(
            context,
            &mut commands,
            &mut programs,
            0,
            &[uniform(&initial), storage(&self.data), writable(&self.tau)],
            [p.dims[3].div_ceil(8), p.dims[2].div_ceil(8), 1],
        )?;
        dispatch(
            context,
            &mut commands,
            &mut programs,
            5,
            &[
                uniform(&initial),
                storage(&self.data),
                sampled(&self.tau),
                Binding::Sampler(self.sampler),
                storage(&directions),
                storage(&incoming),
                writable(&self.field.ground),
            ],
            [p.dims[1].div_ceil(64), 1, 1],
        )?;
        let mut uniforms = Vec::new();
        uniforms.try_reserve_exact(84)?;
        uniforms.push(initial);
        let banks = [&self.field, &extra];
        let mut current = 0;
        for iteration in 1..=8 {
            let next = 1 - current;
            for first in (0..p.dims[0]).step_by(4) {
                let mut params = p;
                params.batch = [first, 4.min(p.dims[0] - first), 0, 0];
                params.solve[3] = iteration;
                let u = upload(context, bytemuck::bytes_of(&params))?;
                let previous = banks[current];
                let target = banks[next];
                let states = params.batch[1] * p.dims[1];
                dispatch(
                    context,
                    &mut commands,
                    &mut programs,
                    1,
                    &[uniform(&u), storage(&self.data), storage(&directions)],
                    [p.solve[0].div_ceil(64), states, 1],
                )?;
                dispatch(
                    context,
                    &mut commands,
                    &mut programs,
                    2,
                    &[
                        uniform(&u),
                        storage(&self.data),
                        sampled(&self.tau),
                        sampled(&previous.source),
                        sampled(&previous.mean),
                        sampled(&previous.ground),
                        Binding::Sampler(self.sampler),
                        storage(&directions),
                        storage(&incoming),
                        sampled(&previous.high),
                    ],
                    [p.solve[0].div_ceil(64), states, 1],
                )?;
                dispatch(
                    context,
                    &mut commands,
                    &mut programs,
                    3,
                    &[
                        uniform(&u),
                        storage(&directions),
                        storage(&incoming),
                        storage(&moments),
                        writable(&target.high),
                        writable(&target.mean),
                    ],
                    [states.div_ceil(64), 1, 1],
                )?;
                dispatch(
                    context,
                    &mut commands,
                    &mut programs,
                    4,
                    &[
                        uniform(&u),
                        storage(&self.data),
                        writable(&target.source),
                        sampled(&target.mean),
                        storage(&directions),
                        storage(&incoming),
                        storage(&moments),
                    ],
                    [
                        (p.dims[1] * p.solve[1]).div_ceil(8),
                        (p.offsets1[1].saturating_sub(first).min(params.batch[1]) * p.solve[2])
                            .div_ceil(8),
                        1,
                    ],
                )?;
                dispatch(
                    context,
                    &mut commands,
                    &mut programs,
                    5,
                    &[
                        uniform(&u),
                        storage(&self.data),
                        sampled(&self.tau),
                        Binding::Sampler(self.sampler),
                        storage(&directions),
                        storage(&incoming),
                        writable(&target.ground),
                    ],
                    [p.dims[1].div_ceil(64), 1, 1],
                )?;
                uniforms.push(u);
            }
            current = next;
        }
        let bytes = Bank::bytes(&p)
            + count * 32
            + 4 * u64::from(p.dims[1]) * 7 * 16
            + uniforms.len() as u64 * 400;
        uniforms.extend([incoming, directions, moments]);
        let recorded = commands.finish(context)?;
        self.scratch = Some(Scratch {
            _bank: extra,
            _buffers: uniforms,
            bytes,
        });
        self.initialized = true;
        Ok(recorded)
    }

    /// 释放初始化 scratch；宿主须先确认初始化完成，不等待或修改最终散射场。
    pub fn release_initialization(&mut self) {
        self.scratch = None;
    }

    /// 校验视图并写入参数；须先由 Context 取消或完成上次使用这些 buffer 的命令。
    /// 太阳 azimuth 复用轴对称 SkyView；高度/elevation 使 SkyView 失效，视锥等仅影响 Aerial。
    /// # Errors
    /// 未初始化、输入无效或参数仍被待执行/在途命令使用时返回错误。
    pub fn prepare(&mut self, context: &mut Context, view: View) -> Result<Update, Error> {
        if !self.initialized {
            return Err(Error::NotInitialized);
        }
        let mut frame = self.frame.clone();
        let previous = frame.pending;
        let update = frame.prepare(view, &self.calibration)?;
        if !update.sky && !update.aerial && !previous.sky && !previous.aerial {
            self.frame = frame;
            return Ok(update);
        }
        let mut sky_parameters = frame.parameters;
        sky_parameters.sun[0] = 0.0;
        let aerial_parameters = frame.view.expect("prepare 已设置视图").aerial_parameters();
        let mut writes = Vec::with_capacity(4);
        if update.sky || previous.sky {
            writes.push((&self.sky_uniform, 0, bytemuck::bytes_of(&sky_parameters)));
            writes.push((&self.mapping, 0, bytemuck::cast_slice(&frame.mapping)));
        }
        if update.aerial || previous.aerial {
            writes.push((
                &self.frame_uniform,
                0,
                bytemuck::bytes_of(&frame.parameters),
            ));
            writes.push((
                &self.aerial_uniform,
                0,
                bytemuck::cast_slice(&aerial_parameters),
            ));
        }
        Buffer::write_batch(context, &writes)?;
        self.frame = frame;
        Ok(update)
    }

    /// 记录最新 prepare 的更新；稳态返回 None，不提交队列或推进 CPU cache key。
    /// # Errors
    /// 未初始化、范围或资源使用约束失败时返回错误。
    pub fn record(&mut self, context: &mut Context) -> Result<Option<Recorded>, Error> {
        if !self.initialized {
            return Err(Error::NotInitialized);
        }
        let update = self.frame.pending;
        if !update.sky && !update.aerial {
            return Ok(None);
        }
        let mut commands = Commands::new(context)?;
        if update.sky {
            commands.dispatch(
                self.sky_program,
                [SKY_SIZE.div_ceil(8), SKY_SIZE.div_ceil(8), 1],
            )?;
            commands.barrier();
        }
        if update.aerial {
            commands.dispatch(self.aerial_program, [AERIAL_SIZE.div_ceil(4); 3])?;
            commands.barrier();
        }
        let recorded = commands.finish(context)?;
        Ok(Some(recorded))
    }

    /// 对应 record 已被宿主实际提交后推进 cache key；取消未提交命令时不要调用。
    pub fn submitted(&mut self) {
        self.frame.commit();
    }

    /// 当前场、视图与初始化 scratch 的发布根；命令依赖由 Context 另外自动保留。
    pub fn roots(&self) -> impl Iterator<Item = Resource> + '_ {
        let fixed = [
            self.data.into(),
            self.field.source.into(),
            self.field.mean.into(),
            self.field.ground.into(),
            self.field.high.into(),
            self.tau.into(),
            self.sky.into(),
            self.aerial_light.into(),
            self.aerial_t.into(),
            self.sampler.into(),
            self.sky_uniform.into(),
            self.frame_uniform.into(),
            self.aerial_uniform.into(),
            self.mapping.into(),
            self.sky_program.into(),
            self.aerial_program.into(),
        ];
        let scratch = self.scratch.iter().flat_map(|s| {
            [
                s._bank.source.into(),
                s._bank.mean.into(),
                s._bank.ground.into(),
                s._bank.high.into(),
            ]
            .into_iter()
            .chain(s._buffers.iter().copied().map(Resource::from))
        });
        fixed.into_iter().chain(scratch)
    }

    /// 借用现有四波长模型与最终散射场，为同设备的地形体积光建立静态绑定。
    /// # Errors
    /// 设备不是本所有者的设备、参数缓冲分配或上传失败时返回错误。
    pub fn terrain_model(
        &self,
        context: &mut Context,
    ) -> Result<crate::terrain::TerrainModel, Error> {
        crate::terrain::TerrainModel::new(
            context,
            &self.base,
            [
                storage(&self.data),
                sampled(&self.tau),
                sampled(&self.field.source),
                sampled(&self.field.mean),
                sampled(&self.field.ground),
                Binding::Sampler(self.sampler),
                sampled(&self.field.high),
            ],
        )
    }

    /// 借用供积分器使用的原生资源，GPU 内容遵守模块的提交和复用约定。
    pub const fn bindings(&self) -> Bindings<'_> {
        Bindings {
            optical_depth: &self.tau,
            sky: &self.sky,
            aerial_light: &self.aerial_light,
            aerial_transmittance: &self.aerial_t,
            sampler: &self.sampler,
            sky_mapping: &self.mapping,
            parameters: &self.frame_uniform,
        }
    }
    /// 最新 prepare 的 400 B CPU 镜像，raw words 可供其他 storage ABI 使用。
    pub const fn parameters(&self) -> &Parameters {
        &self.frame.parameters
    }
    /// 最新 SkyView mapping 的 CPU 镜像；首次 prepare 前尚未生成。
    pub const fn sky_mapping(&self) -> &[[f32; 4]; MAPPING_ROWS] {
        &self.frame.mapping
    }
    /// 当前资源 payload；不含 descriptor、pipeline、驱动分配粒度和外部消费者。
    pub fn memory(&self) -> Memory {
        Memory {
            resident_bytes: self.resident_bytes,
            initialization_bytes: self.scratch.as_ref().map_or(0, |s| s.bytes),
            cpu_bytes: std::mem::size_of::<Self>() as u64
                + self.scratch.as_ref().map_or(0, |s| {
                    (s._buffers.capacity() * std::mem::size_of::<Buffer>()) as u64
                }),
        }
    }
}
