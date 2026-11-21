//! 原生 Slang SPIR-V 泛光录制器。管线在设备准备时建立，视图按 FFT 尺寸复用频谱与工作区。

use super::{APERTURE_SIDE, Aperture, CORE_RADIUS, Kernel, Settings};
use crate::rect::{RectFilter, upload_kernel};
use vulkan::{
    interop::RayShader,
    native::{
        Binding, Bindings, Buffer, Commands, ComputePipeline, Context, Image, ImageFormat,
        ImagePasses, Resource,
    },
};

type AxisShaders<'a, const N: usize> = (u32, u32, [RayShader<'a>; N]);

/// 匹配的矩形 FFT 轴程序。由构建表产生，保证长度、工作组、频率排列与三个颜色的存储协议一致。
#[derive(Clone, Copy)]
pub struct RectPrograms<'a> {
    pub(crate) rows: &'a [AxisShaders<'a, 3>],
    pub(crate) columns: &'a [AxisShaders<'a, 2>],
}

impl<'a> RectPrograms<'a> {
    /// 按轴和长度的构建顺序枚举着色器，供核对实际执行的 SPIR-V 指纹。
    pub fn shaders(self) -> impl Iterator<Item = RayShader<'a>> {
        self.rows
            .iter()
            .flat_map(|(_, _, shaders)| *shaders)
            .chain(self.columns.iter().flat_map(|(_, _, shaders)| *shaders))
    }
}

include!(concat!(env!("OUT_DIR"), "/rect_programs.rs"));

/// 同一版本的 Slang 程序。普通块和宽块使用相同的线性卷积、颜色与补零约定。
#[derive(Clone, Copy)]
pub struct Programs<'a> {
    /// 256、512、1024 点变换。
    pub compact: FftPrograms<'a>,
    /// 4096 点变换，256 线程共用 16 KiB 组内存。
    pub wide: FftPrograms<'a>,
    /// 大光学核的矩形变换。传入 `None` 时用方形变换作为独立对照。
    pub rectangular: Option<RectPrograms<'a>>,
}

/// 一种 FFT 块尺寸的频谱准备与逐帧计算入口，须与对应资源布局配套。
#[derive(Clone, Copy)]
pub struct FftPrograms<'a> {
    /// `kernel_rows_main`，核的行正变换。
    pub kernel_rows: RayShader<'a>,
    /// `kernel_columns_main`，核的列正变换。
    pub kernel_columns: RayShader<'a>,
    /// `forward_rows_main`，提取高光并对行做正变换。
    pub forward_rows: RayShader<'a>,
    /// `filter_columns_main`，列正变换、复数频谱相乘与列逆变换。
    pub filter_columns: RayShader<'a>,
    /// `composite_rows_main`，行逆变换、裁去重叠边缘并叠加原图。
    pub composite_rows: RayShader<'a>,
}

/// GPU 回归与 4K benchmark 共用的生产泛光入口。
pub const PROGRAMS: Programs<'static> = Programs {
    rectangular: Some(RECT_PROGRAMS),
    compact: FftPrograms {
        kernel_rows: RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/kernel_rows.spv")),
            entry: c"kernel_rows_main",
        },
        kernel_columns: RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/kernel_columns.spv")),
            entry: c"kernel_columns_main",
        },
        forward_rows: RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/forward_rows.spv")),
            entry: c"forward_rows_main",
        },
        filter_columns: RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/filter_columns.spv")),
            entry: c"filter_columns_main",
        },
        composite_rows: RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/composite_rows.spv")),
            entry: c"composite_rows_main",
        },
    },
    wide: FftPrograms {
        kernel_rows: RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/wide_kernel_rows.spv")),
            entry: c"wide_kernel_rows_main",
        },
        kernel_columns: RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/wide_kernel_columns.spv")),
            entry: c"wide_kernel_columns_main",
        },
        forward_rows: RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/wide_forward_rows.spv")),
            entry: c"wide_forward_rows_main",
        },
        filter_columns: RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/wide_filter_columns.spv")),
            entry: c"wide_filter_columns_main",
        },
        composite_rows: RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/wide_composite_rows.spv")),
            entry: c"wide_composite_rows_main",
        },
    },
};

struct Parameters {
    radius: [u32; 2],
    size: u32,
    settings: Settings,
    center: [f32; 3],
}

