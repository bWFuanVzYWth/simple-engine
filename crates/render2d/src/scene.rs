use std::ops::Range;

/// 逻辑像素坐标；x 向右，y 向下。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// 水平位置。
    pub x: f32,
    /// 垂直位置。
    pub y: f32,
}

impl Point {
    /// 直接保存坐标，不校验有限性。
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// 逻辑像素矩形；内部几何须有限且尺寸非负，外部输入使用 [`Self::checked`]。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// 左边界。
    pub x: f32,
    /// 上边界。
    pub y: f32,
    /// 水平跨度，可为零。
    pub width: f32,
    /// 垂直跨度，可为零。
    pub height: f32,
}

impl Rect {
    /// 直接保存几何，不校验有限性或尺寸。
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 拒绝非有限的边界、负尺寸及相加溢出的右／下边界。
    pub fn checked(x: f32, y: f32, width: f32, height: f32) -> Result<Self, SceneError> {
        let rect = Self::new(x, y, width, height);
        if rect.valid() {
            Ok(rect)
        } else {
            Err(SceneError::InvalidGeometry)
        }
    }

    /// 检查有限边界与非负尺寸，允许空矩形。
    pub fn valid(self) -> bool {
        [
            self.x,
            self.y,
            self.width,
            self.height,
            self.right(),
            self.bottom(),
        ]
        .into_iter()
        .all(f32::is_finite)
            && self.width >= 0.0
            && self.height >= 0.0
    }

    /// 右边界，不执行溢出校验。
    pub const fn right(self) -> f32 {
        self.x + self.width
    }
    /// 下边界，不执行溢出校验。
    pub const fn bottom(self) -> f32 {
        self.y + self.height
    }
    /// 任一尺寸不为正时无面积。
    pub const fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// 半开区域，避免相邻控件共边同时命中。
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x && point.x < self.right() && point.y >= self.y && point.y < self.bottom()
    }

    /// 求交；无交集时返回零尺寸，调用者须提供有效矩形。
    pub fn intersection(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Self::new(
            x,
            y,
            (self.right().min(other.right()) - x).max(0.0),
            (self.bottom().min(other.bottom()) - y).max(0.0),
        )
    }

    /// 向内收缩，正值最多收至中心；负值向外扩张。
    pub fn inset(self, amount: f32) -> Self {
        let x = amount.min(self.width * 0.5);
        let y = amount.min(self.height * 0.5);
        Self::new(
            self.x + x,
            self.y + y,
            (self.width - 2.0 * x).max(0.0),
            (self.height - 2.0 * y).max(0.0),
        )
    }

    /// 左上角逻辑位置。
    pub const fn origin(self) -> Point {
        Point::new(self.x, self.y)
    }
}

/// linear sRGB、premultiplied alpha；HDR 策略由调用者决定。
///
/// 构造函数不裁剪颜色；场景校验要求各分量有限、alpha 位于 `0..=1`，RGB 可超出 SDR 范围。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color(
    /// 已预乘的 `[r, g, b, a]`。
    pub [f32; 4],
);

impl Color {
    /// 不透明线性白。
    pub const WHITE: Self = Self([1.0; 4]);
    /// 全零透明色。
    pub const TRANSPARENT: Self = Self([0.0; 4]);

    /// 从未预乘的线性 RGB 与 alpha 构造，在线性域预乘。
    pub const fn linear(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self([r * a, g * a, b * a, a])
    }

    /// 从未预乘的 8 位 sRGB 与线性 alpha 解码，再在线性域预乘。
    pub fn srgb8(r: u8, g: u8, b: u8, a: u8) -> Self {
        let [r, g, b, a] = color::Srgb8Rgba::new(r, g, b, a)
            .to_linear_srgb()
            .to_array();
        Self::linear(r, g, b, a)
    }

    /// 同时缩放 RGB 与 alpha；乘数不裁剪，调用者维持有效 alpha。
    pub fn opacity(self, alpha: f32) -> Self {
        Self(self.0.map(|v| v * alpha))
    }

