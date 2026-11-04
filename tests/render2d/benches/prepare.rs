//! 真实 GPU 准备链路；四阶段分别采样，每次迭代均提交并完成，避免积累 staging。
//!
//! 输入、设备、shader、目标和 pipeline 在计时外准备；shape_rebuild 包含场景重建。
//! glyph_cold 每次重建文字系统、布局和 renderer 后再计时，测 CPU raster 与 atlas miss，
//! 不包含字体解析／排版；glyph_warm 保留同一组缓存。静态包不重复 prepare。
//! complete 是宿主等待／回调成本，包含当时尚未完成的 GPU 工作，不是 GPU timestamp。
//! 各阶段独立运行完整链路，不能将独立统计分位数相加；total 直接测整段。
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use criterion::{BenchmarkId, Criterion};
use simple_engine::{
    gpu_test::{Capability, Gpu},
    render2d::{Color, DrawList, Point, Rect, gpu::*, text::*},
    vulkan::interop::WgpuContext,
    wgpu,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const FONT: &[u8] = include_bytes!("../../../assets/fonts/NotoSansSC-Regular.otf");
const EXTENT: [u32; 2] = [640, 360];
const BOUNDS: Rect = Rect::new(0.0, 0.0, 640.0, 360.0);
const CONFIG: TargetConfig = TargetConfig {
    format: wgpu::TextureFormat::Rgba8Unorm,
    samples: 1,
};
const SHAPES: usize = 2048;

#[derive(Clone, Copy)]
enum Case {
    Static,
    Rebuild,
    GlyphCold,
    GlyphWarm,
    ViewsInflight,
}

impl Case {
    const fn name(self) -> &'static str {
        match self {
            Self::Static => "static_reuse",
            Self::Rebuild => "shape_rebuild",
            Self::GlyphCold => "glyph_cold",
            Self::GlyphWarm => "glyph_warm",
            Self::ViewsInflight => "views4_inflight4",
        }
    }
    const fn frames(self) -> usize {
        if matches!(self, Self::ViewsInflight) {
            4
        } else {
            1
        }
    }
    const fn views(self) -> usize {
        if matches!(self, Self::ViewsInflight) {
            4
        } else {
            1
        }
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Prepare,
    EncodeSubmit,
    Complete,
    Total,
}

impl Phase {
    const fn name(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::EncodeSubmit => "encode_submit",
            Self::Complete => "complete",
            Self::Total => "total",
        }
    }
}

struct Sample {
    prepare: Duration,
    encode_submit: Duration,
    complete: Duration,
    total: Duration,
}

impl Sample {
    const fn phase(&self, phase: Phase) -> Duration {
        match phase {
            Phase::Prepare => self.prepare,
            Phase::EncodeSubmit => self.encode_submit,
            Phase::Complete => self.complete,
            Phase::Total => self.total,
        }
    }
}

struct Package {
    _geometry: Prepared2d,
    views: Vec<PreparedView2d>,
}

struct Fixture {
    renderer: Renderer2d,
    list: DrawList,
    text: Option<TextSystem>,
    targets: Vec<wgpu::TextureView>,
    packages: Vec<Package>,
}

fn renderer(context: &WgpuContext) -> Result<Renderer2d> {
    Ok(Renderer2d::new(
        context.device(),
        context.queue(),
        Shaders2d {
            vertex: context.shader(VERTEX_SPIRV, "vertex_main", [0; 3])?,
            fragment: context.shader(FRAGMENT_SPIRV, "fragment_main", [0; 3])?,
        },
        Budgets::default(),
    )?)
}

fn shapes(list: &mut DrawList) {
    list.clear();
    let mut canvas = list.canvas(BOUNDS);
    for index in 0..SHAPES {
        canvas.rounded_rect(
            Rect::new(
                (index % 64) as f32 * 10.0,
                (index / 64) as f32 * 10.0,
                9.0,
                9.0,
            ),
            2.0,
            Color::linear(0.2, 0.4, 0.8, 0.5),
        );
    }
}

