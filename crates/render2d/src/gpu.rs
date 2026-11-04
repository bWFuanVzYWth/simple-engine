//! wgpu 2D 编码；调用者拥有设备、队列、encoder、附件与提交，不在此管理窗口或呈现。
//!
//! [`Renderer2d::prepare`] 校验 CPU 场景，解析图片身份并批量准备上传；`text` 下的
//! `Renderer2d::prepare_text` 另解析字体身份；只合并相邻兼容
//! 状态，保留透明叠加顺序。不可变 [`Prepared2d`] 可用独立 [`ViewParams`] 编码到多个目标。
//! clip 映射为向外取整的物理 scissor，不计算分数像素裁剪覆盖率；glyph 按原栅格密度
//! 转回逻辑尺寸。附件 `Load` 可接续应用的 3D pass，也可直接采样应用产生的 3D 纹理。
//!
//! Renderer 独占包、视图和资源版本；唯一 Recorded2d 在提交前拥有 command buffer。
//! 客户端唯一身份与录制／在途批次分别作为保留根；扫描根集合回收，不维护引用计数。
//! 首次成功提交前保留几何与 uniform 上传，之后的新批次直接复用；取消录制可重试。
//! 调用者须正常 submit／poll 推进回调；renderer 返回 [`RenderError::Busy`]，不隐式等待 GPU。
//! 丢弃客户端身份不代表 GPU 完成；跨线程可移动唯一批次，Web GPU 句柄留在所属线程。
//!
//! [`Budgets`] 限制保留工作集，预留容量也计入 buffer 预算。几何增长会为随后一次
//! prepare_view 预留 uniform 字节，更多视图仍各需槽与预算。外部图片、包的 CPU 元数据、
//! 分配器、驱动与 wgpu staging 另计；反复 prepare 后不提交可能积累待提交上传，统计不是
//! 进程硬内存上限。prepare 失败不回滚缓存或已排定上传。
//!
//! 生产 shader 由 Slang 生成；native 应通过 Vulkan 安全入口加载原始 SPIR-V，Web 使用
//! 同源 WGSL。构建核对两个目标的反射、binding 与 [`abi`]，但任意外部 shader、设备、
//! TextureView 的格式／层／mip 及 sampler 兼容性仍由调用者和 wgpu error scope 维护。
//! 参考 [WebGPU](https://www.w3.org/TR/webgpu/) 与
//! [命令完成回调](https://docs.rs/wgpu/30.0.1/wgpu/struct.CommandEncoder.html#method.on_submitted_work_done)。

#[cfg(feature = "text")]
mod atlas;
mod pool;

use std::{
    collections::HashMap,
    num::NonZeroU64,
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(feature = "text")]
use crate::text::{TextError, TextSystem};
use crate::{DrawList, DrawOp, ImageKey, Rect, SceneError};
#[cfg(feature = "text")]
use atlas::Atlas;
use pool::{BufferPool, Kind};
use std::sync::mpsc;

/// 经 Slang 反射核对的 shader 数据布局；Rust 结构尺寸另作编译期断言。
pub mod abi {
    include!(concat!(env!("OUT_DIR"), "/render2d.abi.rs"));
}

/// Slang 生成的 native 顶点 SPIR-V 字节，入口为 `vertex_main`。
pub const VERTEX_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/render2d.vert.spv"));
/// Slang 生成的 native 片元 SPIR-V 字节，入口为 `fragment_main`。
pub const FRAGMENT_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/render2d.frag.spv"));
/// 同一 Slang 源码生成的 Web 顶点 WGSL，入口为 `vertex_main`。
pub const VERTEX_WGSL: &str = include_str!(concat!(env!("OUT_DIR"), "/render2d.vert.wgsl"));
/// 同一 Slang 源码生成的 Web 片元 WGSL，入口为 `fragment_main`。
pub const FRAGMENT_WGSL: &str = include_str!(concat!(env!("OUT_DIR"), "/render2d.frag.wgsl"));

/// 调用者在目标设备创建的生产 shader；两个模块须使用本模块导出的匹配构建产物。
pub struct Shaders2d {
    /// 含 `vertex_main` 的顶点模块。
    pub vertex: wgpu::ShaderModule,
    /// 含 `fragment_main` 的片元模块。
    pub fragment: wgpu::ShaderModule,
}