impl Parameters {
    fn upload(
        &self,
        context: &mut Context,
        origin: [u32; 2],
        accumulate: bool,
    ) -> Result<Buffer, vulkan::Error> {
        let [rx, ry] = self.radius;
        let [threshold, intensity] = self.settings.to_array().map(f32::to_bits);
        let [r, g, b] = self.center.map(f32::to_bits);
        let words = [
            rx,
            ry,
            self.size,
            u32::from(self.settings.optical),
            threshold,
            intensity,
            r,
            g,
            b,
            origin[1],
            origin[0],
            u32::from(accumulate),
        ];
        Buffer::upload(context, words.map(u32::to_le_bytes).as_flattened())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Support {
    Full,
    Wings,
    Core,
}

struct Pass {
    spectrum: Image,
    parameters: Parameters,
    pipelines: [ComputePipeline; 3],
}

/// 设备级核频谱和管线。大光学核按位移拆成互不重叠的中心与远翼，分别以原始权重卷积。
/// 两个结果在场景线性空间相加，避免单精度 FFT 把太阳核心的舍入误差带入微弱远翼。
pub struct Filter {
    main: MainPass,
    core: Option<Pass>,
}

enum MainPass {
    Complex {
        pass: Pass,
        wide_work: Option<Image>,
    },
    Rectangular {
        filter: RectFilter,
        weights: Buffer,
    },
}

impl Filter {
    /// 放弃上次绑定的矩形频谱和工作图。已发布视图和在途命令按各自生命周期保活它们。
    /// 设备管线与标定权重可在重新启用时复用，下一次绑定按当时的视图尺寸准备。
    pub fn release_rectangle(&mut self) {
        if let MainPass::Rectangular { filter: rect, .. } = &mut self.main {
            rect.release_workspace();
        }
    }

    /// 同步准备管线与固定核。大光学核保留完整空间权重，所选矩形的频谱在绑定视图时准备。
    /// 函数返回后可以释放 CPU 光斑。
    /// # Errors
    /// 设备资源、SPIR-V、提交与等待错误逐层返回。未发布的资源随 Context 回收。
    pub fn new(
        context: &mut Context,
        programs: Programs<'_>,
        kernel: &Kernel,
        settings: Settings,
    ) -> Result<Self, vulkan::Error> {
        let [width, height] = kernel.extent();
        let wide = width.max(height) > 513;
        let split = wide && settings.optical;
        let placeholder = Image::new(context, [1; 3], ImageFormat::Rgba32Float)?;
        let rectangular = programs.rectangular.filter(|_| split);
        let main = if let Some(rectangular) = rectangular {
            let weights = upload_kernel(context, kernel)?;
            MainPass::Rectangular {
                weights,
                filter: RectFilter::new(
                    context,
                    rectangular,
                    [width / 2, height / 2],
                    placeholder,
                )?,
            }
        } else {
            let wide_work = if wide {
                if context.max_compute_group_width() < 256 {
                    return Err(vulkan::Error::Unsupported(
                        "宽复数 FFT 需要每组至少 256 个线程",
                    ));
                }
                Some(Image::new(
                    context,
                    [4098, 4096, 1],
                    ImageFormat::Rgba32Float,
                )?)
            } else {
                None
            };
            let pass = Pass::new(
                context,
                programs,
                kernel,
                settings,
                if split { Support::Wings } else { Support::Full },
                placeholder,
            )?;
            MainPass::Complex { pass, wide_work }
        };
        let core = if split {
            Some(Pass::new(
                context,
                programs,
                kernel,
                settings,
                Support::Core,
                placeholder,
            )?)
        } else {
            None
        };
        Ok(Self { main, core })
    }

    /// 绑定二维线性 HDR 输入与同尺寸输出。普通核按 128 MiB 预算分条带。
    /// 大光学核选择容纳有效像素的矩形，较大视图按重叠块执行。同一计划的视图在同一队列串行复用工作图。
    /// 跨计划只缓存最近一个矩形，旧视图的绑定与已提交命令分别保活其资源直到释放或完成。
    /// 每个视图独占输出。画外补黑，落在视图之外的光量自然丢失。
    /// # Errors
    /// 设备归属、尺寸溢出、分配及描述符绑定失败时返回错误。
    pub fn bind(&mut self, context: &mut Context, input: Image) -> Result<Bloom, vulkan::Error> {
        bind_passes(
            context,
            input,
            self.core.as_ref(),
            |context, output, passes| match &mut self.main {
                MainPass::Complex { pass, wide_work } => {
                    pass.bind(context, input, output, *wide_work, false, passes)?;
                    Ok((*wide_work, None))
                }
                MainPass::Rectangular { filter, weights } => {
                    let (work, extent) = filter.bind(context, input, output, passes, |_| {
                        Ok::<_, vulkan::Error>(*weights)
                    })?;
                    Ok((Some(work), Some(extent)))
                }
            },
        )
    }

    /// 设备回收时保留的频谱管线模板与所有视图共用的工作图。
    pub fn roots(&self) -> impl Iterator<Item = Resource> + '_ {
        let (pass, work, rect, weights) = match &self.main {
            MainPass::Complex { pass, wide_work } => (Some(pass), *wide_work, None, None),
            MainPass::Rectangular { filter, weights } => (None, None, Some(filter), Some(*weights)),
        };
        pass.into_iter()
            .chain(self.core.iter())
            .flat_map(|pass| pass.pipelines.map(Resource::from))
            .chain(work.map(Resource::from))
            .chain(rect.into_iter().flat_map(RectFilter::roots))
            .chain(weights.map(Resource::from))
    }
}

/// 可切换的标定镜头。同一设备共享矩形 FFT 管线，仅当前光瞳持有视图尺寸的频谱与工作图。
pub struct Lens {
    rect: RectFilter,
    cores: [Pass; 2],
    aperture: Aperture,
}

impl Lens {
    /// 预备各光瞳的中心频谱与管线，默认使用相机光圈。绑定视图时上传所选镜头的完整核。
    /// # Errors
    /// 缺少矩形程序、设备不支持所需 FFT 尺寸或资源准备失败时返回底层错误。
    pub fn new(context: &mut Context, programs: Programs<'_>) -> Result<Self, crate::Error> {
        let rectangular = programs
            .rectangular
            .ok_or(vulkan::Error::Unsupported("标定镜头需要矩形 FFT 程序"))?;
        let placeholder = Image::new(context, [1; 3], ImageFormat::Rgba32Float)?;
        let extent = [APERTURE_SIDE as u32; 2];
        let aperture = Aperture::default();
        let rect = RectFilter::new(context, rectangular, extent.map(|n| n / 2), placeholder)?;
        let mut core = |shape: Aperture| {
            let pixels = shape.core_rgb32le().as_chunks::<12>().0;
            Pass::from_weights(
                context,
                programs,
                [crate::aperture_layout::CORE_SIDE as u32; 2],
                |at| std::array::from_fn(|c| f32::from_le_bytes(pixels[at].as_chunks::<4>().0[c])),
                Settings::OPTICAL,
                Support::Core,
                placeholder,
            )
        };
        Ok(Self {
            rect,
            cores: [core(Aperture::Camera)?, core(Aperture::Pupil)?],
            aperture,
        })
    }

