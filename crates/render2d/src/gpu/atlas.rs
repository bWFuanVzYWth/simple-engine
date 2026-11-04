use std::collections::{HashMap, VecDeque};

use crate::{
    Rect,
    text::{RasterContent, TextSystem, TextSystemId},
};

use super::{Budgets, Material, RenderError, SampledImage};

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct Key {
    font_set: TextSystemId,
    glyph: cosmic_text::CacheKey,
}

#[derive(Clone, Copy)]
pub(super) struct Glyph {
    pub page: usize,
    pub uv: Rect,
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub mask: bool,
}

struct Page {
    texture: wgpu::Texture,
    material: Material,
    color: bool,
    x: u32,
    y: u32,
    row: u32,
}

impl Page {
    fn place(&mut self, width: u32, height: u32, side: u32) -> Option<[u32; 2]> {
        if self.x + width > side {
            self.x = 0;
            self.y += self.row;
            self.row = 0;
        }
        if self.y + height > side {
            return None;
        }
        let origin = [self.x, self.y];
        self.x += width;
        self.row = self.row.max(height);
        Some(origin)
    }
}

struct Upload {
    id: u64,
    page: usize,
    origin: [u32; 2],
    width: u32,
    height: u32,
    pitch: u32,
    data: Vec<u8>,
}

#[derive(Default)]
pub(super) struct Atlas {
    pages: Vec<Page>,
    glyphs: HashMap<Key, Option<Glyph>>,
    order: VecDeque<Key>,
    upload: Vec<u8>,
    pending: Vec<Upload>,
    next_upload: u64,
    bytes: u64,
}

impl Atlas {
    #[expect(clippy::too_many_arguments)]
    pub fn resolve(
        &mut self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        budgets: &Budgets,
        text: &mut TextSystem,
        key: cosmic_text::CacheKey,
        pages: &mut Vec<usize>,
        protected: &[usize],
    ) -> Result<Option<Glyph>, RenderError> {
        let key = Key {
            font_set: text.id(),
            glyph: key,
        };
        if let Some(value) = self.glyphs.get(&key).copied() {
            if let Some(glyph) = value {
                Self::retain(glyph.page, pages);
            }
            return Ok(value);
        }
        let value = if let Some(raster) = text.raster(key.glyph)?
            && raster.width > 0
            && raster.height > 0
        {
            let width = raster.width.checked_add(2).ok_or(RenderError::Budget)?;
            let height = raster.height.checked_add(2).ok_or(RenderError::Budget)?;
            if width > budgets.atlas_side || height > budgets.atlas_side {
                return Err(RenderError::Budget);
            }
            let color = raster.content == RasterContent::Color;
            let mut location = None;
            for (index, page) in self
                .pages
                .iter_mut()
                .enumerate()
                .filter(|(_, p)| p.color == color)
            {
                if let Some(origin) = page.place(width, height, budgets.atlas_side) {
                    location = Some((index, origin));
                    break;
                }
            }
            let (page, origin) = match location {
                Some(value) => value,
                None => {
                    let page = self.new_page(device, layout, budgets, color, protected, pages)?;
                    let origin = self.pages[page]
                        .place(width, height, budgets.atlas_side)
                        .ok_or(RenderError::Budget)?;
                    (page, origin)
                }
            };
            Self::retain(page, pages);
            let channels = if color { 4 } else { 1 };
            let pitch = (width as usize * channels).next_multiple_of(256);
            self.upload.resize(pitch * height as usize, 0);
            self.upload.fill(0);
            for row in 0..raster.height as usize {
                let source = row * raster.width as usize * channels;
                let dest = (row + 1) * pitch + channels;
                if color {
                    for (source, dest) in raster.pixels[source..source + raster.width as usize * 4]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(
                            self.upload[dest..dest + raster.width as usize * 4]
                                .as_chunks_mut::<4>()
                                .0
                                .iter_mut(),
                        )
                    {
                        *dest = color_pixel(*source, raster.premultiplied);
                    }
                } else {
                    self.upload[dest..dest + raster.width as usize]
                        .copy_from_slice(&raster.pixels[source..source + raster.width as usize]);
                }
            }
            self.next_upload = self.next_upload.checked_add(1).ok_or(RenderError::Budget)?;
            self.pending.push(Upload {
                id: self.next_upload,
                page,
                origin,
                width,
                height,
                pitch: pitch as u32,
                data: std::mem::take(&mut self.upload),
            });
            let side = budgets.atlas_side as f32;
            Some(Glyph {
                page,
                uv: Rect::new(
                    (origin[0] + 1) as f32 / side,
                    (origin[1] + 1) as f32 / side,
                    raster.width as f32 / side,
                    raster.height as f32 / side,
                ),
                left: raster.left,
                top: raster.top,
                width: raster.width,
                height: raster.height,
                mask: !color,
            })
        } else {
            None
        };
        if budgets.max_atlas_glyphs == 0 {
            return Err(RenderError::Budget);
        }
        while self.glyphs.len() >= budgets.max_atlas_glyphs {
            if let Some(oldest) = self.order.pop_front() {
                self.glyphs.remove(&oldest);
            }
        }
        self.glyphs.insert(key, value);
        self.order.push_back(key);
        Ok(value)
    }

