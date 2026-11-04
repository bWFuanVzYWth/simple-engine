# 固定 Earth 物理输入

`medium.bin` 是 12,474 个小端 float4（199,584 B），
包括高度、剖面消光/五类散射系数、四种气溶胶相函数、相函数质量、积分与坐标节点。
`params.bin` 是与 Slang `atmosphere::Params` 同顺序的 400 B POD；`params.json` 供人工核对。
`mapping_fit.json` 和 `wavelengths.json` 保留源标定；文件摘要在 `sha256.json`。
这些数据不包含 optical depth、MS source、SkyView 或 aerial 辐射场。

模型保留源 450/510/580/650 nm、半径 6360 km、大气厚度 120 km、地表反照率 .18、
太阳角半径 .00465047 rad，以及四种气溶胶 inso/waso/soot/suso。
长度使用 km，消光/散射系数使用 km⁻¹，相函数使用 sr⁻¹，太阳谱密度和 RGB 投影沿用源标定。
拟合 RGB 矩阵含负系数；四波长不是全光谱解，RGB T 为太阳谱加权近似。

固定 balanced 网格为 40 heights ×160 suns ×20 phases ×12 cones；方向求积数 1536，
路径 64 步，8 轮，4 高度一批。optical 为 128×512、256 步；SkyView 256²。
所有场在 GPU 初始化时求解，源表只作为只读物理输入。

从仓库根目录导出（源仓库只读，要求上述精确版本）：

先按[本机配置](../../../manual/local.md)设置 `ENGINE_ATMOSPHERE_SOURCE` 并加载当前会话。

```powershell
python crates/atmosphere/data/export.py $env:ENGINE_ATMOSPHERE_SOURCE
```

工具复制源到 `temp/atmosphere/20261027/source-export`，独立使用 `target/atmosphere-export`，
执行原 `Medium::new`，不创建 GPU。不会写入旧仓库。需要 little-endian 主机和源匹配的 Rust/Cargo。

独立 WGSL GPU 参考的重新生成命令：

```powershell
python crates/atmosphere/data/export.py $env:ENGINE_ATMOSPHERE_SOURCE --reference temp/atmosphere/20261027/reference-dx12
```

此模式需要先协调 GPU 独占。只在测试工具使用源 wgpu 29/WGSL，生产仍为 Slang 直接 SPIR-V。
默认 oracle 选 DX12：源 WGSL→Vulkan 的 Naga 产物在当前验证层触发函数局部类型布局 VUID-10684，
不能作为验证通过；失败现场保留在 `temp/atmosphere/20261027/reference.log`。
已冻结参考由 RTX 4090、DX12 驱动 32.0.16.1656 生成，验证开启；原始场及日志放在 temp。
正式样本只保留在 `tests/fixtures/reference.json`，含每场的固定点、4 组观察者和 10 组光谱传输。
oracle 仅关闭 `skip_unused_ground` 的视锥裁图优化，以便读取全部三个 chart，不改变输运或求解预算。
跨后端比较使用数值容差，不能要求所有 GPU 的 f32 超越函数、累加和归一化系数逐位相同。

## 来源

移植自同作者 Sky Tracer `9e9430d52c50aa3df97fdf0fb6515abc67f44bef` 的 `crates/sky-realtime`。

GPU 数学来自源 `solver.wgsl`、`transport.wgsl`、`sky_view.wgsl`；物理介质来自源
`model.rs`、`physics/` 与 `data/`，坐标来自最新 `mapping.rs` 和 `configs/mapping_fit.json`。
保留四波长输运、8 轮 successive scattering、表格相函数质量校正和高空精确 Rayleigh
二次矩。移植改变着色器语言、显式资源绑定、矩阵存储声明和资源所有者，未改用离线散射场。

有限距离 `integrate_at` 同样来自该源。32³ 透视体积、z² 距离映射及首层与原点间的
L/T 插值用于近端片层的连续过渡。
当前 Aerial 的 L 只包含介质散射，显式排除末端理想行星地表辐射，防止与场景表面重复相加；
MS 求解、SkyView 和独立传输 oracle 保留源地表边界条件。

`tests/fixtures/reference.json` 来自原 Sky Tracer WGSL 的独立实际 GPU 求解；
生成脚手架 `reference.rs` 仅进入临时源副本，不进入生产依赖或构建。