impl Fixture {
    fn new(context: &WgpuContext, case: Case) -> Result<Self> {
        let targets = (0..case.views())
            .map(|_| {
                context
                    .device()
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("2D bench target"),
                        size: wgpu::Extent3d {
                            width: EXTENT[0],
                            height: EXTENT[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: CONFIG.format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            })
            .collect();
        let mut fixture = Self {
            renderer: renderer(context)?,
            list: DrawList::new(),
            text: None,
            targets,
            packages: Vec::with_capacity(case.frames()),
        };
        if matches!(case, Case::GlyphCold | Case::GlyphWarm) {
            fixture.glyphs()?;
        } else {
            shapes(&mut fixture.list);
        }
        fixture.prepare(case)?;
        fixture.submit_complete(context)?;
        if !matches!(case, Case::Static) {
            fixture.packages.clear();
        }
        Ok(fixture)
    }

    fn glyphs(&mut self) -> Result<()> {
        let mut text = TextSystem::new([FONT.to_vec()], TextBudgets::default())?;
        let body = "体素世界 voxel 0123456789，批量计算与独占输出。山川草木日月星辰。".repeat(4);
        let layout = text.layout(&body, BOUNDS.width, TextStyle::default())?;
        self.list.clear();
        self.list.canvas(BOUNDS).text(
            &text.resolve(&layout).unwrap(),
            Point::new(0.0, 0.0),
            Color::WHITE,
        );
        self.text = Some(text);
        Ok(())
    }

    fn cold(&mut self, context: &WgpuContext) -> Result<()> {
        self.renderer = renderer(context)?;
        // 先建立同格式 pipeline；空包不触碰 glyph/raster/atlas。
        let empty = self.renderer.prepare(&DrawList::new())?;
        let view = self
            .renderer
            .prepare_view(&empty, ViewParams::new(BOUNDS, EXTENT), CONFIG)?;
        drop((view, empty));
        let setup = context.queue().submit([]);
        wait(context, setup)?;
        self.glyphs()
    }

    fn prepare(&mut self, case: Case) -> Result<()> {
        for _ in 0..case.frames() {
            if matches!(case, Case::Rebuild | Case::ViewsInflight) {
                shapes(&mut self.list);
            }
            let prepared = match &mut self.text {
                Some(text) => self.renderer.prepare_text(black_box(&self.list), text)?,
                None => self.renderer.prepare(black_box(&self.list))?,
            };
            let views = (0..case.views())
                .map(|_| {
                    self.renderer
                        .prepare_view(&prepared, ViewParams::new(BOUNDS, EXTENT), CONFIG)
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            self.packages.push(Package {
                _geometry: prepared,
                views,
            });
        }
        Ok(())
    }

    fn submit(&mut self, context: &WgpuContext) -> Result<wgpu::SubmissionIndex> {
        let mut last = None;
        for package in &self.packages {
            let encoder = context.device().create_command_encoder(&Default::default());
            let mut recording = self.renderer.recording(encoder)?;
            for (view, target) in package.views.iter().zip(&self.targets) {
                recording.encode(
                    view,
                    RenderTarget {
                        view: target,
                        resolve: None,
                        extent: EXTENT,
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        timestamp_writes: None,
                    },
                )?;
            }
            let recorded = recording.finish()?;
            last = Some(self.renderer.submit(recorded)?);
        }
        last.ok_or_else(|| "bench 没有绘制包".into())
    }

    fn submit_complete(&mut self, context: &WgpuContext) -> Result<()> {
        let last = self.submit(context)?;
        wait(context, last)
    }

    fn step(&mut self, context: &WgpuContext, case: Case) -> Result<Sample> {
        if matches!(case, Case::GlyphCold) {
            self.cold(context)?;
        }
        let start = Instant::now();
        if !matches!(case, Case::Static) {
            self.prepare(case)?;
        }
        let prepared = Instant::now();
        let last = self.submit(context)?;
        let submitted = Instant::now();
        wait(context, last)?;
        let completed = Instant::now();
        if !matches!(case, Case::Static) {
            self.packages.clear();
        }
        Ok(Sample {
            prepare: prepared - start,
            encode_submit: submitted - prepared,
            complete: completed - submitted,
            total: start.elapsed(),
        })
    }
}

fn wait(context: &WgpuContext, submission: wgpu::SubmissionIndex) -> Result<()> {
    context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(Duration::from_secs(30)),
    })?;
    Ok(())
}

fn main() {
    let mut criterion = Criterion::default()
        .sample_size(20)
        .measurement_time(Duration::from_secs(2))
        .configure_from_args();
    let gpu = Gpu::new(Capability::Compute, 0, false).expect("初始化 2D bench GPU 失败");
    eprintln!(
        "GPU {:?}; 初始化不计时；保留量不含驱动/staging/Prepared CPU/第三方缓存",
        gpu.context().adapter_info()
    );
    {
        let context = gpu.context();
        let mut group = criterion.benchmark_group("render2d_chain");
        for case in [
            Case::Static,
            Case::Rebuild,
            Case::GlyphCold,
            Case::GlyphWarm,
            Case::ViewsInflight,
        ] {
            let mut fixture = Fixture::new(context, case).expect("建立 2D bench fixture 失败");
            for phase in [
                Phase::Prepare,
                Phase::EncodeSubmit,
                Phase::Complete,
                Phase::Total,
            ] {
                if matches!((case, phase), (Case::Static, Phase::Prepare)) {
                    continue;
                }
                group.bench_function(BenchmarkId::new(case.name(), phase.name()), |bench| {
                    bench.iter_custom(|iterations| {
                        let mut duration = Duration::ZERO;
                        for _ in 0..iterations {
                            duration += fixture
                                .step(context, case)
                                .expect("2D bench 链路失败")
                                .phase(phase);
                        }
                        duration
                    })
                });
            }
            eprintln!(
                "{}: shapes={} glyphs={} views={} inflight={} input_bytes={} target_bytes={} renderer={:?} text={:?}",
                case.name(),
                fixture.list.shapes().len(),
                fixture.list.glyphs().len(),
                case.views(),
                case.frames(),
                fixture.list.retained_bytes(),
                u64::from(EXTENT[0]) * u64::from(EXTENT[1]) * 4 * case.views() as u64,
                fixture.renderer.stats(),
                fixture.text.as_ref().map(TextSystem::stats)
            );
        }
        group.finish();
    }
    gpu.finish().expect("2D bench GPU 检查失败");
    criterion.final_summary();
}
