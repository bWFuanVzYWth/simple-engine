# native 设备与 validation

Windows x86-64 默认以 `x86-64-v3` 构建。原生示例默认使用 mimalloc，分配器由各可执行程序持有；库和 wasm 不安装全局分配器。原生示例的 System 分配器对照使用 `--no-default-features`。

mimalloc 的 native 构建需要 C++17 工具链，Windows 使用 MSVC C++ 构建工具。v3 程序要求支持该指令集的 CPU；需要较低 CPU 基线时使用下述显式覆盖。

比较 baseline CPU 目标时，显式以 `RUSTFLAGS=-C target-cpu=x86-64` 覆盖仓库配置；构建结束后再串行采样，保持分配器、worker、batch、输入和 profile 相同。

native 运行需要 Vulkan 1.4 loader 与 Vulkan 驱动；普通 GUI/compute 不要求硬件光追。native RT 使用 Vulkan 1.4、`VK_KHR_device_address_commands`、KHR acceleration structure、ray tracing pipeline 与 RayQuery。设备启用 `VK_KHR_opacity_micromap` 且满足当前精确 2-state 表的细分要求时使用 OMM，否则使用独立的 shader cutout 特化。SER 使用可选的 `VK_EXT_ray_tracing_invocation_reorder`，支持时默认启用。设备选择和实际请求能力见 [`WgpuContext`](../target/doc/vulkan/interop/struct.WgpuContext.html)，共享资源与提交规则见[互操作 API](../target/doc/vulkan/interop/index.html)。

ReSTIR workload 显式请求 32-lane subgroup，设备默认 64 也可使用；需启用 compute 规模控制、可请求范围包含 32，并支持 BASIC/BALLOT/ARITHMETIC。融合配置的实际要求见 [`restir_pt::gpu::Renderer`](../target/doc/restir_pt/gpu/struct.Renderer.html#method.new)。

外部宿主设备通过 [`native::Context`](../target/doc/vulkan/native/struct.Context.html) 借入；资源、能力和完成租约契约见 [native API](../target/doc/vulkan/native/index.html)。通用 PT 的三阶段录制入口见 [`pt::gpu::Renderer`](../target/doc/pt/gpu/struct.Renderer.html)。

Windows 上启用 DLSS RR 或执行全 feature 文档检查前，按 [Streamline 手册](streamline.md) 安装并在当前进程设置 SDK。普通 Vulkan 和 CPU 构建可直接运行。

Debug 默认启用 validation，Release 默认关闭。诊断 release 程序时显式开启：

```powershell
$env:WGPU_DEBUG = "1"
$env:WGPU_VALIDATION = "1"
$env:VK_LAYER_VALIDATE_SYNC = "1"
cargo run --release -p hello_triangle -- --frames 120
```

KHR OMM 需要能识别新接口的 validation。Windows x86_64 可在同一 PowerShell 会话执行 `./scripts/install_validation.ps1`：校验[官方 SDK 1.4.363.0](https://vulkan.lunarg.com/sdk/home)的 SHA256，仅提取 validation DLL 与 manifest 到 `target/tools`，设置当前进程 `VK_LAYER_PATH`；需安装 7-Zip，不执行 SDK 安装或改注册表。固定版本、内嵌段及文件 SHA256 见 `scripts/tools.json`。其它平台自行准备新版 `VK_LAYER_KHRONOS_validation`。

诊断结束后恢复原环境变量；性能比较应固定相同验证设置。脚本 `compute` / `rt` 检查档会临时开启验证并恢复环境：

```powershell
powershell -NoProfile -File scripts/check.ps1 -Suite compute
powershell -NoProfile -File scripts/check.ps1 -Suite rt
```

初始化失败时先确认使用 Vulkan 后端、适配器索引与所需 feature，再查看日志和设备/驱动报告。普通 wgpu 管线的 SPIR-V binding、入口及工作组问题从 [`WgpuContext::shader`](../target/doc/vulkan/interop/struct.WgpuContext.html#method.shader) 对照产物；原生 RT 的资源交接从 [`RayTracingPipeline`](../target/doc/vulkan/native/struct.RayTracingPipeline.html) 对照。

交接数据、复用、保活、并发提交与窗口 resize 的定向回归见[互操作集成测试](../tests/wgpu_vulkan/README.md)。GPU hang 的外层进程处理见[超时运行器](gpu-runner.md)。
