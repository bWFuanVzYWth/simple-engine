//! 原生 Vulkan 离线制核。光瞳频谱与波长无关的部分只求一次，像素面积积分使用独立 16×16 格的局部和以保留暗翼。
//! 光学频率单位为孔径直径的倒数。纤维薄相位屏的光程相位按实际波长组合，保留偏心光瞳的两侧差异。

use crate::{
    calibration::{self, Plan, Response, Source},
    pupil::Pupil,
};
use std::f64::consts::{PI, TAU};
use vulkan::{
    interop::{RayShader, WgpuContext},
    native::{
        Binding, Bindings, Buffer, Commands, ComputePipeline, Context, Image, ImageFormat, Resource,
    },
};

include!(concat!(env!("OUT_DIR"), "/pupil_programs.rs"));
const PASSES: [RayShader<'static>; 5] = [
    RayShader {
        spirv: include_bytes!(concat!(env!("OUT_DIR"), "/interpolate_rows.spv")),
        entry: c"interpolate_rows",
    },
    RayShader {
        spirv: include_bytes!(concat!(env!("OUT_DIR"), "/integrate_cells.spv")),
        entry: c"integrate_cells",
    },
    RayShader {
        spirv: include_bytes!(concat!(env!("OUT_DIR"), "/project_pixels.spv")),
        entry: c"project_pixels",
    },
    RayShader {
        spirv: include_bytes!(concat!(env!("OUT_DIR"), "/square_pixels.spv")),
        entry: c"square_pixels",
    },
    RayShader {
        spirv: include_bytes!(concat!(env!("OUT_DIR"), "/rasterize_pupil.spv")),
        entry: c"rasterize_pupil",
    },
];

