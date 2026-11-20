//! 通用 2D/3D 组件的统一入口；应用策略由上层维护。
//!
//! # 组件边界
//!
//! CPU 组件提供可独立使用的计算、输入和场景数据。普通 GPU 绘制经 wgpu 接入，原生光追可以
//! 使用 wgpu 的 Vulkan 设备，也可借用外部宿主设备。设备句柄、命令和资源安全契约由 Vulkan
//! crate 维护；应用负责场景适配、调度、提交完成与呈现。
//!
//! # 功能裁剪
//!
//! 默认没有 feature，不引入 GPU 或 shader 编译。底层 crate 只依赖实际使用的组件，
//! 应用可直接依赖它们，或通过本入口选择功能。Cargo feature 只叠加；`--workspace` 会统一成员请求，
//! 检查裁剪时应单独选择 `-p simple_engine --no-default-features`。
//!
//! | Feature | 组件与附加能力 |
//! |---|---|
//! | `color` | 线性与编码色彩转换 |
//! | `spectrum` | CIE 1931 观察者、D65、黑体光谱、波段积分与 XYZ 到线性 sRGB |
//! | `camera` | 正交 pinhole 相机、锚点相对坐标、无 jitter 矩阵与 Halton 偏移 |
//! | `display` | Prime DRT、自动曝光与 GPU 测光，原生执行另启用 `vulkan` |
//! | `fft_bloom` | 二维光斑与标定镜头的 FFT 卷积，原生录制另启用 `vulkan` |
//! | `circular_bloom` | 原生 Vulkan 的快速圆孔泛光，按空间尺度滤波 |
//! | `fft_bloom_calibration` | 光源与探测器的求积计划，供镜头标定与衍射核编辑器使用 |
//! | `openpbr` / `openpbr_program` / `openpbr_gpu` | OpenPBR 能力子集材料与介质；后两者分别增加 Slang 程序准备与原作者 HALF4 能量纹理 |
//! | `pt` / `pt_gpu` | 场景无关的路径预算、样本事务；后者增加 Vulkan 三阶段录制 |
//! | `restir_pt` / `restir_pt_gpu` | ReSTIR PT 默认point profile、Morton工作集与历史事务；后者增加十阶段Vulkan录制 |
//! | `atmosphere` / `atmosphere_gpu` / `atmosphere_native` | 四波长球层大气参数；后两者提供 wgpu／native 多重散射、SkyView 与 Aerial owner |
//! | `input` / `input_recording` | 设备无关命令；后者增加 JSON 录制 |
//! | `frame_loop` | 会话时钟、绝对 tick 期限、单槽完整值交接与帧间隔统计/保峰 |
//! | `render2d` | CPU 有序 2D 场景与图表 |
//! | `render2d_text` | 字体排版与文字编辑 |
//! | `render2d_gpu` | wgpu 2D 与 Slang 构建，文字和 backend 由应用选择 |
//! | `ui` | CPU GUI，包含 input 与 render2d/text |
//! | `rectangle_decomposition` / `voxel_mesh` | 最少矩形划分、分层稀疏体素表面；后者包含前者 |
//! | `chunk_manager` | 预算驱动的 3D LOD 区块规划、异步状态与资源所有权 |
//! | `scene_cache` | 场景资源目录、属性槽与增量发布事务 |
//! | `hash_dag` | 64 叉体素 DAG、批量读写、区域展开与 CPU 射线 |
//! | `light_bvh` | quad 光源树的底层构建 |
//! | `light_sampling` | 统一光源树、面积与纹理 alias、活动灯目录 |
//! | `opacity` | 精确 texel 对齐的二值 OMM 准备与 bird 遍历 |
//! | `sip_hash` / `perlin_noise` | SipHash 与批量梯度噪声；缓存和地形策略由应用持有 |
//! | `gpu_log` | 进程日志与错误计数，无 GPU 依赖 |
//! | `wgpu` / `webgpu` | wgpu API；后者选择 WebGPU backend 与 WGSL 输入 |
//! | `vulkan` | native 设备接入、SPIR-V 直通与光追，同时选择 Vulkan backend |
//! | `dlss_rr` | Windows Streamline／DLSS RR；SDK 路径由 `STREAMLINE_SDK` 显式指定 |
//! | `nrd` | Windows x64 Vulkan 的 REBLUR SH 与 SIGMA，使用 NRD 官方 SPIR-V |
//! | `gpu_test` | native GPU 检验，包含 vulkan 与 gpu_log |
//! | `shader_build` / `shader_library` | Slang 构建工具与数值模块源码入口 |
//!
//! `pt_gpu`、`restir_pt_gpu`、`atmosphere_native`、`vulkan`、`gpu_test` 仅支持 native。native 直接执行 Slang SPIR-V；
//! Web 使用 Slang WGSL。普通绘制使用 wgpu，native PT 可借用 wgpu 或外部宿主设备。
//! 构建依赖只需按需启用 shader feature；算法与资源生命周期见各组件文档。
//!
//! ```toml
//! [dependencies]
//! simple_engine = { path = "path/to/simple_engine", features = ["ui", "render2d_gpu", "vulkan"] }
//!
//! [build-dependencies]
//! simple_engine = { path = "path/to/simple_engine", features = ["shader_build", "shader_library"] }
//! ```

