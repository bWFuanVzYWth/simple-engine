//! 引擎生产 Slang 模块的源码入口，不读取场景或绑定 GPU 资源。
//!
//! [`root`] 只返回随包分发的源码目录，本 feature 不依赖 GPU 或编译器；调用构建设施时
//! 才需要 Slang。源码使用 [Slang 2026](https://docs.shader-slang.org/en/stable/external/slang/docs/user-guide/11-language-version.html)，
//! 主文件显式声明 `module`，公开 API 标注 `public`，其余顶层声明为 `internal`。
//! `import` 表达模块依赖，`namespace` 组织符号，不要求对应 Rust crate；大文件可按
//! [模块与访问控制](https://docs.shader-slang.org/en/latest/external/slang/docs/user-guide/04-modules-and-access-control.html)
//! 使用 `__include`／`implementing` 拆分。
//!
//! 具体参数、数值前提与返回语义写在各 Slang 公开声明的文档注释中：
//!
//! | 模块 | 接口与约定索引 |
//! |---|---|
//! | `math.constants` | `math::pi`、`math::tau` 等共享 float32 数学常量；不混入编码尺度或算法容差 |
//! | `display.prime_drt` | Prime DRT；线性 BT.709／Rec.2020 输入，编码 sRGB 或线性显示输出，参数由 `display` feature 准备 |
//! | `display.exposure` / `display.exposure_compute` | 256 桶对数测光、裁尾与时域适应；未曝光线性 buffer／texture 输入，资源与提交由宿主维护 |
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
