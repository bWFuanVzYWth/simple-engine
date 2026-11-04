//! 固定字体集的 Unicode 排版、栅格与独占文本编辑。
//!
//! [`TextDocument`] 独占 UTF-8 正文、grapheme 选区与 IME 组合态：Preedit 只改变显示文本，
//! Commit 才替换正文。位置以 UTF-8 字节计，编辑后重新对齐扩展 grapheme 边界。
//! revision 跟踪正文／组合显示的变化，单独移动选区不换版本；旧布局命中须由
//! [`TextDocument::hit`] 拒绝。平台 IME 会话代次在上层输入／GUI 维护，不能用 revision 代替。
//!
//! [`TextSystem`] 只加载显式字体，不扫描系统字库。[`TextLayout`] 保存缓存身份，负载由文字系统独占
//! 并按显式代次保留；布局坐标为逻辑像素，`raster_scale` 决定物理栅格密度并参与缓存身份。
//! [`TextSystemId`] 与局部 [`CacheKey`] 共同标识字形，不能跨系统解释同一个局部键。
//!
//! 字体、布局和栅格预算只统计显式保留负载；分配器、第三方内部 cache／scratch 及 miss
//! 的临时工作集另计，不是进程硬内存上限。完整正文始终参与 CPU 排版，GPU 裁剪只减少
//! 后续栅格与上传。排版使用 [cosmic-text](https://docs.rs/cosmic-text/0.19.0/cosmic_text/)，
//! 字素与词边界采用 [Unicode UAX #29](https://www.unicode.org/reports/tr29/)。

