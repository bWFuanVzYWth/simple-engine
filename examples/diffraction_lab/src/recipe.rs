//! 可持久化的光瞳、探测器与预览设置。版本 2 将基础开口、灰尘与纤维独立保存，PNG 内嵌为 base64。
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use simple_engine::fft_bloom::{
    calibration::{Response, Source},
    diffraction::{Frame, Quality, Request},
    pupil::{Blades, Dust, Edge, Fibers, Iris, Opening, Pupil},
};
use std::{
    error::Error,
    fs,
    io::{BufWriter, Cursor, Write},
    path::Path,
};

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;
pub(crate) const SIZES: [u32; 6] = [129, 257, 513, 1025, 1825, 2049];
pub(crate) const FIELDS: [u32; 3] = [193, 513, 1825];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Shape {
    Blades,
    Iris,
    Combined,
    Custom,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Lamp {
    Equal,
    D65,
    Blackbody,
    Line,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Channels {
    Rgb,
    Observer,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Precision {
    Fast,
    Calibrated,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum View {
    Point,
    Sun,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Parameters {
    pub shape: Shape,
    pub base_enabled: bool,
    pub dust_enabled: bool,
    pub fibers_enabled: bool,
    pub blades: Blades,
    pub iris: Iris,
    pub dust: Dust,
    pub fibers: Fibers,
    pub f_number: f64,
    pub pitch_um: f64,
    pub source: Lamp,
    pub temperature: f64,
    pub wavelength: f64,
    pub response: Channels,
    pub preview_side: u32,
    pub sun_radius: u32,
    pub side: u32,
    pub quality: Precision,
    pub mask: String,
}
impl Default for Parameters {
    fn default() -> Self {
        Self {
            shape: Shape::Blades,
            base_enabled: true,
            dust_enabled: true,
            fibers_enabled: true,
            blades: Blades::default(),
            iris: Iris::default(),
            dust: Dust::default(),
            fibers: Fibers::default(),
            f_number: 4.,
            pitch_um: 5.,
            source: Lamp::Equal,
            temperature: 6500.,
            wavelength: 550.,
            response: Channels::Rgb,
            preview_side: 513,
            sun_radius: 7,
            side: 1825,
            quality: Precision::Calibrated,
            mask: String::new(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Recipe {
    pub version: u32,
    pub parameters: Parameters,
    pub view: View,
    pub exposure: f32,
    pub zoom: f32,
}
impl Default for Recipe {
    fn default() -> Self {
        Self {
            version: 2,
            parameters: Parameters::default(),
            view: View::Point,
            exposure: 19.,
            zoom: 1.,
        }
    }
}
impl Recipe {
    pub fn open(path: &Path) -> Result<Self> {
        let bytes = read_limited(path, 16 * 1024 * 1024)?;
        let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value["version"] == 1 {
            migrate_v1(&mut value)?;
        }
        let recipe: Self = serde_json::from_value(value)?;
        let p = &recipe.parameters;
        if recipe.version != 2
            || !(-16.0..=40.).contains(&recipe.exposure)
            || !(0.125..=16.).contains(&recipe.zoom)
            || !SIZES.contains(&p.side)
            || !FIELDS.contains(&p.preview_side)
            || !(1..=32).contains(&p.sun_radius)
            || !(2.0..=16.).contains(&p.f_number)
            || !(2.0..=8.).contains(&p.pitch_um)
            || !(1500.0..=15000.).contains(&p.temperature)
            || !(360.0..=830.).contains(&p.wavelength)
            || !p.blades.is_valid()
            || !p.iris.is_valid()
            || !p.dust.is_valid()
            || !p.fibers.is_valid()
        {
            return Err("参数文件版本、光学参数或尺寸无效".into());
        }
        if p.shape == Shape::Custom || !p.mask.is_empty() {
            decode_mask(&STANDARD.decode(&p.mask)?)?;
        }
        Ok(recipe)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        stage_file(path, |w| {
            serde_json::to_writer_pretty(&mut *w, self)?;
            w.write_all(b"\n")?;
            Ok(())
        })?
        .persist(path)?;
        Ok(())
    }
}
// 第 1 版的固定镜头无法精确分解。保留可解释的开口、污染参数、探测器和内嵌图像。
fn migrate_v1(recipe: &mut serde_json::Value) -> Result<()> {
    use serde_json::{Value, json};
    #[derive(Deserialize, Default)]
    #[serde(default, deny_unknown_fields)]
    struct Legacy {
        shape: Option<String>,
        fibers: Option<u32>,
        hex_seed: Option<u32>,
        transmission: Option<f64>,
        opd_nm: Option<f64>,
        roughness: Option<f64>,
        iris_seed: Option<u32>,
    }
    let p = recipe
        .get_mut("parameters")
        .and_then(Value::as_object_mut)
        .ok_or("旧参数缺少光瞳")?;
    let mut old = serde_json::Map::new();
    for name in [
        "shape",
        "fibers",
        "hex_seed",
        "transmission",
        "opd_nm",
        "roughness",
        "iris_seed",
    ] {
        if let Some(value) = p.remove(name) {
            old.insert(name.into(), value);
        }
    }
    for name in ["chamfer", "obstruction", "dust_sigma"] {
        p.remove(name);
    }
    let old: Legacy = serde_json::from_value(Value::Object(old))?;
    let shape = old.shape.as_deref().unwrap_or("hex");
    let new_shape = match shape {
        "hex" | "square" => Shape::Blades,
        "iris" => Shape::Iris,
        "custom" => Shape::Custom,
        _ => return Err(format!("未知的旧光瞳 {shape}").into()),
    };
    let mut fibers = Fibers::default();
    if let Some(v) = old.fibers {
        fibers.count = v;
    }
    if let Some(v) = old.hex_seed {
        fibers.seed = v;
    }
    if let Some(v) = old.transmission {
        fibers.transmission = v;
    }
    if let Some(v) = old.opd_nm {
        fibers.opd_nm = v;
    }
    let mut iris = Iris::default();
    if let Some(v) = old.roughness {
        iris.strength = v;
    }
    if let Some(v) = old.iris_seed {
        iris.seed = v;
    }
    let square = shape == "square";
    let blades = Blades {
        count: if square { 4 } else { 6 },
        edge: Edge::Straight,
        rotation_deg: if square { 0. } else { 90. },
        ..Blades::default()
    };
    p.insert("shape".into(), serde_json::to_value(new_shape)?);
    p.insert("blades".into(), serde_json::to_value(blades)?);
    p.insert("iris".into(), serde_json::to_value(iris)?);
    p.insert("fibers".into(), serde_json::to_value(fibers)?);
    p.insert("dust_enabled".into(), json!(false));
    p.insert(
        "fibers_enabled".into(),
        json!(shape == "hex" && fibers.count != 0),
    );
    recipe["version"] = json!(2);
    Ok(())
}
impl Parameters {
    pub fn light(&self) -> Source {
        match self.source {
            Lamp::Equal => Source::Equal,
            Lamp::D65 => Source::D65,
            Lamp::Blackbody => Source::Blackbody(self.temperature),
            Lamp::Line => Source::Line(self.wavelength),
        }
    }
    pub fn pupil(&self) -> Result<Pupil> {
        let opening = if self.base_enabled {
            match self.shape {
                Shape::Blades => Opening::Blades(self.blades),
                Shape::Iris => Opening::Iris(self.iris),
                Shape::Combined => Opening::Intersection(self.blades, self.iris),
                Shape::Custom => decode_mask(&STANDARD.decode(&self.mask)?)?,
            }
        } else {
            Opening::Clear
        };
        Ok(Pupil::new(
            opening,
            self.dust_enabled.then_some(self.dust),
            self.fibers_enabled.then_some(self.fibers),
        )?)
    }
    pub fn request(&self, preview: bool) -> Request {
        let support = self.side / 2;
        Request {
            radius: if preview {
                support.min(self.preview_side / 2 + self.sun_radius)
            } else {
                support
            },
            support,
            f_number: self.f_number,
            pitch_um: self.pitch_um,
            source: self.light(),
            response: if preview && self.response == Channels::Observer {
                Response::Observer
            } else {
                Response::EngineRgb
            },
            quality: if preview || self.quality == Precision::Fast {
                Quality::Draft
            } else {
                Quality::Calibrated
            },
        }
    }
    pub fn import_png(&mut self, path: &Path) -> Result<()> {
        let bytes = read_limited(path, 10 * 1024 * 1024)?;
        decode_mask(&bytes)?;
        self.mask = STANDARD.encode(&bytes);
        self.shape = Shape::Custom;
        self.base_enabled = true;
        Ok(())
    }
}
fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err("文件超过允许大小".into());
    }
    Ok(bytes)
}
fn decode_mask(bytes: &[u8]) -> Result<Opening> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let [width, height] = [reader.info().width, reader.info().height];
    if !(1..=1024).contains(&width) || !(1..=1024).contains(&height) {
        return Err("PNG 的长边最多 1024 像素".into());
    }
    let mut buf = vec![0; reader.output_buffer_size().ok_or("PNG 尺寸无效")?];
    let info = reader.next_frame(&mut buf)?;
    let channels = info.color_type.samples();
    let word = if info.bit_depth == png::BitDepth::Sixteen {
        2
    } else {
        1
    };
    let data: Vec<_> = buf[..info.buffer_size()]
        .chunks_exact(channels * word)
        .map(|pixel| {
            let at = |c: usize| {
                if word == 2 {
                    f32::from(u16::from_be_bytes([pixel[c * 2], pixel[c * 2 + 1]])) / 65535.
                } else {
                    f32::from(pixel[c]) / 255.
                }
            };
            let value = if channels >= 3 {
                0.299 * at(0) + 0.587 * at(1) + 0.114 * at(2)
            } else {
                at(0)
            };
            value.min(1.)
                * if channels.is_multiple_of(2) {
                    at(channels - 1)
                } else {
                    1.
                }
        })
        .collect();
    if !data.iter().any(|v| *v > 0.) {
        return Err("PNG 的振幅全部为零".into());
    }
    Ok(Opening::Mask {
        extent: [width, height],
        amplitude: data,
    })
}
fn stage_file(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<&mut fs::File>) -> Result<()>,
) -> Result<tempfile::NamedTempFile> {
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    {
        let mut writer = BufWriter::new(file.as_file_mut());
        write(&mut writer)?;
        writer.flush()?;
    }
    Ok(file)
}
pub(crate) fn stage_kernel(path: &Path, f: &Frame) -> Result<tempfile::NamedTempFile> {
    stage_file(path, |w| {
        #[cfg(target_endian = "little")]
        w.write_all(bytemuck::cast_slice(&f.pixels))?;
        #[cfg(target_endian = "big")]
        for pixel in &f.pixels {
            for v in pixel {
                w.write_all(&v.to_le_bytes())?;
            }
        }
        Ok(())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_png_survives_saving_and_reopening() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("pupil.png");
        {
            let mut encoder = png::Encoder::new(fs::File::create(&path)?, 3, 2);
            encoder.set_color(png::ColorType::GrayscaleAlpha);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&[0, 255, 255, 255, 255, 128, 128, 255, 255, 0, 100, 255])?;
        }
        let mut r = Recipe::default();
        r.parameters.import_png(&path)?;
        r.parameters.source = Lamp::Blackbody;
        r.parameters.temperature = 3200.;
        let j = dir.path().join("pupil.json");
        r.save(&j)?;
        fs::remove_file(&path)?;
        let loaded = Recipe::open(&j)?;
        assert_eq!(loaded.parameters.mask, r.parameters.mask);
        assert_eq!(
            loaded.parameters.pupil()?.thumbnail(21, 500.)?,
            r.parameters.pupil()?.thumbnail(21, 500.)?
        );
        Ok(())
    }
    #[test]
    fn old_iris_recipe_keeps_its_seed_and_detector() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("old.json");
        fs::write(&file,br#"{"version":1,"parameters":{"shape":"iris","roughness":0.031,"iris_seed":1001,"f_number":7.5}}"#)?;
        let r = Recipe::open(&file)?;
        assert_eq!(r.version, 2);
        assert_eq!(r.parameters.shape, Shape::Iris);
        assert_eq!(r.parameters.iris.seed, 1001);
        assert_eq!(r.parameters.iris.strength, 0.031);
        assert_eq!(r.parameters.f_number, 7.5);
        assert!(!r.parameters.dust_enabled && !r.parameters.fibers_enabled);
        Ok(())
    }
    #[test]
    fn disabled_base_accepts_dust_and_fibers_independently() -> Result<()> {
        let mut p = Parameters {
            base_enabled: false,
            dust_enabled: false,
            fibers_enabled: false,
            ..Default::default()
        };
        let clear = p.pupil()?.thumbnail(192, 550.)?;
        assert_eq!(
            clear,
            Pupil::new(Opening::Clear, None, None)?.thumbnail(192, 550.)?
        );
        p.dust_enabled = true;
        let dust = p.pupil()?.thumbnail(192, 550.)?;
        p.fibers_enabled = true;
        let both = p.pupil()?.thumbnail(192, 550.)?;
        p.dust_enabled = false;
        let fibers = p.pupil()?.thumbnail(192, 550.)?;
        assert!(dust != clear && fibers != clear && both != dust && both != fibers);
        Ok(())
    }
    #[test]
    fn sixteen_bit_mask_preserves_linear_amplitude() -> Result<()> {
        let mut bytes = vec![];
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Sixteen);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&[0x40, 0])?;
        }
        let mask = Pupil::new(decode_mask(&bytes)?, None, None)?;
        assert_eq!(mask.thumbnail(1, 550.)?, [64]);
        Ok(())
    }
}