/// 限制 renderer 保留的工作集；外部图片、包的 CPU 元数据、驱动和 wgpu staging 另计。
#[derive(Clone, Copy, Debug)]
pub struct Budgets {
    /// 单次 prepare 的 shape／image／glyph 请求总数上限，在 glyph 裁剪前计数。
    pub max_instances: usize,
    /// 几何与视图 buffer 的累计保留容量，包含空闲槽。
    pub max_buffer_bytes: u64,
    /// 几何与视图 buffer 槽的总上限，含在途和空闲槽。
    pub max_buffer_slots: usize,
    /// renderer 图片注册槽数上限，移除后的槽可复用。
    pub max_images: usize,
    /// `text` 下的 atlas 正方形页边长，至少 3px，且不超过设备纹理限制。
    pub atlas_side: u32,
    /// Mask 与 Color atlas 页的共同上限。
    pub max_atlas_pages: usize,
    /// atlas 纹理保留字节上限；Mask 为 R8，Color 为 RGBA8。
    pub max_atlas_bytes: u64,
    /// atlas glyph 索引条目上限，含无墨迹结果。
    pub max_atlas_glyphs: usize,
    /// 按目标格式／采样数缓存的 pipeline 上限；无客户端或批次保留的视图时可回收。
    pub max_pipelines: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_instances: 262_144,
            max_buffer_bytes: 32 * 1024 * 1024,
            max_buffer_slots: 64,
            max_images: 1024,
            atlas_side: 1024,
            max_atlas_pages: 16,
            max_atlas_bytes: 16 * 1024 * 1024,
            max_atlas_glyphs: 8192,
            max_pipelines: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// pipeline 的附件配置；对应 view 格式须可混合并支持指定采样数。
pub struct TargetConfig {
    /// 实际颜色附件的 view 格式，决定 sRGB 输出编码。
    pub format: wgpu::TextureFormat,
    /// 颜色附件的 MSAA 采样数，单采样为 1。
    pub samples: u32,
}

#[derive(Clone, Copy, Debug)]
/// 将逻辑矩形映射到附件上的物理 viewport；两个范围都必须有限且有正面积。
pub struct ViewParams {
    /// 逻辑像素可见区域，左上角不必为零。
    pub logical: Rect,
    /// 目标附件上的物理像素区域。
    pub viewport: Rect,
}

impl ViewParams {
    /// 将 logical 映射到从原点开始的整个物理 extent；prepare_view 再校验参数。
    pub const fn new(logical: Rect, extent: [u32; 2]) -> Self {
        Self {
            logical,
            viewport: Rect::new(0.0, 0.0, extent[0] as f32, extent[1] as f32),
        }
    }
}

/// 本次编码借用的颜色附件；纹理生命周期和提交由调用者管理。
pub struct RenderTarget<'a> {
    /// 与 prepare_view 配置匹配的颜色 view，须具有 RENDER_ATTACHMENT 用途。
    pub view: &'a wgpu::TextureView,
    /// 可选单采样 resolve 目标，仅用于多采样附件，格式和 extent 须匹配。
    pub resolve: Option<&'a wgpu::TextureView>,
    /// 本次使用的物理尺寸，必须非零并覆盖 viewport，不能超过附件。
    pub extent: [u32; 2],
    /// 清除或加载已有颜色；Load 可直接接续前面的 3D pass。
    pub load: wgpu::LoadOp<wgpu::Color>,
    /// 调用者拥有的可选时间戳 query；必须启用相应 feature 并提供有效索引。
    pub timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'a>>,
}

/// 可过滤 2D 纹理、LOD 0；内容须为 linear premul。
/// sRGB 纹理存储预乘线性 RGB 的 sRGB 编码，解码由 TextureView 格式决定。
///
/// 须在采样过滤前在线性域预乘，不能直接预乘 sRGB 字节；否则透明边缘会产生暗边／色渗。
pub struct SampledImage {
    /// 可过滤、单采样的 2D view，纹理须具有 TEXTURE_BINDING 用途。
    pub view: wgpu::TextureView,
    /// 与 view 兼容的过滤 sampler；只采样 mip 0，寻址方式由调用者决定。
    pub sampler: wgpu::Sampler,
}

/// CPU 输入、资源身份或有界 GPU 工作集失败；底层 wgpu 验证由调用者 error scope 接收。
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// CPU 场景几何／颜色无效。
    #[error(transparent)]
    Scene(
        /// 原始场景校验错误。
        #[from]
        SceneError,
    ),
    /// 文字排版或栅格失败。
    #[cfg(feature = "text")]
    #[error(transparent)]
    Text(
        /// 原始文字错误。
        #[from]
        TextError,
    ),
    /// 单份数据或资源需求超出预算／设备限制，不能靠等待在途资源解决。
    #[error("2D 工作集超出预算或设备限制")]
    Budget,
    /// 预算已被活动批次占用，当前没有足够可回收资源；不会等待 GPU。
    #[error("2D 工作集均在使用，无法回收")]
    Busy,
    /// renderer 初始预算不满足容量、身份索引或设备限制。
    #[error("2D 预算无效")]
    InvalidBudget,
    /// 注册图片的基础纹理不是可过滤、单采样、可采样 2D 纹理。
    #[error("图片不是可过滤的单采样 2D 纹理")]
    InvalidImage,
    /// 图片已移除，或身份属于其他 renderer。
    #[error("图片键已移除或来自其他 renderer")]
    StaleImage,
    /// 移除图片时槽代次耗尽。
    #[error("图片键版本溢出")]
    ImageGenerationOverflow,
    /// 进程内 renderer 身份耗尽。
    #[error("renderer 身份溢出")]
    RendererIdOverflow,
    /// prepare_view 接收了其他 renderer 生成的绘制包。
    #[error("Prepared 来自其他 renderer")]
    ForeignPrepared,
    /// glyph 请求未提供文字系统，或其 TextSystemId 不匹配。
    #[cfg(feature = "text")]
    #[error("glyph 必须由其原始字体系统栅格化")]
    FontSystemMismatch,
    /// 附件、格式、采样数、extent 或 viewport 不满足编码约定。
    #[error("目标或视图参数无效")]
    InvalidTarget,
}

#[derive(Clone, Copy, Debug, Default)]
/// renderer 保留资源的当前快照；驱动、wgpu staging 与外部图片负载另计。
pub struct Stats {
    /// 所有几何／视图 buffer 槽的容量，含在途和空闲槽。
    pub buffer_bytes: u64,
    /// 几何／视图 buffer 槽数。
    pub buffer_slots: usize,
    /// 保留 atlas 纹理的像素字节。
    pub atlas_bytes: u64,
    /// 保留的 atlas 页数。
    pub atlas_pages: usize,
    /// atlas glyph 索引条目数，含无墨迹结果。
    pub atlas_glyphs: usize,
    /// 尚未移除的注册图片数，不计内部白纹理与 atlas。
    pub images: usize,
    /// 当前缓存 pipeline 数。
    pub pipelines: usize,
    /// 上传准备的保留容量；不含 HashMap 桶、分配器及第三方内部缓存。
    pub scratch_bytes: usize,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct View {
    transform: [f32; 4],
    pixel: [f32; 4],
}

const _: () = assert!(size_of::<Instance>() as u64 == abi::INSTANCE_STRIDE);
const _: () = assert!(size_of::<View>() as u64 == abi::VIEW_SIZE);

struct Material {
    group: wgpu::BindGroup,
}

impl Material {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, image: SampledImage) -> Self {
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("2D 材质"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&image.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&image.sampler),
                },
            ],
        });
        Self { group }
    }
}