use std::{
    collections::VecDeque,
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

use cosmic_text::{
    Attrs, Buffer, CacheKey, CacheKeyFlags, Cursor, FontSystem, Metrics, Shaping, SwashCache,
    SwashContent, SwashImage, Wrap, fontdb,
};
use input::TextRevision;
use unicode_segmentation::UnicodeSegmentation;

mod document;
pub use document::{Composition, TextChange, TextCursor, TextDocument, TextSelection};

use crate::{Point, Rect};

/// 参数、版本或固定工作集不满足文字处理契约。
#[derive(Debug, thiserror::Error)]
pub enum TextError {
    /// 非正／非有限度量，或物理字号超过 512px。
    #[error("文字参数必须有限且为正数")]
    InvalidStyle,
    /// 未提供字体，或某个输入不能解析出任何字体。
    #[error("字体集为空或含无法解析的字体")]
    InvalidFont,
    /// 正文、字体、布局、glyph 数或栅格负载超出预算。
    #[error("文字、字体或栅格超出预算")]
    Budget,
    /// 布局缓存已满且所有候选仍属于当前保留代次。
    #[error("文字缓存均被使用，无法回收")]
    Busy,
    /// 正文位置越界或落在 UTF-8 编码内部。
    #[error("文本位置不在 UTF-8 边界")]
    InvalidCursor,
    /// IME 选区越界或落在组合串 UTF-8 编码内部。
    #[error("IME 选区不在 UTF-8 边界")]
    InvalidComposition,
    /// 命中使用的布局版本与当前正文／组合显示版本不同。
    #[error("文字布局版本已过期")]
    StaleRevision,
    /// 文本版本无法继续递增。
    #[error("文本版本溢出")]
    RevisionOverflow,
    /// 栅格器返回当前不支持的 subpixel mask。
    #[error("不支持的 glyph 栅格格式")]
    UnsupportedRaster,
    /// 进程内文字系统身份耗尽。
    #[error("字体系统身份溢出")]
    SystemIdOverflow,
}

static NEXT_SYSTEM_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// 进程内文字系统身份；不作为持久化字体标识。
pub struct TextSystemId(u64);

impl TextSystemId {
    /// 原始身份值，仅适用于当前进程。
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// 逻辑排版与物理栅格度量；字号、行高和密度须有限且为正。
pub struct TextStyle {
    /// 逻辑像素字号；乘 raster_scale 后不得超过 512px。
    pub font_size: f32,
    /// 逻辑像素行高。
    pub line_height: f32,
    /// 每逻辑像素的栅格像素数，不改变逻辑排版尺寸。
    pub raster_scale: f32,
    /// false 只禁止自动折行，正文中的换行仍生效。
    pub wrap: bool,
    /// 是否执行字体 hinting；默认开启，关闭后才可能使用保守墨迹预裁剪。
    pub hinting: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            line_height: 20.0,
            raster_scale: 1.0,
            wrap: true,
            hinting: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
/// 分别限制字体字节、布局负载与栅格缓存；不包含分配器和第三方内部 cache／scratch。
pub struct TextBudgets {
    /// 单次正文／组合显示的 UTF-8 字节上限。
    pub max_text_bytes: usize,
    /// 加载字体原始字节的累计上限。
    pub max_font_bytes: usize,
    /// 同时保留的布局条目上限，必须非零。
    pub max_layouts: usize,
    /// 保留的布局负载；第三方内部 scratch 及单次 miss 的临时结果另计。
    pub max_layout_bytes: usize,
    /// 单份布局 glyph 数上限，必须非零。
    pub max_glyphs: usize,
    /// 栅格缓存像素数组保留容量的累计上限。
    pub max_raster_bytes: usize,
    /// 栅格缓存条目上限，含无墨迹结果，必须非零。
    pub max_raster_glyphs: usize,
}

impl Default for TextBudgets {
    fn default() -> Self {
        Self {
            max_text_bytes: 64 * 1024,
            max_font_bytes: 32 * 1024 * 1024,
            max_layouts: 256,
            max_layout_bytes: 16 * 1024 * 1024,
            max_glyphs: 64 * 1024,
            max_raster_bytes: 16 * 1024 * 1024,
            max_raster_glyphs: 4096,
        }
    }
}

#[derive(Clone, Copy, Debug)]
/// 排版后的字形请求；字体系统身份与局部缓存键必须同时保留。
pub struct PositionedGlyph {
    /// 原文字系统中的物理栅格键，含字号、subpixel bin 与 hinting 标记。
    pub cache_key: CacheKey,
    /// 签发缓存键的文字系统。
    pub font_set: TextSystemId,
    /// 物理像素除以此值转换为逻辑尺寸。
    pub scale: f32,
    /// 逻辑基线位置；栅格左上角为 origin + (left, -top) / scale。
    pub origin: Point,
}

struct LayoutData {
    key: u64,
    retained: bool,
    text: String,
    width: f32,
    style: TextStyle,
    buffer: Buffer,
    line_starts: Vec<usize>,
    glyphs: Vec<PositionedGlyph>,
    size: [f32; 2],
    advance_x: [f32; 2],
    bytes: usize,
}

#[derive(Clone, Debug)]
struct CompositionMap {
    replace: Range<usize>,
    inserted: usize,
    caret: Option<usize>,
    document_caret: usize,
}

/// 排版身份与文档映射；负载由原 TextSystem 独占，克隆只复制小型元数据。
///
/// owner 的 `begin_epoch` 决定保留集合；未保留项可被缓存淘汰，访问须经 `resolve` 校验。
#[derive(Clone)]
pub struct TextLayout {
    owner: TextSystemId,
    key: u64,
    size: [f32; 2],
    advance_x: [f32; 2],
    revision: Option<TextRevision>,
    composition: Option<CompositionMap>,
}

impl TextLayout {
    /// 创建时的逻辑尺寸；不访问缓存。
    pub const fn size(&self) -> [f32; 2] {
        self.size
    }
    /// 创建时的水平 advance 范围。
    pub const fn advance_x(&self) -> [f32; 2] {
        self.advance_x
    }
    /// 文档修订；独立字符串布局为 None。
    pub const fn revision(&self) -> Option<TextRevision> {
        self.revision
    }
}

/// 一次读取期间借用的排版负载；借用结束后可继续修改文字系统。
pub struct LayoutRef<'a> {
    data: &'a LayoutData,
    layout: &'a TextLayout,
}

impl LayoutRef<'_> {
    /// 逻辑宽高；空正文仍保留一行高度，宽度不是墨迹包围盒。
    pub const fn size(&self) -> [f32; 2] {
        self.data.size
    }
    /// 排版 advance 的水平范围，不代表字形墨迹边界。
    pub const fn advance_x(&self) -> [f32; 2] {
        self.data.advance_x
    }
    /// 按绘制顺序读取已定位字形，尚未栅格化。
    pub const fn glyphs(&self) -> &[PositionedGlyph] {
        self.data.glyphs.as_slice()
    }
    /// 文档生成布局时记录的版本；独立字符串布局为 None。
    pub const fn revision(&self) -> Option<TextRevision> {
        self.layout.revision
    }

    /// 用局部逻辑坐标命中正文位置；非有限坐标或无法命中时返回 None。
    /// 组合串内的命中折回被替换的正文边界，写回时仍须核对 revision。
    pub fn hit(&self, point: Point) -> Option<TextCursor> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return None;
        }
        let cursor = self.data.buffer.hit(point.x, point.y)?;
        let byte = self.data.line_starts.get(cursor.line)? + cursor.index;
        Some(TextCursor::new(self.to_document(byte)))
    }

    /// 一逻辑像素宽的 caret 区域；无效位置或 IME 未提供 caret 时为 None。
    /// 活动组合使用 IME 选区映射，结果尚未叠加控件滚动与平移。
    pub fn caret_rect(&self, cursor: TextCursor) -> Option<Rect> {
        let byte = match &self.layout.composition {
            Some(map) if cursor.byte == map.document_caret => map.caret?,
            _ => self.to_display(cursor.byte),
        };
        let cursor = self.cursor(byte)?;
        self.data.buffer.layout_runs().find_map(|run| {
            run.cursor_position(&cursor)
                .map(|x| Rect::new(x, run.line_top, 1.0, run.line_height))
        })
    }

    /// 分配并返回局部逻辑选区矩形；组合期间不绘制正文选区。
    pub fn selection_rects(&self, selection: TextSelection) -> Vec<Rect> {
        let mut out = Vec::new();
        self.selection_rects_into(selection, &mut out);
        out
    }

    /// 向 out 追加正文选区矩形，不清空缓冲；组合期间、空选区或无效位置不追加。
    pub fn selection_rects_into(&self, selection: TextSelection, out: &mut Vec<Rect>) {
        if self.layout.composition.is_some() || selection.is_empty() {
            return;
        }
        self.highlight(selection.range(), out);
    }

    /// 向 out 追加当前组合串的局部逻辑高亮，不清空缓冲；无组合时不追加。
    pub fn composition_rects_into(&self, out: &mut Vec<Rect>) {
        if let Some(map) = &self.layout.composition {
            self.highlight(map.replace.start..map.replace.start + map.inserted, out);
        }
    }

    fn highlight(&self, range: Range<usize>, out: &mut Vec<Rect>) {
        let (Some(start), Some(end)) = (self.cursor(range.start), self.cursor(range.end)) else {
            return;
        };
        for run in self.data.buffer.layout_runs() {
            if run.line_i < start.line || run.line_i > end.line {
                continue;
            }
            let mut region: Option<(f32, f32)> = None;
            for glyph in run.glyphs {
                let cluster = &run.text[glyph.start..glyph.end];
                let width = glyph.w / cluster.graphemes(true).count().max(1) as f32;
                for (index, (offset, part)) in cluster.grapheme_indices(true).enumerate() {
                    let selected = (start.line != run.line_i
                        || glyph.start + offset + part.len() > start.index)
                        && (end.line != run.line_i || glyph.start + offset < end.index);
                    if selected {
                        let x = if glyph.level.is_rtl() {
                            glyph.x + glyph.w - width * (index + 1) as f32
                        } else {
                            glyph.x + width * index as f32
                        };
                        region = Some(region.map_or((x, x + width), |(left, right)| {
                            (left.min(x), right.max(x + width))
                        }));
                    } else {
                        flush_highlight(out, &mut region, run.line_top, run.line_height);
                    }
                }
            }
            flush_highlight(out, &mut region, run.line_top, run.line_height);
            if run.glyphs.is_empty() && run.line_i < end.line {
                out.push(Rect::new(
                    0.0,
                    run.line_top,
                    self.data.width,
                    run.line_height,
                ));
            }
        }
    }

    fn cursor(&self, byte: usize) -> Option<Cursor> {
        if !self.data.text.is_char_boundary(byte) {
            return None;
        }
        let line = self
            .data
            .line_starts
            .partition_point(|start| *start <= byte)
            .checked_sub(1)?;
        let index = byte - self.data.line_starts[line];
        (index <= self.data.buffer.lines[line].text().len()).then(|| Cursor::new(line, index))
    }

    fn to_document(&self, byte: usize) -> usize {
        match &self.layout.composition {
            Some(map) if byte > map.replace.start => {
                if byte < map.replace.start + map.inserted {
                    map.replace.start
                } else {
                    byte - map.inserted + map.replace.len()
                }
            }
            _ => byte,
        }
    }

    fn to_display(&self, byte: usize) -> usize {
        match &self.layout.composition {
            Some(map) if byte > map.replace.start => {
                if byte < map.replace.end {
                    map.replace.start
                } else {
                    byte - map.replace.len() + map.inserted
                }
            }
            _ => byte,
        }
    }
}

