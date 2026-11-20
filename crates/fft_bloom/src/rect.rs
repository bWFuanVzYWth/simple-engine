//! 实值光斑的矩形变换。设备所有者预备各轴程序，只缓存一个尺寸的传递函数与转置工作图。
//! 混合基变换在寄存器内处理相邻级，按组交换，列正变换、频乘与逆变换合为一次派发。
//! GPU 上控制全局访存次数的思路见 [VkFFT](https://github.com/DTolm/VkFFT)。

use crate::{
    Aperture, Kernel,
    gpu::{CORE_RADIUS, Dispatch, RectPrograms},
};
use vulkan::{
    Error,
    native::{
        Binding, Bindings, Buffer, Commands, ComputePipeline, Context, Image, ImageFormat, Resource,
    },
};

type Axis<const N: usize> = (u32, [ComputePipeline; N]);

#[derive(Clone, Copy)]
struct AxisPlan {
    length: u32,
    halo: u32,
    step: u32,
}

impl AxisPlan {
    fn select<const N: usize>(
        axes: &[Axis<N>],
        extent: u32,
        radius: u32,
    ) -> (Self, [ComputePipeline; N]) {
        let visible = radius.min(extent - 1);
        axes.iter()
            .filter_map(|&(length, pipelines)| {
                // 同尺寸输出的最大位移是 extent-1。整轴时只需在末端补 visible 个零；
                // 分块时两端各让出完整半径，所有相邻输出互不重叠。
                if length <= 2 * visible {
                    return None;
                }
                let (halo, step) = if length - visible >= extent {
                    (0, extent)
                } else {
                    (visible, length - 2 * visible)
                };
                let tiles = extent.div_ceil(step);
                Some((
                    (u64::from(length) * u64::from(tiles), tiles),
                    Self { length, halo, step },
                    pipelines,
                ))
            })
            .min_by_key(|(cost, _, _)| *cost)
            .map(|(_, plan, pipelines)| (plan, pipelines))
            .expect("滤波器已验证每轴至少有一个变换可容纳完整光斑")
    }
}

#[derive(Clone, Copy)]
struct Workspace {
    extent: [u32; 2],
    work: Image,
    transfer: Image,
}

pub(crate) struct RectFilter {
    rows: Vec<Axis<3>>,
    columns: Vec<Axis<2>>,
    weights: RectWeights,
    radius: [u32; 2],
    cached: Option<Workspace>,
}

pub(crate) enum RectWeights {
    Device(Buffer),
    Calibrated(Aperture),
}

impl RectWeights {
    pub(crate) fn upload(context: &mut Context, kernel: &Kernel) -> Result<Self, Error> {
        let channels = kernel.channels();
        let mut spatial = Vec::new();
        spatial.try_reserve_exact(channels[0].len())?;
        for i in 0..channels[0].len() {
            spatial.push(channels.map(|channel| channel[i].to_le_bytes()));
        }
        Ok(Self::Device(Buffer::upload(
            context,
            spatial.as_flattened().as_flattened(),
        )?))
    }
}

fn parameters(
    context: &mut Context,
    extent: [u32; 2],
    radius: [u32; 2],
    origin: [u32; 2],
    halo: [u32; 2],
    valid: [u32; 2],
    read_rows: [u32; 2],
) -> Result<Buffer, Error> {
    let words = [
        extent[0],
        extent[1],
        radius[0],
        radius[1],
        origin[0],
        origin[1],
        halo[0],
        halo[1],
        valid[0],
        valid[1],
        read_rows[0],
        read_rows[1],
        CORE_RADIUS,
    ];
    Buffer::upload(context, words.map(u32::to_le_bytes).as_flattened())
}

fn bindings(
    context: &mut Context,
    params: Buffer,
    work: Image,
    transfer: Image,
    input: Image,
    output: Image,
    weights: Buffer,
) -> Result<Bindings, Error> {
    Bindings::new(
        context,
        &[
            Binding::StorageBuffer(params),
            Binding::StorageImage(work),
            Binding::StorageImage(transfer),
            Binding::SampledImage(input),
            Binding::StorageImage(output),
            Binding::StorageBuffer(weights),
        ],
    )
}

impl RectFilter {
    pub(crate) fn release_workspace(&mut self) {
        self.cached = None;
    }

    pub(crate) fn select_aperture(&mut self, aperture: Aperture) {
        self.weights = RectWeights::Calibrated(aperture);
        self.release_workspace();
    }