struct ImageSlot {
    generation: u32,
    material: Option<Material>,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum MaterialKey {
    White,
    Image(ImageKey),
    #[cfg(feature = "text")]
    Atlas(usize),
}

struct Batch {
    range: Range<u32>,
    clip: Rect,
    material: u32,
}

struct PreparedData {
    key: u64,
    client: bool,
    geometry: usize,
    geometry_bytes: u64,
    batches: Vec<Batch>,
    materials: Vec<MaterialKey>,
    upload: Vec<Instance>,
    #[cfg(feature = "text")]
    pages: Vec<usize>,
}

/// 一份不可变几何包的唯一客户端身份，可跨提交复用，不复制几何。
///
/// 数据由 Renderer2d 保存；丢弃身份只释放客户端根，已录制批次仍保留资源至取消或完成。
pub struct Prepared2d {
    owner: u64,
    key: u64,
    index: usize,
    notices: mpsc::Sender<Notice>,
}
impl Drop for Prepared2d {
    fn drop(&mut self) {
        let _ = self
            .notices
            .send(Notice::ReleasePackage(self.index, self.key));
    }
}

/// 不可变视图的唯一客户端身份；uniform 独立，可跨提交复用。
///
/// 此视图由 owner 保留其几何包；丢弃身份后，已录制批次继续保留所需资源。
pub struct PreparedView2d {
    owner: u64,
    key: u64,
    index: usize,
    notices: mpsc::Sender<Notice>,
}
impl Drop for PreparedView2d {
    fn drop(&mut self) {
        let _ = self.notices.send(Notice::ReleaseView(self.index, self.key));
    }
}

/// 一次独占录制；拥有 encoder，取消不能遗漏其已录入的上传。
pub struct Recording2d<'a> {
    renderer: &'a mut Renderer2d,
    encoder: Option<wgpu::CommandEncoder>,
    epoch: u64,
    handed_off: bool,
}
impl Recording2d<'_> {
    /// 编码同批次视图；几何包和视图数据仍由 renderer 唯一保存。
    pub fn encode(
        &mut self,
        view: &PreparedView2d,
        target: RenderTarget<'_>,
    ) -> Result<(), RenderError> {
        self.renderer
            .encode(view, self.encoder.as_mut().expect("活动录制"), target)
    }
    /// 暂借 encoder 录制其他阶段；调用者须保持同一 encoder，不得替换。
    pub fn encoder(&mut self) -> &mut wgpu::CommandEncoder {
        self.encoder.as_mut().expect("活动录制")
    }
    /// 固化唯一命令批次，之后仅原 renderer 可以提交或取消。
    pub fn finish(mut self) -> Result<Recorded2d, RenderError> {
        let recorded = self
            .renderer
            .finish_batch(self.encoder.take().expect("活动录制"))?;
        self.handed_off = true;
        Ok(recorded)
    }
    /// 固化并经上层唯一提交器提交本批次，成功后由实际队列完成消息退休。
    pub fn submit_with<T, E: From<RenderError>>(
        mut self,
        submit: impl FnOnce(wgpu::CommandBuffer) -> Result<T, E>,
    ) -> Result<T, E> {
        let recorded = self
            .renderer
            .finish_batch(self.encoder.take().expect("活动录制"))?;
        self.handed_off = true;
        self.renderer.submit_with(recorded, submit)
    }
}
impl Drop for Recording2d<'_> {
    fn drop(&mut self) {
        drop(self.encoder.take());
        if !self.handed_off {
            self.renderer.retire(self.epoch);
        }
    }
}

struct ViewData {
    key: u64,
    client: bool,
    package: usize,
    group: wgpu::BindGroup,
    uniform: usize,
    upload: Option<View>,
    params: ViewParams,
    config: TargetConfig,
}

struct FrameData {
    epoch: u64,
    packages: Vec<usize>,
    views: Vec<usize>,
    sealed: bool,
    #[cfg(feature = "text")]
    atlas_upload: u64,
    #[cfg(feature = "text")]
    atlas_pages: Vec<usize>,
}
enum Notice {
    Cancel(u64),
    Complete(u64),
    ReleasePackage(usize, u64),
    ReleaseView(usize, u64),
}

/// 唯一拥有尚未提交的 command buffer；仅能交给原 renderer 提交或取消。
///
/// 丢弃此值只取消其仍拥有的未提交命令。提交会先取走命令，之后资源仅由完成消息退休。
pub struct Recorded2d {
    owner: u64,
    epoch: u64,
    raw: Option<wgpu::CommandBuffer>,
    notices: mpsc::Sender<Notice>,
}
impl Drop for Recorded2d {
    fn drop(&mut self) {
        if let Some(raw) = self.raw.take() {
            drop(raw);
            let _ = self.notices.send(Notice::Cancel(self.epoch));
        }
    }
}

static NEXT_RENDERER_ID: AtomicU64 = AtomicU64::new(1);

/// 持有有界上传池、glyph atlas、图片注册表和 pipeline 缓存的编码器。
///
/// 设备和队列句柄从调用者克隆，生命周期独立；本类型不等待或管理呈现；提交可经本 owner 或显式上层适配。
pub struct Renderer2d {
    owner: u64,
    device: wgpu::Device,
    queue: wgpu::Queue,
    shaders: Shaders2d,
    budgets: Budgets,
    geometry_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipeline_cache: Option<wgpu::PipelineCache>,
    white: Material,
    images: Vec<ImageSlot>,
    pipelines: Vec<(TargetConfig, wgpu::RenderPipeline)>,
    pool: BufferPool,
    #[cfg(feature = "text")]
    atlas: Atlas,
    instances: Vec<Instance>,
    material_indices: HashMap<MaterialKey, u32>,
    frames: Vec<FrameData>,
    packages: Vec<Option<PreparedData>>,
    views: Vec<Option<ViewData>>,
    resource_key: u64,
    epoch: u64,
    notices: mpsc::Sender<Notice>,
    received: mpsc::Receiver<Notice>,
    retired_images: Vec<(ImageKey, Material)>,
}