fn flush_highlight(out: &mut Vec<Rect>, region: &mut Option<(f32, f32)>, y: f32, height: f32) {
    if let Some((left, right)) = region.take()
        && right > left
    {
        out.push(Rect::new(left, y, right - left, height));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// 栅格像素编码；彩色结果的预乘状态另见 [`RasterGlyph::premultiplied`]。
pub enum RasterContent {
    /// 每像素一字节线性覆盖率，不包含文字颜色。
    Mask,
    /// 每像素四字节 sRGB RGBA，alpha 为线性覆盖率。
    Color,
}

#[derive(Clone, Copy, Debug)]
/// 借用栅格缓存的紧密像素行；下一次可变访问文字系统前必须释放借用。
pub struct RasterGlyph<'a> {
    /// 墨迹左边缘相对基线原点的物理像素偏移。
    pub left: i32,
    /// 基线向上到墨迹顶边的物理像素偏移。
    pub top: i32,
    /// 物理像素列数。
    pub width: u32,
    /// 物理像素行数。
    pub height: u32,
    /// 决定像素通道数与颜色解释。
    pub content: RasterContent,
    /// ColorOutline 为 sRGB 字节域预乘；ColorBitmap 为 straight RGBA。
    pub premultiplied: bool,
    /// 紧密行优先像素，无额外行 padding；Mask 为 R8，Color 为 RGBA8。
    pub pixels: &'a [u8],
}

/// 固定字体集、排版缓存和 CPU 栅格缓存的所有者；不访问系统字体目录。
///
/// 活动代次保留其缓存预算，只有未保留条目可淘汰；栅格缓存按插入顺序回收。
/// 统计范围见 [`TextBudgets`]，不包含第三方临时排版负载。
pub struct TextSystem {
    id: TextSystemId,
    fonts: FontSystem,
    swash: SwashCache,
    layouts: VecDeque<LayoutData>,
    next_layout: u64,
    raster_order: VecDeque<CacheKey>,
    budgets: TextBudgets,
    font_bytes: usize,
    layout_bytes: usize,
    raster_bytes: usize,
    font_bounds: Option<FontBounds>,
}

struct FontBounds {
    id: fontdb::ID,
    weight: fontdb::Weight,
    em: Option<Rect>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// 显式保留的字体与缓存负载快照，不是进程总内存。
pub struct TextStats {
    /// 原始字体数据累计字节。
    pub font_bytes: usize,
    /// 当前布局缓存的负载估计，包含显式保留代次的条目。
    pub layout_bytes: usize,
    /// 缓存栅格像素数组的保留容量。
    pub raster_bytes: usize,
    /// 当前布局缓存条目数。
    pub layouts: usize,
    /// 栅格缓存条目数，包含无墨迹结果。
    pub raster_glyphs: usize,
}

impl TextSystem {
    /// 接管显式字体字节，以首个字体族作为 sans-serif，使用 `zh-CN` locale。
    ///
    /// 字体为空或某个输入无法解析时返回 [`TextError::InvalidFont`]；字体总量超预算、
    /// 布局／栅格条目上限为零时返回 [`TextError::Budget`]。固定输入字体有助于保持回放度量。
    pub fn new(
        fonts: impl IntoIterator<Item = Vec<u8>>,
        budgets: TextBudgets,
    ) -> Result<Self, TextError> {
        if budgets.max_layouts == 0 || budgets.max_raster_glyphs == 0 || budgets.max_glyphs == 0 {
            return Err(TextError::Budget);
        }
        let mut db = fontdb::Database::new();
        let mut font_bytes = 0usize;
        for bytes in fonts {
            font_bytes = font_bytes
                .checked_add(bytes.len())
                .ok_or(TextError::Budget)?;
            if font_bytes > budgets.max_font_bytes {
                return Err(TextError::Budget);
            }
            let old = db.len();
            db.load_font_data(bytes);
            if db.len() == old {
                return Err(TextError::InvalidFont);
            }
        }
        let family = db
            .faces()
            .next()
            .and_then(|face| face.families.first())
            .map(|family| family.0.clone())
            .ok_or(TextError::InvalidFont)?;
        db.set_sans_serif_family(family);
        let id = NEXT_SYSTEM_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map(TextSystemId)
            .map_err(|_| TextError::SystemIdOverflow)?;
        Ok(Self {
            id,
            fonts: FontSystem::new_with_locale_and_db("zh-CN".to_owned(), db),
            swash: SwashCache::new(),
            layouts: VecDeque::new(),
            next_layout: 1,
            raster_order: VecDeque::new(),
            budgets,
            font_bytes,
            layout_bytes: 0,
            raster_bytes: 0,
            font_bounds: None,
        })
    }

    /// 开始缓存保留代次，只保留传入快照与本代次新生成的布局。
    ///
    /// 调用者传入仍供已发布 UI/持久内容使用的身份；候选取消后，下个代次不再传入即可回收。
    /// 只扫描元数据，不复制正文、排版 Buffer 或字形。旧身份只能通过 resolve 访问。
    pub fn begin_epoch<'a>(&mut self, retained: impl IntoIterator<Item = &'a TextLayout>) {
        for entry in &mut self.layouts {
            entry.retained = false;
        }
        for layout in retained {
            if layout.owner == self.id
                && let Some(entry) = self
                    .layouts
                    .iter_mut()
                    .find(|entry| entry.key == layout.key)
            {
                entry.retained = true;
            }
        }
    }

    /// 校验所有者与缓存代次后借用负载；已淘汰或来自其他系统时返回 None。
    pub fn resolve<'a>(&'a self, layout: &'a TextLayout) -> Option<LayoutRef<'a>> {
        if layout.owner != self.id {
            return None;
        }
        let data = self.layouts.iter().find(|entry| entry.key == layout.key)?;
        Some(LayoutRef { data, layout })
    }

    /// 此系统签发的身份，须与局部 glyph 缓存键一同传递。
    pub const fn id(&self) -> TextSystemId {
        self.id
    }

    /// 当前显式保留负载；第三方 cache／scratch 与分配器开销另计。
    pub fn stats(&self) -> TextStats {
        TextStats {
            font_bytes: self.font_bytes,
            layout_bytes: self.layout_bytes,
            raster_bytes: self.raster_bytes,
            layouts: self.layouts.len(),
            raster_glyphs: self.swash.image_cache.len(),
        }
    }

    /// 按 UTF-8 内容、逻辑 width 和完整 style 缓存排版，命中时返回同一负载的身份。
    ///
    /// width 必须有限且为正；`wrap = false` 禁止自动折行，正文换行仍有效。
    /// 无效度量返回 [`TextError::InvalidStyle`]，超工作集返回 [`TextError::Budget`]；
    /// 无可回收布局条目时返回 [`TextError::Busy`]，不等待其他代次退休。
    pub fn layout(
        &mut self,
        text: &str,
        width: f32,
        style: TextStyle,
    ) -> Result<TextLayout, TextError> {
        let physical_size = style.font_size * style.raster_scale;
        if !width.is_finite()
            || width <= 0.0
            || !style.font_size.is_finite()
            || style.font_size <= 0.0
            || !style.line_height.is_finite()
            || style.line_height <= 0.0
            || !style.raster_scale.is_finite()
            || style.raster_scale <= 0.0
            || !physical_size.is_finite()
            || physical_size <= 0.0
            || physical_size > 512.0
        {
            return Err(TextError::InvalidStyle);
        }
        if text.len() > self.budgets.max_text_bytes {
            return Err(TextError::Budget);
        }
        if let Some(at) = self
            .layouts
            .iter()
            .position(|entry| entry.text == text && entry.width == width && entry.style == style)
        {
            let mut data = self.layouts.remove(at).expect("找到的缓存条目");
            data.retained = true;
            let layout = TextLayout {
                owner: self.id,
                key: data.key,
                size: data.size,
                advance_x: data.advance_x,
                revision: None,
                composition: None,
            };
            self.layouts.push_back(data);
            return Ok(layout);
        }
        if self.layouts.len() >= self.budgets.max_layouts
            && self.layouts.iter().all(|entry| entry.retained)
        {
            return Err(TextError::Busy);
        }
        let mut buffer = Buffer::new(
            &mut self.fonts,
            Metrics::new(style.font_size, style.line_height),
        );
        buffer.set_size(Some(width), None);
        buffer.set_wrap(if style.wrap {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });
        let flags = if style.hinting {
            CacheKeyFlags::empty()
        } else {
            CacheKeyFlags::DISABLE_HINTING
        };
        buffer.set_text(
            text,
            &Attrs::new().cache_key_flags(flags),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.fonts, false);
        let mut glyphs = Vec::new();
        let mut size = [0.0_f32, style.line_height];
        let mut advance_x = [0.0_f32; 2];
        for run in buffer.layout_runs() {
            size[0] = size[0].max(run.line_w);
            size[1] = size[1].max(run.line_top + run.line_height);
            for glyph in run.glyphs {
                advance_x[0] = advance_x[0].min(glyph.x);
                advance_x[1] = advance_x[1].max(glyph.x + glyph.w);
                if glyphs.len() == self.budgets.max_glyphs {
                    return Err(TextError::Budget);
                }
                let physical =
                    glyph.physical((0.0, run.line_y * style.raster_scale), style.raster_scale);
                glyphs.push(PositionedGlyph {
                    cache_key: physical.cache_key,
                    font_set: self.id,
                    scale: style.raster_scale,
                    origin: Point::new(
                        physical.x as f32 / style.raster_scale,
                        physical.y as f32 / style.raster_scale,
                    ),
                });
            }
        }
        let mut line_starts = Vec::with_capacity(buffer.lines.len());
        let mut start = 0;
        for line in &buffer.lines {
            line_starts.push(start);
            start += line.text().len() + line.ending().as_str().len();
        }
        let bytes = layout_bytes(&buffer, text, &glyphs, &line_starts);
        if bytes > self.budgets.max_layout_bytes {
            return Err(TextError::Budget);
        }
        while self.layouts.len() >= self.budgets.max_layouts
            || self.layout_bytes > self.budgets.max_layout_bytes - bytes
        {
            let at = self
                .layouts
                .iter()
                .position(|entry| !entry.retained)
                .ok_or(TextError::Busy)?;
            let old = self.layouts.remove(at).expect("找到的空闲缓存条目");
            self.layout_bytes -= old.bytes;
        }
        let key = self.next_layout;
        self.next_layout = key.checked_add(1).ok_or(TextError::SystemIdOverflow)?;
        let data = LayoutData {
            key,
            retained: true,
            text: text.to_owned(),
            width,
            style,
            buffer,
            line_starts,
            glyphs,
            size,
            advance_x,
            bytes,
        };
        self.layout_bytes += bytes;
        let layout = TextLayout {
            owner: self.id,
            key,
            size: data.size,
            advance_x: data.advance_x,
            revision: None,
            composition: None,
        };
        self.layouts.push_back(data);
        Ok(layout)
    }

    /// 相对字形基线的保守物理像素包围盒，用于栅格前裁剪。
    ///
    /// 只接受本系统的静态、无 hint、无合成变换 outline；变体、彩色／位图或未知范围
    /// 返回 None，调用者须保留字形，不能用 advance 代替墨迹边界。
    /// 使用 [OpenType head](https://learn.microsoft.com/en-us/typography/opentype/spec/head)
    /// 全字体包围盒，额外 2px 覆盖 subpixel bin 和栅格取整。
    pub fn ink_bounds(&mut self, key: CacheKey) -> Option<Rect> {
        if !key.flags.contains(CacheKeyFlags::DISABLE_HINTING)
            || key.flags.contains(CacheKeyFlags::FAKE_ITALIC)
        {
            return None;
        }
        let size = f32::from_bits(key.font_size_bits);
        if !size.is_finite() || size <= 0.0 || size > 512.0 {
            return None;
        }
        if !self
            .font_bounds
            .as_ref()
            .is_some_and(|bounds| bounds.id == key.font_id && bounds.weight == key.font_weight)
        {
            let em = self
                .fonts
                .get_font(key.font_id, key.font_weight)
                .and_then(|font| {
                    let raw = font.as_swash();
                    let unsupported = [
                        b"fvar", b"CFF2", b"COLR", b"CBDT", b"CBLC", b"sbix", b"SVG ", b"EBDT",
                        b"EBLC",
                    ];
                    if unsupported
                        .into_iter()
                        .any(|tag| raw.table(u32::from_be_bytes(*tag)).is_some())
                        || (raw.table(u32::from_be_bytes(*b"glyf")).is_none()
                            && raw.table(u32::from_be_bytes(*b"CFF ")).is_none())
                    {
                        return None;
                    }
                    let metrics = font.metrics();
                    let bounds = metrics.bounds?;
                    let em = f32::from(metrics.units_per_em);
                    let rect = Rect::new(
                        bounds.x_min / em,
                        -bounds.y_max / em,
                        (bounds.x_max - bounds.x_min) / em,
                        (bounds.y_max - bounds.y_min) / em,
                    );
                    rect.valid().then_some(rect)
                });
            self.font_bounds = Some(FontBounds {
                id: key.font_id,
                weight: key.font_weight,
                em,
            });
        }
        let em = self.font_bounds.as_ref()?.em?;
        Some(Rect::new(
            em.x * size - 2.0,
            em.y * size - 2.0,
            em.width * size + 4.0,
            em.height * size + 4.0,
        ))
    }

    /// 解析本系统签发的物理缓存键，复用或更新栅格缓存；无墨迹时返回 None。
    ///
    /// key 必须来自本系统，跨系统键应由上层结合 [`TextSystemId`] 拒绝。
    /// 拒绝无效物理字号、超栅格预算及不支持的 subpixel mask；结果借用缓存，不能跨
    /// 下一次可变访问保留。彩色像素保留来源预乘语义，GPU atlas 负责在线性域规范化。
    pub fn raster(&mut self, key: CacheKey) -> Result<Option<RasterGlyph<'_>>, TextError> {
        let size = f32::from_bits(key.font_size_bits);
        if !size.is_finite() || size <= 0.0 || size > 512.0 {
            return Err(TextError::InvalidStyle);
        }
        if !self.swash.image_cache.contains_key(&key) {
            let image = self.swash.get_image_uncached(&mut self.fonts, key);
            let bytes = image.as_ref().map_or(0, |value| value.data.capacity());
            if bytes > self.budgets.max_raster_bytes {
                return Err(TextError::Budget);
            }
            while self.swash.image_cache.len() >= self.budgets.max_raster_glyphs
                || self.raster_bytes > self.budgets.max_raster_bytes - bytes
            {
                let old = self.raster_order.pop_front().ok_or(TextError::Budget)?;
                if let Some(old) = self.swash.image_cache.remove(&old).flatten() {
                    self.raster_bytes -= old.data.capacity();
                }
            }
            self.swash.image_cache.insert(key, image);
            self.raster_order.push_back(key);
            self.raster_bytes += bytes;
        }
        self.swash
            .image_cache
            .get(&key)
            .and_then(Option::as_ref)
            .map(raster_ref)
            .transpose()
    }
}