    pub(crate) fn new(
        context: &mut Context,
        programs: RectPrograms<'_>,
        radius: [u32; 2],
        weights: RectWeights,
        placeholder: Image,
    ) -> Result<Self, Error> {
        let [rx, ry] = radius;
        let dummy = Buffer::upload(context, &[0; 52])?;
        let layout = bindings(
            context,
            dummy,
            placeholder,
            placeholder,
            placeholder,
            placeholder,
            dummy,
        )?;
        let threads = context.max_compute_group_width();
        let image_limit = context.max_image_dimension_2d();
        let mut prepare = |shader| ComputePipeline::new(context, layout, shader);
        let mut rows = Vec::new();
        rows.try_reserve_exact(programs.rows.len())?;
        for &(n, lanes, [kernel, forward, inverse]) in programs.rows {
            if lanes <= threads && 3 * n / 4 + 2 <= image_limit {
                rows.push((n, [prepare(kernel)?, prepare(forward)?, prepare(inverse)?]));
            }
        }
        let mut columns = Vec::new();
        columns.try_reserve_exact(programs.columns.len())?;
        for &(n, lanes, [kernel, filter]) in programs.columns {
            if lanes <= threads && n <= image_limit {
                columns.push((n, [prepare(kernel)?, prepare(filter)?]));
            }
        }
        if !rows.iter().any(|&(n, _)| n > 2 * rx) || !columns.iter().any(|&(n, _)| n > 2 * ry) {
            return Err(Error::Unsupported("设备支持的 FFT 矩形无法容纳光斑核"));
        }
        Ok(Self {
            rows,
            columns,
            weights,
            radius,
            cached: None,
        })
    }

    pub(crate) fn roots(&self) -> impl Iterator<Item = Resource> + '_ {
        self.rows
            .iter()
            .flat_map(|(_, p)| p.map(Resource::from))
            .chain(self.columns.iter().flat_map(|(_, p)| p.map(Resource::from)))
            .chain(match self.weights {
                RectWeights::Device(weights) => Some(weights.into()),
                RectWeights::Calibrated(_) => None,
            })
            .chain(
                self.cached
                    .into_iter()
                    .flat_map(|w| [w.work.into(), w.transfer.into()]),
            )
    }

    pub(crate) fn bind(
        &mut self,
        context: &mut Context,
        input: Image,
        output: Image,
        passes: &mut Vec<Dispatch>,
    ) -> Result<(Image, [u32; 2]), Error> {
        let [width, height, _] = input.extent();
        let (x, [kernel_row, forward, inverse]) =
            AxisPlan::select(&self.rows, width, self.radius[0]);
        let (y, [kernel_column, filter]) = AxisPlan::select(&self.columns, height, self.radius[1]);
        let extent = [x.length, y.length];
        let packed = 3 * x.length / 4 + 2;
        let kernel_rows = 2 * self.radius[1].min((y.length - 1) / 2) + 1;
        // 工作图已转置。整轴只存画面行与准备核用的有符号位移行；分块轴为内部块保留完整高度。
        // 中心核复用同一图像，宽度至少要容纳其 256 点变换以及 DC、Nyquist。
        let work_width = (if y.halo == 0 { height } else { y.length })
            .max(kernel_rows)
            .max(258);
        let cached = self.cached.filter(|w| w.extent == extent);
        let work = if let Some(w) = cached.filter(|w| w.work.extent()[0] >= work_width) {
            w.work
        } else {
            Image::new(
                context,
                [work_width, packed.max(256), 1],
                ImageFormat::Rgba32Float,
            )?
        };
        let transfer = if let Some(w) = cached {
            w.transfer
        } else {
            // 每列打包两路复数，保留完整纵向频率；实值输入只需横向非负频率。
            let transfer = Image::new(context, [y.length, packed, 1], ImageFormat::Rgba32Float)?;
            let params = parameters(context, extent, self.radius, [0; 2], [0; 2], [0; 2], [0; 2])?;
            // 校准核直接从随包字节上传，完成频谱准备后这份空间副本就能回收。
            let weights = match self.weights {
                RectWeights::Device(weights) => weights,
                RectWeights::Calibrated(aperture) => Buffer::upload(context, aperture.rgb32le())?,
            };
            let layout = bindings(context, params, work, transfer, input, output, weights)?;
            let rows = kernel_row.rebind(context, layout)?;
            let columns = kernel_column.rebind(context, layout)?;
            let mut commands = Commands::new(context)?;
            commands.barrier();
            commands.dispatch(rows, [kernel_rows, 1, 1])?;
            commands.barrier();
            commands.dispatch(columns, [packed, 1, 1])?;
            commands.barrier();
            commands.finish(context)?.submit_and_wait(context)?;
            transfer
        };

        passes.try_reserve_exact(
            3 * width.div_ceil(x.step) as usize * height.div_ceil(y.step) as usize,
        )?;
        for top in (0..height).step_by(y.step as usize) {
            let read_rows = [
                y.halo.saturating_sub(top),
                y.length.min((height - top).saturating_add(y.halo)),
            ];
            for left in (0..width).step_by(x.step as usize) {
                let valid = [(width - left).min(x.step), (height - top).min(y.step)];
                let params = parameters(
                    context,
                    extent,
                    self.radius,
                    [left, top],
                    [x.halo, y.halo],
                    valid,
                    read_rows,
                )?;
                let layout = bindings(context, params, work, transfer, input, output, params)?;
                passes.extend([
                    (
                        forward.rebind(context, layout)?,
                        [read_rows[1] - read_rows[0], 1, 1],
                    ),
                    (filter.rebind(context, layout)?, [packed, 1, 1]),
                    (inverse.rebind(context, layout)?, [valid[1], 1, 1]),
                ]);
            }
        }
        self.cached = Some(Workspace {
            extent,
            work,
            transfer,
        });
        Ok((work, extent))
    }
}