/// 计算精度。两档都在用户指定的原图像素上积分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// 每个传感器波段 12 个波长，适合参数编辑。
    Draft,
    /// 每带 96 个波长，中心区域使用每个光学频率单位 12 格的空间积分。
    Calibrated,
}
/// 单次制核输入。视场和最终支持域分别给出，便于仅预览中心区域。
#[derive(Debug, Clone, Copy)]
pub struct Request {
    /// 当前求解的半径，单位原图像素。
    pub radius: u32,
    /// 导出支持域的半径，至少为当前半径，至多 1024。
    pub support: u32,
    /// 光圈数，2–16。
    pub f_number: f64,
    /// 感光像素间距，2–8 µm。
    pub pitch_um: f64,
    /// 用于探测器积分的入射光。
    pub source: Source,
    /// 独立传感器 RGB 或 CIE XYZ。
    pub response: Response,
    /// 空间和波长的积分精度。
    pub quality: Quality,
}
/// 输入、设备与校准失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 数值、支持域或尺寸超出约定。
    #[error("探测器参数或核尺寸无效")]
    Domain,
    /// 用户取消当前任务。
    #[error("计算已取消")]
    Cancelled,
    /// 校准光谱无效。
    #[error(transparent)]
    Spectrum(#[from] calibration::Error),
    /// GPU 创建、录制或读取失败。
    #[error(transparent)]
    Device(#[from] vulkan::Error),
    /// 求解器检测到非有限的强度或零能量。
    #[error("积分核包含非有限强度或没有有效通量")]
    Flux,
    /// 无法保留主机缓冲。
    #[error(transparent)]
    Allocation(#[from] std::collections::TryReserveError),
}
/// 行优先的 RGB 或 XYZ 像面，中心格为零位移。
pub struct Frame {
    /// 每个像素三个线性权重，完整 RGB 核各通道能量和为一。
    pub pixels: Vec<[f32; 3]>,
    /// 边长，单位原图像素。
    pub side: u32,
    /// 求解所采用的入射光 XYZ，Y 为 1。
    pub source_xyz: [f64; 3],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OpticalGrid {
    kind: u32,
    side: u32,
    node_count: u32,
    dust_count: u32,
    fiber_count: u32,
    dust_transmission: f32,
    raster_radius: u32,
    density: f32,
    extent: [u32; 2],
    fft_side: u32,
    sampling: u32,
    origin: [i32; 2],
    cells: [u32; 2],
    screen: [u32; 2],
    end_screen: [u32; 2],
    first_fft_row: i32,
    fft_rows: u32,
    unused: [u32; 2],
}
const _: () = assert!(size_of::<OpticalGrid>() == 96);
#[derive(Clone, Copy)]
struct Work {
    pupil: Buffer,
    nodes: Buffer,
    field: Image,
    integral: Buffer,
    output: Buffer,
    coeff: Buffer,
    horizontal: Buffer,
    particles: Buffer,
    bins: Buffer,
}
impl Work {
    fn bind(self, ctx: &mut Context, params: Buffer) -> Result<Bindings, vulkan::Error> {
        use Binding::{StorageBuffer as B, StorageImage as I};
        Bindings::new(
            ctx,
            &[
                B(params),
                B(self.pupil),
                B(self.nodes),
                I(self.field),
                B(self.integral),
                B(self.output),
                B(self.coeff),
                B(self.horizontal),
                B(self.particles),
                B(self.bins),
            ],
        )
    }
}

/// 独占一台用于离线计算的设备，复用 Slang 管线。提交边界回收每批临时绑定。
pub struct Solver {
    device: WgpuContext,
    passes: [ComputePipeline; 5],
    fft: Vec<(u32, [ComputePipeline; 2])>,
    layout: Bindings,
    coeff: Buffer,
}
impl Solver {
    /// 接管专用设备并准备积分管线，FFT 变体首次使用时编译并复用。设备资源或程序不受支持时保留 Vulkan 错误。
    pub fn new(mut device: WgpuContext) -> Result<Self, Error> {
        let ctx = device.native();
        let b = Buffer::new(ctx, 96, false)?;
        let image = Image::new(ctx, [1, 1, 1], ImageFormat::Rg32Float)?;
        let coeff = Buffer::upload(ctx, bytemuck::cast_slice(&interpolation_weights()))?;
        let layout = Work {
            pupil: b,
            nodes: b,
            field: image,
            integral: b,
            output: b,
            coeff,
            horizontal: b,
            particles: b,
            bins: b,
        }
        .bind(ctx, b)?;
        let [a, b, c, d, e] = PASSES;
        let mut make = |s| ComputePipeline::new(ctx, layout, s);
        let passes = [make(a)?, make(b)?, make(c)?, make(d)?, make(e)?];
        Ok(Self {
            device,
            passes,
            fft: Vec::new(),
            layout,
            coeff,
        })
    }
    /// 计算真实光瞳并读回像面。进度回调在提交间运行，返回 false 取消，完成或失败后设备可复用。
    /// RGB 各通道按当前积分范围归一，XYZ 共用 Y。外圈以最终支持域的最后 32 像素收尾。
    pub fn solve(
        &mut self,
        pupil: &Pupil,
        request: Request,
        progress: impl FnMut(f32) -> bool,
    ) -> Result<Frame, Error> {
        let result = self.compute(pupil, request, progress);
        let roots = self.roots();
        self.device.native().collect(&roots)?;
        result
    }
    fn roots(&self) -> Vec<Resource> {
        self.passes
            .into_iter()
            .chain(self.fft.iter().flat_map(|p| p.1))
            .map(Resource::from)
            .chain([self.layout.into()])
            .collect()
    }
    fn fourier(&mut self, n: u32) -> Result<Option<[ComputePipeline; 2]>, Error> {
        if n == 1 {
            return Ok(None);
        }
        if let Some(&(_, pair)) = self.fft.iter().find(|v| v.0 == n) {
            return Ok(Some(pair));
        }
        let (_, entries) = FFT
            .iter()
            .find(|v| v.0 == n)
            .expect("受限光瞳 FFT 均有对应变体");
        let ctx = self.device.native();
        let pair = [
            ComputePipeline::new(ctx, self.layout, entries[0])?,
            ComputePipeline::new(ctx, self.layout, entries[1])?,
        ];
        self.fft.push((n, pair));
        Ok(Some(pair))
    }
    fn compute(
        &mut self,
        p: &Pupil,
        r: Request,
        mut progress: impl FnMut(f32) -> bool,
    ) -> Result<Frame, Error> {
        if r.radius > r.support
            || r.support > 1024
            || !(2.0..=16.).contains(&r.f_number)
            || !(2.0..=8.).contains(&r.pitch_um)
        {
            return Err(Error::Domain);
        }
        if !progress(0.) {
            return Err(Error::Cancelled);
        }
        let fine = r.quality == Quality::Calibrated;
        let count = match (r.response, fine) {
            (Response::Observer, true) => 144,
            (Response::Observer, false) => 36,
            (_, true) => 96,
            (_, false) => 12,
        };
        let plan = Plan::new(r.source, r.response, count)?;
        let mut nodes = Vec::new();
        let mut min = f64::INFINITY;
        let mut max = 0_f64;
        for s in &plan.samples {
            let scale = 1000. * r.pitch_um / (r.f_number * s.nm);
            min = min.min(scale);
            max = max.max(scale);
            let (sine, cosine) = (TAU * p.opd_nm / s.nm).sin_cos();
            let t = p.fiber_transmission;
            // |A + (t·exp(iφ)−1)D|² 的四项，A 是含灰尘的开口频谱，D 是纤维覆盖区域的频谱。
            nodes.push(
                [
                    scale,
                    s.weight[0],
                    s.weight[1],
                    s.weight[2],
                    1.,
                    1. + t * t - 2. * t * cosine,
                    2. * (t * cosine - 1.),
                    -2. * t * sine,
                ]
                .map(|v| v as f32),
            );
        }
        let native_mask = p.mask && p.particles.is_empty();
        let square = native_mask && p.extent == [1, 1];
        let extent = (f64::from(r.radius) + 0.5) * max;
        let density = if native_mask {
            p.extent[0].max(p.extent[1])
        } else {
            ((2. * extent).ceil() as u32)
                .next_power_of_two()
                .clamp(if fine { 2048 } else { 1024 }, 4096)
        };
        let fft_side = if square {
            1
        } else {
            (2 * density).next_power_of_two().max(128)
        };
        let fourier = self.fourier(fft_side)?;
        let mut keep = self.roots();
        let ctx = self.device.native();
        let inner = if fine { 12 } else { 4 };
        let units = if r.radius > 300
            && u64::from(ctx.max_storage_buffer_range()) >= 16 * (256 * inner as u64).pow(2)
        {
            256
        } else if r.radius > 128 {
            128
        } else {
            64
        };
        let groups = (extent / f64::from(units)).ceil().max(1.) as i32;
        let side = r.radius * 2 + 1;
        let tile = if square { 1 } else { units * inner };
        let max_rows = ((units * fft_side).div_ceil(density) + 9).min(fft_side);
        let field = Image::new(ctx, [fft_side, fft_side, 1], ImageFormat::Rg32Float)?;
        let square_axis = if square {
            square_weights(r, &plan)
        } else {
            Vec::new()
        };
        let work = Work {
            pupil: Buffer::upload(
                ctx,
                if square {
                    bytemuck::cast_slice(&square_axis)
                } else {
                    bytemuck::cast_slice(&p.data)
                },
            )?,
            nodes: Buffer::upload(ctx, bytemuck::cast_slice(&nodes))?,
            field,
            integral: Buffer::new(ctx, u64::from(tile).pow(2) * 16, false)?,
            output: Buffer::new(ctx, u64::from(side).pow(2) * 12, false)?,
            coeff: self.coeff,
            horizontal: Buffer::new(ctx, 16 * u64::from(tile) * u64::from(max_rows), false)?,
            particles: Buffer::upload(
                ctx,
                if p.particles.is_empty() {
                    &[0; 4]
                } else {
                    bytemuck::cast_slice(&p.particles)
                },
            )?,
            bins: Buffer::upload(ctx, bytemuck::cast_slice(&p.bins))?,
        };
        let params = OpticalGrid {
            kind: if native_mask { 2 } else { u32::from(p.mask) },
            side,
            node_count: nodes.len() as u32,
            dust_count: p.dust_count,
            fiber_count: p.particles.len() as u32 - p.dust_count,
            dust_transmission: p.dust_transmission,
            raster_radius: (p.outer_radius * density as f32).ceil() as u32 + 1,
            density: density as f32,
            extent: p.extent,
            fft_side,
            sampling: inner,
            origin: [0; 2],
            cells: [tile; 2],
            screen: [0; 2],
            end_screen: [side; 2],
            first_fft_row: 0,
            fft_rows: 0,
            unused: [0; 2],
        };
        let uniform = Buffer::upload(ctx, bytemuck::bytes_of(&params))?;
        let init = work.bind(ctx, uniform)?;
        keep.push(init.into());
        let mut cmd = Commands::new(ctx)?;
        if square {
            cmd.dispatch(
                self.passes[3].rebind(ctx, init)?,
                [side.div_ceil(16), side.div_ceil(16), 1],
            )?;
        } else {
            cmd.clear_buffer(work.output)?;
        }
        cmd.barrier();
        if let Some([row, col]) = fourier {
            if !native_mask {
                let tiles = (2 * params.raster_radius + 1).div_ceil(16);
                cmd.dispatch(self.passes[4].rebind(ctx, init)?, [tiles, tiles, 1])?;
                cmd.barrier();
            }
            cmd.dispatch(row.rebind(ctx, init)?, [fft_side, 1, 1])?;
            cmd.barrier();
            cmd.dispatch(col.rebind(ctx, init)?, [fft_side, 1, 1])?;
            cmd.barrier();
        }
        cmd.finish(ctx)?.submit_and_wait(ctx)?;
        let stripes = if square { 0 } else { groups };
        for y in -stripes..stripes {
            let mut cmd = Commands::new(ctx)?;
            for x in -groups..groups {
                let sampling = if (-1..=0).contains(&x) && (-1..=0).contains(&y) {
                    inner
                } else if fine {
                    4
                } else {
                    2
                };
                let cells = units * sampling;
                let origin = [x * units as i32, y * units as i32];
                let bounds = origin.map(|start| {
                    let a = f64::from(start);
                    let b = a + f64::from(units);
                    let offset = f64::from(r.radius);
                    let lo = (a / min).min(a / max) - 0.5 + offset;
                    let hi = (b / min).max(b / max) + 0.5 + offset;
                    [
                        (lo.floor() as i32).clamp(0, side as i32) as u32,
                        (hi.ceil() as i32 + 1).clamp(0, side as i32) as u32,
                    ]
                });
                let [[x0, x1], [y0, y1]] = bounds;
                if x0 == x1 || y0 == y1 {
                    continue;
                }
                let fft_scale = f64::from(fft_side) / f64::from(density);
                let first = (f64::from(origin[1]) * fft_scale).floor() as i32 - 3;
                let fft_rows =
                    (((f64::from(origin[1]) + f64::from(units)) * fft_scale).ceil() as i32 + 5
                        - first)
                        .min(fft_side as i32) as u32;
                let block = OpticalGrid {
                    sampling,
                    origin,
                    cells: [cells; 2],
                    screen: [x0, y0],
                    end_screen: [x1, y1],
                    first_fft_row: first,
                    fft_rows,
                    ..params
                };
                let buffer = Buffer::upload(ctx, bytemuck::bytes_of(&block))?;
                let bindings = work.bind(ctx, buffer)?;
                if fft_side > 1 {
                    cmd.dispatch(
                        self.passes[0].rebind(ctx, bindings)?,
                        [cells.div_ceil(16), fft_rows.div_ceil(16), 1],
                    )?;
                    cmd.barrier();
                }
                cmd.dispatch(
                    self.passes[1].rebind(ctx, bindings)?,
                    [cells / 16, cells / 16, 1],
                )?;
                cmd.barrier();
                cmd.dispatch(
                    self.passes[2].rebind(ctx, bindings)?,
                    [(x1 - x0).div_ceil(8), (y1 - y0).div_ceil(8), 1],
                )?;
                cmd.barrier();
            }
            cmd.finish(ctx)?.submit_and_wait(ctx)?;
            ctx.collect(&keep)?;
            if !progress((y + groups + 1) as f32 / (2 * groups) as f32) {
                return Err(Error::Cancelled);
            }
        }
        if !progress(1.) {
            return Err(Error::Cancelled);
        }
        let readback = Buffer::new(ctx, work.output.len(), true)?;
        work.output
            .copy_prefix_to(ctx, &readback, work.output.len())?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact((side * side) as usize)?;
        pixels.resize((side * side) as usize, [0_f32; 3]);
        readback.read(ctx, 0, bytemuck::cast_slice_mut(&mut pixels))?;
        normalize(&mut pixels, r)?;
        Ok(Frame {
            pixels,
            side,
            source_xyz: plan.source_xyz,
        })
    }
}
fn normalize(pixels: &mut [[f32; 3]], r: Request) -> Result<(), Error> {
    let side = r.radius * 2 + 1;
    let width = r.support.min(32);
    let mut mass = [0_f64; 3];
    for (y, row) in pixels.chunks_exact_mut(side as usize).enumerate() {
        for (x, p) in row.iter_mut().enumerate() {
            let radius = (x as u32)
                .abs_diff(r.radius)
                .max((y as u32).abs_diff(r.radius));
            let t = radius.saturating_sub(r.support - width);
            let window = if t > 0 {
                (0.5 + 0.5 * (PI * f64::from(t) / f64::from(width)).cos()) as f32
            } else {
                1.
            };
            for (k, s) in p.iter_mut().zip(&mut mass) {
                *k *= window;
                *s += f64::from(*k);
            }
        }
    }
    if r.response == Response::Observer {
        mass = [mass[1]; 3];
    }
    if mass.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return Err(Error::Flux);
    }
    let gain = mass.map(|v| (1. / v) as f32);
    for p in pixels {
        for (k, g) in p.iter_mut().zip(gain) {
            *k *= g;
        }
    }
    Ok(())
}
// 在光瞳支撑的频带内做八点最小二乘插值。局部频域插值保留复振幅的相位。
fn interpolation_weights() -> Vec<f32> {
    let sinc = |x: f64| {
        if x.abs() < 1e-12 {
            1.
        } else {
            (PI * x).sin() / (PI * x)
        }
    };
    let mut table = Vec::with_capacity(257 * 8);
    for phase in 0..=256 {
        let mut a = [[0.; 9]; 8];
        for (i, row) in a.iter_mut().enumerate() {
            for (j, v) in row[..8].iter_mut().enumerate() {
                *v = sinc(0.56 * (i as f64 - j as f64));
            }
            row[8] = sinc(0.56 * (i as f64 - 3. - f64::from(phase) / 256.));
        }
        for k in 0..8 {
            let d = a[k][k];
            for v in &mut a[k][k..] {
                *v /= d;
            }
            let pivot = a[k];
            for (i, row) in a.iter_mut().enumerate() {
                if i != k {
                    let d = row[k];
                    for j in k..9 {
                        row[j] -= d * pivot[j];
                    }
                }
            }
        }
        let sum = a.iter().map(|row| row[8]).sum::<f64>();
        table.extend(a.iter().map(|row| (row[8] / sum) as f32));
    }
    table
}
// 方孔强度可分离。对每段宽度不超过一个条纹的区间做八点 Gauss–Legendre 求积，避免远翼原函数相减丢失精度。
fn square_weights(r: Request, plan: &Plan) -> Vec<f32> {
    const HALF: [(f64, f64); 4] = [
        (0.1834346424956498, 0.362683783378362),
        (0.525532409916329, 0.3137066458778873),
        (0.7966664774136267, 0.2223810344533745),
        (0.9602898564975363, 0.1012285362903763),
    ];
    let mut axis = Vec::with_capacity((r.radius as usize + 1) * plan.samples.len());
    for s in &plan.samples {
        let scale = 1000. * r.pitch_um / (r.f_number * s.nm);
        let count = scale.ceil() as u32;
        let half = scale / (2. * f64::from(count));
        for pixel in 0..=r.radius {
            let mut sum = 0.;
            let left = scale * (f64::from(pixel) - 0.5);
            for j in 0..count {
                let mid = left + f64::from(2 * j + 1) * half;
                for (x, w) in HALF {
                    for t in [mid - half * x, mid + half * x] {
                        let v = PI * t;
                        let sinc = if v.abs() < 1e-10 { 1. } else { v.sin() / v };
                        sum += w * sinc * sinc;
                    }
                }
            }
            axis.push((half * sum) as f32);
        }
    }
    axis
}