impl Renderer2d {
    /// 建立布局和内部白纹理；device、queue 和 shader 模块必须来自同一设备。
    ///
    /// 拒绝无效预算并分配进程内身份；不在此检查任意 shader 接口或 GPU error scope。
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shaders: Shaders2d,
        budgets: Budgets,
    ) -> Result<Self, RenderError> {
        Self::new_with_pipeline_cache(device, queue, shaders, budgets, None)
    }

    /// 建立同 [`Self::new`] 的 2D owner，并保留同设备的驱动缓存供后续目标格式使用。
    /// 缓存句柄不改变本地 pipeline 版本的分配或退休策略。
    pub fn new_with_pipeline_cache(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shaders: Shaders2d,
        budgets: Budgets,
        pipeline_cache: Option<wgpu::PipelineCache>,
    ) -> Result<Self, RenderError> {
        if budgets.max_instances == 0
            || budgets.max_instances > u32::MAX as usize
            || budgets.max_buffer_bytes < abi::INSTANCE_STRIDE
            || budgets.max_buffer_slots == 0
            || budgets.max_images > u32::MAX as usize
            || (cfg!(feature = "text")
                && (budgets.atlas_side < 3
                    || budgets.atlas_side > device.limits().max_texture_dimension_2d))
            || budgets.max_pipelines == 0
        {
            return Err(RenderError::InvalidBudget);
        }
        let owner = NEXT_RENDERER_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| RenderError::RendererIdOverflow)?;
        let geometry_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("2D 几何布局"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(abi::INSTANCE_STRIDE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(abi::VIEW_SIZE),
                    },
                    count: None,
                },
            ],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("2D 材质布局"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("2D pipeline 布局"),
            bind_group_layouts: &[Some(&geometry_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("2D white"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &[255; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            texture.size(),
        );
        let white = Material::new(
            device,
            &material_layout,
            SampledImage {
                view: texture.create_view(&Default::default()),
                sampler: device.create_sampler(&Default::default()),
            },
        );
        let (notices, received) = mpsc::channel();
        Ok(Self {
            owner,
            device: device.clone(),
            queue: queue.clone(),
            shaders,
            budgets,
            geometry_layout,
            material_layout,
            pipeline_layout,
            pipeline_cache,
            white,
            images: Vec::new(),
            pipelines: Vec::new(),
            pool: BufferPool::default(),
            #[cfg(feature = "text")]
            atlas: Atlas::default(),
            instances: Vec::new(),
            material_indices: HashMap::new(),
            frames: Vec::new(),
            packages: Vec::new(),
            views: Vec::new(),
            resource_key: 1,
            epoch: 1,
            notices,
            received,
            retired_images: Vec::new(),
        })
    }

    /// 接管 view／sampler 句柄并签发当前 renderer 图片键；可直接注册 3D pass 输出。
    ///
    /// 检查基础纹理用途与采样类型；具体 view 子资源和 sampler 兼容性仍由 wgpu 验证。
    /// 槽数超限返回 Budget，像素预乘／编码契约见 [`SampledImage`]。
    pub fn register_image(&mut self, image: SampledImage) -> Result<ImageKey, RenderError> {
        self.collect();
        let texture = image.view.texture();
        if texture.dimension() != wgpu::TextureDimension::D2
            || texture.sample_count() != 1
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
            || texture
                .format()
                .sample_type(None, Some(self.device.features()))
                != Some(wgpu::TextureSampleType::Float { filterable: true })
        {
            return Err(RenderError::InvalidImage);
        }
        let vacant = self.images.iter().position(|slot| slot.material.is_none());
        if vacant.is_none() && self.images.len() >= self.budgets.max_images {
            return Err(RenderError::Budget);
        }
        let material = Material::new(&self.device, &self.material_layout, image);
        let index = if let Some(index) = vacant {
            self.images[index].material = Some(material);
            index
        } else {
            self.images.push(ImageSlot {
                generation: 1,
                material: Some(material),
            });
            self.images.len() - 1
        };
        Ok(ImageKey {
            owner: self.owner,
            slot: index as u32,
            generation: self.images[index].generation,
        })
    }

    /// 移除图片并推进槽代次；已 Prepared 的图片继续有效，新 prepare 拒绝旧键。
    /// 不属于本 renderer／已过期的键返回 StaleImage，代次耗尽时不移除。
    pub fn remove_image(&mut self, key: ImageKey) -> Result<(), RenderError> {
        self.collect();
        self.image(key)?;
        let slot = &mut self.images[key.slot as usize];
        let generation = slot
            .generation
            .checked_add(1)
            .ok_or(RenderError::ImageGenerationOverflow)?;
        if let Some(material) = slot.material.take() {
            self.retired_images.push((key, material));
        }
        slot.generation = generation;
        self.reclaim();
        Ok(())
    }

    fn image(&self, key: ImageKey) -> Result<&Material, RenderError> {
        self.images
            .get(key.slot as usize)
            .filter(|slot| key.owner == self.owner && slot.generation == key.generation)
            .and_then(|slot| slot.material.as_ref())
            .ok_or(RenderError::StaleImage)
    }

    /// 校验场景，解析资源并批量准备上传，返回 owner 管理的几何包身份。
    ///
    /// 仅接受 shape／image；启用 `text` 后，含 glyph 的场景须改用 `prepare_text`。
    /// 超预算／无空闲槽分别返回 Budget／Busy。
    /// 失败可能已改变缓存、回收空闲资源或排定上传，不提供事务回滚。
    pub fn prepare(&mut self, list: &DrawList) -> Result<Prepared2d, RenderError> {
        self.prepare_impl(
            list,
            #[cfg(feature = "text")]
            None,
        )
    }

    /// 使用签发 glyph 请求的文字系统准备场景；无文字的场景也可使用。
    ///
    /// 静态无 hint outline 在栅格前按保守墨迹范围裁剪，其他字形正常栅格并由 scissor
    /// 裁剪。预算、上传和失败约定与 [`Self::prepare`] 相同。
    #[cfg(feature = "text")]
    pub fn prepare_text(
        &mut self,
        list: &DrawList,
        text: &mut TextSystem,
    ) -> Result<Prepared2d, RenderError> {
        self.prepare_impl(list, Some(text))
    }

    fn prepare_impl(
        &mut self,
        list: &DrawList,
        #[cfg(feature = "text")] mut text: Option<&mut TextSystem>,
    ) -> Result<Prepared2d, RenderError> {
        self.collect();
        list.validate()?;
        let count = list
            .shapes()
            .len()
            .checked_add(list.images().len())
            .ok_or(RenderError::Budget)?;
        #[cfg(feature = "text")]
        let count = count
            .checked_add(list.glyphs().len())
            .ok_or(RenderError::Budget)?;
        if count > self.budgets.max_instances {
            return Err(RenderError::Budget);
        }
        self.instances.clear();
        self.material_indices.clear();
        let mut batches = Vec::new();
        let mut materials = Vec::new();
        #[cfg(feature = "text")]
        let mut atlas_pages = Vec::new();
        #[cfg(feature = "text")]
        let protected_pages: Vec<_> = self
            .packages
            .iter()
            .flatten()
            .flat_map(|package| &package.pages)
            .chain(self.frames.iter().flat_map(|frame| &frame.atlas_pages))
            .copied()
            .collect();
        for op in list.ops() {
            match op {
                DrawOp::Shapes { range, clip } => {
                    let material = material_index(
                        &mut self.material_indices,
                        &mut materials,
                        MaterialKey::White,
                    );
                    for shape in &list.shapes()[range.start as usize..range.end as usize] {
                        append(
                            &mut self.instances,
                            &mut batches,
                            material,
                            *clip,
                            Instance {
                                rect: rectangle(shape.rect),
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: shape.color.0,
                                params: [0.0, shape.radius, shape.border, 0.0],
                            },
                        );
                    }
                }
                DrawOp::Images { range, image, clip } => {
                    self.image(*image)?;
                    let material = material_index(
                        &mut self.material_indices,
                        &mut materials,
                        MaterialKey::Image(*image),
                    );
                    for image in &list.images()[range.start as usize..range.end as usize] {
                        append(
                            &mut self.instances,
                            &mut batches,
                            material,
                            *clip,
                            Instance {
                                rect: rectangle(image.rect),
                                uv: rectangle(image.uv),
                                color: image.tint.0,
                                params: [1.0, 0.0, 0.0, 0.0],
                            },
                        );
                    }
                }
                #[cfg(feature = "text")]
                DrawOp::Glyphs { range, clip } => {
                    let text = text.as_deref_mut().ok_or(RenderError::FontSystemMismatch)?;
                    for request in &list.glyphs()[range.start as usize..range.end as usize] {
                        if request.font_set != text.id() {
                            return Err(RenderError::FontSystemMismatch);
                        }
                        if let Some(bounds) = text.ink_bounds(request.cache_key) {
                            let scale = request.scale;
                            let bounds = Rect::new(
                                request.origin.x + bounds.x / scale,
                                request.origin.y + bounds.y / scale,
                                bounds.width / scale,
                                bounds.height / scale,
                            );
                            if bounds.intersection(*clip).is_empty() {
                                continue;
                            }
                        }
                        let Some(glyph) = self.atlas.resolve(
                            &self.device,
                            &self.queue,
                            &self.material_layout,
                            &self.budgets,
                            text,
                            request.cache_key,
                            &mut atlas_pages,
                            &protected_pages,
                        )?
                        else {
                            continue;
                        };
                        let material = material_index(
                            &mut self.material_indices,
                            &mut materials,
                            MaterialKey::Atlas(glyph.page),
                        );
                        let scale = request.scale;
                        let rect = Rect::new(
                            request.origin.x + glyph.left as f32 / scale,
                            request.origin.y - glyph.top as f32 / scale,
                            glyph.width as f32 / scale,
                            glyph.height as f32 / scale,
                        );
                        if !rect.valid() {
                            return Err(SceneError::InvalidGeometry.into());
                        }
                        append(
                            &mut self.instances,
                            &mut batches,
                            material,
                            *clip,
                            Instance {
                                rect: rectangle(rect),
                                uv: rectangle(glyph.uv),
                                color: request.color.0,
                                params: [if glyph.mask { 3.0 } else { 1.0 }, 0.0, 0.0, 0.0],
                            },
                        );
                    }
                }
            }
        }
        let bytes = (self.instances.len() as u64 * abi::INSTANCE_STRIDE).max(abi::INSTANCE_STRIDE);
        if bytes > self.device.limits().max_storage_buffer_binding_size {
            return Err(RenderError::Budget);
        }
        let key = self.resource_key;
        let next_key = key.checked_add(1).ok_or(RenderError::Budget)?;
        let geometry = self
            .pool
            .acquire(&self.device, bytes, Kind::Geometry, &self.budgets)?;
        let index = self
            .packages
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.packages.len());
        let data = PreparedData {
            key,
            client: true,
            geometry,
            geometry_bytes: bytes,
            batches,
            materials,
            upload: std::mem::take(&mut self.instances),
            #[cfg(feature = "text")]
            pages: atlas_pages,
        };
        if index == self.packages.len() {
            self.packages.push(Some(data));
        } else {
            self.packages[index] = Some(data);
        }
        self.resource_key = next_key;
        Ok(Prepared2d {
            owner: self.owner,
            key,
            index,
            notices: self.notices.clone(),
        })
    }

    /// 为本 renderer 的绘制包创建独立 uniform，并取得格式／采样数对应 pipeline。
    ///
    /// 拒绝其他 renderer 的包、无效映射或不支持的目标配置；viewport 须非负，
    /// 是否落在实际附件内延至 encode 检查。需要单独的 buffer 槽与预算，不覆盖其他视图。
    pub fn prepare_view(
        &mut self,
        prepared: &Prepared2d,
        params: ViewParams,
        config: TargetConfig,
    ) -> Result<PreparedView2d, RenderError> {
        self.collect();
        if prepared.owner != self.owner
            || !self
                .packages
                .get(prepared.index)
                .and_then(Option::as_ref)
                .is_some_and(|data| data.key == prepared.key && data.client)
        {
            return Err(RenderError::ForeignPrepared);
        }
        if !params.logical.valid()
            || params.logical.is_empty()
            || !params.viewport.valid()
            || params.viewport.is_empty()
            || params.viewport.x < 0.0
            || params.viewport.y < 0.0
        {
            return Err(RenderError::InvalidTarget);
        }
        let features = config
            .format
            .guaranteed_format_features(self.device.features());
        if !features
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            || !features
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE)
            || !features.flags.sample_count_supported(config.samples)
        {
            return Err(RenderError::InvalidTarget);
        }
        self.pipeline(config)?;
        let logical = params.logical;
        let view = View {
            transform: [
                2.0 / logical.width,
                -2.0 / logical.height,
                -1.0 - 2.0 * logical.x / logical.width,
                1.0 + 2.0 * logical.y / logical.height,
            ],
            pixel: [
                logical.width / params.viewport.width,
                logical.height / params.viewport.height,
                0.0,
                0.0,
            ],
        };
        if !view
            .transform
            .into_iter()
            .chain(view.pixel)
            .all(f32::is_finite)
        {
            return Err(RenderError::InvalidTarget);
        }
        let key = self.resource_key;
        let next_key = key.checked_add(1).ok_or(RenderError::Budget)?;
        let uniform = self
            .pool
            .acquire(&self.device, abi::VIEW_SIZE, Kind::View, &self.budgets)?;
        let package = self.packages[prepared.index].as_ref().expect("活动几何");
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("2D 视图"),
            layout: &self.geometry_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: self.pool.buffer(package.geometry),
                        offset: 0,
                        size: NonZeroU64::new(package.geometry_bytes),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.pool.buffer(uniform).as_entire_binding(),
                },
            ],
        });
        let index = self
            .views
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.views.len());
        let data = ViewData {
            key,
            client: true,
            package: prepared.index,
            group,
            uniform,
            upload: Some(view),
            params,
            config,
        };
        if index == self.views.len() {
            self.views.push(Some(data));
        } else {
            self.views[index] = Some(data);
        }
        self.resource_key = next_key;
        Ok(PreparedView2d {
            owner: self.owner,
            key,
            index,
            notices: self.notices.clone(),
        })
    }

    fn pipeline(&mut self, config: TargetConfig) -> Result<(), RenderError> {
        if let Some((_, _pipeline)) = self.pipelines.iter().find(|(key, _)| *key == config) {
            return Ok(());
        }
        if self.pipelines.len() >= self.budgets.max_pipelines {
            let index = self
                .pipelines
                .iter()
                .position(|(config, _)| {
                    !self
                        .views
                        .iter()
                        .flatten()
                        .any(|view| view.config == *config)
                })
                .ok_or(RenderError::Busy)?;
            self.pipelines.swap_remove(index);
        }
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("2D"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shaders.vertex,
                    entry_point: Some("vertex_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: config.samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shaders.fragment,
                    entry_point: Some("fragment_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: self.pipeline_cache.as_ref(),
            });
        self.pipelines.push((config, pipeline));
        Ok(())
    }

    /// 接管本批次 encoder；guard 结束前不能开始其他录制或移出命令。
    pub fn recording(
        &mut self,
        encoder: wgpu::CommandEncoder,
    ) -> Result<Recording2d<'_>, RenderError> {
        self.collect();
        let epoch = self.epoch;
        self.epoch = epoch.checked_add(1).ok_or(RenderError::Budget)?;
        self.frames.push(FrameData {
            epoch,
            packages: Vec::new(),
            views: Vec::new(),
            sealed: false,
            #[cfg(feature = "text")]
            atlas_upload: 0,
            #[cfg(feature = "text")]
            atlas_pages: Vec::new(),
        });
        Ok(Recording2d {
            renderer: self,
            encoder: Some(encoder),
            epoch,
            handed_off: false,
        })
    }

    /// 固化当前批次并接管 encoder；未提交对象可显式取消或直接丢弃。
    /// 所属批次的全部 encode 必须录入这个 encoder；几何包和视图身份仍可供后续批次复用。
    fn finish_batch(&mut self, encoder: wgpu::CommandEncoder) -> Result<Recorded2d, RenderError> {
        let frame = self
            .frames
            .iter_mut()
            .find(|frame| !frame.sealed)
            .ok_or(RenderError::ForeignPrepared)?;
        frame.sealed = true;
        Ok(Recorded2d {
            owner: self.owner,
            epoch: frame.epoch,
            raw: Some(encoder.finish()),
            notices: self.notices.clone(),
        })
    }

    /// 使用本 renderer 的队列提交唯一命令批次；完成回调只发送批次身份。
    pub fn submit(&mut self, recorded: Recorded2d) -> Result<wgpu::SubmissionIndex, RenderError> {
        let queue = self.queue.clone();
        self.submit_with(recorded, |raw| Ok(queue.submit([raw])))
    }

    /// 交给上层唯一提交器，可和同队列 native 命令合批；成功须表示实际已提交。
    /// 失败后完成状态未知，保留资源至 owner 销毁，不把错误或断线当作 GPU 完成。
    pub fn submit_with<T, E: From<RenderError>>(
        &mut self,
        mut recorded: Recorded2d,
        submit: impl FnOnce(wgpu::CommandBuffer) -> Result<T, E>,
    ) -> Result<T, E> {
        self.collect();
        if recorded.owner != self.owner {
            return Err(RenderError::ForeignPrepared.into());
        }
        let raw = recorded
            .raw
            .take()
            .ok_or_else(|| E::from(RenderError::ForeignPrepared))?;
        let result = submit(raw)?;
        if let Some(frame) = self
            .frames
            .iter()
            .find(|frame| frame.epoch == recorded.epoch)
        {
            for &index in &frame.packages {
                let data = self.packages[index].as_mut().expect("批次几何");
                let mut upload = std::mem::take(&mut data.upload);
                upload.clear();
                if upload.capacity() > self.instances.capacity() {
                    self.instances = upload;
                }
            }
            for &index in &frame.views {
                self.views[index].as_mut().expect("批次视图").upload = None;
            }
        }
        #[cfg(feature = "text")]
        if let Some(frame) = self
            .frames
            .iter()
            .find(|frame| frame.epoch == recorded.epoch)
        {
            self.atlas.submitted(frame.atlas_upload);
        }
        let notices = self.notices.clone();
        let epoch = recorded.epoch;
        self.queue.on_submitted_work_done(move || {
            let _ = notices.send(Notice::Complete(epoch));
        });
        Ok(result)
    }

    /// 取消仍由此对象独占的未提交命令；不会伪造 GPU 完成。
    pub fn cancel(&mut self, mut recorded: Recorded2d) -> Result<(), RenderError> {
        if recorded.owner != self.owner {
            return Err(RenderError::ForeignPrepared);
        }
        drop(recorded.raw.take());
        self.retire(recorded.epoch);
        Ok(())
    }

    /// 吸收已提交批次的完成消息和唯一未提交对象的取消消息。
    pub fn collect(&mut self) {
        while let Ok(notice) = self.received.try_recv() {
            match notice {
                Notice::Cancel(epoch) | Notice::Complete(epoch) => self.retire(epoch),
                Notice::ReleasePackage(index, key) => {
                    if let Some(data) = self
                        .packages
                        .get_mut(index)
                        .and_then(Option::as_mut)
                        .filter(|data| data.key == key)
                    {
                        data.client = false;
                    }
                }
                Notice::ReleaseView(index, key) => {
                    if let Some(data) = self
                        .views
                        .get_mut(index)
                        .and_then(Option::as_mut)
                        .filter(|data| data.key == key)
                    {
                        data.client = false;
                    }
                }
            }
        }
        self.reclaim();
    }

    fn retire(&mut self, epoch: u64) {
        if let Some(index) = self.frames.iter().position(|frame| frame.epoch == epoch) {
            self.frames.swap_remove(index);
        }
        self.reclaim();
    }

    fn reclaim(&mut self) {
        for (index, slot) in self.views.iter_mut().enumerate() {
            if slot.as_ref().is_some_and(|data| !data.client)
                && !self.frames.iter().any(|frame| frame.views.contains(&index))
            {
                let data = slot.take().expect("退休视图");
                self.pool.release(data.uniform);
            }
        }
        for (index, slot) in self.packages.iter_mut().enumerate() {
            if slot.as_ref().is_some_and(|data| !data.client)
                && !self
                    .views
                    .iter()
                    .flatten()
                    .any(|view| view.package == index)
                && !self
                    .frames
                    .iter()
                    .any(|frame| frame.packages.contains(&index))
            {
                let data = slot.take().expect("退休几何");
                self.pool.release(data.geometry);
            }
        }
        self.retired_images.retain(|(key, _)| {
            self.packages
                .iter()
                .flatten()
                .any(|package| package.materials.contains(&MaterialKey::Image(*key)))
        });
    }

    /// 读取当前保留工作集，不等待 GPU，也不包含驱动／staging／外部图片容量。
    pub fn stats(&self) -> Stats {
        #[cfg(feature = "text")]
        let (atlas_bytes, atlas_pages, atlas_glyphs, atlas_scratch) = (
            self.atlas.bytes(),
            self.atlas.pages(),
            self.atlas.glyphs(),
            self.atlas.scratch_bytes(),
        );
        #[cfg(not(feature = "text"))]
        let (atlas_bytes, atlas_pages, atlas_glyphs, atlas_scratch) = (0, 0, 0, 0);
        Stats {
            buffer_bytes: self.pool.bytes(),
            buffer_slots: self.pool.len(),
            atlas_bytes,
            atlas_pages,
            atlas_glyphs,
            images: self
                .images
                .iter()
                .filter(|slot| slot.material.is_some())
                .count(),
            pipelines: self.pipelines.len(),
            scratch_bytes: self.instances.capacity() * size_of::<Instance>()
                + self
                    .packages
                    .iter()
                    .flatten()
                    .map(|package| package.upload.capacity() * size_of::<Instance>())
                    .sum::<usize>()
                + atlas_scratch,
        }
    }
}

