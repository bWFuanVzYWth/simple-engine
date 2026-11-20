//! 控制面板、光谱图与像面读数。所有尺寸使用窗口逻辑像素。
use crate::{
    controls::{Control, Field, Hit, Layer, Model},
    recipe::{Channels, FIELDS, Lamp, Precision, Result, SIZES, Shape, View},
};
use simple_engine::{
    fft_bloom::calibration::Source,
    render2d::{
        Canvas, Color, DrawList, ImageKey, Point, Rect,
        text::{TextStyle, TextSystem},
    },
};

fn color(rgb: u32) -> Color {
    Color::srgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
}
const BG: u32 = 0x111519;
const PANEL: u32 = 0x1a2025;
const INSET: u32 = 0x11171b;
const LINE: u32 = 0x354048;
const TEXT: u32 = 0xe4eaec;
const MUTE: u32 = 0x99a9af;
const ACCENT: u32 = 0x9fe3cd;
const SELECT: u32 = 0x293f3b;
struct Paint<'a> {
    canvas: Canvas<'a>,
    text: &'a mut TextSystem,
    scale: f32,
    clip: Rect,
    hits: &'a mut Vec<Hit>,
    cursor: Point,
}
impl Paint<'_> {
    fn label(&mut self, x: f32, y: f32, s: &str, size: f32, ink: u32) -> Result<()> {
        if y + size + 6. < self.clip.y || y >= self.clip.bottom() {
            return Ok(());
        }
        let layout = self.text.layout(
            s,
            (self.clip.right() - x).max(1.),
            TextStyle {
                font_size: size,
                line_height: size + 5.,
                raster_scale: self.scale,
                wrap: false,
                ..Default::default()
            },
        )?;
        self.canvas.text(
            &self.text.resolve(&layout).expect("本轮刚排版"),
            Point::new(x, y),
            color(ink),
        );
        Ok(())
    }
    fn hit(&mut self, r: Rect, a: Control) {
        let r = r.intersection(self.clip);
        if !r.is_empty() {
            self.hits.push(Hit { rect: r, action: a });
        }
    }
    fn button(&mut self, r: Rect, title: &str, action: Control, active: bool) -> Result<()> {
        self.canvas.rounded_rect(
            r,
            4.,
            color(if active {
                SELECT
            } else if r.contains(self.cursor) {
                0x303b42
            } else {
                0x242c32
            }),
        );
        if active {
            self.canvas.border(r, 4., 1., color(0x4d7064));
        }
        let size = 12.;
        let label = self.text.layout(
            title,
            r.width,
            TextStyle {
                font_size: size,
                line_height: 17.,
                raster_scale: self.scale,
                wrap: false,
                ..Default::default()
            },
        )?;
        let w = label.advance_x()[1] - label.advance_x()[0];
        self.canvas.text(
            &self.text.resolve(&label).expect("本轮刚排版"),
            Point::new(
                r.x + ((r.width - w) * 0.5).max(3.),
                r.y + (r.height - 17.) / 2.,
            ),
            color(if active { ACCENT } else { TEXT }),
        );
        self.hit(r, action);
        Ok(())
    }
    fn group(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        choices: &[(&str, Control, bool)],
    ) -> Result<()> {
        let unit = (width + 4.) / choices.len() as f32;
        for (i, &(v, a, on)) in choices.iter().enumerate() {
            self.button(Rect::new(x + unit * i as f32, y, unit - 4., 29.), v, a, on)?;
        }
        Ok(())
    }
    fn slider(&mut self, x: f32, y: f32, w: f32, f: Field, m: &Model) -> Result<()> {
        let (name, lo, hi, _) = f.spec();
        let v = f.value(&m.recipe);
        self.label(x, y + 1., name, 12., MUTE)?;
        let editing = m.edit.as_ref().filter(|(e, _)| *e == f);
        let value = editing
            .map(|(_, s)| s.clone())
            .unwrap_or_else(|| f.format(&m.recipe));
        let box_w = if f.is_seed() { 110. } else { 62. };
        let number = Rect::new(x + w - box_w, y - 1., box_w, 22.);
        self.canvas.rounded_rect(number, 3., color(INSET));
        if editing.is_some() {
            self.canvas.border(number, 3., 1., color(ACCENT));
        }
        self.label(
            number.x + 7.,
            y,
            &value,
            12.,
            if editing.is_some() { ACCENT } else { TEXT },
        )?;
        self.hit(number, Control::Edit(f));
        if f.is_seed() {
            return Ok(());
        }
        let track = Rect::new(x, y + 29., w, 3.);
        self.canvas.fill_rect(track, color(LINE));
        let length = w * ((v - lo) / (hi - lo)).clamp(0., 1.) as f32;
        self.canvas
            .fill_rect(Rect::new(x, track.y, length, 3.), color(0x669d8b));
        self.canvas.rounded_rect(
            Rect::new(x + length - 3.5, track.y - 2., 7., 7.),
            3.5,
            color(ACCENT),
        );
        self.hit(Rect::new(x, y + 19., w, 23.), Control::Slider(f));
        Ok(())
    }
}
pub(crate) fn draw(
    m: &mut Model,
    text: &mut TextSystem,
    b: Rect,
    scale: f32,
    keys: [Option<ImageKey>; 2],
) -> Result<DrawList> {
    let mut scene = DrawList::new();
    let mut hits = Vec::new();
    text.begin_epoch([]);
    let left = 263.;
    let inspector = if b.width >= 1040. { 174. } else { 0. };
    let bar = 58.;
    let bottom = 32.;
    let top = bar + 18.;
    let footer = b.height - bottom;
    let mut paint = Paint {
        canvas: scene.canvas(b),
        text,
        scale,
        clip: b,
        hits: &mut hits,
        cursor: m.cursor,
    };
    paint.canvas.fill_rect(b, color(BG));
    paint
        .canvas
        .fill_rect(Rect::new(0., 0., b.width, bar), color(0x1c2329));
    paint
        .canvas
        .fill_rect(Rect::new(0., bar - 1., b.width, 1.), color(LINE));
    paint
        .canvas
        .rounded_rect(Rect::new(19., 17., 4., 24.), 2., color(ACCENT));
    paint.label(35., 13., "衍射核工作台", 21., TEXT)?;
    let is_exporting = m.exporting;
    for (i, (name, a, on, width)) in [
        ("打开参数", Control::Open, false, 82.),
        ("保存参数", Control::Save, false, 82.),
        (
            if is_exporting {
                "取消导出"
            } else {
                "导出 RGB 核"
            },
            if is_exporting {
                Control::Cancel
            } else {
                Control::Export
            },
            true,
            108.,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let x = b.width - 18. - [288., 198., 108.][i];
        paint.button(Rect::new(x, 14., width, 30.), name, a, on)?;
    }
    controls(&mut paint, m, left, bar, top, footer)?;
    let center = Rect::new(
        left + 14.,
        top,
        b.width - left - inspector - 32.,
        footer - top - 9.,
    );
    paint.label(center.x + 2., top + 2., "像面", 17., TEXT)?;
    let mode_x = center.right() - 143.;
    paint.group(
        mode_x,
        top,
        143.,
        &[
            (
                "点光源",
                Control::View(View::Point),
                m.recipe.view == View::Point,
            ),
            (
                "太阳盘",
                Control::View(View::Sun),
                m.recipe.view == View::Sun,
            ),
        ],
    )?;
    let viewport = Rect::new(
        center.x,
        top + 42.,
        center.width,
        (center.height - 111.).max(5.),
    );
    m.screen_rect = viewport;
    paint.canvas.fill_rect(viewport, color(0x090c0f));
    let size = viewport.width.min(viewport.height) * m.recipe.zoom;
    let image = Rect::new(
        viewport.x + (viewport.width - size) * 0.5,
        viewport.y + (viewport.height - size) * 0.5,
        size,
        size,
    );
    m.image_rect = image;
    {
        let mut img = paint.canvas.clipped(viewport);
        if let Some(k) = keys[0] {
            img.image(k, image, Rect::new(0., 0., 1., 1.), Color::WHITE);
        }
    }
    if keys[0].is_none() {
        paint.label(
            viewport.x + 18.,
            viewport.y + 16.,
            "正在计算点扩散函数…",
            13.,
            MUTE,
        )?;
    }
    paint.canvas.border(viewport, 1., 1., color(LINE));
    let toolbar = viewport.bottom() + 13.;
    paint.slider(
        center.x + 2.,
        toolbar,
        (center.width - 150.).clamp(104., 270.),
        Field::Exposure,
        m,
    )?;
    paint.group(
        center.right() - 132.,
        toolbar + 4.,
        132.,
        &[
            ("−", Control::Zoom((m.recipe.zoom / 2.).max(0.25)), false),
            (&format!("{}×", m.recipe.zoom), Control::Zoom(1.), true),
            ("+", Control::Zoom((m.recipe.zoom * 2.).min(8.)), false),
        ],
    )?;
    if inspector > 0. {
        inspect(
            &mut paint,
            m,
            b.width - inspector + 3.,
            top,
            inspector - 22.,
            keys[1],
        )?;
    }
    paint
        .canvas
        .fill_rect(Rect::new(0., footer, b.width, bottom), color(0x1c2329));
    paint
        .canvas
        .fill_rect(Rect::new(0., footer, b.width, 1.), color(LINE));
    paint.label(15., footer + 6., &m.preview_status, 12., ACCENT)?;
    let mut readout = m.file_status.clone();
    if viewport.contains(m.cursor)
        && image.contains(m.cursor)
        && let Some(v) = &m.preview
    {
        let sx = ((m.cursor.x - image.x) / image.width * v.side as f32) as usize;
        let sy = ((m.cursor.y - image.y) / image.height * v.side as f32) as usize;
        if sx < v.side as usize && sy < v.side as usize {
            let q = if m.recipe.view == View::Point {
                &v.point
            } else {
                &v.sun
            };
            let [r, g, b] = q[sy * v.side as usize + sx];
            readout = format!(
                "({:+}, {:+}) px     {:.3e}    {:.3e}    {:.3e}",
                sx as i32 - v.side as i32 / 2,
                sy as i32 - v.side as i32 / 2,
                r,
                g,
                b
            );
        }
    }
    paint.label(left + 14., footer + 6., &readout, 12., MUTE)?;
    m.hits = hits;
    Ok(scene)
}
fn controls(
    paint: &mut Paint<'_>,
    m: &mut Model,
    left: f32,
    bar: f32,
    top: f32,
    footer: f32,
) -> Result<()> {
    let sidebar = Rect::new(0., bar, left, footer - bar);
    paint.canvas.fill_rect(sidebar, color(PANEL));
    paint.group(
        14.,
        top,
        left - 28.,
        &[
            ("开口", Control::Tab(0), m.tab == 0),
            ("灰尘", Control::Tab(1), m.tab == 1),
            ("纤维", Control::Tab(2), m.tab == 2),
            ("采集", Control::Tab(3), m.tab == 3),
            ("输出", Control::Tab(4), m.tab == 4),
        ],
    )?;
    let content = Rect::new(0., top + 42., left, (footer - top - 42.).max(1.));
    let p = &m.recipe.parameters;
    let x = 17.;
    let width = left - 34.;
    {
        let mut u = Paint {
            canvas: paint.canvas.clipped(content),
            text: paint.text,
            scale: paint.scale,
            clip: content,
            hits: paint.hits,
            cursor: m.cursor,
        };
        let mut y = content.y + 5. - m.scroll;
        match m.active_layer() {
            Some(layer) => {
                y = layer_controls(&mut u, m, layer, x, y, width)?;
            }
            None if m.tab == 3 => {
                u.label(x, y, "入射光谱", 14., TEXT)?;
                y += 28.;
                u.group(
                    x,
                    y,
                    width,
                    &[
                        ("等能", Control::Light(Lamp::Equal), p.source == Lamp::Equal),
                        ("D65", Control::Light(Lamp::D65), p.source == Lamp::D65),
                        (
                            "黑体",
                            Control::Light(Lamp::Blackbody),
                            p.source == Lamp::Blackbody,
                        ),
                        ("谱线", Control::Light(Lamp::Line), p.source == Lamp::Line),
                    ],
                )?;
                y += 44.;
                if let Some(f) = match p.source {
                    Lamp::Blackbody => Some(Field::Temperature),
                    Lamp::Line => Some(Field::Wavelength),
                    _ => None,
                } {
                    u.slider(x, y, width, f, m)?;
                    y += 49.;
                }
                u.label(x, y, "预览响应", 14., TEXT)?;
                y += 26.;
                u.group(
                    x,
                    y,
                    width,
                    &[
                        (
                            "引擎 RGB",
                            Control::Channels(Channels::Rgb),
                            p.response == Channels::Rgb,
                        ),
                        (
                            "CIE 观察者",
                            Control::Channels(Channels::Observer),
                            p.response == Channels::Observer,
                        ),
                    ],
                )?;
                y += 50.;
                for f in [Field::FNumber, Field::Pitch, Field::SunRadius] {
                    u.slider(x, y, width, f, m)?;
                    y += 49.;
                }
            }
            _ => {
                u.label(x, y, "核边长  /  px", 14., TEXT)?;
                y += 28.;
                for part in SIZES.chunks(3) {
                    let labels = part.iter().map(u32::to_string).collect::<Vec<_>>();
                    let choices: Vec<_> = part
                        .iter()
                        .zip(&labels)
                        .map(|(&s, t)| (t.as_str(), Control::Size(s), s == p.side))
                        .collect();
                    u.group(x, y, width, &choices)?;
                    y += 35.;
                }
                y += 12.;
                u.label(x, y, "导出积分精度", 14., TEXT)?;
                y += 26.;
                u.group(
                    x,
                    y,
                    width,
                    &[
                        (
                            "快速",
                            Control::Precision(Precision::Fast),
                            p.quality == Precision::Fast,
                        ),
                        (
                            "标定",
                            Control::Precision(Precision::Calibrated),
                            p.quality == Precision::Calibrated,
                        ),
                    ],
                )?;
                y += 50.;
                u.label(x, y, "预览视场  /  px", 14., TEXT)?;
                y += 26.;
                u.group(
                    x,
                    y,
                    width,
                    &FIELDS.map(|v| {
                        (
                            match v {
                                193 => "193",
                                513 => "513",
                                _ => "1825",
                            },
                            Control::FieldOfView(v),
                            v == p.preview_side,
                        )
                    }),
                )?;
                y += 50.;
                u.label(x, y, "RGB32LE  ·  每通道单位通量", 12., MUTE)?;
                y += 30.;
            }
        }
        m.max_scroll = (y + m.scroll - content.bottom() + 10.).max(0.);
    }
    Ok(())
}

fn layer_controls(
    u: &mut Paint<'_>,
    m: &Model,
    layer: Layer,
    x: f32,
    mut y: f32,
    width: f32,
) -> Result<f32> {
    use simple_engine::fft_bloom::pupil::Edge;
    let p = &m.recipe.parameters;
    let title = match layer {
        Layer::Base => "基础开口",
        Layer::Dust => "圆形灰尘",
        Layer::Fibers => "矩形纤维",
    };
    u.label(x, y, title, 14., TEXT)?;
    y += 26.;
    let enabled = layer.enabled(p);
    u.group(
        x,
        y,
        width,
        &[
            ("开启", Control::Enable(layer, true), enabled),
            ("关闭", Control::Enable(layer, false), !enabled),
        ],
    )?;
    y += 44.;
    if !enabled {
        let label = if layer == Layer::Base {
            "基础开口关闭时使用理想圆孔"
        } else {
            "本层已关闭，参数保留"
        };
        u.label(x, y, label, 12., MUTE)?;
        y += 28.;
    }
    if layer == Layer::Base {
        let png = if p.mask.is_empty() {
            Control::Mask
        } else {
            Control::Shape(Shape::Custom)
        };
        u.group(
            x,
            y,
            width,
            &[
                (
                    "叶片",
                    Control::Shape(Shape::Blades),
                    p.shape == Shape::Blades,
                ),
                ("瞳孔", Control::Shape(Shape::Iris), p.shape == Shape::Iris),
                (
                    "叠加",
                    Control::Shape(Shape::Combined),
                    p.shape == Shape::Combined,
                ),
                ("PNG", png, p.shape == Shape::Custom),
            ],
        )?;
        y += 44.;
        if p.shape == Shape::Custom {
            u.button(
                Rect::new(x, y, width, 29.),
                "替换光瞳 PNG",
                Control::Mask,
                false,
            )?;
            return Ok(y + 43.);
        }
        if matches!(p.shape, Shape::Blades | Shape::Combined) {
            u.group(
                x,
                y,
                width,
                &[
                    (
                        "直边",
                        Control::BladeEdge(Edge::Straight),
                        p.blades.edge == Edge::Straight,
                    ),
                    (
                        "圆弧边",
                        Control::BladeEdge(Edge::Arc),
                        p.blades.edge == Edge::Arc,
                    ),
                ],
            )?;
            y += 44.;
            for f in [Field::BladeCount, Field::Rotation, Field::Curvature] {
                if f != Field::Curvature || p.blades.edge == Edge::Arc {
                    u.slider(x, y, width, f, m)?;
                    y += 48.;
                }
            }
        }
    }
    let fields: &[Field] = match layer {
        Layer::Base if matches!(p.shape, Shape::Iris | Shape::Combined) => {
            u.label(x, y, "圆周 Perlin fBm", 14., TEXT)?;
            y += 28.;
            &[
                Field::Strength,
                Field::Frequency,
                Field::Octaves,
                Field::Lacunarity,
                Field::Persistence,
                Field::IrisSeed,
            ]
        }
        Layer::Base => &[],
        Layer::Dust => &[
            Field::DustCount,
            Field::DustMean,
            Field::DustSigma,
            Field::DustTransmission,
            Field::DustSeed,
        ],
        Layer::Fibers => &[
            Field::FiberCount,
            Field::FiberWidth,
            Field::FiberMean,
            Field::FiberSigma,
            Field::FiberTransmission,
            Field::Opd,
            Field::FiberSeed,
        ],
    };
    for &f in fields {
        u.slider(x, y, width, f, m)?;
        y += if f.is_seed() { 31. } else { 48. };
    }
    if let Some(&seed) = fields.last().filter(|f| f.is_seed()) {
        u.group(
            x,
            y,
            width,
            &[
                ("更换种子", Control::Seed(seed), false),
                ("恢复本层参数", Control::Reset, false),
            ],
        )?;
    } else {
        u.button(
            Rect::new(x, y, width, 29.),
            "恢复本层参数",
            Control::Reset,
            false,
        )?;
    }
    Ok(y + 45.)
}

fn inspect(
    paint: &mut Paint<'_>,
    m: &Model,
    x: f32,
    top: f32,
    w: f32,
    key: Option<ImageKey>,
) -> Result<()> {
    paint.label(x, top + 5., "光瞳振幅", 14., TEXT)?;
    let side = w.min(143.);
    let rect = Rect::new(x, top + 43., side, side);
    paint.canvas.fill_rect(rect, color(INSET));
    if let Some(k) = key {
        paint
            .canvas
            .image(k, rect, Rect::new(0., 0., 1., 1.), Color::WHITE);
    }
    paint.canvas.border(rect, 1., 1., color(LINE));
    let mut y = rect.bottom() + 20.;
    paint.label(x, y, "入射光谱", 14., TEXT)?;
    y += 30.;
    let plot = Rect::new(x, y, w, 53.);
    paint.canvas.fill_rect(plot, color(INSET));
    if let Some(pv) = &m.preview {
        let bars = 64;
        let power: Vec<_> = (0..bars)
            .map(|i| {
                let nm = 360. + 470. * (i as f64 + 0.5) / bars as f64;
                match pv.source {
                    Source::Line(line) => {
                        if (nm - line).abs() <= 235. / bars as f64 {
                            1.
                        } else {
                            0.
                        }
                    }
                    s => s.relative_power(nm).unwrap_or(0.),
                }
            })
            .collect();
        let max = power.iter().copied().fold(0., f64::max).max(1e-20);
        let step = w / bars as f32;
        for (i, v) in power.into_iter().enumerate() {
            let h = (v / max) as f32 * (plot.height - 7.);
            let ink = [
                0x8c8bbc, 0x7599c9, 0x75b4be, 0x8dbf92, 0xc1b474, 0xc78b70, 0xad7673, 0x906775,
            ][i / 8];
            paint.canvas.fill_rect(
                Rect::new(x + i as f32 * step, plot.bottom() - h, step, h),
                color(ink),
            );
        }
    }
    y = plot.bottom() + 4.;
    paint.label(x, y, "360", 10., MUTE)?;
    paint.label(x + w - 40., y, "830 nm", 10., MUTE)?;
    y += 33.;
    if let Some(pv) = &m.preview {
        let [a, b, c] = pv.xyz;
        let total = a + b + c;
        for (title, value) in [
            (
                "光源色度 xy",
                format!("{:.4}   {:.4}", a / total, b / total),
            ),
            (
                "视场内通量",
                format!("{:.3}   {:.3}   {:.3}", pv.flux[0], pv.flux[1], pv.flux[2]),
            ),
            (
                if pv.observer { "中心 Y" } else { "中心 G" },
                format!("{:.7}", pv.point[pv.point.len() / 2][1]),
            ),
        ] {
            paint.label(x, y, title, 11., MUTE)?;
            paint.label(x, y + 18., &value, 13., TEXT)?;
            y += 50.;
        }
    }

    Ok(())
}