#![forbid(unsafe_code)]

#[cfg(all(feature = "vulkan", target_arch = "wasm32"))]
compile_error!("vulkan 和 gpu_test 仅支持 native；Web 使用 webgpu feature。");

#[cfg(feature = "atmosphere")]
pub use atmosphere;
#[cfg(feature = "camera")]
pub use camera;
#[cfg(feature = "chunk_manager")]
pub use chunk_manager;
#[cfg(all(feature = "circular_bloom", not(target_arch = "wasm32")))]
pub use circular_bloom;
#[cfg(feature = "color")]
pub use color;
#[cfg(feature = "display")]
pub use display;
#[cfg(feature = "fft_bloom")]
pub use fft_bloom;
#[cfg(feature = "frame_loop")]
pub use frame_loop;
#[cfg(feature = "gpu_log")]
pub use gpu_log;
#[cfg(all(feature = "gpu_test", not(target_arch = "wasm32")))]
pub use gpu_test;
#[cfg(feature = "hash_dag")]
pub use hash_dag;
#[cfg(feature = "input")]
pub use input;
#[cfg(feature = "light_bvh")]
pub use light_bvh;
#[cfg(feature = "openpbr")]
pub use openpbr;
#[cfg(feature = "pt")]
pub use pt;
#[cfg(feature = "restir_pt")]
pub use restir_pt;

#[cfg(feature = "light_sampling")]
#[doc(inline)]
pub use light_sampling;
#[cfg(feature = "perlin_noise")]
pub use perlin_noise;
#[cfg(feature = "rectangle_decomposition")]
pub use rectangle_decomposition;
#[cfg(feature = "render2d")]
pub use render2d;
#[cfg(feature = "scene_cache")]
pub use scene_cache;
#[cfg(feature = "shader_build")]
pub use shader_build;
#[cfg(feature = "sip_hash")]
pub use sip_hash;
#[cfg(feature = "ui")]
pub use ui;
#[cfg(feature = "voxel_mesh")]
pub use voxel_mesh;
#[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
pub use vulkan;
#[cfg(feature = "wgpu")]
pub use wgpu;

#[cfg(feature = "opacity")]
pub use opacity;

