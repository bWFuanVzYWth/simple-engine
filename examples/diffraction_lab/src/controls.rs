//! 参数控件和值域。数值可拖动，也可点选读数直接输入。
use crate::recipe::{Channels, Lamp, Parameters, Precision, Recipe, Shape, View};
use simple_engine::{fft_bloom::pupil::Edge, render2d::Rect};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    BladeCount,
    Curvature,
    Rotation,
    Strength,
    Frequency,
    Octaves,
    Lacunarity,
    Persistence,
    IrisSeed,
    DustCount,
    DustMean,
    DustSigma,
    DustTransmission,
    DustSeed,
    FiberCount,
    FiberWidth,
    FiberMean,
    FiberSigma,
    FiberTransmission,
    Opd,
    FiberSeed,
    FNumber,
    Pitch,
    Temperature,
    Wavelength,
    SunRadius,
    Exposure,
}
impl Field {
    pub const fn is_seed(self) -> bool {
        matches!(self, Self::IrisSeed | Self::DustSeed | Self::FiberSeed)
    }
    pub const fn spec(self) -> (&'static str, f64, f64, f64) {
        use Field::*;
        match self {
            BladeCount => ("叶片数", 3., 32., 1.),
            Curvature => ("圆弧弓高比例", 0., 1., 0.005),
            Rotation => ("开口转角  /  °", 0., 360., 0.5),
            Strength => ("边缘扰动幅度", 0., 0.30, 0.001),
            Frequency => ("初层噪声频率", 0.25, 12., 0.05),
            Octaves => ("倍频层数", 1., 8., 1.),
            Lacunarity => ("频率倍率", 1., 3., 0.01),
            Persistence => ("振幅倍率", 0., 1., 0.01),
            IrisSeed => ("瞳孔种子", 0., u32::MAX as f64, 1.),
            DustCount => ("颗粒数", 0., 1024., 1.),
            DustMean => ("半径均值  /  D", 0.0002, 0.15, 0.0001),
            DustSigma => ("半径标准差  /  D", 0., 0.15, 0.0001),
            DustTransmission => ("振幅透射率", 0., 1., 0.01),
            DustSeed => ("灰尘种子", 0., u32::MAX as f64, 1.),
            FiberCount => ("纤维数", 0., 1024., 1.),
            FiberWidth => ("统一宽度  /  D", 0.0002, 0.03, 0.00001),
            FiberMean => ("长度均值  /  D", 0.0002, 1., 0.0001),
            FiberSigma => ("长度标准差  /  D", 0., 0.5, 0.0001),
            FiberTransmission => ("振幅透射率", 0., 1., 0.01),
            Opd => ("额外光程  /  nm", 0., 300., 0.5),
            FiberSeed => ("纤维种子", 0., u32::MAX as f64, 1.),
            FNumber => ("光圈数  /  ƒ", 2., 16., 0.1),
            Pitch => ("像素间距  /  μm", 2., 8., 0.1),
            Temperature => ("色温  /  K", 1500., 15000., 10.),
            Wavelength => ("波长  /  nm", 360., 830., 1.),
            SunRadius => ("太阳盘半径  /  px", 1., 32., 1.),
            Exposure => ("预览曝光  /  EV", -16., 40., 0.25),
        }
    }
    pub fn value(self, r: &Recipe) -> f64 {
        use Field::*;
        let p = &r.parameters;
        match self {
            BladeCount => f64::from(p.blades.count),
            Curvature => p.blades.curvature,
            Rotation => p.blades.rotation_deg,
            Strength => p.iris.strength,
            Frequency => p.iris.frequency,
            Octaves => f64::from(p.iris.octaves),
            Lacunarity => p.iris.lacunarity,
            Persistence => p.iris.persistence,
            IrisSeed => f64::from(p.iris.seed),
            DustCount => f64::from(p.dust.count),
            DustMean => p.dust.radius_mean,
            DustSigma => p.dust.radius_sigma,
            DustTransmission => p.dust.transmission,
            DustSeed => f64::from(p.dust.seed),
            FiberCount => f64::from(p.fibers.count),
            FiberWidth => p.fibers.width,
            FiberMean => p.fibers.length_mean,
            FiberSigma => p.fibers.length_sigma,
            FiberTransmission => p.fibers.transmission,
            Opd => p.fibers.opd_nm,
            FiberSeed => f64::from(p.fibers.seed),
            FNumber => p.f_number,
            Pitch => p.pitch_um,
            Temperature => p.temperature,
            Wavelength => p.wavelength,
            SunRadius => f64::from(p.sun_radius),
            Exposure => f64::from(r.exposure),
        }
    }
    pub fn set(self, r: &mut Recipe, value: f64) {
        use Field::*;
        let (_, lo, hi, step) = self.spec();
        let v = ((value / step).round() * step).clamp(lo, hi);
        let p = &mut r.parameters;
        match self {
            BladeCount => p.blades.count = v as u32,
            Curvature => p.blades.curvature = v,
            Rotation => p.blades.rotation_deg = v,
            Strength => p.iris.strength = v,
            Frequency => p.iris.frequency = v,
            Octaves => p.iris.octaves = v as u32,
            Lacunarity => p.iris.lacunarity = v,
            Persistence => p.iris.persistence = v,
            IrisSeed => p.iris.seed = v as u32,
            DustCount => p.dust.count = v as u32,
            DustMean => p.dust.radius_mean = v,
            DustSigma => p.dust.radius_sigma = v,
            DustTransmission => p.dust.transmission = v,
            DustSeed => p.dust.seed = v as u32,
            FiberCount => p.fibers.count = v as u32,
            FiberWidth => {
                p.fibers.width = v;
                p.fibers.length_mean = p.fibers.length_mean.max(v);
            }
            FiberMean => p.fibers.length_mean = v.max(p.fibers.width),
            FiberSigma => p.fibers.length_sigma = v,
            FiberTransmission => p.fibers.transmission = v,
            Opd => p.fibers.opd_nm = v,
            FiberSeed => p.fibers.seed = v as u32,
            FNumber => p.f_number = v,
            Pitch => p.pitch_um = v,
            Temperature => p.temperature = v,
            Wavelength => p.wavelength = v,
            SunRadius => p.sun_radius = v as u32,
            Exposure => r.exposure = v as f32,
        }
    }
    pub fn format(self, r: &Recipe) -> String {
        let v = self.value(r);
        let step = self.spec().3;
        let digits = if step >= 1. {
            0
        } else {
            (-step.log10()).ceil() as usize
        };
        format!("{v:.digits$}")
    }
}
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Layer {
    Base,
    Dust,
    Fibers,
}
impl Layer {
    pub const fn enabled(self, p: &Parameters) -> bool {
        match self {
            Self::Base => p.base_enabled,
            Self::Dust => p.dust_enabled,
            Self::Fibers => p.fibers_enabled,
        }
    }
    pub fn set(self, p: &mut Parameters, enabled: bool) {
        match self {
            Self::Base => p.base_enabled = enabled,
            Self::Dust => p.dust_enabled = enabled,
            Self::Fibers => p.fibers_enabled = enabled,
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Control {
    Tab(usize),
    Shape(Shape),
    Light(Lamp),
    Channels(Channels),
    Precision(Precision),
    Size(u32),
    FieldOfView(u32),
    Slider(Field),
    Edit(Field),
    Seed(Field),
    Enable(Layer, bool),
    BladeEdge(Edge),
    View(View),
    Zoom(f32),
    Open,
    Save,
    Export,
    Cancel,
    Mask,
    Reset,
}
#[derive(Clone, Copy)]
pub(crate) struct Hit {
    pub rect: Rect,
    pub action: Control,
}
pub(crate) struct Model {
    pub recipe: Recipe,
    pub tab: usize,
    pub scroll: f32,
    pub hits: Vec<Hit>,
    pub cursor: simple_engine::render2d::Point,
    pub drag: Option<(Field, Rect)>,
    pub edit: Option<(Field, String)>,
    pub replace_text: bool,
    pub preview_status: String,
    pub file_status: String,
    pub exporting: bool,
    pub preview: Option<crate::preview::Preview>,
    pub screen_rect: Rect,
    pub image_rect: Rect,
    pub max_scroll: f32,
}
impl Model {
    pub fn new(recipe: Recipe) -> Self {
        Self {
            recipe,
            tab: 0,
            scroll: 0.,
            hits: Vec::new(),
            cursor: Default::default(),
            drag: None,
            edit: None,
            replace_text: false,
            preview_status: "正在准备预览".into(),
            file_status: String::new(),
            exporting: false,
            preview: None,
            screen_rect: Rect::default(),
            image_rect: Rect::default(),
            max_scroll: 0.,
        }
    }
    pub fn active_layer(&self) -> Option<Layer> {
        [Layer::Base, Layer::Dust, Layer::Fibers]
            .get(self.tab)
            .copied()
    }
    pub fn reset_layer(&mut self) {
        let p = &mut self.recipe.parameters;
        match self.tab {
            0 => match p.shape {
                Shape::Blades => p.blades = Default::default(),
                Shape::Iris => p.iris = Default::default(),
                Shape::Combined => {
                    p.blades = Default::default();
                    p.iris = Default::default();
                }
                Shape::Custom => {}
            },
            1 => p.dust = Default::default(),
            2 => p.fibers = Default::default(),
            _ => {}
        }
    }
}