impl Renderer2d {
    /// 录制一个颜色 render pass，不提交队列；附件可用 Load 接续应用的 3D pass。
    ///
    /// 先检查附件基础参数及 viewport，再录制当前批次上传和绘制。整数 scissor
    /// 向外取整，无分数像素裁剪覆盖率；同批次可编码到多个目标。query set、view 子资源、
    /// shader 与设备的完整兼容性由调用者和 wgpu 验证。完成回调不代表屏幕显示时间。
    fn encode(
        &mut self,
        identity: &PreparedView2d,
        encoder: &mut wgpu::CommandEncoder,
        target: RenderTarget<'_>,
    ) -> Result<(), RenderError> {
        use wgpu::util::DeviceExt;
        self.collect();
        let frame_index = self
            .frames
            .iter()
            .position(|frame| !frame.sealed)
            .ok_or(RenderError::ForeignPrepared)?;
        if identity.owner != self.owner {
            return Err(RenderError::ForeignPrepared);
        }
        let view = self
            .views
            .get(identity.index)
            .and_then(Option::as_ref)
            .filter(|view| view.key == identity.key && view.client)
            .ok_or(RenderError::ForeignPrepared)?;
        check_attachment(target.view, target.extent, view.config)?;
        if let Some(resolve) = target.resolve {
            if view.config.samples == 1 {
                return Err(RenderError::InvalidTarget);
            }
            check_attachment(
                resolve,
                target.extent,
                TargetConfig {
                    samples: 1,
                    ..view.config
                },
            )?;
        }
        let viewport = view.params.viewport;
        if viewport.right() > target.extent[0] as f32 || viewport.bottom() > target.extent[1] as f32
        {
            return Err(RenderError::InvalidTarget);
        }
        let package_index = view.package;
        let frame = &mut self.frames[frame_index];
        let first_encode = frame.views.is_empty();
        if !frame.packages.contains(&package_index) {
            frame.packages.push(package_index);
            let package = self.packages[package_index].as_ref().expect("活动几何");
            if !package.upload.is_empty() {
                let staging = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("2D geometry upload"),
                        contents: bytemuck::cast_slice(&package.upload),
                        usage: wgpu::BufferUsages::COPY_SRC,
                    });
                encoder.copy_buffer_to_buffer(
                    &staging,
                    0,
                    self.pool.buffer(package.geometry),
                    0,
                    package.upload.len() as u64 * abi::INSTANCE_STRIDE,
                );
            }
        }
        if !frame.views.contains(&identity.index) {
            frame.views.push(identity.index);
            if let Some(upload) = &view.upload {
                let staging = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("2D view upload"),
                        contents: bytemuck::bytes_of(upload),
                        usage: wgpu::BufferUsages::COPY_SRC,
                    });
                encoder.copy_buffer_to_buffer(
                    &staging,
                    0,
                    self.pool.buffer(view.uniform),
                    0,
                    abi::VIEW_SIZE,
                );
            }
        }
        #[cfg(feature = "text")]
        if first_encode {
            self.atlas.pending_pages(&mut frame.atlas_pages);
            frame.atlas_upload = self.atlas.encode_uploads(&self.device, encoder);
        }
        #[cfg(not(feature = "text"))]
        let _ = first_encode;
        let data = self.packages[view.package].as_ref().expect("活动几何");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("2D"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: target.resolve,
                ops: wgpu::Operations {
                    load: target.load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: target.timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(
            &self
                .pipelines
                .iter()
                .find(|(config, _)| *config == view.config)
                .expect("活动管线")
                .1,
        );
        pass.set_bind_group(0, &view.group, &[]);
        pass.set_viewport(
            viewport.x,
            viewport.y,
            viewport.width,
            viewport.height,
            0.0,
            1.0,
        );
        let mut current_material = None;
        for batch in &data.batches {
            let Some([x, y, width, height]) = scissor(batch.clip, view.params, target.extent)
            else {
                continue;
            };
            if current_material != Some(batch.material) {
                pass.set_bind_group(
                    1,
                    &match data.materials[batch.material as usize] {
                        MaterialKey::White => &self.white,
                        MaterialKey::Image(key) => self
                            .images
                            .get(key.slot as usize)
                            .filter(|slot| slot.generation == key.generation)
                            .and_then(|slot| slot.material.as_ref())
                            .or_else(|| {
                                self.retired_images
                                    .iter()
                                    .find(|(old, _)| *old == key)
                                    .map(|(_, m)| m)
                            })
                            .expect("活动材料版本"),
                        #[cfg(feature = "text")]
                        MaterialKey::Atlas(page) => self.atlas.material(page),
                    }
                    .group,
                    &[],
                );
                current_material = Some(batch.material);
            }
            pass.set_scissor_rect(x, y, width, height);
            pass.draw(0..6, batch.range.clone());
        }
        Ok(())
    }
}