fn raster_ref(image: &SwashImage) -> Result<RasterGlyph<'_>, TextError> {
    let content = match image.content {
        SwashContent::Mask => RasterContent::Mask,
        SwashContent::Color => RasterContent::Color,
        SwashContent::SubpixelMask => return Err(TextError::UnsupportedRaster),
    };
    Ok(RasterGlyph {
        left: image.placement.left,
        top: image.placement.top,
        width: image.placement.width,
        height: image.placement.height,
        content,
        premultiplied: matches!(image.source, swash::scale::Source::ColorOutline(_)),
        pixels: &image.data,
    })
}

fn layout_bytes(
    buffer: &Buffer,
    text: &str,
    glyphs: &Vec<PositionedGlyph>,
    starts: &Vec<usize>,
) -> usize {
    let mut bytes = std::mem::size_of::<LayoutData>()
        + 2 * std::mem::size_of::<usize>()
        + text.len() * 2
        + glyphs.capacity() * std::mem::size_of::<PositionedGlyph>()
        + starts.capacity() * std::mem::size_of::<usize>()
        + buffer.lines.capacity() * std::mem::size_of::<cosmic_text::BufferLine>();
    for line in &buffer.lines {
        if let Some(shape) = line.shape_opt() {
            bytes += shape.spans.capacity() * std::mem::size_of::<cosmic_text::ShapeSpan>();
            for span in &shape.spans {
                bytes += span.words.capacity() * std::mem::size_of::<cosmic_text::ShapeWord>();
                for word in &span.words {
                    bytes +=
                        word.glyphs.capacity() * std::mem::size_of::<cosmic_text::ShapeGlyph>();
                }
            }
        }
        if let Some(layouts) = line.layout_opt() {
            bytes += layouts.capacity() * std::mem::size_of::<cosmic_text::LayoutLine>();
            for layout in layouts {
                bytes += layout.glyphs.capacity() * std::mem::size_of::<cosmic_text::LayoutGlyph>()
                    + layout.decorations.capacity()
                        * std::mem::size_of::<cosmic_text::DecorationSpan>();
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use input::{EditUnit, TextMovement, TextOp, TextRange};

    fn system(budgets: TextBudgets) -> TextSystem {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/NotoSansSC-Regular.otf"
        ));
        TextSystem::new([bytes.to_vec()], budgets).unwrap()
    }

    fn end(document: &mut TextDocument) {
        document
            .apply(TextOp::MoveSelection {
                movement: TextMovement::DocumentEnd,
                extend: false,
            })
            .unwrap();
    }

    #[test]
    fn preedit_changes_only_display_until_commit() {
        let mut document = TextDocument::new("原文").unwrap();
        end(&mut document);
        document
            .apply(TextOp::Preedit {
                text: "输入",
                selection: Some(TextRange::new("输入", 6, 6).unwrap()),
            })
            .unwrap();
        assert_eq!(document.text(), "原文");
        document.apply(TextOp::Commit("新文")).unwrap();
        assert_eq!(document.text(), "原文新文");
    }

    #[test]
    fn ending_composition_preserves_selected_body() {
        let mut document = TextDocument::new("正文").unwrap();
        document
            .apply(TextOp::MoveSelection {
                movement: TextMovement::DocumentEnd,
                extend: true,
            })
            .unwrap();
        let selected = document.selection();
        document
            .apply(TextOp::Preedit {
                text: "临时",
                selection: None,
            })
            .unwrap();
        document.apply(TextOp::EndComposition).unwrap();
        assert_eq!((document.text(), document.selection()), ("正文", selected));
    }

    #[test]
    fn backspace_removes_a_complete_extended_grapheme() {
        let mut document = TextDocument::new("a👩‍👩‍👧‍👦").unwrap();
        end(&mut document);
        document
            .apply(TextOp::Delete(EditUnit::GraphemeBackward))
            .unwrap();
        assert_eq!(document.text(), "a");
    }

    #[test]
    fn joining_graphemes_keeps_the_caret_on_a_boundary() {
        let mut document = TextDocument::new("👩👧").unwrap();
        document
            .hit(document.revision(), TextCursor::new("👩".len()), false)
            .unwrap();
        document.apply(TextOp::Commit("\u{200d}")).unwrap();
        document
            .apply(TextOp::Delete(EditUnit::GraphemeBackward))
            .unwrap();
        assert_eq!(document.text(), "");
    }

    #[test]
    fn deleting_a_separator_preserves_rejoined_graphemes() {
        let mut document = TextDocument::new("🇨 🇳").unwrap();
        let at = "🇨 ".len();
        document
            .hit(document.revision(), TextCursor::new(at), false)
            .unwrap();
        document
            .apply(TextOp::Delete(EditUnit::GraphemeBackward))
            .unwrap();
        document
            .apply(TextOp::Delete(EditUnit::GraphemeBackward))
            .unwrap();
        assert_eq!(document.text(), "");
    }

    #[test]
    fn stale_hit_cannot_edit_new_text() {
        let mut document = TextDocument::new("旧").unwrap();
        let revision = document.revision();
        document.apply(TextOp::Commit("新")).unwrap();
        assert!(matches!(
            document.hit(revision, TextCursor::new(0), false),
            Err(TextError::StaleRevision)
        ));
    }

    #[test]
    fn hit_keeps_the_caret_on_a_grapheme_boundary() {
        let mut document = TextDocument::new("🇦🇧🇨").unwrap();
        document
            .hit(document.revision(), TextCursor::new("🇦".len()), false)
            .unwrap();
        assert_eq!(document.caret().byte, "🇦🇧".len());
    }

    #[test]
    fn rejected_commit_preserves_composition() {
        let mut document = TextDocument::with_limit("a", 4).unwrap();
        end(&mut document);
        document
            .apply(TextOp::Preedit {
                text: "中",
                selection: None,
            })
            .unwrap();
        assert!(matches!(
            document.apply(TextOp::Commit("中文")),
            Err(TextError::Budget)
        ));
        assert_eq!(
            (document.text(), document.composition().unwrap().text),
            ("a", "中")
        );
    }

    #[test]
    fn live_layouts_keep_their_budget_reserved() {
        let mut system = system(TextBudgets {
            max_layouts: 1,
            ..Default::default()
        });
        let layout = system.layout("甲", 100.0, TextStyle::default()).unwrap();
        let cached = system.layout("甲", 100.0, TextStyle::default()).unwrap();
        system.begin_epoch([&layout, &cached]);
        assert!(matches!(
            system.layout("乙", 100.0, TextStyle::default()),
            Err(TextError::Busy)
        ));
        system.begin_epoch(std::iter::empty());
        assert!(system.layout("乙", 100.0, TextStyle::default()).is_ok());
    }

    #[test]
    fn evicted_identity_cannot_resolve_a_reused_cache_slot() {
        let mut system = system(TextBudgets {
            max_layouts: 1,
            ..Default::default()
        });
        let old = system.layout("甲", 100.0, TextStyle::default()).unwrap();
        system.begin_epoch(std::iter::empty());
        let current = system.layout("乙", 100.0, TextStyle::default()).unwrap();
        assert!(system.resolve(&old).is_none());
        assert!(system.resolve(&current).is_some());
    }

    #[test]
    fn unwrapped_text_keeps_the_final_caret_on_the_same_line() {
        let mut system = system(TextBudgets::default());
        let text = "用于单行输入的长文本";
        let layout = system
            .layout(
                text,
                24.0,
                TextStyle {
                    wrap: false,
                    ..Default::default()
                },
            )
            .unwrap();
        let caret = system
            .resolve(&layout)
            .unwrap()
            .caret_rect(TextCursor::new(text.len()))
            .unwrap();
        assert_eq!(caret.y, 0.0);
        assert!(caret.x > 24.0);
    }

    #[test]
    fn fixed_cjk_font_produces_complete_mask_tiles() {
        let mut system = system(TextBudgets::default());
        let layout = system.layout("中", 100.0, TextStyle::default()).unwrap();
        let image = system
            .raster(system.resolve(&layout).unwrap().glyphs()[0].cache_key)
            .unwrap()
            .unwrap();
        assert_eq!(image.content, RasterContent::Mask);
        assert_eq!(
            image.pixels.len(),
            image.width as usize * image.height as usize
        );
        assert!(image.pixels.iter().any(|pixel| *pixel != 0));
    }

    #[test]
    fn static_outline_bounds_contain_raster_tiles() {
        let mut system = system(TextBudgets::default());
        let layout = system
            .layout(
                "Ag中边",
                100.0,
                TextStyle {
                    hinting: false,
                    raster_scale: 2.0,
                    ..Default::default()
                },
            )
            .unwrap();
        for glyph in system.resolve(&layout).unwrap().glyphs().to_vec() {
            let bounds = system.ink_bounds(glyph.cache_key).unwrap();
            let tile = system.raster(glyph.cache_key).unwrap().unwrap();
            let tile = Rect::new(
                tile.left as f32,
                -tile.top as f32,
                tile.width as f32,
                tile.height as f32,
            );
            assert_eq!(tile.intersection(bounds), tile);
        }
    }

    #[test]
    fn raster_density_does_not_change_logical_layout() {
        let mut system = system(TextBudgets::default());
        let normal = system.layout("中", 100.0, TextStyle::default()).unwrap();
        let dense = system
            .layout(
                "中",
                100.0,
                TextStyle {
                    raster_scale: 2.0,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(normal.size(), dense.size());
        let normal_width = system
            .raster(system.resolve(&normal).unwrap().glyphs()[0].cache_key)
            .unwrap()
            .unwrap()
            .width;
        let dense_width = system
            .raster(system.resolve(&dense).unwrap().glyphs()[0].cache_key)
            .unwrap()
            .unwrap()
            .width;
        assert!(dense_width > normal_width);
    }

    #[test]
    fn composition_caret_uses_ime_selection() {
        let mut system = system(TextBudgets::default());
        let mut document = TextDocument::new("a").unwrap();
        end(&mut document);
        document
            .apply(TextOp::Preedit {
                text: "输入",
                selection: Some(TextRange::new("输入", 6, 6).unwrap()),
            })
            .unwrap();
        let layout = document
            .layout(&mut system, 200.0, TextStyle::default())
            .unwrap();
        let caret = system
            .resolve(&layout)
            .unwrap()
            .caret_rect(document.caret())
            .unwrap();
        assert!(caret.x > 16.0);
        assert_eq!(
            system
                .resolve(&layout)
                .unwrap()
                .hit(Point::new(caret.x + 1.0, caret.y + 1.0)),
            Some(document.caret())
        );
    }

    #[test]
    fn selection_covers_a_complete_grapheme() {
        let mut system = system(TextBudgets::default());
        let layout = system
            .layout("e\u{301}", 100.0, TextStyle::default())
            .unwrap();
        let selected = system
            .resolve(&layout)
            .unwrap()
            .selection_rects(TextSelection {
                anchor: 0,
                focus: "e\u{301}".len(),
            });
        let caret = system
            .resolve(&layout)
            .unwrap()
            .caret_rect(TextCursor::new("e\u{301}".len()))
            .unwrap();
        assert_eq!(selected.len(), 1);
        assert!((selected[0].right() - caret.x).abs() < 0.001);
    }
}