#[cfg(feature = "shader_library")]
pub mod shader_library {
    //! 引擎生产 Slang 模块的源码入口，不读取场景或绑定 GPU 资源。
    //!
    //! [`root`] 只返回随包分发的源码目录，本 feature 不依赖 GPU 或编译器；调用构建设施时
    //! 才需要 Slang。源码使用 [Slang 2026](https://docs.shader-slang.org/en/stable/external/slang/docs/user-guide/11-language-version.html)，
    //! 主文件显式声明 `module`，公开 API 标注 `public`，其余顶层声明为 `internal`。
    //! `import` 表达模块依赖，`namespace` 组织符号，不要求对应 Rust crate；大文件可按
    //! [模块与访问控制](https://docs.shader-slang.org/en/latest/external/slang/docs/user-guide/04-modules-and-access-control.html)
    //! 使用 `__include`／`implementing` 拆分。
    //!
    //! 显示模块的额外 include 根由 `display::shader_root()` 提供；`fft_bloom` 和 `circular_bloom` 各自构建所需的计算程序。
    //! 具体参数、数值前提与返回语义写在各 Slang 公开声明的文档注释中：
    //!
    //! | 模块 | 接口与约定索引 |
    //! |---|---|
    //! | `math.constants` | `math::pi`、`math::tau` 等共享 float32 数学常量；不混入编码尺度或算法容差 |
    //! | `color.primaries` / `color.srgb` / `color.hsv` | 线性 D65 原色与亮度、sRGB 传递函数、RGB 与 HSV 转换；显示裁剪策略由调用方选择 |
    //! | `openpbr.api` | OpenPBR 能力子集；完整闭包 response/PDF、显式介质及原始能量纹理，不宣称完整标准 |
    //! | `openpbr.denoise` | 与闭包一致的方向 diffuse/specular 能量；不包含照明、Beer 或曝光 |
    //! | `atmosphere.parameters` / `atmosphere.sampling` | 四波长球形边界、SkyView、太阳 T 与有限距离 Aerial L/T；参数与数据由 atmosphere crate 维护 |
    //! | `transport.weights` / `transport.medium` | 指数拆分贡献、同量度 PDF 的稳定 power heuristic、均匀介质的 Beer 透射 |
    //! | `transport.surface_material` | PT 与 ReSTIR 共用的 OpenPBR 表面翻译结果、介质边界和着色法线 |
    //! | `pt.scene` / `pt.integrator` | 场景到 OpenPBR 的编译期适配、K1/K2 输运、NEE/MIS/RR；Rust 阶段录制由 `pt_gpu` 提供 |
    //! | `restir_pt.*` | ReSTIR PT Enhanced默认point profile的reservoir、重放、shift与MIS；场景适配与程序由上层注入，Rust录制由 `restir_pt_gpu` 提供 |
    //! | `restir_pt.control_variate` | ReSTCV 双向差值与像素颜色累计；独立于路径赢家，保留有限负 RGB，系数及颜色历史由上层维护 |
    //! | `pt.path_sampler` | PT 固定消费域、256 点 epoch 与灯／BSDF 组内定向映射；分批不改变绝对样本身份 |
    //! | `pt.surface_guides` / `denoise.guide` | K1 实际首表面导引，以及共享的线性深度和左上原点像素运动约定 |
    //! | `pt.reconstruction` | 独立规范 delta guide、PSR、刚体前态和反射运动；预算/表面/运动代理显式标记，不改变照明 |
    //! | `denoise.spatial` | 5x5 单帧几何/材质边界保护与尺寸映射重建；不含时域历史或显示变换 |
    //! | `pt.accumulation` | 每样本线性 HDR 累积与粘滞失败；输出适配器负责合成及显示 |
    //! | `sampling.pcg` / `sampling.hemisphere` / `sampling.cone` | PCG hash、状态随机数、余弦半球和均匀立体角圆锥 |
    //! | `sampling.z_sobol` | 固定 domain 的分层 Morton 排列、二维 Sobol 与 FastOwen；支持范围由调用方校验 |
    //! | `sampling.binary_sobol` | 专用 S8 binary nested 原语；定向 3D 保护四点半区投影，接受 scalar 两点退化，非通用高维采样器 |
    //! | `transport.roulette` | etaScale 更新、maxRGB 存活概率与重加权；启动散射次数由积分器选择 |
    //! | `pt.guide_output` | 共享天空、表面、反射导引写入与 FP16 HDR 输入边界 |
    //! | `pt.pixel_transport` | 一／二样本和分离 lane 的外层 PT 循环 |
    //! | `light_sampling.quad` / `light_sampling.sample` | 均匀或纹理发光提案、连接射线与正反向 PDF，以及直接照明样本 |
    //! | `light_bvh.sample` / `light_sampling.area` | 光源树的选择概率、灯面面积表示与立体角 PDF |
    //! | `bsdf.lambert` | 单面 Lambertian eval／PDF／sample，方向指离开表面，PDF 为立体角量度 |
    //! | `geometry.ray` | 三角形重建及散射／连接射线；只有单位方向的 t 才表示距离 |
    //! | `camera.pinhole` | 左上原点 pinhole 射线、正 view-Z 投影与像素 jitter；使用输出 aspect、输入 extent |
    //! | `scene_cache.identity` | 依据完整 token 查询活动身份目录；目录与恢复数据在同一事务中发布 |
    //! | `opacity/micromap.inc.slang` | KHR OMM 的 RayQuery 逐入口准入，须由支持该扩展的宿主启用 |
    //! | `voxel.dag` | 64 叉 DAG 叶查询、祖先缓存与整数 DDA；资源、坐标映射和叶策略由调用者提供 |
    //!
    //! CPU 光源树的构建与 words 布局见 `light_bvh`。共享积分器不读取游戏场景；
    //! 使用底层数值模块的其他积分器可自行选择 NEE、随机域、RR 与路径预算。
    //! 重建的图像格式、场景对应和历史由接入层维护；可选 `vulkan::streamline` 封装 SDK，
    //! 应用仍负责设备接入、提交和完成证明。空间核不能替代时域重建质量验收。
    //! 采样与 BSDF 来源为 [PCG](https://www.jcgt.org/published/0009/03/02/) 和 PBRT 的
    //! [半球采样](https://pbr-book.org/4ed/Sampling_Algorithms/Sampling_Multidimensional_Functions)、
    //! [Lambertian](https://pbr-book.org/4ed/Reflection_Models/Diffuse_Reflection)、
    //! [MIS](https://pbr-book.org/4ed/Monte_Carlo_Integration/Improving_Efficiency)。
    //!
    //! 自相交规避移植自 NVIDIA
    //! [self-intersection-avoidance](https://github.com/NVIDIA/self-intersection-avoidance/tree/5a5c45bc61aca5d095032860058481e0497350e0)
    //! 的 [HLSL](https://github.com/NVIDIA/self-intersection-avoidance/blob/5a5c45bc61aca5d095032860058481e0497350e0/SelfIntersectionAvoidance.hlsl)
    //! ／[GLSL](https://github.com/NVIDIA/self-intersection-avoidance/blob/5a5c45bc61aca5d095032860058481e0497350e0/SelfIntersectionAvoidance.glsl)，
    //! 许可随包保留于 `LICENSES/NVIDIA-self-intersection-avoidance.txt`。其硬件求交误差系数
    //! 面向 NVIDIA RTX，其他 GPU 需验证，极掠射与 float32 退化也不属于统一保证。
    //! 连接射线扩展依据 [Connection rays 分析](https://developer.nvidia.com/blog/solving-self-intersection-artifacts-in-directx-raytracing/#connection_rays)。

    /// 随引擎包分发的源码目录；不依赖调用方的工作目录。
    pub const fn root() -> &'static str {
        concat!(env!("CARGO_MANIFEST_DIR"), "/shaders")
    }

    /// Falcor point profile 的原始三组 paired 邻居表；每组 65536 个 little-endian u32。
    /// 与 `restir_pt.paired` 的尺寸和变换共同使用，不依赖工作目录或运行时源码路径。
    pub const fn restir_paired_neighbors() -> &'static [u8] {
        include_bytes!("../shaders/restir_pt/paired.bin")
    }
}
