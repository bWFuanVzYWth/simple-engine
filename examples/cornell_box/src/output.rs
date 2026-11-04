//! 参考辐射与显示预览的编码边界。

use crate::Error;
use exr::{
    meta::attribute::Chromaticities,
    prelude::{Encoding, Image, Layer, LayerAttributes, SpecificChannels, Vec2, WritableImage},
};
use simple_engine::color::LinearSrgbRgba;
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

/// 覆盖前缀对应的 EXR、PNG 和 JSON，并创建父目录；失败不回滚已写文件。
///
/// 调用方保证 pixels 为 size² 个有限 RGB，按行排列。EXR 为 float32 linear sRGB/D65，
/// 无损、不曝光或裁剪；PNG 仅为 sRGB 预览，超出显示范围裁剪至 `[0,1]`，不用于 HDR 比较。
pub fn write(
    base: &Path,
    size: u32,
    pixels: &[[f32; 3]],
    metadata: &serde_json::Value,
) -> Result<(), Error> {
    if let Some(parent) = base
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let layer = Layer::new(
        (size as usize, size as usize),
        LayerAttributes::named("radiance"),
        Encoding::SMALL_FAST_LOSSLESS,
        SpecificChannels::rgb(|position: Vec2<usize>| {
            let pixel = pixels[position.y() * size as usize + position.x()];
            (pixel[0], pixel[1], pixel[2])
        }),
    );
    let mut image = Image::from_layer(layer);
    image.attributes.chromaticities = Some(Chromaticities {
        red: Vec2(0.64, 0.33),
        green: Vec2(0.30, 0.60),
        blue: Vec2(0.15, 0.06),
        white: Vec2(0.3127, 0.3290),
    });
    image.write().to_file(base.with_extension("exr"))?;

    let mut bytes = Vec::with_capacity(pixels.len() * 3);
    for &[r, g, b] in pixels {
        bytes.extend_from_slice(&LinearSrgbRgba::new(r, g, b, 1.0).to_srgb8().to_array()[..3]);
    }
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(base.with_extension("png"))?),
        size,
        size,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::RelativeColorimetric);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&bytes)?;
    writer.finish()?;
    let mut file = BufWriter::new(File::create(base.with_extension("json"))?);
    serde_json::to_writer_pretty(&mut file, metadata)?;
    file.flush()?;
    Ok(())
}
