# simple_engine

通用 2D/3D 算法与渲染基础。可分发的接入示例位于 `examples/`，本地应用可作为独立 Cargo workspace 放在 `app/`。

应用可通过 `simple_engine` 入口按 feature 选择组件，也可直接依赖底层 crate；默认不启用功能。[操作手册](manual/README.md)记录构建、运行与维护；接口和契约由代码生成 [API 文档](target/doc/simple_engine/index.html)。

普通渲染与呈现使用 wgpu；native PT 可复用同一个 Vulkan device，也可借用外部宿主设备。`vulkan` crate 隔离设备接入、资源、SPIR-V 直通和原生命令的 `unsafe`，宿主负责提交与完成。

## 工具链

- Rust `nightly-2026-10-31`，由 `rust-toolchain.toml` 声明；HashDag、最少矩形分解、SipHash 与噪声使用 `portable_simd`。
- 默认入口与 CPU 组件只需要 Rust，不依赖 wgpu、Vulkan 或 Slang。Windows 渲染另需 MSVC C++ 构建工具、Vulkan 1.4 loader 和驱动；当前已在 Windows 实际验证，其他 native 平台待验证。
- 光追示例另需 Vulkan 1.4 与 `VK_KHR_device_address_commands`；场景所需 KHR OMM 与可选 EXT SER 等能力见 [native 手册](manual/native.md)。普通窗口不要求光追能力。
- Slang 基线为 `2026.20`；构建时按 `SLANGC`、`VULKAN_SDK`、`PATH` 顺序查找，允许其它版本并记录实际身份。安装及检查见 [开发脚本](scripts/README.md)。
- 格式化：rustfmt、LLVM clang-format、Git Bash；安装方式见 [格式钩子](scripts/README.md)。

Windows x86-64 默认使用 v3；原生示例默认使用 mimalloc。System 分配器和 baseline CPU 对照方式见 [native 手册](manual/native.md)。

```sh
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Suite cpu
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Suite features
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Suite docs
cargo run -p hello_triangle
cargo run --release -p cornell_box
cargo test -p wgpu_vulkan_tests --test interop -- --ignored --test-threads=1
cargo check -p simple_engine --features webgpu,render2d_gpu --target wasm32-unknown-unknown
```

离屏参考渲染与场景约定见 [Cornell box](examples/cornell_box/README.md)。

窗口入门见 [hello_triangle](examples/hello_triangle/README.md)。原生与 Web 宿主可复用同一 GUI 应用模型。GUI 所有权与输入契约见 [文档](target/doc/ui/index.html)。

`frame_loop` 提供时钟与交接工具，应用可按需组合自己的循环与调度。

[`chunk_manager`](target/doc/chunk_manager/index.html) 提供预算驱动的 3D LOD 区块管理，按距离调度异步更新；状态持有资源，规划不依赖应用或 GPU 后端。

可复用 Z Sobol、RR、BSDF、MIS、光源 BVH 采样及射线数值函数通过 `shader_library` feature 提供源码入口，采用 Slang 2026 模块，见 [共享 Slang 模块](target/doc/simple_engine/shader_library/index.html)。CPU quad 光源树通过 [`light_bvh`](target/doc/light_bvh/index.html) feature 提供，无应用或 GPU 依赖。

[`camera`](target/doc/camera/index.html) 提供 pinhole 相机、像素中心与 Halton jitter、f64 锚点转换和无 jitter 矩阵；CPU 与 `geometry.camera` 共享左上原点、正 view-Z 和输入／输出尺寸约定。

`dlss_rr` 提供 Windows [`vulkan::streamline`](target/doc/vulkan/streamline/index.html) 接入；引擎默认不启用，Windows 启用时须安装锁定 SDK。应用提供项目身份、图像和相机历史，`WgpuContext` 维护 SDK、设备与提交完成边界；外部宿主也可借入设备。安装与无窗口验证见 [native 手册](manual/native.md)。

[`pt`](target/doc/pt/index.html) 提供通用路径预算和绝对样本事务，`pt_gpu` 增加 K1/K2/K3 原生录制器；`shaders/pt` 共享 OpenPBR 输运、NEE/MIS/RR、采样和累积。场景适配器负责命中重建、覆盖率及材质翻译、OMM、光源和大气策略。录制器保活资源，提交、等待和呈现由接入层负责。

[`restir_pt`](target/doc/restir_pt/index.html) 提供 ReSTIR PT Enhanced 默认 point profile 的工作集、历史事务和十阶段调度；`restir_pt_gpu` 增加 Vulkan 录制。来源与许可见[通知](shaders/restir_pt/NOTICE.md)。

[`scene_cache`](target/doc/scene_cache/index.html) 提供两个场景适配器共用的资源目录、属性槽、稳定身份和增量发布事务；几何捕获、LOD与GPU提交留在各自所有者。材质资源扫描与控制流推导见 [`openpbr::resource`](target/doc/openpbr/resource/index.html)，运行时 callable 内容缓存由该 crate 的可选 `program` feature 接入；固定资源仍可使用同一描述符生成并审计预编译产物。

primeDRT 参数与自动曝光入口由可选 [`display`](target/doc/simple_engine/display/index.html) feature 提供；叠加 `vulkan` 可录制 GPU 直方图与时域适应。共享 Slang 模块位于 `display.*`。

[`openpbr`](target/doc/openpbr/index.html) 提供 OpenPBR 的 opaque／metallic、solid／thin dielectric 与薄层 SSS 子集；`openpbr_gpu` 增加原作者能量纹理。[`atmosphere`](target/doc/atmosphere/index.html) 提供 sky_tracer 四波长参数，`atmosphere_gpu`／`atmosphere_native` 分别提供 wgpu／native 的多重散射、SkyView 与 Aerial 资源所有者。物理模型与缓存判定共享，应用维护太阳、相机和场景策略。

GPU 单元/属性测试与性能分析使用引擎 [测试设施](manual/gpu-test.md)，GPU 用例默认 `ignore`。

CI 与本地共用 [检查入口](scripts/README.md)，普通 CI 不运行 GPU。CPU 性能分析使用 [Criterion 基准](manual/cpu-bench.md)。

矩形划分提供最少数量内核；体素建网直接处理分层稀疏输入。接口见 [`rectangle_decomposition`](target/doc/rectangle_decomposition/index.html) 和 [`voxel_mesh::sparse`](target/doc/voxel_mesh/sparse/index.html)，性能入口见 [CPU 基准](manual/cpu-bench.md)。

wgpu / ash 的资源交接契约见 [互操作 API](target/doc/vulkan/interop/index.html)，离屏与窗口回归见 [集成测试](tests/README.md)。

公开 API 要求完整 rustdoc 覆盖，检查同时验证链接、示例与 native/wasm 文档。`temp/` 保存不可再生的本地临时材料，人工清理；`target/` 仅存可再生产物，可无条件清理，两者均不纳入版本管理。

## 许可证

Copyright (c) 2026 linlin。项目自有内容采用 [GPL-3.0-only](LICENSE)，范围见 [NOTICE](NOTICE)。[第三方代码与资产](THIRD_PARTY_NOTICES.md)保留各自许可和版权，不由本项目重新授权。