    /// 选择既有光瞳；形状变化时撤下矩形缓存并返回 true。已录制视图保有原频谱，须重新绑定才能使用新光瞳。
    pub fn select(&mut self, aperture: Aperture) -> bool {
        if self.aperture == aperture {
            return false;
        }
        self.rect.release_workspace();
        self.aperture = aperture;
        true
    }

    /// 撤下矩形工作区，保留光瞳选择和设备管线。旧视图与在途命令按各自的完成边界回收。
    pub fn release_rectangle(&mut self) {
        self.rect.release_workspace();
    }

    /// 绑定场景线性 HDR 输入，完整分辨率、黑色画外与输出所有权同 [`Filter::bind`]。
    /// # Errors
    /// 设备、图像、临时上传、矩形频谱准备或描述符绑定错误逐层返回。
    pub fn bind(&mut self, context: &mut Context, input: Image) -> Result<Bloom, crate::Error> {
        let core = match self.aperture {
            Aperture::Camera => &self.cores[0],
            Aperture::Pupil => &self.cores[1],
        };
        bind_passes(context, input, Some(core), |ctx, output, passes| {
            let (work, extent) = self.rect.bind(
                ctx,
                input,
                output,
                passes,
                |ctx| -> Result<_, crate::Error> {
                    Ok(Buffer::upload(ctx, &self.aperture.rgb32le()?)?)
                },
            )?;
            Ok((Some(work), Some(extent)))
        })
    }

    /// 回收时保留各光瞳的中心频谱、共用管线和当前矩形资源。
    pub fn roots(&self) -> impl Iterator<Item = Resource> + '_ {
        self.rect.roots().chain(
            self.cores
                .iter()
                .flat_map(|core| core.pipelines.map(Resource::from)),
        )
    }
}