    fn valid(self) -> bool {
        self.0.into_iter().all(f32::is_finite) && (0.0..=1.0).contains(&self.0[3])
    }
}

/// GPU renderer 签发的图片身份，含所属 renderer 与槽代次，不能跨 renderer 使用。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ImageKey {
    pub(crate) owner: u64,
    pub(crate) slot: u32,
    pub(crate) generation: u32,
}

/// 矩形或圆角边框实例；圆角半径由渲染阶段限制到短边的一半。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    /// 外轮廓的逻辑像素范围。
    pub rect: Rect,
    /// 非负圆角半径。
    pub radius: f32,
    /// 向内的边框宽度；零表示填充。
    pub border: f32,
    /// 预乘线性颜色。
    pub color: Color,
}

/// 纹理矩形实例；采样与颜色约定由 GPU 边界维护。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Image {
    /// 仍有效的 renderer 图片键。
    pub key: ImageKey,
    /// 目标逻辑范围。
    pub rect: Rect,
    /// 归一化纹理坐标范围，可超出 `0..=1`，寻址由 sampler 决定。
    pub uv: Rect,
    /// 与采样结果逐分量相乘的预乘线性颜色。
    pub tint: Color,
}

#[cfg(feature = "text")]
/// 待栅格化的 glyph；缓存键必须与生成它的文字系统一同解释。
#[derive(Clone, Copy, Debug)]
pub struct GlyphRequest {
    /// 包含字体、大小和 subpixel bin 的局部栅格键。
    pub cache_key: cosmic_text::CacheKey,
    /// 签发缓存键的文字系统身份。
    pub font_set: crate::text::TextSystemId,
    /// 每逻辑像素对应的栅格密度，必须有限且为正。
    pub scale: f32,
    /// 逻辑基线位置；墨迹偏移由栅格返回。
    pub origin: Point,
    /// Mask 的预乘线性颜色；彩色 glyph 用作 tint。
    pub color: Color,
}

/// 引用对应连续实例数组的有序批次；range 以实例索引计，右端不含。
#[derive(Clone, Debug, PartialEq)]
pub enum DrawOp {
    /// 同裁剪的形状批次。
    Shapes {
        /// [`DrawList::shapes`] 中的实例范围。
        range: Range<u32>,
        /// 逻辑像素裁剪区域。
        clip: Rect,
    },
    /// 同图片与裁剪的纹理批次。
    Images {
        /// [`DrawList::images`] 中的实例范围。
        range: Range<u32>,
        /// 此批次共用的图片身份。
        image: ImageKey,
        /// 逻辑像素裁剪区域。
        clip: Rect,
    },
    #[cfg(feature = "text")]
    /// 同裁剪的文字请求；GPU 准备时可因 atlas 页而拆批。
    Glyphs {
        /// [`DrawList::glyphs`] 中的请求范围。
        range: Range<u32>,
        /// 逻辑像素裁剪区域。
        clip: Rect,
    },
}

/// 透明绘制保持登记顺序，仅合并相邻兼容状态。
#[derive(Default)]
pub struct DrawList {
    ops: Vec<DrawOp>,
    shapes: Vec<Shape>,
    images: Vec<Image>,
    #[cfg(feature = "text")]
    glyphs: Vec<GlyphRequest>,
}