    fn retain(index: usize, pages: &mut Vec<usize>) {
        if !pages.contains(&index) {
            pages.push(index);
        }
    }

    pub fn pending_pages(&self, pages: &mut Vec<usize>) {
        for upload in &self.pending {
            Self::retain(upload.page, pages);
        }
    }

    pub fn encode_uploads(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) -> u64 {
        use wgpu::util::DeviceExt;
        let last = self.pending.last().map_or(0, |upload| upload.id);
        for upload in &self.pending {
            let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("2D atlas upload"),
                contents: &upload.data,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(upload.pitch),
                        rows_per_image: Some(upload.height),
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.pages[upload.page].texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: upload.origin[0],
                        y: upload.origin[1],
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: upload.width,
                    height: upload.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        last
    }

    pub fn submitted(&mut self, uploaded: u64) {
        self.pending.retain(|upload| upload.id > uploaded);
    }

    fn new_page(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        budgets: &Budgets,
        color: bool,
        protected: &[usize],
        current: &[usize],
    ) -> Result<usize, RenderError> {
        let bytes = u64::from(budgets.atlas_side).pow(2) * if color { 4 } else { 1 };
        if bytes > budgets.max_atlas_bytes {
            return Err(RenderError::Budget);
        }
        let recycle = if self.pages.len() < budgets.max_atlas_pages
            && self.bytes + bytes <= budgets.max_atlas_bytes
        {
            None
        } else {
            Some(
                self.pages
                    .iter()
                    .enumerate()
                    .position(|(index, page)| {
                        !protected.contains(&index)
                            && !current.contains(&index)
                            && self.bytes - page_bytes(page, budgets.atlas_side) + bytes
                                <= budgets.max_atlas_bytes
                    })
                    .ok_or(RenderError::Busy)?,
            )
        };
        if let Some(index) = recycle {
            self.pending.retain(|upload| upload.page != index);
            self.glyphs
                .retain(|_, value| value.is_none_or(|glyph| glyph.page != index));
            self.order.retain(|key| self.glyphs.contains_key(key));
            if self.pages[index].color == color {
                let page = &mut self.pages[index];
                page.x = 0;
                page.y = 0;
                page.row = 0;
                return Ok(index);
            }
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("2D glyph atlas"),
            size: wgpu::Extent3d {
                width: budgets.atlas_side,
                height: budgets.atlas_side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if color {
                wgpu::TextureFormat::Rgba8UnormSrgb
            } else {
                wgpu::TextureFormat::R8Unorm
            },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let material = Material::new(
            device,
            layout,
            SampledImage {
                view: texture.create_view(&Default::default()),
                sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                }),
            },
        );
        let page = Page {
            texture,
            material,
            color,
            x: 0,
            y: 0,
            row: 0,
        };
        self.bytes += bytes;
        if let Some(index) = recycle {
            self.bytes -= page_bytes(&self.pages[index], budgets.atlas_side);
            self.pages[index] = page;
            Ok(index)
        } else {
            self.pages.push(page);
            Ok(self.pages.len() - 1)
        }
    }

    pub fn material(&self, page: usize) -> &Material {
        &self.pages[page].material
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn pages(&self) -> usize {
        self.pages.len()
    }
    pub fn glyphs(&self) -> usize {
        self.glyphs.len()
    }
    pub fn scratch_bytes(&self) -> usize {
        self.upload.capacity()
            + self
                .pending
                .iter()
                .map(|upload| upload.data.capacity())
                .sum::<usize>()
    }
}

fn page_bytes(page: &Page, side: u32) -> u64 {
    u64::from(side).pow(2) * if page.color { 4 } else { 1 }
}

/// atlas 存储 sRGB 编码的 linear premul，硬件采样后可直接混合。
fn color_pixel(pixel: [u8; 4], premultiplied: bool) -> [u8; 4] {
    let [r, g, b, a] = pixel.map(|v| f32::from(v) / 255.0);
    let unpremultiply = if premultiplied && a > 0.0 {
        1.0 / a
    } else {
        1.0
    };
    let [r, g, b, _] = color::SrgbRgba::new(
        (r * unpremultiply).min(1.0),
        (g * unpremultiply).min(1.0),
        (b * unpremultiply).min(1.0),
        a,
    )
    .to_linear_srgb()
    .to_array();
    color::LinearSrgbRgba::new(r * a, g * a, b * a, a)
        .to_srgb8()
        .to_array()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_sources_have_one_linear_premultiplied_representation() {
        let straight = color_pixel([255, 0, 0, 128], false);
        let outline = color_pixel([128, 0, 0, 128], true);
        assert_eq!(straight, outline);
        let [red, _, _, alpha] = color::Srgb8Rgba::new(straight[0], 0, 0, straight[3])
            .to_linear_srgb()
            .to_array();
        assert!((red - alpha).abs() < 0.005);
    }
}