fn bind_passes<E: From<vulkan::Error>>(
    context: &mut Context,
    input: Image,
    core: Option<&Pass>,
    main: impl FnOnce(
        &mut Context,
        Image,
        &mut Vec<Dispatch>,
    ) -> Result<(Option<Image>, Option<[u32; 2]>), E>,
) -> Result<Bloom, E> {
    let [width, height, depth] = input.raw(context)?.extent;
    if depth != 1 {
        return Err(vulkan::Error::InvalidArgument("泛光输入须为二维图像").into());
    }
    let output = Image::new(context, input.extent(), ImageFormat::Rgba32Float)?;
    let mut passes = Vec::new();
    // 任意两个可见像素的位移都落在中心区内时，远翼的贡献严格为零。
    let has_main = core.is_none() || width.max(height) > CORE_RADIUS + 1;
    let (work, fft_rectangle) = if has_main {
        main(context, output, &mut passes)?
    } else {
        (None, None)
    };
    if let Some(core) = core {
        core.bind(context, input, output, work, has_main, &mut passes)?;
    }
    Ok(Bloom {
        image: ImagePasses::new(output, passes),
        fft_rectangle,
    })
}

impl Pass {
    fn new(
        context: &mut Context,
        programs: Programs<'_>,
        kernel: &Kernel,
        settings: Settings,
        support: Support,
        placeholder: Image,
    ) -> Result<Self, vulkan::Error> {
        let channels = kernel.channels();
        Self::from_weights(
            context,
            programs,
            kernel.extent(),
            |at| channels.map(|ch| ch[at]),
            settings,
            support,
            placeholder,
        )
    }

    fn from_weights(
        context: &mut Context,
        programs: Programs<'_>,
        full: [u32; 2],
        weight: impl Fn(usize) -> [f32; 3],
        settings: Settings,
        support: Support,
        placeholder: Image,
    ) -> Result<Self, vulkan::Error> {
        let middle = full.map(|v| v / 2);
        let radius = if support == Support::Core {
            middle.map(|v| v.min(CORE_RADIUS))
        } else {
            middle
        };
        let [width, height] = radius.map(|v| 2 * v + 1);
        let (size, programs) = if width.max(height) <= 513 {
            (
                (2 * (width.max(height) - 1)).next_power_of_two().max(256),
                programs.compact,
            )
        } else {
            (4096, programs.wide)
        };
        let center = if support == Support::Wings {
            [0.; 3]
        } else {
            weight((middle[1] * full[0] + middle[0]) as usize)
        };
        let params = Parameters {
            radius,
            size,
            settings,
            center,
        };
        let parameters = params.upload(context, [0; 2], false)?;
        let mut spatial = Vec::new();
        spatial.try_reserve_exact((width * height) as usize)?;
        for y in 0..height {
            for x in 0..width {
                let empty = support == Support::Wings
                    && x.abs_diff(radius[0]).max(y.abs_diff(radius[1])) <= CORE_RADIUS;
                let index = (y + middle[1] - radius[1]) * full[0] + x + middle[0] - radius[0];
                spatial.push(if empty {
                    [[0; 4]; 3]
                } else {
                    weight(index as usize).map(f32::to_le_bytes)
                });
            }
        }
        let weights = Buffer::upload(context, spatial.as_flattened().as_flattened())?;
        let spectrum = Image::new(context, [size + 2, size, 1], ImageFormat::Rgba32Float)?;
        let bindings = Bindings::new(
            context,
            &[
                Binding::StorageImage(spectrum),
                Binding::StorageBuffer(parameters),
                Binding::StorageBuffer(weights),
            ],
        )?;
        let rows = ComputePipeline::new(context, bindings, programs.kernel_rows)?;
        let columns = ComputePipeline::new(context, bindings, programs.kernel_columns)?;
        let mut commands = Commands::new(context)?;
        commands.barrier();
        commands.dispatch(rows, [size, 1, 1])?;
        commands.barrier();
        commands.dispatch(columns, [size + 2, 1, 1])?;
        commands.barrier();
        commands.finish(context)?.submit_and_wait(context)?;
        let bindings = frame_bindings(
            context,
            spectrum,
            parameters,
            placeholder,
            placeholder,
            placeholder,
        )?;
        Ok(Self {
            spectrum,
            parameters: params,
            pipelines: [
                ComputePipeline::new(context, bindings, programs.forward_rows)?,
                ComputePipeline::new(context, bindings, programs.filter_columns)?,
                ComputePipeline::new(context, bindings, programs.composite_rows)?,
            ],
        })
    }