impl DrawList {
    /// 创建未预留容量的空列表。
    pub fn new() -> Self {
        Self::default()
    }
    /// 清空本帧内容并保留各数组容量。
    pub fn clear(&mut self) {
        self.ops.clear();
        self.shapes.clear();
        self.images.clear();
        #[cfg(feature = "text")]
        self.glyphs.clear();
    }
    /// 按登记顺序读取批次，不可按材质全局重排。
    pub fn ops(&self) -> &[DrawOp] {
        &self.ops
    }
    /// 连续形状数组，顺序由 [`Self::ops`] 引用。
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }
    /// 连续图片数组，顺序由 [`Self::ops`] 引用。
    pub fn images(&self) -> &[Image] {
        &self.images
    }
    /// 数组保留容量的字节数，不含分配器开销及外部资源。
    pub fn retained_bytes(&self) -> usize {
        let bytes = self.ops.capacity() * size_of::<DrawOp>()
            + self.shapes.capacity() * size_of::<Shape>()
            + self.images.capacity() * size_of::<Image>();
        #[cfg(feature = "text")]
        let bytes = bytes + self.glyphs.capacity() * size_of::<GlyphRequest>();
        bytes
    }
    #[cfg(feature = "text")]
    /// 连续 glyph 请求数组，顺序由 [`Self::ops`] 引用。
    pub fn glyphs(&self) -> &[GlyphRequest] {
        &self.glyphs
    }

    /// 借用列表并设置初始逻辑裁剪；不在此处校验几何。
    pub fn canvas(&mut self, clip: Rect) -> Canvas<'_> {
        Canvas { list: self, clip }
    }

    /// 校验几何、颜色和 glyph 密度；不解析图片键、字体键或 GPU 兼容性。
    pub fn validate(&self) -> Result<(), SceneError> {
        for op in &self.ops {
            let clip = match op {
                DrawOp::Shapes { clip, .. } | DrawOp::Images { clip, .. } => clip,
                #[cfg(feature = "text")]
                DrawOp::Glyphs { clip, .. } => clip,
            };
            if !clip.valid() {
                return Err(SceneError::InvalidGeometry);
            }
        }
        if self.shapes.iter().any(|s| {
            !s.rect.valid()
                || !s.radius.is_finite()
                || s.radius < 0.0
                || !s.border.is_finite()
                || s.border < 0.0
                || !s.color.valid()
        }) || self
            .images
            .iter()
            .any(|i| !i.rect.valid() || !i.uv.valid() || !i.tint.valid())
        {
            return Err(SceneError::InvalidGeometry);
        }
        #[cfg(feature = "text")]
        if self.glyphs.iter().any(|g| {
            !g.origin.x.is_finite()
                || !g.origin.y.is_finite()
                || !g.color.valid()
                || !g.scale.is_finite()
                || g.scale <= 0.0
        }) {
            return Err(SceneError::InvalidGeometry);
        }
        Ok(())
    }
}

/// 向借用列表追加操作；子画布的裁剪只在该借用期间生效。
///
/// 完全位于裁剪外的形状和图片不登记；文字保留到 GPU 准备再按墨迹边界剔除。
/// 几何与颜色的完整有效性由 [`DrawList::validate`] 在输入边界检查。
pub struct Canvas<'a> {
    list: &'a mut DrawList,
    clip: Rect,
}

impl<'a> Canvas<'a> {
    /// 使用 `[-2²⁴, 2²⁴]` 的有限逻辑裁剪；大坐标场景可用 [`DrawList::canvas`] 指定范围。
    pub fn new(list: &'a mut DrawList) -> Self {
        Self {
            list,
            clip: Rect::new(-16_777_216.0, -16_777_216.0, 33_554_432.0, 33_554_432.0),
        }
    }

