# Streamline RR

可选 `vulkan/dlss_rr` Windows 接入。API、图像与完成契约见
[`vulkan::streamline`](../target/doc/vulkan/streamline/index.html)，C ABI 见 [bridge.h](../crates/vulkan/streamline/bridge.h)。
宿主显式提供应用身份；不包含场景策略或着色器。

SDK 锁定 Streamline 2.14.1、DLSS 310.9.1、Vulkan-Headers 1.4.350，来源与 SHA-256
在 [sdk-lock.json](../crates/vulkan/streamline/sdk-lock.json)。SDK 默认安装到 `local/tools/streamline/` 下的锁定版本目录，换终端和执行 `cargo clean` 后仍可使用。
安装器只从锁定官方归档提取指定文件，校验文件与 NVIDIA DLL 签名；构建不下载。

首次从仓库根目录的 PowerShell 运行 `./crates/vulkan/streamline/setup.ps1`。看到 SDK 校验成功和安装目录后，执行所需检查：

```powershell
cargo check --locked -p simple_engine --no-default-features --features dlss_rr
cargo test --locked -p vulkan --features dlss_rr --lib
./crates/vulkan/streamline/test_bridge.ps1
./crates/vulkan/streamline/test_gpu.ps1 -InitializationOnly
./crates/vulkan/streamline/test_gpu.ps1
./crates/vulkan/streamline/test_gpu.ps1 -Manual
```

Cargo 与测试自动发现默认安装。自定义目录可用绝对路径 `STREAMLINE_SDK` 覆盖，安装器和 fixture 也接受 `-SdkRoot`。排查 SDK 时可运行 `./crates/vulkan/streamline/setup.ps1 -VerifyOnly`。构建校验 SDK，并部署四个 runtime DLL 和许可证到 Cargo profile、deps、OUT_DIR。
发布宿主仍须保留其同目录 runtime 与许可，不可仅复制自身 DLL。early interposer 需在
Vulkan 创建前 bootstrap，且宿主从该目录的 interposer 加载 Vulkan；不使用 OTA。
未启用 feature 时不读取 SDK，非 Windows 不编译此模块。

GPU fixture 为独立进程，直接编译生产 bridge，使用 DLAA/preset F、显式 specular motion，
执行 evaluate、提交、等待、读回与释放；日志默认放 `temp/streamline-test`。
默认严格同步验证会把每个 validation error 记为失败，不过滤 SDK 私有资源。
`-StandardValidationOnly` 只运行普通验证；`-ReportOnlyValidation` 允许收集全部同步错误，
仍以非零状态报告失败。读回有限、非零不是画质验收。

本机在 validation 1.4.363 同步验证下，Streamline 2.14.1 / NGX 310.9.1 与
2.14.16 / NGX 310.9.16 均复现 SDK 私有图像的 `WRITE_AFTER_WRITE`，
与上游 [issue #126](https://github.com/NVIDIA-RTX/Streamline/issues/126) 同型；升级未消除本机现象，
不代表所有环境。首次复测先运行 `./scripts/validation.ps1 -Install` 固定 layer，再运行上述 fixture；
保留原始错误日志，不将关闭同步验证视为修复。