    fn bind(
        &self,
        context: &mut Context,
        input: Image,
        output: Image,
        shared: Option<Image>,
        accumulate: bool,
        passes: &mut Vec<Dispatch>,
    ) -> Result<(), vulkan::Error> {
        let [width, height, _] = input.extent();
        let size = self.parameters.size;
        let [sx, sy] = self.parameters.radius.map(|r| size - 2 * r);
        let nx = width.div_ceil(sx);
        let ny = height.div_ceil(sy);
        let work = if let Some(work) = shared {
            work
        } else {
            let work_width = nx
                .checked_mul(size + 2)
                .ok_or(vulkan::Error::InvalidArgument("泛光工作图过宽"))?;
            let row_bytes = u64::from(work_width) * u64::from(size) * 16;
            let rows = ((128 << 20) / row_bytes).clamp(1, u64::from(ny)) as u32;
            Image::new(
                context,
                [work_width, size * rows, 1],
                ImageFormat::Rgba32Float,
            )?
        };
        let [w, h, _] = work.extent();
        let batch_x = w / (size + 2);
        let batch_y = h / size;
        passes
            .try_reserve_exact(3 * nx.div_ceil(batch_x) as usize * ny.div_ceil(batch_y) as usize)?;
        let [rows, columns, composite] = self.pipelines;
        for top in (0..ny).step_by(batch_y as usize) {
            for left in (0..nx).step_by(batch_x as usize) {
                let tile_rows = batch_y.min(ny - top);
                let parameters =
                    self.parameters
                        .upload(context, [left * sx, top * sy], accumulate)?;
                let bindings =
                    frame_bindings(context, self.spectrum, parameters, input, work, output)?;
                let columns_in_strip = batch_x.min(nx - left);
                passes.extend([
                    (
                        rows.rebind(context, bindings)?,
                        [columns_in_strip, tile_rows * size, 1],
                    ),
                    (
                        columns.rebind(context, bindings)?,
                        [columns_in_strip * (size + 2), tile_rows, 1],
                    ),
                    (
                        composite.rebind(context, bindings)?,
                        [columns_in_strip, (height - top * sy).min(tile_rows * sy), 1],
                    ),
                ]);
            }
        }
        Ok(())
    }
}

fn frame_bindings(
    context: &mut Context,
    spectrum: Image,
    parameters: Buffer,
    input: Image,
    work: Image,
    output: Image,
) -> Result<Bindings, vulkan::Error> {
    Bindings::new(
        context,
        &[
            Binding::StorageImage(spectrum),
            Binding::StorageBuffer(parameters),
            Binding::SampledImage(input),
            Binding::StorageImage(work),
            Binding::StorageImage(output),
        ],
    )
}

pub(crate) type Dispatch = (ComputePipeline, [u32; 3]);

/// 单一输入视图的帧资源，绑定时固定各级管线、工作组与输出。
/// Context 回收时保留发布根，同队列按生产、泛光、消费顺序提交。
pub struct Bloom {
    image: ImagePasses,
    fft_rectangle: Option<[u32; 2]>,
}

impl Bloom {
    /// 为独立视图同步准备光斑与工作区。视图改变尺寸时可通过 [`Filter`] 复用准备结果。
    /// # Errors
    /// 核或视图的设备资源准备失败时返回底层错误。
    pub fn new(
        context: &mut Context,
        programs: Programs<'_>,
        input: Image,
        kernel: &Kernel,
        settings: Settings,
    ) -> Result<Self, vulkan::Error> {
        Filter::new(context, programs, kernel, settings)?.bind(context, input)
    }

    /// 合成结果的设备图像，首次帧命令执行完毕后有效，每次运行覆盖全部像素。
    pub const fn output(&self) -> Image {
        self.image.output()
    }

    /// 本视图远翼 FFT 的宽和高。紧凑核或通用复数路径返回 `None`。
    pub const fn fft_rectangle(&self) -> Option<[u32; 2]> {
        self.fft_rectangle
    }

    /// 所有计算阶段及其传递资源的发布根，保活输入、工作图和输出。
    pub fn roots(&self) -> impl Iterator<Item = Resource> + '_ {
        self.image.roots()
    }

    /// 按滤波器的生产顺序追加各计算阶段及其依赖屏障。提交、完成与回收由宿主负责。
    /// # Errors
    /// 命令属于其他设备或工作组数超过设备限制时返回错误，调用方须丢弃已追加前缀的命令。
    pub fn record_into(&self, commands: &mut Commands) -> Result<(), vulkan::Error> {
        self.image.record_into(commands)
    }
}

impl From<Bloom> for ImagePasses {
    fn from(view: Bloom) -> Self {
        view.image
    }
}