    /// 借用一个与父裁剪求交的子画布，结束借用后父裁剪仍有效。
    pub fn clipped(&mut self, rect: Rect) -> Canvas<'_> {
        Canvas {
            list: self.list,
            clip: self.clip.intersection(rect),
        }
    }

    /// 追加填充矩形；无可见面积时不登记。
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.rounded_rect(rect, 0.0, color);
    }

    /// 追加圆角填充，半径以逻辑像素计且须非负。
    pub fn rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        self.shape(Shape {
            rect,
            radius,
            border: 0.0,
            color,
        });
    }

    /// 追加向内圆角边框；宽度为零采用填充语义。
    pub fn border(&mut self, rect: Rect, radius: f32, width: f32, color: Color) {
        self.shape(Shape {
            rect,
            radius,
            border: width,
            color,
        });
    }

    fn shape(&mut self, shape: Shape) {
        if shape.rect.intersection(self.clip).is_empty() {
            return;
        }
        let start = self.list.shapes.len() as u32;
        self.list.shapes.push(shape);
        if let Some(DrawOp::Shapes { range, clip }) = self.list.ops.last_mut()
            && *clip == self.clip
        {
            range.end = start + 1;
        } else {
            self.list.ops.push(DrawOp::Shapes {
                range: start..start + 1,
                clip: self.clip,
            });
        }
    }

    /// 追加纹理矩形；`uv` 为归一化范围，tint 与采样结果逐分量相乘。
    pub fn image(&mut self, key: ImageKey, rect: Rect, uv: Rect, tint: Color) {
        if rect.intersection(self.clip).is_empty() {
            return;
        }
        let start = self.list.images.len() as u32;
        self.list.images.push(Image {
            key,
            rect,
            uv,
            tint,
        });
        if let Some(DrawOp::Images { range, image, clip }) = self.list.ops.last_mut()
            && *image == key
            && *clip == self.clip
        {
            range.end = start + 1;
        } else {
            self.list.ops.push(DrawOp::Images {
                range: start..start + 1,
                image: key,
                clip: self.clip,
            });
        }
    }

    #[cfg(feature = "text")]
    /// 追加字形，并给每个基线叠加逻辑 `origin`；不在此处栅格化或逐字裁剪。
    pub fn glyphs(&mut self, glyphs: &[crate::text::PositionedGlyph], origin: Point, color: Color) {
        if glyphs.is_empty() || self.clip.is_empty() {
            return;
        }
        let start = self.list.glyphs.len() as u32;
        self.list.glyphs.extend(glyphs.iter().map(|g| GlyphRequest {
            cache_key: g.cache_key,
            font_set: g.font_set,
            scale: g.scale,
            origin: Point::new(origin.x + g.origin.x, origin.y + g.origin.y),
            color,
        }));
        let end = self.list.glyphs.len() as u32;
        if let Some(DrawOp::Glyphs { range, clip }) = self.list.ops.last_mut()
            && *clip == self.clip
        {
            range.end = end;
        } else {
            self.list.ops.push(DrawOp::Glyphs {
                range: start..end,
                clip: self.clip,
            });
        }
    }

    #[cfg(feature = "text")]
    /// 追加布局字形；`origin` 为布局左上角的逻辑位置。
    pub fn text(&mut self, layout: &crate::text::LayoutRef<'_>, origin: Point, color: Color) {
        self.glyphs(layout.glyphs(), origin, color);
    }
}

/// CPU 场景校验失败，不包含外部图片或 GPU 资源错误。
#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    /// 非有限几何、负尺寸／半径／边宽、无效 alpha 或非正 glyph 密度。
    #[error("2D 几何或颜色超出有效范围")]
    InvalidGeometry,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touching_rectangles_have_one_hit_owner() {
        let p = Point::new(10.0, 5.0);
        assert!(!Rect::new(0.0, 0.0, 10.0, 10.0).contains(p));
        assert!(Rect::new(10.0, 0.0, 10.0, 10.0).contains(p));
    }

    #[test]
    fn nested_clip_is_scoped() {
        let mut list = DrawList::new();
        let mut canvas = list.canvas(Rect::new(0.0, 0.0, 100.0, 100.0));
        canvas
            .clipped(Rect::new(0.0, 0.0, 10.0, 10.0))
            .fill_rect(Rect::new(20.0, 20.0, 10.0, 10.0), Color::WHITE);
        canvas.fill_rect(Rect::new(20.0, 20.0, 10.0, 10.0), Color::WHITE);
        assert_eq!(list.shapes.len(), 1);
    }

    #[test]
    fn image_splits_shape_batches_without_reordering() {
        let mut list = DrawList::new();
        let mut canvas = Canvas::new(&mut list);
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        canvas.fill_rect(rect, Color::WHITE);
        canvas.image(
            ImageKey {
                owner: 0,
                slot: 0,
                generation: 0,
            },
            rect,
            Rect::new(0.0, 0.0, 1.0, 1.0),
            Color::WHITE,
        );
        canvas.fill_rect(rect, Color::WHITE);
        assert!(matches!(
            list.ops.as_slice(),
            [
                DrawOp::Shapes { .. },
                DrawOp::Images { .. },
                DrawOp::Shapes { .. }
            ]
        ));
    }
}