fn check_attachment(
    view: &wgpu::TextureView,
    extent: [u32; 2],
    config: TargetConfig,
) -> Result<(), RenderError> {
    let texture = view.texture();
    if extent.contains(&0)
        || extent[0] > texture.width()
        || extent[1] > texture.height()
        || texture.sample_count() != config.samples
        || texture.format().remove_srgb_suffix() != config.format.remove_srgb_suffix()
        || !texture
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
    {
        return Err(RenderError::InvalidTarget);
    }
    Ok(())
}

fn material_index(
    indices: &mut HashMap<MaterialKey, u32>,
    materials: &mut Vec<MaterialKey>,
    key: MaterialKey,
) -> u32 {
    *indices.entry(key).or_insert_with(|| {
        let index = materials.len() as u32;
        materials.push(key);
        index
    })
}

fn rectangle(rect: Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}

fn append(
    instances: &mut Vec<Instance>,
    batches: &mut Vec<Batch>,
    material: u32,
    clip: Rect,
    instance: Instance,
) {
    let index = instances.len() as u32;
    instances.push(instance);
    if let Some(batch) = batches.last_mut()
        && batch.material == material
        && batch.clip == clip
    {
        batch.range.end = index + 1;
    } else {
        batches.push(Batch {
            range: index..index + 1,
            clip,
            material,
        });
    }
}

/// clip 向外取整为物理像素 scissor；不计算分数像素裁剪覆盖率。
fn scissor(clip: Rect, view: ViewParams, extent: [u32; 2]) -> Option<[u32; 4]> {
    let clip = clip.intersection(view.logical);
    if clip.is_empty() {
        return None;
    }
    let viewport = view.viewport;
    let sx = viewport.width / view.logical.width;
    let sy = viewport.height / view.logical.height;
    let x = (viewport.x + (clip.x - view.logical.x) * sx)
        .floor()
        .max(0.0) as u32;
    let y = (viewport.y + (clip.y - view.logical.y) * sy)
        .floor()
        .max(0.0) as u32;
    let right = ((viewport.x + (clip.right() - view.logical.x) * sx).ceil() as u32).min(extent[0]);
    let bottom =
        ((viewport.y + (clip.bottom() - view.logical.y) * sy).ceil() as u32).min(extent[1]);
    (right > x && bottom > y).then_some([x, y, right.saturating_sub(x), bottom.saturating_sub(y)])
}
