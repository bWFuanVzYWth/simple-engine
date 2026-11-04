# Streamline RR

可选 `vulkan/dlss_rr` Windows 接入。API、图像与完成契约见
[`vulkan::streamline`](../target/doc/vulkan/streamline/index.html)，C ABI 见 [bridge.h](../crates/vulkan/streamline/bridge.h)。
宿主显式提供应用身份；不包含场景策略或着色器。

SDK 锁定 Streamline 2.14.1、DLSS 310.9.1、Vulkan-Headers 1.4.350，来源与 SHA-256
在 [sdk-lock.json](../crates/vulkan/streamline/sdk-lock.json)。SDK 不纳入源码，默认安装到可再生的 `target/streamline-sdk`。
安装器只从锁定官方归档提取指定文件，校验文件与 NVIDIA DLL 签名；构建不下载。

从仓库根目录运行：

```powershell
.\scripts\install_streamline.ps1
$env:STREAMLINE_SDK = (Resolve-Path target/streamline-sdk).Path
cargo check --locked -p simple_engine --no-default-features --features dlss_rr
cargo test --locked -p vulkan --features dlss_rr --lib
.\scripts\gpu\test_streamline.ps1 -InitializationOnly
.\scripts\gpu\test_streamline.ps1
.\scripts\gpu\test_streamline.ps1 -Manual
```

自定义 SDK 缓存用 `-SdkRoot`；仅校验用 `-VerifyOnly`。构建从显式 `STREAMLINE_SDK`
读取 SDK 并再次校验，部署四个 runtime DLL 和许可证到 Cargo profile、deps、OUT_DIR。
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
不代表所有环境。复测先运行 `scripts/install_validation.ps1` 固定 layer，再运行上述 fixture；
保留原始错误日志，不将关闭同步验证视为修复。
