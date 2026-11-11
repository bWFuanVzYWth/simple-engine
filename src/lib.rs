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
//! | `camera` | 正交 pinhole 相机、锚点相对坐标、无 jitter 矩阵与 Halton 偏移 |
//! | `display` | Prime DRT 参数与自动曝光帧校验；叠加 `vulkan` 提供 GPU 直方图与时域适应 |
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
#[cfg(feature = "color")]
pub use color;
#[cfg(feature = "openpbr")]
pub use openpbr;
#[cfg(feature = "pt")]
pub use pt;
#[cfg(feature = "restir_pt")]
pub use restir_pt;
#[cfg(feature = "display")]
pub mod display;
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
#[cfg(feature = "shader_library")]
pub mod shader_library;
#[cfg(feature = "voxel_mesh")]
pub use voxel_mesh;
#[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
pub use vulkan;
#[cfg(feature = "wgpu")]
pub use wgpu;

#[cfg(feature = "opacity")]
pub use opacity;
